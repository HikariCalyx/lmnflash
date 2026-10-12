//! Where the logs a flash writes go.
//!
//! Both flashing modes keep what they did under `Documents\lmnflash\logs`: the
//! multi-device workspace in a directory per run (`multiflash_<moment>`, one
//! file per phone), a single-device flash in one file named after the phone it
//! flashed and the moment it started
//! (`single_flash_<serial>_<moment>.log`).
//!
//! The documents directory is where a user looks for what they ran by hand; a
//! machine that has none (a Linux account with no XDG directories) falls back
//! to the configuration directory, so the log is at least somewhere the app
//! owns.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::Local;

/// The directory both modes write their logs into.
pub(crate) fn logs_root() -> PathBuf {
    let base = directories::UserDirs::new()
        .and_then(|dirs| dirs.document_dir().map(Path::to_path_buf))
        .unwrap_or_else(crate::config::config_dir);

    base.join("lmnflash").join("logs")
}

/// The moment a log was started, written the way a file name can hold it.
pub(crate) fn timestamp() -> String {
    Local::now().format("%Y%m%d_%H%M%S").to_string()
}

/// The file a single-device flash writes:
/// `single_flash_<serial>_<timestamp>.log`.
pub(crate) fn single_flash_path(serial: &str) -> PathBuf {
    logs_root().join(format!(
        "single_flash_{}_{}.log",
        file_name_part(serial),
        timestamp()
    ))
}

/// A string that is safe to use as part of a file name: a serial number comes
/// from the device, and a file name may not hold everything a device may say.
pub(crate) fn file_name_part(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect();

    if cleaned.is_empty() {
        "unknown".to_owned()
    } else {
        cleaned
    }
}

/// A log file: every line carries the local time it was written at.
///
/// A file that cannot be created is not worth failing a flash over — the run
/// goes on without it, and the UI shows every line either way.
#[derive(Debug)]
pub(crate) struct FlashLog {
    path: PathBuf,
    file: Option<File>,
}

impl FlashLog {
    /// Creates the file, and the directory it goes into: a single-device flash
    /// has nothing else to make it, and the multi-device run makes its own
    /// directory before it writes its first device log.
    pub(crate) fn create(path: PathBuf) -> Self {
        if let Some(directory) = path.parent() {
            let _ = fs::create_dir_all(directory);
        }

        Self {
            file: File::create(&path).ok(),
            path,
        }
    }

    /// Where this log is written, whether or not the file could be opened.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Writes one stamped line.
    pub(crate) fn line(&mut self, line: &str) {
        let Some(file) = self.file.as_mut() else {
            return;
        };

        let _ = writeln!(
            file,
            "[{}] {line}",
            Local::now().format("%Y-%m-%d %H:%M:%S")
        );
        // Flushed per line: the log is what says what happened when a flash
        // goes wrong, and a run that is interrupted must not lose the last of
        // it.
        let _ = file.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of this test's own, under the temporary directory.
    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "lmnflash-flash-log-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);

        directory
    }

    /// The time a log is named after, spelled the way the file names of both
    /// modes expect: `yyyyMMdd_HHmmss`.
    fn assert_stamp(stamp: &str) {
        assert_eq!(stamp.len(), 15, "{stamp}");
        assert_eq!(stamp.as_bytes()[8], b'_', "{stamp}");
        assert_eq!(
            stamp.chars().filter(char::is_ascii_digit).count(),
            14,
            "{stamp}"
        );
    }

    /// A serial the device reported becomes a file name.
    #[test]
    fn a_serial_becomes_a_file_name() {
        assert_eq!(file_name_part("ZY12345678"), "ZY12345678");
        // `:*?` and a path separator are not a file name on every platform.
        assert_eq!(file_name_part("AB:C/D*E?F"), "AB_C_D_E_F");
        assert_eq!(file_name_part(""), "unknown");
    }

    /// The log of a single-device flash sits with the other logs, named after
    /// the phone and the moment it started.
    #[test]
    fn a_single_flash_log_is_named_after_the_phone() {
        let path = single_flash_path("ZY12345678");

        assert_eq!(path.parent(), Some(logs_root().as_path()));

        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let stamp = name
            .strip_prefix("single_flash_ZY12345678_")
            .and_then(|rest| rest.strip_suffix(".log"))
            .expect("the file is named after the phone and the moment");

        assert_stamp(stamp);
        // A serial is a device's to choose: it may not reach the file name
        // as it is.
        assert!(
            single_flash_path("AB:C/D")
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .starts_with("single_flash_AB_C_D_")
        );
    }

    /// Every line says when it was written, and is on disk before the next one
    /// is written — a log read while the flash runs may not be behind.
    #[test]
    fn every_line_carries_the_time_it_was_written() {
        let directory = scratch("lines");
        let path = directory.join("single_flash_ZY12345678_20261012_101112.log");

        let mut log = FlashLog::create(path.clone());
        log.line("flash super (super.img)");
        log.line("finished");

        let written = fs::read_to_string(&path).expect("the log was written");
        let lines: Vec<&str> = written.lines().collect();

        assert_eq!(lines.len(), 2, "{written}");
        assert!(lines[0].ends_with("] flash super (super.img)"), "{written}");
        assert!(lines[1].ends_with("] finished"), "{written}");

        for line in lines {
            let stamp = line
                .strip_prefix('[')
                .and_then(|rest| rest.split_once("] "))
                .map(|(stamp, _)| stamp)
                .unwrap_or_default();

            // `yyyy-MM-dd HH:mm:ss`.
            assert_eq!(stamp.len(), 19, "{written}");
            assert_eq!(stamp.as_bytes()[10], b' ', "{written}");
        }

        assert_eq!(log.path(), path.as_path());
        let _ = fs::remove_dir_all(&directory);
    }

    /// The directory is made when it is not there yet: nothing else creates it
    /// for a single-device flash, and a log that is lost to a missing folder is
    /// a log the user cannot send on.
    #[test]
    fn the_directory_is_made_for_the_log() {
        let directory = scratch("directory").join("logs");
        let path = directory.join("single_flash_ZY12345678_20261012_101112.log");

        let mut log = FlashLog::create(path.clone());
        log.line("flashing");

        assert!(path.is_file(), "{}", path.display());
        let _ = fs::remove_dir_all(directory.parent().unwrap_or(&directory));
    }
}
