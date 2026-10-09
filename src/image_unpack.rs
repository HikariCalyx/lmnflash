//! Unpacking Motorola images whose header magic is `SINGLE_N_LONELY`.
//!
//! Such an image (for example `radio.img`) is a small container: a 256-byte
//! header whose magic names the format, followed by up to 64 directory
//! entries. Every entry is a 248-byte file name and a little-endian `u64`
//! size, immediately followed by the file's payload and padding that aligns
//! the next entry to 4096 bytes. The table ends with an entry whose name is
//! `LONELY_N_SINGLE`.
//!
//! ```text
//! "SINGLE_N_LONELY\0…" (256) | name (248) | u64 size | payload | pad | … |
//! "LONELY_N_SINGLE" (248) | 8 bytes
//! ```
//!
//! This is a port of `reference/unpack-moto-img.py`.

use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

/// Bytes at the start of the header that name the format.
pub const HEADER_MAGIC: &[u8] = b"SINGLE_N_LONELY";

/// Name of the directory entry that ends the table.
const TERMINATOR_NAME: &[u8] = b"LONELY_N_SINGLE";

/// Size of the fixed header at the start of an image.
const HEADER_SIZE: u64 = 256;

/// Size of a directory entry's name field and of its size field.
const NAME_SIZE: usize = 248;
const SIZE_SIZE: usize = 8;

/// Most images hold far fewer files; the reference stops at 64 as well.
const MAX_ENTRIES: usize = 64;

/// Payloads are padded so the next entry starts on this boundary.
const ALIGNMENT: u64 = 4096;

/// Bytes copied at a time while a payload is written out.
const COPY_BUFFER: usize = 64 * 1024;

/// A file stored inside an image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The file name, as it will be written to the output directory.
    pub name: String,
    /// Payload size in bytes.
    pub size: u64,
    /// Padding that follows the payload, to the next 4096-byte boundary.
    pub padding: u64,
}

/// Why an image could not be read or unpacked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnpackError {
    /// The image could not be read.
    Read(String),
    /// The header does not carry the expected magic: not this format.
    NotAnImage,
    /// The directory table is truncated or otherwise malformed.
    BadTable,
    /// An entry names something that is not a plain file.
    BadName(String),
    /// A file could not be written.
    Write(String),
}

impl std::fmt::Display for UnpackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read(error) => write!(f, "failed to read the image: {error}"),
            Self::NotAnImage => write!(f, "not a Motorola SINGLE_N_LONELY image"),
            Self::BadTable => write!(f, "the image directory is truncated or malformed"),
            Self::BadName(name) => write!(f, "`{name}` is not a usable file name"),
            Self::Write(error) => write!(f, "failed to write the files: {error}"),
        }
    }
}

impl std::error::Error for UnpackError {}

/// What a successful run produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    /// Number of files written.
    pub files: usize,
    /// Total number of payload bytes written.
    pub bytes: u64,
}

/// Progress reported while an image is unpacked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnpackEvent {
    /// A file is about to be written (`index` counts from 0).
    File { index: usize, name: String, size: u64 },
    /// `bytes` payload bytes have been written so far (across all files).
    Progress { bytes: u64, total: u64 },
    /// The run is over. Always the last event: the worker reports its outcome
    /// itself, so the dialog can never wait for an event that does not come.
    Finished(Result<Summary, UnpackError>),
}

/// Removes the NUL padding from a fixed-size name or header field.
fn strip_nuls(field: &[u8]) -> &[u8] {
    let end = field
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(field.len());
    &field[..end]
}

/// The padding that follows a payload of `size` bytes.
fn padding_for(size: u64) -> u64 {
    let remainder = size % ALIGNMENT;

    if remainder == 0 {
        0
    } else {
        ALIGNMENT - remainder
    }
}

/// Reads the fixed header and checks its magic.
fn read_header(file: &mut File) -> Result<(), UnpackError> {
    let mut header = [0u8; HEADER_SIZE as usize];
    file.read_exact(&mut header).map_err(|error| match error.kind() {
        // A file that is shorter than the header cannot be one of ours.
        io::ErrorKind::UnexpectedEof => UnpackError::NotAnImage,
        _ => UnpackError::Read(error.to_string()),
    })?;

    if strip_nuls(&header) != HEADER_MAGIC {
        return Err(UnpackError::NotAnImage);
    }

    Ok(())
}

/// Reads one directory entry, or `None` when the table's terminator is
/// reached. The caller's position ends up right after the entry's header.
fn read_entry(file: &mut File) -> Result<Option<Entry>, UnpackError> {
    let mut name_field = [0u8; NAME_SIZE];
    file.read_exact(&mut name_field).map_err(|error| match error.kind() {
        // Running out of file here means the table never ended.
        io::ErrorKind::UnexpectedEof => UnpackError::BadTable,
        _ => UnpackError::Read(error.to_string()),
    })?;

    let name_bytes = strip_nuls(&name_field);

    let mut size_field = [0u8; SIZE_SIZE];
    file.read_exact(&mut size_field).map_err(|error| match error.kind() {
        io::ErrorKind::UnexpectedEof => UnpackError::BadTable,
        _ => UnpackError::Read(error.to_string()),
    })?;

    if name_bytes == TERMINATOR_NAME {
        return Ok(None);
    }

    let name = String::from_utf8_lossy(name_bytes).into_owned();

    if name.is_empty() {
        return Err(UnpackError::BadTable);
    }

    let size = u64::from_le_bytes(size_field);

    Ok(Some(Entry {
        name,
        size,
        padding: padding_for(size),
    }))
}

/// Skips over an entry's payload and padding, for a pass that only reads the
/// table.
fn skip_entry(file: &mut File, entry: &Entry) -> Result<(), UnpackError> {
    seek_forward(file, entry.size + entry.padding)
}

/// Skips the padding that follows a payload which was just copied.
fn skip_padding(file: &mut File, entry: &Entry) -> Result<(), UnpackError> {
    seek_forward(file, entry.padding)
}

/// Moves the file position forward by `bytes`.
fn seek_forward(file: &mut File, bytes: u64) -> Result<(), UnpackError> {
    let advance = i64::try_from(bytes).map_err(|_| UnpackError::BadTable)?;

    file.seek(SeekFrom::Current(advance))
        .map_err(|error| UnpackError::Read(error.to_string()))?;

    Ok(())
}

/// Reads the directory of an image without copying any payload.
///
/// This is what the dialog shows after a file was picked: it both validates
/// the header and lists what is inside before anything is written.
pub fn list(image: &Path) -> Result<Vec<Entry>, UnpackError> {
    let mut file = File::open(image).map_err(|error| UnpackError::Read(error.to_string()))?;

    read_header(&mut file)?;
    file.seek(SeekFrom::Start(HEADER_SIZE))
        .map_err(|error| UnpackError::Read(error.to_string()))?;

    let mut entries = Vec::new();

    for _ in 0..MAX_ENTRIES {
        let Some(entry) = read_entry(&mut file)? else {
            return Ok(entries);
        };

        skip_entry(&mut file, &entry)?;
        entries.push(entry);
    }

    // 64 entries and still no terminator: the table is not one of ours.
    Err(UnpackError::BadTable)
}

/// A file name that can safely be written into the output directory.
///
/// Entry names come from the image, so anything that could escape the chosen
/// directory (a path separator, `.`, `..`) is refused instead of followed.
fn safe_file_name(name: &str) -> Result<&str, UnpackError> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\'])
        || name.contains(':')
    {
        return Err(UnpackError::BadName(name.to_string()));
    }

    Ok(name)
}

/// The name and spelling of each file, for the log and the throttle below.
fn copy_payload(
    file: &mut File,
    entry: &Entry,
    output: &Path,
    written: &mut u64,
    total: u64,
    emit: &dyn Fn(UnpackEvent),
    last_reported: &mut u64,
) -> Result<(), UnpackError> {
    let mut output = File::create(output).map_err(|error| UnpackError::Write(error.to_string()))?;

    let mut remaining = entry.size;
    let mut buffer = vec![0u8; COPY_BUFFER.min(entry.size.max(1) as usize)];

    while remaining > 0 {
        let chunk = remaining.min(buffer.len() as u64) as usize;
        file.read_exact(&mut buffer[..chunk])
            .map_err(|error| UnpackError::Read(error.to_string()))?;
        output
            .write_all(&buffer[..chunk])
            .map_err(|error| UnpackError::Write(error.to_string()))?;

        remaining -= chunk as u64;
        *written += chunk as u64;

        // Report at most every permille of the payload, like the flash engine
        // reports its own progress.
        if total > 0 {
            let permille = *written * 1000 / total;
            if permille != *last_reported {
                *last_reported = permille;
                emit(UnpackEvent::Progress {
                    bytes: *written,
                    total,
                });
            }
        }
    }

    output
        .flush()
        .map_err(|error| UnpackError::Write(error.to_string()))?;

    Ok(())
}

/// Extracts every file of an image into `destination`.
///
/// The directory is created when missing. Progress is reported through `emit`,
/// which always receives [`UnpackEvent::Finished`] last.
pub fn unpack(
    image: &Path,
    destination: &Path,
    emit: &dyn Fn(UnpackEvent),
) -> Result<Summary, UnpackError> {
    let result = unpack_inner(image, destination, emit);

    // The outcome is reported in exactly one place, so no early return can
    // skip it and leave the dialog waiting.
    emit(UnpackEvent::Finished(result.clone()));

    result
}

fn unpack_inner(
    image: &Path,
    destination: &Path,
    emit: &dyn Fn(UnpackEvent),
) -> Result<Summary, UnpackError> {
    let entries = list(image)?;
    let total: u64 = entries.iter().map(|entry| entry.size).sum();

    fs::create_dir_all(destination).map_err(|error| UnpackError::Write(error.to_string()))?;

    let mut file = File::open(image).map_err(|error| UnpackError::Read(error.to_string()))?;
    read_header(&mut file)?;
    file.seek(SeekFrom::Start(HEADER_SIZE))
        .map_err(|error| UnpackError::Read(error.to_string()))?;

    let mut written = 0u64;
    let mut reported = 0u64;
    let mut files = 0usize;

    for index in 0..entries.len() {
        let entry = read_entry(&mut file)?.ok_or(UnpackError::BadTable)?;
        safe_file_name(&entry.name)?;

        emit(UnpackEvent::File {
            index,
            name: entry.name.clone(),
            size: entry.size,
        });

        let output = destination.join(&entry.name);
        copy_payload(
            &mut file,
            &entry,
            &output,
            &mut written,
            total,
            emit,
            &mut reported,
        )?;

        skip_padding(&mut file, &entry)?;
        files += 1;
    }

    // Make sure the bar reaches its end even for a tiny payload.
    emit(UnpackEvent::Progress { bytes: total, total });

    Ok(Summary {
        files,
        bytes: total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds an image the way the reference script reads it.
    fn build_image(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut image = vec![0u8; HEADER_SIZE as usize];
        image[..HEADER_MAGIC.len()].copy_from_slice(HEADER_MAGIC);

        for (name, payload) in entries {
            let mut name_field = [0u8; NAME_SIZE];
            name_field[..name.len()].copy_from_slice(name.as_bytes());
            image.extend_from_slice(&name_field);
            image.extend_from_slice(&(payload.len() as u64).to_le_bytes());
            image.extend_from_slice(payload);

            let padding = padding_for(payload.len() as u64);
            image.extend(vec![0u8; padding as usize]);
        }

        let mut terminator = [0u8; NAME_SIZE];
        terminator[..TERMINATOR_NAME.len()].copy_from_slice(TERMINATOR_NAME);
        image.extend_from_slice(&terminator);
        image.extend_from_slice(&0u64.to_le_bytes());

        image
    }

    fn write_image(directory: &Path, bytes: &[u8]) -> std::path::PathBuf {
        let path = directory.join("radio.img");
        std::fs::write(&path, bytes).expect("write image");
        path
    }

    fn temp_directory(name: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "lmnflash-image-unpack-test-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("create temp dir");
        directory
    }

    #[test]
    fn the_directory_is_read_without_copying_the_payloads() {
        let directory = temp_directory("list");
        let image = write_image(&directory, &build_image(&[("modem.img", b"abc"), ("fsg.img", &[7u8; 5000])]));

        let entries = list(&image).expect("list");

        assert_eq!(
            entries,
            vec![
                Entry {
                    name: "modem.img".to_string(),
                    size: 3,
                    padding: 4093,
                },
                Entry {
                    name: "fsg.img".to_string(),
                    size: 5000,
                    padding: 3192,
                },
            ]
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_file_without_the_magic_is_rejected() {
        let directory = temp_directory("no-magic");
        let image = directory.join("plain.img");
        std::fs::write(&image, vec![0u8; 1024]).expect("write");

        assert_eq!(list(&image), Err(UnpackError::NotAnImage));

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn unpacking_writes_every_file_and_reports_its_progress() {
        use std::cell::RefCell;

        let directory = temp_directory("unpack");
        let payload: Vec<u8> = (0..9000u32).map(|value| value as u8).collect();
        let image = write_image(
            &directory,
            &build_image(&[("modem.img", b"abc"), ("fsg.img", &payload)]),
        );
        let output = directory.join("out");

        let events = RefCell::new(Vec::new());
        let summary = unpack(&image, &output, &|event| events.borrow_mut().push(event))
            .expect("unpack");

        assert_eq!(
            summary,
            Summary {
                files: 2,
                bytes: 9003,
            }
        );
        assert_eq!(std::fs::read(output.join("modem.img")).unwrap(), b"abc");
        assert_eq!(std::fs::read(output.join("fsg.img")).unwrap(), payload);

        let events = events.into_inner();
        assert!(matches!(
            events.last(),
            Some(UnpackEvent::Finished(Ok(summary))) if summary.files == 2
        ));
        assert!(events.iter().any(|event| matches!(
            event,
            UnpackEvent::File { name, .. } if name == "fsg.img"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            UnpackEvent::Progress { bytes, total } if *bytes == 9003 && *total == 9003
        )));

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// The outcome is the last event even when the job fails early, so a
    /// dialog that waits for it can never hang.
    #[test]
    fn a_failed_run_still_reports_its_outcome() {
        use std::cell::RefCell;

        let directory = temp_directory("failed");
        let image = directory.join("not-an-image.img");
        std::fs::write(&image, b"hello").expect("write");

        let events = RefCell::new(Vec::new());
        let result = unpack(&image, &directory.join("out"), &|event| {
            events.borrow_mut().push(event)
        });

        assert_eq!(result, Err(UnpackError::NotAnImage));
        assert_eq!(
            events.into_inner().last(),
            Some(&UnpackEvent::Finished(Err(UnpackError::NotAnImage)))
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// A name that would escape the output directory is refused instead of
    /// written somewhere else.
    #[test]
    fn names_that_escape_the_output_directory_are_refused() {
        assert_eq!(safe_file_name("../evil"), Err(UnpackError::BadName("../evil".to_string())));
        assert_eq!(
            safe_file_name("nested/evil"),
            Err(UnpackError::BadName("nested/evil".to_string()))
        );
        assert_eq!(
            safe_file_name("C:evil"),
            Err(UnpackError::BadName("C:evil".to_string()))
        );
        assert_eq!(safe_file_name(".."), Err(UnpackError::BadName("..".to_string())));
        assert_eq!(safe_file_name("modem.img"), Ok("modem.img"));
    }

    #[test]
    fn padding_aligns_payloads_to_four_kilobytes() {
        assert_eq!(padding_for(0), 0);
        assert_eq!(padding_for(1), 4095);
        assert_eq!(padding_for(4096), 0);
        assert_eq!(padding_for(4097), 4095);
        assert_eq!(padding_for(5000), 3192);
        assert_eq!(padding_for(9000), 3288);
    }
}
