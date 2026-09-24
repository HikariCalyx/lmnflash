//! Flashing engine for the "Firmware Flash" feature (Mode 2).
//!
//! Two backends can run a [`FlashPackage`]:
//!
//! * **External `mfastboot`** — Motorola's `fastboot` fork, looked up next to
//!   the application (see [`discover_mfastboot`]). Preferred, because its
//!   behaviour matches the firmware packages exactly.
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
///   `<platform>` names the architecture the binary was built for
///   (`windows_amd64`, `darwin_amd64`, …) — mirroring the `prebuilt_binary`
///   tree. A platform that is not the one we run on is skipped, so an
///   x86_64-only `mfastboot` is never offered on ARM (and on macOS an
///   x86_64 build is not silently started through Rosetta).
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

    let mut seen = std::collections::HashSet::new();

    for root in roots {
        let mut candidates: Vec<(String, PathBuf)> = Vec::new();
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
                    candidates.push((version, binary));
                    continue;
                }

                // `<base>/<platform>/<version>/mfastboot[.exe]`
                if !platform_matches(&version) {
                    continue;
                }

                if let Ok(versions) = std::fs::read_dir(&path) {
                    for version_entry in versions.flatten() {
                        let binary = version_entry.path().join(MFASTBOOT_FILE);
                        if binary.is_file() {
                            candidates.push((
                                version_entry.file_name().to_string_lossy().to_string(),
                                binary,
                            ));
                        }
                    }
                }
            }
        }

        // `<root>/mfastboot[.exe]`
        let direct = root.join(MFASTBOOT_FILE);
        if direct.is_file() {
            candidates.push(("local".to_owned(), direct));
        }

        for (version, path) in candidates {
            if seen.insert(version.clone()) {
                found.push(Mfastboot { version, path });
            }
        }
    }

    found.sort_by(|a, b| compare_versions(&b.version, &a.version));

    found
}

/// Whether a `<platform>` directory name (e.g. `windows_amd64`, `darwin_arm64`)
/// can be used on the architecture this process runs as.
///
/// The aliases are checked most specific first, so `arm64` is not mistaken for
/// `arm` and `x86_64` not for `x86`. A name that does not mention any
/// architecture is accepted: whoever laid the directory out knows what was put
/// in it.
fn platform_matches(platform: &str) -> bool {
    /// `(alias, architecture it denotes)`, longest aliases first.
    const ALIASES: &[(&str, &str)] = &[
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

    let platform = platform.to_ascii_lowercase();

    for (alias, architecture) in ALIASES {
        if platform.contains(alias) {
            return *architecture == std::env::consts::ARCH;
        }
    }

    true
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

/// Takes the device out of fastboot once the flash is done: the boot mode
/// flag the firmware sets (`oem config bootmode fastboot`) is cleared first,
/// then the phone is rebooted into the system.
///
/// Both commands run through the engine the package was flashed with, and the
/// tool's output is forwarded to the log like during a flash.
pub fn reboot_to_system(
    engine: &Engine,
    serial: &str,
    emit: &(dyn Fn(FlashEvent) + Send + Sync),
) -> Result<(), String> {
    match engine {
        Engine::Builtin => {
            emit(FlashEvent::Log(format!(
                "connecting to {} (built-in fastboot)",
                if serial.is_empty() { "device" } else { serial }
            )));

            let device = fastboot::FastbootDevice::connect(serial)?;

            emit(FlashEvent::Log("oem fb_mode_clear".to_string()));
            device.oem("fb_mode_clear")?;

            emit(FlashEvent::Log("reboot".to_string()));
            reboot_builtin(&device, emit)
        }
        Engine::Mfastboot(tool) => {
            let directory = std::env::temp_dir();

            emit(FlashEvent::Log(format!(
                "{} -s {} oem fb_mode_clear",
                tool.label(),
                serial
            )));

            let mut clear = Command::new(&tool.path);
            if !serial.is_empty() {
                clear.arg("-s").arg(serial);
            }
            clear.arg("oem").arg("fb_mode_clear");

            let status = run_tool_command(clear, tool, &directory, emit)?;
            if !status.success() {
                return Err(format!("{} failed for oem fb_mode_clear", tool.label()));
            }

            emit(FlashEvent::Log(format!(
                "{} -s {} reboot",
                tool.label(),
                serial
            )));

            let mut restart = Command::new(&tool.path);
            if !serial.is_empty() {
                restart.arg("-s").arg(serial);
            }
            restart.arg("reboot");

            match run_tool_command(restart, tool, &directory, emit) {
                Ok(status) if status.success() => Ok(()),
                // The phone leaves fastboot while the command is running, so
                // a non-zero exit does not mean the reboot did not happen.
                Ok(status) => {
                    emit(FlashEvent::Log(format!(
                        "{} exited with {status}; the phone reboots anyway",
                        tool.label()
                    )));
                    Ok(())
                }
                Err(error) => Err(error),
            }
        }
    }
}

/// Sends the `reboot` command through the built-in fastboot crate.
///
/// The device can disappear from USB before it answers the packet, which is
/// not a failure of the reboot itself, so such an error only goes to the log.
fn reboot_builtin(
    device: &fastboot::FastbootDevice,
    emit: &(dyn Fn(FlashEvent) + Send + Sync),
) -> Result<(), String> {
    if let Err(error) = device.reboot() {
        emit(FlashEvent::Log(format!("reboot: {error}")));
    }

    Ok(())
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
    device: &fastboot::FastbootDevice,
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
            // must not abort the flash.
            match device.getvar_lines(var) {
                Ok(lines) => {
                    for line in lines {
                        emit(FlashEvent::Log(line));
                    }
                }
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
    let mut command = Command::new(&tool.path);
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

    let status = run_tool_command(command, tool, &job.package.directory, emit)?;

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
    tool: &Mfastboot,
    directory: &Path,
    emit: &(dyn Fn(FlashEvent) + Send + Sync),
) -> Result<std::process::ExitStatus, String> {
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
        .map_err(|error| format!("could not run {}: {error}", tool.path.display()))?;

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
        .map_err(|error| format!("could not wait for {}: {error}", tool.path.display()))
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
            },
            Mfastboot {
                version: "34.0.4".to_owned(),
                path: PathBuf::from("b"),
            },
            Mfastboot {
                version: "28.0.2".to_owned(),
                path: PathBuf::from("c"),
            },
        ];

        versions.sort_by(|a, b| compare_versions(&b.version, &a.version));

        assert_eq!(versions[0].version, "34.0.4");
        assert_eq!(versions[2].version, "26.0.0");
    }

    #[test]
    fn architecture_folders_are_filtered() {
        let running = std::env::consts::ARCH;

        // The folder for the architecture we run as is always usable.
        assert!(platform_matches(running));
        assert!(platform_matches(&format!("linux_{running}")));

        // A folder that names no architecture at all is left alone.
        assert!(platform_matches("windows"));
        assert!(platform_matches("bundled"));

        // A folder for another architecture is not.
        let other = if running == "x86_64" {
            "darwin_arm64"
        } else {
            "darwin_amd64"
        };
        assert!(!platform_matches(other));
    }

    #[test]
    fn mfastboot_labels_are_readable() {        let tool = Mfastboot {
            version: "34.0.4".to_owned(),
            path: PathBuf::from("mfastboot"),
        };

        assert_eq!(tool.label(), "mfastboot 34.0.4");
    }
}
