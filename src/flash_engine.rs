//! Flashing engine for the "Firmware Flash" feature (Mode 2).
//!
//! Two backends can run a [`FlashPackage`]:
//!
//! * **External `mfastboot`** — Motorola's `fastboot` fork, looked up next to
//!   the application (see [`discover_mfastboot`]). Preferred, because its
//!   behaviour matches the firmware packages exactly. The builds we ship are
//!   Intel binaries, so an ARM machine runs them through an emulator —
//!   Rosetta 2 on macOS, box64 on Linux — and when that is not installed yet
//!   the user is told to install it (see [`Mfastboot::emulator`]).
//! * **Built-in fastboot** — the pure-Rust `fastboot` crate used by the other
//!   features. Used when no `mfastboot` binary is shipped for the platform.
//!
//! Both backends run the package's steps in order and report through a single
//! callback, which the UI turns into messages: step transitions, byte
//! progress, and the tool's own output as log lines.

use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::flashfile::{self, FlashOp, FlashPackage};

/// An external `mfastboot` executable found next to the application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mfastboot {
    /// Label taken from the containing directory (e.g. `34.0.4`), or `local`
    /// for a binary placed directly next to the application.
    pub version: String,
    pub path: PathBuf,
    /// What starts this build on this machine. The shipped builds are Intel,
    /// so an ARM machine runs them through an emulator, which may still have
    /// to be installed (see [`Emulator::hint_ids`]).
    pub emulator: Emulator,
}

impl Mfastboot {
    /// The label shown in the engine picker.
    pub fn label(&self) -> String {
        format!("mfastboot {}", self.version)
    }
}

/// Which backend runs the steps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Engine {
    /// The built-in `fastboot` crate.
    Builtin,
    /// An external `mfastboot` executable.
    Mfastboot(Mfastboot),
}

/// Events emitted while a package is being loaded (ZIP extraction + parsing).
#[derive(Debug, Clone)]
pub enum PackageEvent {
    /// Extraction progress, in bytes.
    Extract { done: u64, total: u64 },
    Log(String),
    /// The package is ready to be flashed.
    Ready(Box<FlashPackage>),
    Failed(String),
}

/// Events emitted while a package is being flashed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlashEvent {
    /// A step is about to run (`index` is 0-based).
    StepStarted {
        index: usize,
        total: usize,
        label: String,
    },
    /// Byte progress inside the current step.
    Progress { done: u64, total: u64 },
    /// A line of tool output (or an engine note).
    Log(String),
    /// The whole package finished (or was aborted at the first failure).
    Finished(Result<(), String>),
}

/// Everything needed to flash a package onto one device.
pub struct FlashJob {
    pub package: FlashPackage,
    /// `erase` commands added on top of the package (the "Erase Userdata" /
    /// "Erase NV cache" buttons). The device may not have the partition at
    /// all, so a failure is logged instead of aborting the flash.
    pub extra_erases: Vec<String>,
    pub engine: Engine,
    pub serial: String,
    /// Verify the `MD5` attribute of every `flash` step before writing it.
    pub verify_checksums: bool,
}

impl FlashJob {
    /// How many steps the run has in total, extra erases included — this is
    /// the number the progress line counts up to.
    fn total_steps(&self) -> usize {
        self.package.steps.len() + self.extra_erases.len()
    }
}

/// The executable name to look for, including the platform extension.
const MFASTBOOT_FILE: &str = if cfg!(windows) {
    "mfastboot.exe"
} else {
    "mfastboot"
};

/// Looks for `mfastboot` builds shipped next to the application.
///
/// The supported layouts are
///
/// * `<exe dir>/mfastboot/<version>/mfastboot[.exe]`, and
/// * `<exe dir>/mfastboot/<platform>/<version>/mfastboot[.exe]`, where
///   `<platform>` names the OS and architecture the binary was built for
///   (`windows_amd64`, `darwin_amd64`, `linux_amd64`, …) — mirroring the
///   `prebuilt_binary` tree. A platform this machine cannot use is skipped,
///   so an Intel `mfastboot` is never started on ARM Windows: it is only
///   offered where an emulator runs it (Rosetta 2, box64).
///
/// Inside a macOS `.app` bundle, `Contents/Resources` is searched as well.
/// The result is sorted newest version first.
pub fn discover_mfastboot() -> Vec<Mfastboot> {
    let mut found: Vec<Mfastboot> = Vec::new();

    let Ok(executable) = std::env::current_exe() else {
        return found;
    };
    let Some(executable_dir) = executable.parent() else {
        return found;
    };

    // `Contents/MacOS/lmnflash` -> `Contents/Resources`.
    let mut roots = vec![executable_dir.to_path_buf()];
    if let Some(contents) = executable_dir.parent() {
        roots.push(contents.join("Resources"));
    }

    let emulator = intel_emulator();
    let mut seen = std::collections::HashSet::new();

    for root in roots {
        let mut candidates: Vec<Mfastboot> = Vec::new();
        let base = root.join("mfastboot");

        if let Ok(entries) = std::fs::read_dir(&base) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }

                // `<base>/<version>/mfastboot[.exe]`
                let version = entry.file_name().to_string_lossy().to_string();
                let binary = path.join(MFASTBOOT_FILE);
                if binary.is_file() {
                    candidates.push(Mfastboot {
                        version,
                        path: binary,
                        // No `<platform>` folder: whoever laid it out knows
                        // what the binary is for, so it is taken at face
                        // value.
                        emulator: Emulator::Native,
                    });
                    continue;
                }

                // `<base>/<platform>/<version>/mfastboot[.exe]`
                let Some(build_emulator) =
                    platform_fit(&version, std::env::consts::ARCH, emulator.as_ref())
                else {
                    continue;
                };

                if let Ok(versions) = std::fs::read_dir(&path) {
                    for version_entry in versions.flatten() {
                        let binary = version_entry.path().join(MFASTBOOT_FILE);
                        if binary.is_file() {
                            candidates.push(Mfastboot {
                                version: version_entry
                                    .file_name()
                                    .to_string_lossy()
                                    .to_string(),
                                path: binary,
                                emulator: build_emulator.clone(),
                            });
                        }
                    }
                }
            }
        }

        // `<root>/mfastboot[.exe]`
        let direct = root.join(MFASTBOOT_FILE);
        if direct.is_file() {
            candidates.push(Mfastboot {
                version: "local".to_owned(),
                path: direct,
                emulator: Emulator::Native,
            });
        }

        for tool in candidates {
            if seen.insert(tool.version.clone()) {
                found.push(tool);
            }
        }
    }

    found.sort_by(|a, b| compare_versions(&b.version, &a.version));

    found
}

/// How a `<platform>` directory (e.g. `windows_amd64`, `darwin_arm64`) fits
/// this machine, or `None` when its binaries cannot run here at all.
///
/// `architecture` is what this process runs as (`std::env::consts::ARCH`) and
/// `emulator` what this machine runs Intel binaries with (see
/// [`intel_emulator`]). The name has to mention this operating system, and the
/// architecture either ours — `Emulator::Native` — or one the emulator can
/// take over. The aliases are checked most specific first, so `arm64` is not
/// mistaken for `arm` and `x86_64` not for `x86`; a name that mentions no
/// architecture at all is accepted: whoever laid the directory out knows what
/// was put in it.
fn platform_fit(
    platform: &str,
    architecture: &str,
    emulator: Option<&Emulator>,
) -> Option<Emulator> {
    /// `(alias, architecture it denotes)`, longest aliases first.
    const ARCH_ALIASES: &[(&str, &str)] = &[
        ("x86_64", "x86_64"),
        ("amd64", "x86_64"),
        ("x64", "x86_64"),
        ("aarch64", "aarch64"),
        ("arm64", "aarch64"),
        ("i686", "x86"),
        ("i386", "x86"),
        ("ia32", "x86"),
        ("x86", "x86"),
        ("arm", "arm"),
        ("riscv64", "riscv64"),
    ];

    /// `(alias, operating system it denotes)`. `darwin` and `macos` are
    /// checked before `win`, which `darwin` contains.
    const OS_ALIASES: &[(&str, &str)] = &[
        ("darwin", "macos"),
        ("macos", "macos"),
        ("osx", "macos"),
        ("windows", "windows"),
        ("win", "windows"),
        ("linux", "linux"),
        ("freebsd", "freebsd"),
        ("android", "android"),
    ];

    let platform = platform.to_ascii_lowercase();

    // A directory that names another operating system holds binaries this
    // machine cannot execute — Rosetta 2 does not make a Linux `mfastboot`
    // run on macOS, and box64 does not make a Windows one run on Linux.
    let named_os = OS_ALIASES
        .iter()
        .find_map(|(alias, denotes)| platform.contains(alias).then_some(*denotes));

    if named_os.is_some_and(|named_os| named_os != std::env::consts::OS) {
        return None;
    }

    let named_arch = ARCH_ALIASES
        .iter()
        .find_map(|(alias, denotes)| platform.contains(alias).then_some(*denotes));

    let Some(named_arch) = named_arch else {
        return Some(Emulator::Native);
    };

    if named_arch == architecture {
        return Some(Emulator::Native);
    }

    // An Intel binary on an ARM machine: only the emulator can run it (and
    // without one it is not offered at all).
    if named_arch == "x86_64" {
        return emulator.cloned();
    }

    None
}

/// What starts an `mfastboot` build on this machine.
///
/// The shipped builds are Intel binaries, so an ARM machine runs them through
/// an emulator: Rosetta 2 on macOS, box64 on Linux.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Emulator {
    /// The build runs as it is.
    Native,
    /// Rosetta 2 on Apple silicon. macOS puts itself in front of the binary,
    /// so only the installation matters: `installed` is false while Rosetta 2
    /// is still missing, and the dialog says how to install it (macOS 11
    /// through 27 offer it).
    Rosetta { installed: bool },
    /// box64 on ARM Linux, which has to start the binary itself
    /// (`box64 mfastboot …`). `program` is the box64 found on this machine,
    /// or `None` while it is not installed — then the dialog says how to get
    /// it.
    Box64 { program: Option<PathBuf> },
}

impl Emulator {
    /// The two localized lines that tell the user to install the emulator, or
    /// `None` when the build can start right now.
    ///
    /// A hint also means the tool cannot run: the dialog keeps Start disabled
    /// instead of letting the first step fail with a bad CPU type.
    pub fn hint_ids(&self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::Native | Self::Rosetta { installed: true } => None,
            Self::Rosetta { installed: false } => Some((
                "firmware-flash-rosetta-needed",
                "firmware-flash-rosetta-install",
            )),
            Self::Box64 { program: Some(_) } => None,
            Self::Box64 { program: None } => {
                Some(("firmware-flash-box64-needed", "firmware-flash-box64-install"))
            }
        }
    }

    /// The command that runs `binary`, through the emulator when it needs one.
    fn command(&self, binary: &Path) -> Command {
        match self {
            Self::Box64 {
                program: Some(program),
            } => {
                let mut command = Command::new(program);
                command.arg(binary);
                command
            }
            // Rosetta 2 is applied by macOS itself, so the binary is started
            // directly there (and a missing emulator is not started at all:
            // the UI does not let a flash begin).
            _ => Command::new(binary),
        }
    }
}

/// The emulator this machine runs Intel binaries with: Rosetta 2 on Apple
/// silicon, box64 on ARM Linux.
///
/// `None` when there is nothing to emulate with — either because Intel
/// binaries run as they are (an Intel machine) or because they cannot run here
/// at all (ARM Windows, 32-bit ARM Linux, and macOS 28, which dropped
/// Rosetta 2).
fn intel_emulator() -> Option<Emulator> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        if rosetta_runtime_present() {
            return Some(Emulator::Rosetta { installed: true });
        }

        // Apple ships Rosetta 2 for the Apple silicon releases 11 through 27.
        // An unreadable version is treated as one that still has it: a hint
        // the user may not need is harmless, a disabled engine is not.
        match macos_major_version() {
            Some(major) if major > ROSETTA_LAST_MACOS => None,
            _ => Some(Emulator::Rosetta { installed: false }),
        }
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some(Emulator::Box64 {
            program: find_box64(),
        })
    } else {
        None
    }
}

/// The last macOS release Apple ships Rosetta 2 for: it is available on Apple
/// silicon from macOS 11 (the first release for those Macs) through macOS 27.
const ROSETTA_LAST_MACOS: u32 = 27;

/// Whether the Rosetta 2 runtime is installed. Homebrew detects it the same
/// way, by the path the runtime is installed to.
fn rosetta_runtime_present() -> bool {
    Path::new("/Library/Apple/usr/libexec/oah/libRosettaRuntime").exists()
}

/// The major version of the running macOS (`11` = Big Sur, `26` = Tahoe …),
/// read from `sw_vers`.
fn macos_major_version() -> Option<u32> {
    if !cfg!(target_os = "macos") {
        return None;
    }

    let output = Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output()
        .ok()?;

    parse_macos_major(&String::from_utf8(output.stdout).ok()?)
}

/// The leading component of a `sw_vers -productVersion` string (`"26.0"` ->
/// `26`), or `None` when it is not a version.
fn parse_macos_major(version: &str) -> Option<u32> {
    version.trim().split('.').next()?.parse().ok()
}

/// Looks for `box64`, which runs the Intel `mfastboot` on ARM Linux.
///
/// The `PATH` is searched first, then the directories box64 is usually
/// installed into — a window started from a desktop session does not always
/// inherit a login shell's `PATH`.
fn find_box64() -> Option<PathBuf> {
    const NAME: &str = "box64";
    const DIRECTORIES: &[&str] = &["/usr/local/bin", "/usr/bin", "/usr/local/sbin"];

    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            let candidate = directory.join(NAME);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    DIRECTORIES
        .iter()
        .map(|directory| Path::new(directory).join(NAME))
        .find(|candidate| candidate.is_file())
}


/// Compares dotted version labels numerically, so `34.0.4` sorts after
/// `29.0.6` (a plain string compare would not).
fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    fn key(version: &str) -> Vec<u64> {
        version
            .split('.')
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect()
    }

    key(a).cmp(&key(b))
}

/// Loads a firmware package (ZIP archive or `flashfile.xml`) and reports the
/// result through `emit`.
///
/// Runs on a worker thread; the ZIP of a factory firmware is several
/// gigabytes, so extraction is streamed and reported as it goes.
pub fn load_package(path: &Path, emit: &(dyn Fn(PackageEvent) + Send + Sync)) {
    emit(PackageEvent::Log(format!("loading {}", path.display())));

    // Extraction reports every megabyte; the UI only needs coarse updates.
    let mut reported = 0u64;
    let mut on_progress = |done: u64, total: u64| {
        if done == total || done / (8 << 20) != reported / (8 << 20) {
            reported = done;
            emit(PackageEvent::Extract { done, total });
        }
    };

    match flashfile::load(path, &mut on_progress) {
        Ok(package) => {
            emit(PackageEvent::Log(format!(
                "{}: {} steps",
                package
                    .flashfile
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_default(),
                package.steps.len()
            )));

            if !package.ignored_partitions.is_empty() {
                emit(PackageEvent::Log(format!(
                    "ignored partitions: {}",
                    package.ignored_partitions.join(", ")
                )));
            }

            emit(PackageEvent::Ready(Box::new(package)));
        }
        Err(error) => emit(PackageEvent::Failed(error)),
    }
}

/// Runs every step of the package, emitting [`FlashEvent`]s as it goes.
pub fn run(job: FlashJob, emit: &(dyn Fn(FlashEvent) + Send + Sync)) {
    let engine = job.engine.clone();
    let result = match &engine {
        Engine::Mfastboot(tool) => run_with_mfastboot(&job, tool, emit),
        Engine::Builtin => run_with_builtin(&job, emit),
    };

    emit(FlashEvent::Finished(result));
}

/// What the phone should boot into when a reboot is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebootMode {
    /// Leave fastboot for Android. The boot mode flag the firmware sets
    /// (`oem config bootmode fastboot`) is cleared first, otherwise the phone
    /// boots straight back into fastboot.
    System,
    /// `reboot-bootloader`: back into the bootloader (fastboot).
    Bootloader,
    /// `reboot recovery`: the recovery menu.
    Recovery,
    /// `reboot fastboot`: userspace fastboot (`fastbootd`).
    Fastbootd,
    /// Write the sideload payload to `misc` and reboot: the phone comes up in
    /// recovery's "apply update from ADB" mode.
    Sideload,
}

/// Reboots the phone, into `mode`, through the engine the dialog is set to.
///
/// A GUI flash has no command line of its own, so the built-in engine is used
/// for this — unless an external `mfastboot` is selected, which then runs the
/// same commands.
pub fn reboot(
    engine: &Engine,
    serial: &str,
    mode: RebootMode,
    emit: &(dyn Fn(FlashEvent) + Send + Sync),
) -> Result<(), String> {
    match engine {
        Engine::Builtin => reboot_builtin(serial, mode, emit),
        Engine::Mfastboot(tool) => reboot_with_mfastboot(tool, serial, mode, emit),
    }
}

/// Runs one reboot through the built-in `fastboot` crate.
fn reboot_builtin(
    serial: &str,
    mode: RebootMode,
    emit: &(dyn Fn(FlashEvent) + Send + Sync),
) -> Result<(), String> {
    emit(FlashEvent::Log(format!(
        "connecting to {} (built-in fastboot)",
        if serial.is_empty() { "device" } else { serial }
    )));

    let mut device = fastboot::FastbootDevice::connect(serial)?;
    // Show what the bootloader answers, like during a flash.
    device.set_packet_logger(Some(Box::new(|line| emit(FlashEvent::Log(line)))));

    match mode {
        RebootMode::System => {
            emit(FlashEvent::Log("oem fb_mode_clear".to_string()));
            device.oem("fb_mode_clear")?;

            emit(FlashEvent::Log("reboot".to_string()));
            device.send_reboot(None)
        }
        RebootMode::Bootloader => {
            emit(FlashEvent::Log("reboot-bootloader".to_string()));
            device.send_reboot(Some("bootloader"))
        }
        RebootMode::Recovery => {
            emit(FlashEvent::Log("reboot-recovery".to_string()));
            device.send_reboot(Some("recovery"))
        }
        RebootMode::Fastbootd => {
            emit(FlashEvent::Log("reboot-fastboot".to_string()));
            device.send_reboot(Some("fastboot"))
        }
        RebootMode::Sideload => {
            let payload = misc_sideload_bcb();

            emit(FlashEvent::Log(format!(
                "flash misc ({} bytes, ADB sideload)",
                payload.len()
            )));
            device.flash("misc", &payload, None)?;

            emit(FlashEvent::Log("reboot".to_string()));
            device.send_reboot(None)
        }
    }
    // A reboot is a send-and-forget command: nothing is read back, so the only
    // failure left is the write itself (the phone may already be gone).
    .or_else(|error| {
        emit(FlashEvent::Log(format!("reboot: {error}")));
        Ok(())
    })
}

/// Runs one reboot through the external `mfastboot`.
fn reboot_with_mfastboot(
    tool: &Mfastboot,
    serial: &str,
    mode: RebootMode,
    emit: &(dyn Fn(FlashEvent) + Send + Sync),
) -> Result<(), String> {
    let directory = std::env::temp_dir();

    // The sideload payload is written next to the tool's working directory,
    // because `mfastboot flash` takes a file.
    let sideload_path = directory.join("lmnflash-misc-sideload.img");

    let (setup, restart): (Option<Vec<&str>>, Vec<&str>) = match mode {
        RebootMode::System => (
            // Clearing the flag first keeps the phone from booting back into
            // fastboot, and it has to succeed to be worth anything.
            Some(vec!["oem", "fb_mode_clear"]),
            vec!["reboot"],
        ),
        RebootMode::Bootloader => (None, vec!["reboot-bootloader"]),
        RebootMode::Recovery => (None, vec!["reboot", "recovery"]),
        RebootMode::Fastbootd => (None, vec!["reboot", "fastboot"]),
        RebootMode::Sideload => {
            let payload = misc_sideload_bcb();
            std::fs::write(&sideload_path, payload)
                .map_err(|error| format!("could not write {}: {error}", sideload_path.display()))?;

            (Some(vec!["flash", "misc", sideload_path.to_str().unwrap_or_default()]), vec!["reboot"])
        }
    };

    if let Some(args) = setup {
        emit(FlashEvent::Log(tool_command_line(tool, serial, &args)));

        let command = tool_command(tool, serial, &args);
        let status = run_tool_command(command, &directory, emit)?;

        if !status.success() {
            return Err(format!(
                "{} failed for {}",
                tool.label(),
                args.join(" ")
            ));
        }
    }

    emit(FlashEvent::Log(tool_command_line(tool, serial, &restart)));

    let command = tool_command(tool, serial, &restart);
    let status = run_tool_command(command, &directory, emit)?;

    // The phone leaves fastboot while the command runs, so a non-zero exit
    // does not mean the reboot did not happen.
    if !status.success() {
        emit(FlashEvent::Log(format!(
            "{} exited with {status}; the phone reboots anyway",
            tool.label()
        )));
    }

    if mode == RebootMode::Sideload {
        let _ = std::fs::remove_file(&sideload_path);
    }

    Ok(())
}

/// The `mfastboot` command for `args`, addressing `serial` when there is one.
fn tool_command(tool: &Mfastboot, serial: &str, args: &[&str]) -> Command {
    let mut command = tool.emulator.command(&tool.path);

    if !serial.is_empty() {
        command.arg("-s").arg(serial);
    }

    command.args(args);
    command
}

/// How a command is written into the log (`mfastboot 34.0.4 -s ZY22ABC reboot`).
fn tool_command_line(tool: &Mfastboot, serial: &str, args: &[&str]) -> String {
    let mut line = tool.label();

    if !serial.is_empty() {
        line.push_str(" -s ");
        line.push_str(serial);
    }

    for arg in args {
        line.push(' ');
        line.push_str(arg);
    }

    line
}

/// The payload written to `misc` to make the phone boot into recovery's
/// "apply update from ADB" (sideload) mode.
///
/// It is an Android bootloader control block: `command[32]`, `status[32]` and
/// then the arguments handed to recovery. Only these 84 bytes matter — the
/// `recovery` field is `768` bytes long, and the bootloader reads it up to the
/// first newline.
fn misc_sideload_bcb() -> [u8; 84] {
    let mut bcb = [0u8; 84];

    bcb[..13].copy_from_slice(b"boot-recovery");
    bcb[64..83].copy_from_slice(b"recovery\n--sideload");
    bcb[83] = b'\n';

    bcb
}

/// Runs the steps through the built-in `fastboot` crate.
fn run_with_builtin(
    job: &FlashJob,
    emit: &(dyn Fn(FlashEvent) + Send + Sync),
) -> Result<(), String> {
    emit(FlashEvent::Log(format!(
        "connecting to {} (built-in fastboot)",
        job.serial
    )));

    let mut device = fastboot::FastbootDevice::connect(&job.serial)?;
    // Every reply of the bootloader (INFO/DATA/FAIL) goes to the log: the
    // built-in crate would otherwise drop the text that explains what the
    // device is doing (close to what `mfastboot` prints).
    device.set_packet_logger(Some(Box::new(|line| emit(FlashEvent::Log(line)))));
    // The device knows how large a single download may be; without this the
    // crate's conservative 128 MiB default is used.
    device.refresh_max_download();

    let total = job.total_steps();

    for (index, step) in job.package.steps.iter().enumerate() {
        emit(FlashEvent::StepStarted {
            index,
            total,
            label: step.describe(),
        });

        run_builtin_step(&device, step, job, emit)?;
    }

    for (offset, partition) in job.extra_erases.iter().enumerate() {
        emit(FlashEvent::StepStarted {
            index: job.package.steps.len() + offset,
            total,
            label: format!("erase {partition}"),
        });

        // Best effort: not every device has the partition.
        if let Err(error) = device.erase(partition) {
            emit(FlashEvent::Log(format!("erase {partition} skipped: {error}")));
        }
    }

    Ok(())
}

/// Runs one step of the package (a failure aborts the flash).
fn run_builtin_step(
    device: &fastboot::FastbootDevice<'_>,
    step: &FlashOp,
    job: &FlashJob,
    emit: &(dyn Fn(FlashEvent) + Send + Sync),
) -> Result<(), String> {
    match step {
        FlashOp::Flash {
            partition,
            file,
            md5,
        } => {
            let path = job.package.directory.join(file);
            if !path.is_file() {
                return Err(format!("missing image file: {}", path.display()));
            }

            if job.verify_checksums {
                if let Some(expected) = md5 {
                    verify_md5(&path, expected, emit)?;
                }
            }

            // Large partitions are written in 1 MiB chunks; only report
            // when the percentage actually moves (the callback must be
            // `Fn`, so the throttle state lives in a `Cell`).
            let reported = std::cell::Cell::new(u64::MAX);
            let on_progress = |done: usize, total: usize| {
                let (done, total) = (done as u64, total as u64);
                let permille = if total == 0 { 0 } else { done * 1000 / total };
                if permille != reported.get() {
                    reported.set(permille);
                    emit(FlashEvent::Progress { done, total });
                }
            };

            device.flash_file(partition, &path, Some(&on_progress))
        }
        FlashOp::Erase { partition } => device.erase(partition),
        FlashOp::Oem { var } => device.oem(var),
        FlashOp::Getvar { var } => {
            // Informational only: a bootloader that does not know the variable
            // must not abort the flash. The values themselves are already in
            // the log, because the packet logger reports every INFO line.
            match device.getvar_lines(var) {
                Ok(_) => {}
                Err(error) => emit(FlashEvent::Log(format!("getvar {var} skipped: {error}"))),
            }

            Ok(())
        }
    }
}

/// Runs the steps by spawning the external `mfastboot` executable.
fn run_with_mfastboot(
    job: &FlashJob,
    tool: &Mfastboot,
    emit: &(dyn Fn(FlashEvent) + Send + Sync),
) -> Result<(), String> {
    emit(FlashEvent::Log(format!(
        "using {} ({})",
        tool.label(),
        tool.path.display()
    )));

    let total = job.total_steps();

    for (index, step) in job.package.steps.iter().enumerate() {
        emit(FlashEvent::StepStarted {
            index,
            total,
            label: step.describe(),
        });

        run_mfastboot_step(job, tool, step, emit)?;
    }

    for (offset, partition) in job.extra_erases.iter().enumerate() {
        emit(FlashEvent::StepStarted {
            index: job.package.steps.len() + offset,
            total,
            label: format!("erase {partition}"),
        });

        // Best effort: not every device has the partition.
        let erase = FlashOp::Erase {
            partition: partition.clone(),
        };

        if let Err(error) = run_mfastboot_step(job, tool, &erase, emit) {
            emit(FlashEvent::Log(format!(
                "erase {partition} skipped: {error}"
            )));
        }
    }

    Ok(())
}

/// Runs one step with `mfastboot`, forwarding its output to the log.
fn run_mfastboot_step(
    job: &FlashJob,
    tool: &Mfastboot,
    step: &FlashOp,
    emit: &(dyn Fn(FlashEvent) + Send + Sync),
) -> Result<(), String> {
    let mut command = tool.emulator.command(&tool.path);
    if !job.serial.is_empty() {
        command.arg("-s").arg(&job.serial);
    }

    match step {
        FlashOp::Flash {
            partition,
            file,
            md5,
        } => {
            let path = job.package.directory.join(file);
            if !path.is_file() {
                return Err(format!("missing image file: {}", path.display()));
            }

            if job.verify_checksums {
                if let Some(expected) = md5 {
                    verify_md5(&path, expected, emit)?;
                }
            }

            command.arg("flash").arg(partition).arg(&path);
        }
        FlashOp::Erase { partition } => {
            command.arg("erase").arg(partition);
        }
        FlashOp::Oem { var } => {
            command.arg("oem");
            // `var` is a full command line (`config bootmode fastboot`).
            for token in var.split_whitespace() {
                command.arg(token);
            }
        }
        FlashOp::Getvar { var } => {
            command.arg("getvar").arg(var);
        }
    }

    let status = run_tool_command(command, &job.package.directory, emit)?;

    if !status.success() {
        // A `getvar` is informational, so its failure is logged instead of
        // aborting the flash.
        if matches!(step, FlashOp::Getvar { .. }) {
            emit(FlashEvent::Log(format!(
                "getvar {} skipped: {} exited with {status}",
                step.describe(),
                tool.label()
            )));
            return Ok(());
        }

        return Err(format!(
            "{} failed for step ({})",
            tool.label(),
            step.describe()
        ));
    }

    Ok(())
}

/// Runs a prepared `mfastboot` command, forwarding its output to the log and
/// returning its exit status.
fn run_tool_command(
    mut command: Command,
    directory: &Path,
    emit: &(dyn Fn(FlashEvent) + Send + Sync),
) -> Result<std::process::ExitStatus, String> {
    // The emulator, when there is one: failures have to name the program that
    // was actually started (`box64`, not the binary it was given).
    let program = command.get_program().to_string_lossy().to_string();

    command
        .current_dir(directory)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // A GUI application must not flash a console window on screen.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }

    let mut child = command
        .spawn()
        .map_err(|error| format!("could not run {program}: {error}"))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // Read the two pipes concurrently; the tool can fill either buffer
    // while we are blocked on the other one.
    std::thread::scope(|scope| {
        if let Some(stderr) = stderr {
            scope.spawn(|| pump(stderr, emit));
        }

        if let Some(stdout) = stdout {
            pump(stdout, emit);
        }
    });

    child
        .wait()
        .map_err(|error| format!("could not wait for {program}: {error}"))
}

/// Forwards the tool's output as log lines, splitting on both `\n` and the
/// carriage returns `fastboot` uses for in-place progress updates.
fn pump(reader: impl Read, emit: &(dyn Fn(FlashEvent) + Send + Sync)) {
    let mut reader = std::io::BufReader::new(reader);
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                for part in line.split(['\n', '\r']) {
                    let part = part.trim();
                    if !part.is_empty() {
                        emit(FlashEvent::Log(part.to_owned()));
                    }
                }
            }
        }
    }
}

/// Compares a file against the MD5 recorded in the `flashfile.xml`.
fn verify_md5(
    path: &Path,
    expected: &str,
    emit: &(dyn Fn(FlashEvent) + Send + Sync),
) -> Result<(), String> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());
    emit(FlashEvent::Log(format!("verifying {name}")));

    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("could not open {}: {error}", path.display()))?;
    let mut context = md5::Context::new();
    let mut buffer = vec![0u8; 1 << 20];

    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }

        context.consume(&buffer[..read]);
    }

    let actual = format!("{:x}", context.compute());

    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(format!(
            "MD5 mismatch for {name}: expected {expected}, got {actual}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_sort_numerically() {
        assert_eq!(
            compare_versions("34.0.4", "29.0.6"),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            compare_versions("26.0.0", "28.0.2"),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            compare_versions("34.0.4", "34.0.4"),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn newest_version_comes_first() {
        let mut versions = vec![
            Mfastboot {
                version: "26.0.0".to_owned(),
                path: PathBuf::from("a"),
                emulator: Emulator::Native,
            },
            Mfastboot {
                version: "34.0.4".to_owned(),
                path: PathBuf::from("b"),
                emulator: Emulator::Native,
            },
            Mfastboot {
                version: "28.0.2".to_owned(),
                path: PathBuf::from("c"),
                emulator: Emulator::Native,
            },
        ];

        versions.sort_by(|a, b| compare_versions(&b.version, &a.version));

        assert_eq!(versions[0].version, "34.0.4");
        assert_eq!(versions[2].version, "26.0.0");
    }

    #[test]
    fn architecture_folders_are_filtered() {
        let host = std::env::consts::OS;
        // Nothing to translate: Intel binaries run as they are, and any other
        // architecture cannot be run at all.
        let intel_host = None;

        // The folder for the OS and architecture we run as is always usable.
        assert_eq!(
            platform_fit(&format!("{host}_x86_64"), "x86_64", intel_host),
            Some(Emulator::Native)
        );
        assert_eq!(
            platform_fit(&format!("{host}_arm64"), "aarch64", intel_host),
            Some(Emulator::Native)
        );

        // A folder that names neither is left alone.
        assert_eq!(
            platform_fit("bundled", "x86_64", intel_host),
            Some(Emulator::Native)
        );
        assert_eq!(
            platform_fit("vendored", "aarch64", intel_host),
            Some(Emulator::Native)
        );

        // A folder for another architecture is not.
        assert_eq!(
            platform_fit(&format!("{host}_amd64"), "aarch64", intel_host),
            None
        );
        assert_eq!(
            platform_fit(&format!("{host}_arm64"), "x86_64", intel_host),
            None
        );
    }

    #[test]
    fn other_operating_systems_are_filtered() {
        let host = std::env::consts::OS;
        let other = if host == "linux" { "windows" } else { "linux" };
        let running = std::env::consts::ARCH;

        // The architecture we run as, but for another OS: never usable — the
        // emulator would be handed a binary for a foreign operating system.
        let rosetta = Emulator::Rosetta { installed: true };
        assert_eq!(
            platform_fit(&format!("{other}_{running}"), running, Some(&rosetta)),
            None
        );
        assert_eq!(
            platform_fit(&format!("{other}_amd64"), "aarch64", Some(&rosetta)),
            None
        );
    }

    #[test]
    fn intel_mfastboot_is_offered_through_an_emulator() {
        // The OS in the folder name is the one that has to match the host, so
        // these hold on every machine the tests run on.
        let intel = format!("{}_amd64", std::env::consts::OS);
        let arm = "aarch64";

        // An ARM machine with the emulator installed runs the Intel build.
        let rosetta = Emulator::Rosetta { installed: true };
        assert_eq!(
            platform_fit(&intel, arm, Some(&rosetta)),
            Some(rosetta.clone())
        );
        assert_eq!(rosetta.hint_ids(), None);

        let box64 = Emulator::Box64 {
            program: Some(PathBuf::from("/usr/local/bin/box64")),
        };
        assert_eq!(platform_fit(&intel, arm, Some(&box64)), Some(box64.clone()));
        assert_eq!(box64.hint_ids(), None);

        // Without it the build is still offered while it can be installed, so
        // the dialog can say how — and it is marked as needing that.
        let missing_rosetta = Emulator::Rosetta { installed: false };
        assert_eq!(
            platform_fit(&intel, arm, Some(&missing_rosetta)),
            Some(missing_rosetta.clone())
        );
        assert_eq!(
            missing_rosetta.hint_ids(),
            Some((
                "firmware-flash-rosetta-needed",
                "firmware-flash-rosetta-install"
            ))
        );

        let missing_box64 = Emulator::Box64 { program: None };
        assert_eq!(
            platform_fit(&intel, arm, Some(&missing_box64)),
            Some(missing_box64.clone())
        );
        assert_eq!(
            missing_box64.hint_ids(),
            Some(("firmware-flash-box64-needed", "firmware-flash-box64-install"))
        );

        // macOS 28 dropped Rosetta 2 (and 32-bit ARM Linux has no box64), so
        // the Intel build is not offered there at all.
        assert_eq!(platform_fit(&intel, arm, None), None);

        // An emulator does not make an Apple silicon binary run on an Intel
        // machine either.
        assert_eq!(
            platform_fit(&format!("{}_arm64", std::env::consts::OS), "x86_64", Some(&rosetta)),
            None
        );
    }

    #[test]
    fn box64_starts_mfastboot() {
        let binary = PathBuf::from("/opt/lmnflash/mfastboot/linux_amd64/31.0.2/mfastboot");

        let box64 = Emulator::Box64 {
            program: Some(PathBuf::from("/usr/local/bin/box64")),
        };
        let command = box64.command(&binary);
        assert_eq!(command.get_program(), "/usr/local/bin/box64");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [binary.as_os_str()]
        );

        // Rosetta 2 is applied by macOS itself, so the binary is started
        // directly — and so is a native build.
        for emulator in [Emulator::Native, Emulator::Rosetta { installed: true }] {
            let command = emulator.command(&binary);
            assert_eq!(command.get_program(), binary.as_os_str());
            assert_eq!(command.get_args().count(), 0);
        }
    }

    #[test]
    fn the_sideload_payload_matches_the_reference_file() {
        // Hex dump of `reference/misc-sideload`: the command (`boot-recovery`),
        // 51 zero bytes, then the arguments handed to recovery.
        let zeros = "00".repeat(51);
        let payload = misc_sideload_bcb()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();

        assert_eq!(
            payload,
            format!("626f6f742d7265636f76657279{zeros}7265636f766572790a2d2d736964656c6f61640a")
        );
    }

    #[test]
    fn macos_versions_are_parsed() {
        assert_eq!(parse_macos_major("26.0"), Some(26));
        assert_eq!(parse_macos_major("11"), Some(11));
        assert_eq!(parse_macos_major("10.15.7"), Some(10));
        assert_eq!(parse_macos_major(" 27.1\n"), Some(27));
        assert_eq!(parse_macos_major("not a version"), None);
        assert_eq!(parse_macos_major(""), None);
    }

    #[test]
    fn mfastboot_labels_are_readable() {
        let tool = Mfastboot {
            version: "34.0.4".to_owned(),
            path: PathBuf::from("mfastboot"),
            emulator: Emulator::Native,
        };

        assert_eq!(tool.label(), "mfastboot 34.0.4");
    }
}
