//! Driver installation for the "Install Driver" feature (Mode 2).
//!
//! Windows installs Motorola's Mobile Drivers: the MSI that matches the
//! *operating system* architecture is downloaded over several connections and
//! handed to `msiexec`, which is the same thing as double-clicking the file
//! (the Windows Installer service asks for elevation itself when the package
//! needs it). Linux runs the installer of the `android-udev-rules` project —
//! kept as a submodule and embedded in the binary — as root, with the user's
//! password written to `sudo`'s standard input.
//!
//! macOS has neither, and Motorola ships no driver for Windows on ARM, so the
//! feature is hidden there: [`target`] answers `None`.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Motorola's Mobile Drivers installers. Which one is needed is decided by
/// the architecture of the *system*, not of this process (see
/// [`windows_arch`]).
const MSI_X86: &str = "https://motorola-global-portal.custhelp.com/euf/assets/downloads/Motorola_Mobile_Drivers_32bit.msi";
const MSI_X64: &str = "https://motorola-global-portal.custhelp.com/euf/assets/downloads/Motorola_Mobile_Drivers_64bit.msi";

/// Lenovo's download page for "Software Fix", the flashing tool for tablets,
/// which brings its own drivers.
pub const TABLET_SITE: &str = "https://support.lenovo.com/us/en/downloads/ds101291";

/// The udev rules, installed on Linux by the upstream project's own installer.
///
/// The sources live in the `vendor/android-udev-rules` submodule
/// (<https://github.com/M0Rf30/android-udev-rules>, GPL-3.0-or-later) and are
/// compiled into the binary, so installing needs neither the submodule nor a
/// download at run time.
///
/// `install.sh` reads its two files from the directory it is started in, so
/// the names here are the layout it expects.
const LINUX_INSTALLER: &[(&str, &str)] = &[
    (
        "install.sh",
        include_str!("../vendor/android-udev-rules/install.sh"),
    ),
    (
        "51-android.rules",
        include_str!("../vendor/android-udev-rules/51-android.rules"),
    ),
    (
        "android-udev.conf",
        include_str!("../vendor/android-udev-rules/android-udev.conf"),
    ),
];

/// The directory the installer is unpacked into before it runs.
const LINUX_INSTALLER_DIR: &str = "lmnflash-android-udev-rules";

/// How many connections the MSI download is split into.
pub const DOWNLOAD_SEGMENTS: usize = 5;

/// The architecture Windows runs at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsArch {
    /// 32-bit (`x86`) Windows — every edition takes the 32-bit driver.
    X86,
    /// 64-bit (`x86_64`/`amd64`) Windows.
    X64,
}

/// What installing a driver means on this system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Windows: download and run Motorola's Mobile Drivers.
    Windows(WindowsArch),
    /// Linux: install the udev rules with root rights.
    Linux,
}

/// The driver installation this system supports, or `None` where the feature
/// does not exist (macOS, and Windows on ARM).
pub fn target() -> Option<Target> {
    if cfg!(target_os = "windows") {
        let architew6432 = environment("PROCESSOR_ARCHITEW6432");
        let arch = windows_arch(
            &environment("PROCESSOR_ARCHITECTURE"),
            Some(architew6432.as_str()),
        )?;

        return Some(Target::Windows(arch));
    }

    cfg!(target_os = "linux").then_some(Target::Linux)
}

/// Whether installing takes a password (Linux asks for it; Windows lets the
/// installer elevate itself).
pub fn uses_password() -> bool {
    cfg!(target_os = "linux")
}

/// The architecture of the Windows *system*, from the two variables every
/// process sees.
///
/// `PROCESSOR_ARCHITEW6432` is set — to the machine's real architecture — only
/// while a 32-bit process runs on a 64-bit system, so it wins when present:
/// the 32-bit build of this app therefore still picks the 64-bit driver on a
/// 64-bit PC. `None` means Motorola has no driver for that architecture, or
/// `processor_architecture` named one this code does not know; both are
/// treated as "no driver here" rather than guessing.
pub fn windows_arch(
    processor_architecture: &str,
    processor_architew6432: Option<&str>,
) -> Option<WindowsArch> {
    let architecture = processor_architew6432
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(processor_architecture);

    match architecture.trim().to_ascii_uppercase().as_str() {
        // Itanium (`IA64`) runs x64 binaries; the machine is dead, but the
        // value costs nothing to handle.
        "AMD64" | "IA64" => Some(WindowsArch::X64),
        "X86" => Some(WindowsArch::X86),
        // ARM64 (and the old 32-bit ARM) have no Motorola installer.
        _ => None,
    }
}

/// The installer URL for a Windows architecture.
pub fn windows_msi_url(arch: WindowsArch) -> &'static str {
    match arch {
        WindowsArch::X86 => MSI_X86,
        WindowsArch::X64 => MSI_X64,
    }
}

/// The file name the installer is downloaded under (the last segment of the
/// URL, which is also what the user recognizes in the installer's dialogs).
pub fn windows_msi_file_name(arch: WindowsArch) -> &'static str {
    match arch {
        WindowsArch::X86 => "Motorola_Mobile_Drivers_32bit.msi",
        WindowsArch::X64 => "Motorola_Mobile_Drivers_64bit.msi",
    }
}

/// A failure that the dialog reports back to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverError {
    /// `sudo` rejected the password.
    WrongPassword,
    /// `sudo` is not installed, so nothing can be elevated.
    NoSudo,
    /// Anything else; the message is shown as it came back.
    Failed(String),
}

impl From<std::io::Error> for DriverError {
    fn from(error: std::io::Error) -> Self {
        Self::Failed(error.to_string())
    }
}

impl std::fmt::Display for DriverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongPassword => f.write_str("the password was not accepted"),
            Self::NoSudo => f.write_str("sudo was not found"),
            Self::Failed(message) => f.write_str(message),
        }
    }
}

/// Progress of an installation, reported from the worker thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverEvent {
    /// Bytes downloaded so far out of the total (0 while the size is unknown).
    Progress { downloaded: u64, total: u64 },
    /// The download is done and the installer (or the udev rules) is running.
    Installing,
    /// The installation is over; this is always the last event.
    Finished(Result<(), DriverError>),
}

/// Runs the installation this system supports. Blocking — meant for a worker
/// thread. Progress and the outcome reach the UI through `emit`.
pub fn run(
    target: Option<Target>,
    password: &str,
    emit: &(dyn Fn(DriverEvent) + Send + Sync),
) -> Result<(), DriverError> {
    match target {
        Some(Target::Windows(arch)) => {
            let destination = download_directory().join(windows_msi_file_name(arch));

            download(
                windows_msi_url(arch),
                &destination,
                DOWNLOAD_SEGMENTS,
                emit,
            )?;
            emit(DriverEvent::Installing);

            run_msi(&destination)
        }
        Some(Target::Linux) => {
            emit(DriverEvent::Installing);

            install_linux_rules(password)
        }
        None => Err(DriverError::Failed(
            "there is no driver installer for this system".to_owned(),
        )),
    }
}

/// Where a downloaded installer is kept (the OS clears its temporary
/// directory; the installer needs the file until it is done with it).
pub fn download_directory() -> PathBuf {
    std::env::temp_dir().join("lmnflash-drivers")
}

/// Downloads `url` into `destination` over `segments` connections.
///
/// The file is pre-allocated and every segment writes its own byte range at
/// its own offset, so the write positions never overlap. A server that does
/// not answer a byte range (or does not report a length) is downloaded in one
/// piece instead — and a segmented download that fails at all starts over
/// that way, since a half-filled file must not be handed to the installer.
pub fn download(
    url: &str,
    destination: &Path,
    segments: usize,
    emit: &(dyn Fn(DriverEvent) + Send + Sync),
) -> Result<(), DriverError> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        // Only the wait for the next packet: a slow line is not a failure.
        .timeout_read(Duration::from_secs(60))
        .build();

    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }

    match probe(&agent, url) {
        Some(total) if segments > 1 => {
            if let Err(error) = download_segments(&agent, url, destination, total, segments, emit)
            {
                eprintln!("[driver] {segments}-segment download failed ({error}); retrying in one piece");

                let _ = std::fs::remove_file(destination);
                download_whole(&agent, url, destination, emit)?;
            }
        }
        _ => download_whole(&agent, url, destination, emit)?,
    }

    Ok(())
}

/// Asks for the first byte of the file: a `206` answer carries the size of the
/// whole file in `Content-Range` and proves byte ranges are honoured.
fn probe(agent: &ureq::Agent, url: &str) -> Option<u64> {
    let response = agent.get(url).set("Range", "bytes=0-0").call().ok()?;

    if response.status() != 206 {
        return None;
    }

    content_range_total(response.header("Content-Range")?)
}

/// Reads the total out of a `Content-Range` header (`bytes 0-0/3529728`);
/// `*` (an unknown size) yields `None`.
fn content_range_total(value: &str) -> Option<u64> {
    value.rsplit('/').next()?.trim().parse().ok()
}

/// Splits a file of `total` bytes into at most `segments` inclusive byte
/// ranges that cover it completely.
fn segment_ranges(total: u64, segments: usize) -> Vec<(u64, u64)> {
    if total == 0 || segments == 0 {
        return Vec::new();
    }

    let per_segment = total.div_ceil(segments as u64).max(1);
    let mut ranges = Vec::new();
    let mut start = 0;

    while start < total && ranges.len() < segments {
        let end = (start + per_segment - 1).min(total - 1);
        ranges.push((start, end));
        start = end + 1;
    }

    ranges
}

/// Fetches every range on its own thread. The first failure stops the others
/// from starting more work and is reported to the caller.
fn download_segments(
    agent: &ureq::Agent,
    url: &str,
    destination: &Path,
    total: u64,
    segments: usize,
    emit: &(dyn Fn(DriverEvent) + Send + Sync),
) -> Result<(), DriverError> {
    let file = std::fs::File::create(destination)?;
    file.set_len(total)?;
    drop(file);

    let ranges = segment_ranges(total, segments);
    let progress = Progress::new(total);
    let downloaded = AtomicU64::new(0);
    let failed: Mutex<Option<DriverError>> = Mutex::new(None);

    std::thread::scope(|scope| {
        let failed = &failed;
        let downloaded = &downloaded;
        let progress = &progress;

        for (start, end) in ranges {
            scope.spawn(move || {
                if lock(failed).is_some() {
                    return;
                }

                let result = download_segment(
                    agent,
                    url,
                    destination,
                    start,
                    end,
                    downloaded,
                    progress,
                    emit,
                );

                if let Err(error) = result {
                    let mut slot = lock(failed);

                    if slot.is_none() {
                        *slot = Some(error);
                    }
                }
            });
        }
    });

    match lock(&failed).take() {
        Some(error) => Err(error),
        None => {
            emit(DriverEvent::Progress {
                downloaded: total,
                total,
            });

            Ok(())
        }
    }
}

/// Downloads one inclusive byte range into its place in the file.
#[allow(clippy::too_many_arguments)]
fn download_segment(
    agent: &ureq::Agent,
    url: &str,
    destination: &Path,
    start: u64,
    end: u64,
    downloaded: &AtomicU64,
    progress: &Progress,
    emit: &(dyn Fn(DriverEvent) + Send + Sync),
) -> Result<(), DriverError> {
    let length = end - start + 1;

    let response = agent
        .get(url)
        .set("Range", &format!("bytes={start}-{end}"))
        .call()
        .map_err(|error| DriverError::Failed(format!("{url}: {error}")))?;

    // A `200` here would be the whole file, which must not be written into one
    // segment's place; the caller retries the download in one piece.
    if response.status() != 206 {
        return Err(DriverError::Failed(format!(
            "the server answered {} instead of the requested byte range",
            response.status()
        )));
    }

    let mut reader = response.into_reader().take(length);
    let mut file = std::fs::OpenOptions::new().write(true).open(destination)?;
    file.seek(SeekFrom::Start(start))?;

    let mut buffer = vec![0u8; 64 * 1024];
    let mut written = 0u64;

    loop {
        let read = reader.read(&mut buffer).map_err(|error| {
            DriverError::Failed(format!("{url} (byte {start}): {error}"))
        })?;

        if read == 0 {
            break;
        }

        file.write_all(&buffer[..read])?;

        written += read as u64;
        let done = downloaded.fetch_add(read as u64, Ordering::Relaxed) + read as u64;
        progress.report(done, emit);
    }

    if written != length {
        return Err(DriverError::Failed(format!(
            "{url}: the connection ended after {written} of {length} bytes"
        )));
    }

    Ok(())
}

/// Downloads the whole file with a single request, for servers (or retries)
/// that do not use byte ranges.
fn download_whole(
    agent: &ureq::Agent,
    url: &str,
    destination: &Path,
    emit: &(dyn Fn(DriverEvent) + Send + Sync),
) -> Result<(), DriverError> {
    let response = agent
        .get(url)
        .call()
        .map_err(|error| DriverError::Failed(format!("{url}: {error}")))?;

    let total = response
        .header("Content-Length")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);

    let progress = Progress::new(total);
    let mut reader = response.into_reader();
    let mut file = std::fs::File::create(destination)?;
    let mut buffer = vec![0u8; 64 * 1024];
    let mut downloaded = 0u64;

    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| DriverError::Failed(format!("{url}: {error}")))?;

        if read == 0 {
            break;
        }

        file.write_all(&buffer[..read])?;

        downloaded += read as u64;
        progress.report(downloaded, emit);
    }

    file.flush()?;

    if total > 0 {
        emit(DriverEvent::Progress {
            downloaded: total,
            total,
        });
    }

    Ok(())
}

/// Reports the download progress to the UI, at most once every percent (and
/// never for less than one buffer) so a fast line does not flood the dialog
/// with events.
struct Progress {
    total: u64,
    reported: AtomicU64,
}

impl Progress {
    fn new(total: u64) -> Self {
        Self {
            total,
            reported: AtomicU64::new(0),
        }
    }

    fn report(&self, downloaded: u64, emit: &(dyn Fn(DriverEvent) + Send + Sync)) {
        let step = (self.total / 100).max(64 * 1024);
        let previous = self.reported.load(Ordering::Relaxed);

        if downloaded.saturating_sub(previous) < step {
            return;
        }

        self.reported.store(downloaded, Ordering::Relaxed);
        emit(DriverEvent::Progress {
            downloaded,
            total: self.total,
        });
    }
}

/// Starts the downloaded installer.
///
/// `msiexec` hands the package to the Windows Installer service, which asks
/// for elevation itself when the package needs it, and the setup window stays
/// open after this process ends — so the installer is not waited for.
#[cfg(target_os = "windows")]
fn run_msi(path: &Path) -> Result<(), DriverError> {
    use std::process::Stdio;

    std::process::Command::new("msiexec.exe")
        .arg("/i")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| DriverError::Failed(format!("msiexec: {error}")))
}

#[cfg(not(target_os = "windows"))]
fn run_msi(_path: &Path) -> Result<(), DriverError> {
    Err(DriverError::Failed(
        "the installer can only be run on Windows".to_owned(),
    ))
}

/// Installs the udev rules with the upstream project's own installer.
///
/// There is no re-implementation of what `install.sh` does: the embedded copy
/// is written to a temporary directory and run as root, and whatever it does
/// to `/etc/udev/rules.d`, the `adbusers` group and udev is the installation.
///
/// The script has to be started from its own directory — it reads
/// `51-android.rules` and `android-udev.conf` relative to the working
/// directory — and it insists on being root, which is what `sudo` is for.
pub fn install_linux_rules(password: &str) -> Result<(), DriverError> {
    let directory = std::env::temp_dir().join(LINUX_INSTALLER_DIR);
    extract_installer(&directory)?;

    let result = run_sudo(&directory, password);

    // By now the rules are where they belong; the copies do not have to stay.
    let _ = std::fs::remove_dir_all(&directory);

    result
}

/// Writes the embedded installer files into `directory`.
///
/// Line endings are normalized on the way out: the submodule is checked out
/// with this machine's `core.autocrlf`, and both `sh` and udev read these
/// files as they are — a `\r` at the end of a line of `install.sh` (or of a
/// rule) is a syntax error, not whitespace.
fn extract_installer(directory: &Path) -> Result<(), DriverError> {
    std::fs::create_dir_all(directory)?;

    for (name, content) in LINUX_INSTALLER {
        std::fs::write(directory.join(name), content.replace("\r\n", "\n"))?;
    }

    Ok(())
}

/// Runs the unpacked `install.sh` as root through `sudo`.
///
/// The password goes to `sudo`'s standard input (`-S`), which is how a
/// graphical application can elevate without a terminal, and the prompt is
/// silenced (`-p ""`). The installer's output is kept in English so a rejected
/// password can be told apart from any other failure.
fn run_sudo(directory: &Path, password: &str) -> Result<(), DriverError> {
    use std::process::Stdio;

    let mut child = std::process::Command::new("sudo")
        .args(["-S", "-p", "", "--", "sh", "install.sh"])
        .current_dir(directory)
        // The installer's own default, spelled out: its other mode symlinks
        // the rules file into `/etc/udev/rules.d`, which would point into the
        // directory below — and that one is removed again afterwards.
        .env("USE_SYMLINK", "false")
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => DriverError::NoSudo,
            _ => DriverError::Failed(format!("sudo: {error}")),
        })?;

    if let Some(mut stdin) = child.stdin.take() {
        // A failed write only means `sudo` did not need the password (it was
        // still allowed by its timestamp), which the exit status settles.
        let _ = writeln!(stdin, "{password}");
        let _ = stdin.flush();
    }

    let output = child.wait_with_output()?;

    if output.status.success() {
        // The installer is not silent (`cp -v`, the udev restart); its output
        // is only interesting when something goes wrong.
        #[cfg(debug_assertions)]
        eprintln!(
            "[driver] install.sh: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        return Ok(());
    }

    // Whatever the commands printed (stderr first) is what the dialog shows.
    let mut details = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();

    if !stdout.is_empty() {
        if !details.is_empty() {
            details.push('\n');
        }

        details.push_str(&stdout);
    }

    if details.is_empty() {
        details = format!("sudo exited with {}", output.status);
    }

    if wrong_password(&details) {
        return Err(DriverError::WrongPassword);
    }

    Err(DriverError::Failed(details))
}

/// Whether `sudo`'s output says the password was wrong (the messages are
/// English because the command runs with `LC_ALL=C`).
fn wrong_password(output: &str) -> bool {
    let output = output.to_ascii_lowercase();

    output.contains("incorrect password") || output.contains("sorry, try again")
}

/// A copy of the value of an environment variable, or an empty string.
fn environment(name: &str) -> String {
    std::env::var(name).unwrap_or_default()
}

/// Locks a mutex, ignoring a poisoned lock (the data is a plain slot, so a
/// panic elsewhere cannot leave it inconsistent).
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 32-bit build of this app runs on 64-bit Windows through WOW64, so
    /// the *system* architecture decides which driver is installed.
    #[test]
    fn windows_architecture_follows_the_system() {
        assert_eq!(windows_arch("AMD64", None), Some(WindowsArch::X64));
        assert_eq!(windows_arch("amd64", None), Some(WindowsArch::X64));
        assert_eq!(windows_arch("IA64", None), Some(WindowsArch::X64));
        assert_eq!(windows_arch("x86", None), Some(WindowsArch::X86));
        assert_eq!(windows_arch("  x86 ", None), Some(WindowsArch::X86));

        // A 32-bit process on a 64-bit system: WOW64 reports the real
        // architecture in the second variable, which wins.
        assert_eq!(windows_arch("x86", Some("AMD64")), Some(WindowsArch::X64));
        assert_eq!(windows_arch("x86", Some("")), Some(WindowsArch::X86));
        assert_eq!(windows_arch("x86", Some("  ")), Some(WindowsArch::X86));

        // Windows on ARM has no Motorola driver — not with a 32-bit build
        // either, and not on an unknown architecture.
        assert_eq!(windows_arch("ARM64", None), None);
        assert_eq!(windows_arch("ARM64", Some("ARM64")), None);
        assert_eq!(windows_arch("x86", Some("ARM64")), None);
        assert_eq!(windows_arch("ARM", None), None);
        assert_eq!(windows_arch("", None), None);
        assert_eq!(windows_arch("mips", None), None);
    }

    #[test]
    fn the_installer_matches_the_architecture() {
        assert!(windows_msi_url(WindowsArch::X86).ends_with("32bit.msi"));
        assert!(windows_msi_url(WindowsArch::X64).ends_with("64bit.msi"));
        assert_eq!(
            windows_msi_url(WindowsArch::X64).rsplit('/').next(),
            Some(windows_msi_file_name(WindowsArch::X64))
        );
        assert_eq!(
            windows_msi_url(WindowsArch::X86).rsplit('/').next(),
            Some(windows_msi_file_name(WindowsArch::X86))
        );
    }

    #[test]
    fn the_file_is_split_into_covering_ranges() {
        assert_eq!(
            segment_ranges(100, 5),
            vec![(0, 19), (20, 39), (40, 59), (60, 79), (80, 99)]
        );

        // A remainder lands in the last range; the ranges still cover it all.
        assert_eq!(segment_ranges(11, 5), vec![(0, 2), (3, 5), (6, 8), (9, 10)]);

        // Fewer bytes than segments: no empty (or reversed) range.
        assert_eq!(segment_ranges(2, 5), vec![(0, 0), (1, 1)]);

        assert_eq!(segment_ranges(1, 1), vec![(0, 0)]);
        assert_eq!(segment_ranges(0, 5), Vec::new());
        assert_eq!(segment_ranges(100, 0), Vec::new());

        // Whatever the split, every byte is written exactly once.
        for (total, segments) in [(7u64, 3usize), (1_000, 5), (4, 5), (9, 9)] {
            let ranges = segment_ranges(total, segments);
            let mut next = 0;

            for (start, end) in &ranges {
                assert_eq!(*start, next, "gap before byte {start}");
                assert!(start <= end, "reversed range {start}-{end}");
                next = end + 1;
            }

            assert_eq!(next, total, "the ranges do not cover the file");
        }
    }

    #[test]
    fn the_total_size_is_read_from_the_content_range() {
        assert_eq!(content_range_total("bytes 0-0/3529728"), Some(3_529_728));
        assert_eq!(content_range_total("bytes 100-199/200"), Some(200));
        assert_eq!(content_range_total("bytes 0-0/*"), None);
        assert_eq!(content_range_total(""), None);
        assert_eq!(content_range_total("nonsense"), None);
    }

    #[test]
    fn a_rejected_password_is_recognized() {
        assert!(wrong_password("Sorry, try again."));
        assert!(wrong_password("sudo: 1 incorrect password attempt"));
        assert!(wrong_password("[sudo] password for user:\nsorry, try again."));
        assert!(!wrong_password(""));
        assert!(!wrong_password("udevadm: command not found"));
        assert!(!wrong_password("cp: cannot create regular file: Permission denied"));
    }

    /// The installation is the upstream installer, embedded whole: the script
    /// plus the two files it reads from its own directory, unpacked with the
    /// line endings `sh` and udev need.
    #[test]
    fn the_installer_is_unpacked_with_unix_line_endings() {
        let directory = std::env::temp_dir().join("lmnflash-udev-rules-test");
        let _ = std::fs::remove_dir_all(&directory);

        extract_installer(&directory).expect("the installer is embedded");

        for (name, _) in LINUX_INSTALLER {
            let written = std::fs::read_to_string(directory.join(name))
                .unwrap_or_else(|error| panic!("{name} was not written: {error}"));

            assert!(!written.contains('\r'), "{name} was written with CRLF");
            assert!(written.ends_with('\n'), "{name} has no final newline");
        }

        // The layout `install.sh` expects, and a file that is really the
        // upstream rules rather than a truncated or substituted copy.
        let script = read_text(&directory, "install.sh");
        assert!(script.contains("/etc/udev/rules.d/51-android.rules"));
        assert!(script.contains("usermod -a -G adbusers"));

        let rules = read_text(&directory, "51-android.rules");
        assert!(rules.contains(r#"ATTR{idVendor}!="22b8""#)); // Motorola
        assert!(rules.contains(r#"ENV{adb_adb}=="yes""#));
        assert!(rules.contains("SYMLINK+=\"android_fastboot\""));

        assert_eq!(read_text(&directory, "android-udev.conf"), "g adbusers - -\n");

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// Reads one of the unpacked installer files.
    fn read_text(directory: &Path, name: &str) -> String {
        std::fs::read_to_string(directory.join(name))
            .unwrap_or_else(|error| panic!("{name} was not written: {error}"))
    }

    /// The segmented download against Motorola's real server: the same file
    /// fetched over five connections and in one piece must come out byte for
    /// byte identical, which is what proves every segment landed at its own
    /// offset.
    ///
    /// It needs the Internet, so it is not part of the regular suite:
    /// `cargo test -- --ignored the_segmented_download`.
    #[test]
    #[ignore = "needs the Internet"]
    fn the_segmented_download_matches_a_single_connection() {
        let url = windows_msi_url(WindowsArch::X64);
        let directory = download_directory();
        let segmented = directory.join("segments.msi");
        let whole = directory.join("whole.msi");

        download(url, &segmented, DOWNLOAD_SEGMENTS, &|_| {}).expect("segmented download");
        download(url, &whole, 1, &|_| {}).expect("single-connection download");

        let segmented = std::fs::read(&segmented).expect("read the segmented download");
        let whole = std::fs::read(&whole).expect("read the single download");

        assert!(whole.len() > 1024 * 1024, "suspiciously small: {}", whole.len());
        assert_eq!(segmented, whole, "the segments did not rebuild the file");

        // An MSI is a COM structured-storage file.
        assert_eq!(&whole[..8], b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1");
    }
}
