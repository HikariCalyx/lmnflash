//! Google's `fastboot` from the Android platform-tools.
//!
//! The dialog offers it as a third engine next to the built-in fastboot and the
//! shipped `mfastboot` builds. Google publishes one archive per system instead
//! of versioned downloads, so it is fetched the first time the user picks it and
//! kept in the configuration directory (`<config>/platform-tools/`): the release
//! artifacts stay small, and the build is always the current one.
//!
//! Only the files `fastboot` needs at run time are unpacked — the archive also
//! holds `adb` and its helpers, which would need tens of megabytes here.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config;
use crate::flash_engine::{Emulator, Mfastboot, Origin, intel_emulator};

/// The files that are taken out of the archive, compared without case (the
/// Windows archive spells the DLLs `AdbWinApi.dll`).
const NEEDED: &[&str] = &[
    "fastboot",
    "fastboot.exe",
    "AdbWinApi.dll",
    "AdbWinUsbApi.dll",
];

/// How long the download may take. The archive is around 8 MiB; the limit is
/// only there so that a stalled connection cannot hold the dialog forever.
const TIMEOUT: Duration = Duration::from_secs(300);

/// The archive Google publishes for this system, or `None` where there is none.
fn url() -> Option<&'static str> {
    const WINDOWS: &str =
        "https://dl.google.com/android/repository/platform-tools-latest-windows.zip";
    const LINUX: &str =
        "https://dl.google.com/android/repository/platform-tools-latest-linux.zip";
    const DARWIN: &str =
        "https://dl.google.com/android/repository/platform-tools-latest-darwin.zip";

    if cfg!(target_os = "windows") {
        Some(WINDOWS)
    } else if cfg!(target_os = "macos") {
        Some(DARWIN)
    } else if cfg!(target_os = "linux") {
        Some(LINUX)
    } else {
        None
    }
}

/// The `fastboot` executable of the unpacked archive.
fn binary() -> PathBuf {
    install_dir().join(if cfg!(target_os = "windows") {
        "fastboot.exe"
    } else {
        "fastboot"
    })
}

/// The directory the archive is unpacked into.
fn install_dir() -> PathBuf {
    config::config_dir().join("platform-tools")
}

/// What runs the downloaded build: Google's macOS archive is universal and its
/// Windows one runs on every edition, but the Linux one is x86_64 — which an
/// ARM Linux machine needs box64 for, exactly like the shipped `mfastboot`.
fn emulator() -> Emulator {
    if cfg!(target_os = "macos") {
        Emulator::Native
    } else {
        intel_emulator().unwrap_or(Emulator::Native)
    }
}

/// The engine entry for the picker, or `None` where Google publishes no
/// `fastboot` for this system.
///
/// It is built once when the dialog opens: the entry is what the picker
/// compares the selection with, and asking the system whether it can run the
/// build (Rosetta 2, box64) should not happen on every frame.
pub fn engine() -> Option<Mfastboot> {
    url()?;

    Some(Mfastboot {
        version: "latest".to_owned(),
        path: binary(),
        emulator: emulator(),
        origin: Origin::PlatformTools,
    })
}

/// Downloads the archive and unpacks `fastboot` out of it, returning the path
/// of the executable.
pub fn install() -> Result<PathBuf, String> {
    let url = url().ok_or_else(|| "no platform-tools build for this system".to_owned())?;

    let response = ureq::AgentBuilder::new()
        .timeout(TIMEOUT)
        .build()
        .get(url)
        .call()
        .map_err(|error| format!("download failed: {error}"))?;

    let mut archive = Vec::new();
    response
        .into_reader()
        .read_to_end(&mut archive)
        .map_err(|error| format!("download failed: {error}"))?;

    unpack(&archive, &install_dir())
}

/// Writes the files `fastboot` needs out of `archive` into `directory`, and
/// returns the executable's path.
///
/// Only the file names are looked at: everything in the archive lives under
/// `platform-tools/`, and nothing outside [`NEEDED`] is written.
fn unpack(archive: &[u8], directory: &Path) -> Result<PathBuf, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(archive))
        .map_err(|error| format!("unreadable archive: {error}"))?;

    std::fs::create_dir_all(directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("unreadable archive: {error}"))?;

        let name = match entry.name().rsplit(['/', '\\']).next() {
            Some(name) => name.to_owned(),
            None => continue,
        };

        if !NEEDED.iter().any(|needed| needed.eq_ignore_ascii_case(&name)) {
            continue;
        }

        let mut contents = Vec::new();
        entry
            .read_to_end(&mut contents)
            .map_err(|error| format!("unreadable archive entry {name}: {error}"))?;

        let target = directory.join(&name);
        std::fs::write(&target, &contents)
            .map_err(|error| format!("could not write {}: {error}", target.display()))?;

        // The executable bit is ours to set: the archive does not have it
        // everywhere, and a `fastboot` that cannot be started is of no use.
        #[cfg(unix)]
        if name.eq_ignore_ascii_case("fastboot") {
            use std::os::unix::fs::PermissionsExt;

            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))
                .map_err(|error| {
                    format!("could not make {} executable: {error}", target.display())
                })?;
        }
    }

    let binary = directory.join(if cfg!(target_os = "windows") {
        "fastboot.exe"
    } else {
        "fastboot"
    });

    if !binary.is_file() {
        return Err("the archive did not contain fastboot".to_owned());
    }

    Ok(binary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// An archive shaped like Google's: everything under `platform-tools/`.
    fn archive(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));

        for (name, contents) in entries {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(contents.as_bytes()).unwrap();
        }

        writer.finish().unwrap().into_inner()
    }

    /// A directory of its own for a test, removed first so leftovers from an
    /// interrupted run cannot pass for a result.
    fn test_dir(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("lmnflash-platform-tools-{name}"));
        let _ = std::fs::remove_dir_all(&directory);

        directory
    }

    #[test]
    fn unpacks_only_what_fastboot_needs() {
        let directory = test_dir("needed");

        let archive = archive(&[
            ("platform-tools/fastboot", "binary"),
            ("platform-tools/fastboot.exe", "binary"),
            ("platform-tools/AdbWinApi.dll", "helper"),
            ("platform-tools/AdbWinUsbApi.dll", "helper"),
            ("platform-tools/adb.exe", "not needed"),
            ("platform-tools/NOTICE.txt", "not needed"),
        ]);

        let binary = unpack(&archive, &directory).unwrap();

        assert_eq!(binary, directory.join(binary.file_name().unwrap()));
        assert!(binary.is_file());
        // The two DLLs `fastboot.exe` loads next to itself on Windows.
        assert!(directory.join("AdbWinApi.dll").is_file());
        assert!(directory.join("AdbWinUsbApi.dll").is_file());
        // The rest of the platform-tools stays in the archive.
        assert!(!directory.join("adb.exe").exists());
        assert!(!directory.join("NOTICE.txt").exists());

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// The executable bit is what makes the unpacked file usable on Unix, and
    /// the archive does not always carry it.
    #[cfg(unix)]
    #[test]
    fn makes_the_executable_runnable() {
        use std::os::unix::fs::PermissionsExt;

        let directory = test_dir("executable");
        let archive = archive(&[("platform-tools/fastboot", "binary")]);

        let binary = unpack(&archive, &directory).unwrap();
        let mode = std::fs::metadata(&binary).unwrap().permissions().mode();

        assert_eq!(mode & 0o111, 0o111, "mode {mode:o}");

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn rejects_an_archive_without_fastboot() {
        let directory = test_dir("empty");
        let archive = archive(&[("platform-tools/adb.exe", "not needed")]);

        assert!(unpack(&archive, &directory).is_err());

        let _ = std::fs::remove_dir_all(&directory);
    }
}
