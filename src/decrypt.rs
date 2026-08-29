//! Firmware-file decryption ("Firmware Decrypt" mode).
//!
//! Decrypts the encrypted resource files Motorola/Lenovo firmware dumps are
//! made of: every `*.x` file becomes `*.xml` and every `*.t` file becomes
//! `*.txt`, written next to the original file. This is a port of the Go
//! reference implementation in `reference/main.go`.
//!
//! File format: 16-byte AES-CBC IV, 16-byte PBKDF1 salt, ciphertext. The
//! decrypted payload is:
//!
//! ```text
//! u64 LE original size | 8-byte signature | body | SHA-256(body)
//! ```
//!
//! The key is derived from the password with PBKDF1 (SHA-256, 1000
//! iterations); the default password is `OSD`.

use aes::Aes256;
use cbc::cipher::{BlockDecryptMut, KeyIvInit, block_padding::NoPadding};
use cbc::Decryptor;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// Password used when the user does not provide a custom one.
pub const DEFAULT_PASSWORD: &str = "OSD";

/// PBKDF1 iteration count and derived key length used by the format.
const ITERATIONS: u32 = 1000;
const KEY_LENGTH: usize = 32;

/// AES block size (and the size of the IV/salt in the file header).
const BLOCK_SIZE: usize = 16;

/// Magic signature stored at offset 8 of the decrypted payload.
const SIGNATURE_MAGIC: [u8; 8] = [0xcf, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 0xfc];

/// Why a single file could not be decrypted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecryptError {
    /// The input file could not be read.
    Read(String),
    /// The file is smaller than the 32-byte header.
    TooShort,
    /// The file contains no ciphertext.
    Empty,
    /// The ciphertext length is not a multiple of the AES block size.
    NotBlockAligned,
    /// The decrypted header is malformed (body/hash truncated).
    BadHeader,
    /// The decrypted signature does not match the expected magic value.
    BadSignature,
    /// The SHA-256 of the body does not match the stored hash.
    HashMismatch,
    /// The output file could not be written.
    Write(String),
}

impl std::fmt::Display for DecryptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read(error) => write!(f, "failed to read file: {error}"),
            Self::TooShort => write!(f, "file is too short (missing header)"),
            Self::Empty => write!(f, "file contains no ciphertext"),
            Self::NotBlockAligned => {
                write!(f, "ciphertext is not a multiple of the AES block size")
            }
            Self::BadHeader => write!(f, "decrypted header is malformed"),
            Self::BadSignature => write!(f, "invalid signature (wrong password?)"),
            Self::HashMismatch => write!(f, "hash mismatch (wrong password?)"),
            Self::Write(error) => write!(f, "failed to write file: {error}"),
        }
    }
}

impl std::error::Error for DecryptError {}

/// Result of decrypting a whole directory tree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DecryptSummary {
    /// Number of encrypted files (`*.x`/`*.t`) found.
    pub total: usize,
    /// Number of files decrypted successfully.
    pub succeeded: usize,
    /// Files that could not be decrypted, with the reason.
    pub failed: Vec<(PathBuf, String)>,
}

/// PBKDF1-style key derivation, as used by the Go reference: the SHA-256 of
/// `password || salt` hashed `iterations` times in total, truncated.
fn pbkdf1(password: &[u8], salt: &[u8], length: usize, iterations: u32) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(password);
    hasher.update(salt);
    let mut digest = hasher.finalize().to_vec();

    for _ in 1..iterations {
        digest = Sha256::digest(&digest).to_vec();
    }

    digest.truncate(length);
    digest
}

/// Decrypts an in-memory encrypted file, returning the verified body.
fn decrypt_bytes(data: &[u8], password: &str) -> Result<Vec<u8>, DecryptError> {
    if data.len() < 32 {
        return Err(DecryptError::TooShort);
    }

    let iv: [u8; BLOCK_SIZE] = data[..BLOCK_SIZE].try_into().expect("sliced to block size");
    let salt: [u8; BLOCK_SIZE] = data[BLOCK_SIZE..32].try_into().expect("sliced to block size");
    let ciphertext = &data[32..];

    if ciphertext.is_empty() {
        return Err(DecryptError::Empty);
    }
    if ciphertext.len() % BLOCK_SIZE != 0 {
        return Err(DecryptError::NotBlockAligned);
    }

    let key = pbkdf1(password.as_bytes(), &salt, KEY_LENGTH, ITERATIONS);
    let mut key_bytes = [0u8; KEY_LENGTH];
    key_bytes.copy_from_slice(&key);

    let mut plain = ciphertext.to_vec();
    let decryptor = Decryptor::<Aes256>::new((&key_bytes).into(), (&iv).into());
    let plaintext = decryptor
        .decrypt_padded_mut::<NoPadding>(&mut plain)
        .map_err(|_| DecryptError::BadHeader)?;

    parse_payload(plaintext)
}

/// Parses the decrypted payload: length, signature, body, and SHA-256 check.
fn parse_payload(plaintext: &[u8]) -> Result<Vec<u8>, DecryptError> {
    if plaintext.len() < 16 + 32 {
        return Err(DecryptError::BadHeader);
    }

    let original_size = u64::from_le_bytes(plaintext[..8].try_into().expect("8 bytes")) as usize;

    if plaintext[8..16] != SIGNATURE_MAGIC {
        return Err(DecryptError::BadSignature);
    }

    let body_end = 16usize
        .checked_add(original_size)
        .ok_or(DecryptError::BadHeader)?;
    let hash_end = body_end.checked_add(32).ok_or(DecryptError::BadHeader)?;

    let body = plaintext.get(16..body_end).ok_or(DecryptError::BadHeader)?;
    let stored_hash = plaintext
        .get(body_end..hash_end)
        .ok_or(DecryptError::BadHeader)?;

    let computed_hash = Sha256::digest(body);
    if computed_hash.as_slice() != stored_hash {
        return Err(DecryptError::HashMismatch);
    }

    Ok(body.to_vec())
}

/// Decrypts `input` and writes the result to `output`, returning the number
/// of bytes written.
pub fn decrypt_file(input: &Path, output: &Path, password: &str) -> Result<u64, DecryptError> {
    let data = fs::read(input).map_err(|error| DecryptError::Read(error.to_string()))?;
    let body = decrypt_bytes(&data, password)?;
    fs::write(output, &body).map_err(|error| DecryptError::Write(error.to_string()))?;
    Ok(body.len() as u64)
}

/// The decrypted output path for an encrypted file, if it is one.
///
/// `*.x` becomes `*.xml` and `*.t` becomes `*.txt` (case-insensitive
/// extension matching); anything else returns `None`.
fn output_path_for(path: &Path) -> Option<PathBuf> {
    let extension = path.extension()?.to_str()?;

    let output_extension = match extension.to_ascii_lowercase().as_str() {
        "x" => "xml",
        "t" => "txt",
        _ => return None,
    };

    Some(path.with_extension(output_extension))
}

/// Recursively collects all encrypted files under `directory`, ignoring
/// unreadable entries so a single bad subdirectory does not abort the scan.
fn collect_encrypted_files(directory: &Path, files: &mut Vec<(PathBuf, PathBuf)>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };

    for entry in entries.flatten() {
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => continue,
        };

        // Symlinks are skipped to avoid cycles and out-of-tree writes.
        if file_type.is_symlink() {
            continue;
        }

        let path = entry.path();

        if file_type.is_dir() {
            collect_encrypted_files(&path, files);
        } else if file_type.is_file() {
            if let Some(output) = output_path_for(&path) {
                files.push((path, output));
            }
        }
    }
}

/// Recursively decrypts every `*.x` and `*.t` file under `directory`.
///
/// Each file is written next to its encrypted source; files that fail are
/// collected in the summary without aborting the rest of the batch.
pub fn decrypt_directory(directory: &Path, password: &str) -> Result<DecryptSummary, String> {
    let mut files = Vec::new();
    collect_encrypted_files(directory, &mut files);
    files.sort_by(|(a, _), (b, _)| a.cmp(b));

    let total = files.len();
    let mut succeeded = 0usize;
    let mut failed = Vec::new();

    for (input, output) in files {
        match decrypt_file(&input, &output, password) {
            Ok(_) => succeeded += 1,
            Err(error) => failed.push((input, error.to_string())),
        }
    }

    Ok(DecryptSummary {
        total,
        succeeded,
        failed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// Encrypts a payload the way the tool chain does, for round-trip tests.
    fn encrypt_payload(body: &[u8], password: &str) -> Vec<u8> {
        use cbc::Encryptor;
        use cbc::cipher::BlockEncryptMut;

        let iv = [0x5au8; BLOCK_SIZE];
        let salt = [0xa5u8; BLOCK_SIZE];

        let key = pbkdf1(password.as_bytes(), &salt, KEY_LENGTH, ITERATIONS);
        let mut key_bytes = [0u8; KEY_LENGTH];
        key_bytes.copy_from_slice(&key);

        let mut payload = Vec::with_capacity(16 + body.len() + 32);
        payload.extend_from_slice(&(body.len() as u64).to_le_bytes());
        payload.extend_from_slice(&SIGNATURE_MAGIC);
        payload.extend_from_slice(body);
        payload.extend_from_slice(&Sha256::digest(body));

        // Zero-pad to a full number of AES blocks (CBC, no padding scheme).
        let padding = (BLOCK_SIZE - payload.len() % BLOCK_SIZE) % BLOCK_SIZE;
        payload.extend(std::iter::repeat_n(0u8, padding));

        let encryptor = Encryptor::<Aes256>::new((&key_bytes).into(), (&iv).into());
        let mut plain = payload;
        let length = plain.len();
        let ciphertext = encryptor
            .encrypt_padded_mut::<NoPadding>(&mut plain, length)
            .expect("padded to block size");

        let mut file = Vec::with_capacity(32 + ciphertext.len());
        file.extend_from_slice(&iv);
        file.extend_from_slice(&salt);
        file.extend_from_slice(ciphertext);
        file
    }

    #[test]
    fn pbkdf1_matches_the_reference_implementation() {
        // Independently computed with Python hashlib from the Go algorithm:
        // SHA-256(b"OSD" || 16 zero bytes), then 999 more chained hashes.
        let key = pbkdf1(b"OSD", &[0u8; 16], KEY_LENGTH, ITERATIONS);

        assert_eq!(
            hex(&key),
            "732bcd16b49cfb0b1c1b8835bd123da4815534f835ed6dce6a08303474f74cd8"
        );
    }

    #[test]
    fn generated_file_round_trips() {
        let body = b"<?xml version=\"1.0\"?><firmware>hello</firmware>";
        let file = encrypt_payload(body, "OSD");

        assert_eq!(decrypt_bytes(&file, "OSD").unwrap(), body);
    }

    #[test]
    fn wrong_password_is_rejected() {
        let body = b"plain text body";
        let file = encrypt_payload(body, "OSD");

        assert!(decrypt_bytes(&file, "WRONG").is_err());
    }

    #[test]
    fn malformed_files_are_rejected() {
        // Too short for the 32-byte header.
        assert_eq!(decrypt_bytes(&[0u8; 31], "OSD"), Err(DecryptError::TooShort));

        // Valid header but ciphertext not block-aligned.
        let mut misaligned = vec![0u8; 32];
        misaligned.extend_from_slice(&[0u8; 15]);
        assert_eq!(
            decrypt_bytes(&misaligned, "OSD"),
            Err(DecryptError::NotBlockAligned)
        );

        // Valid header but no ciphertext.
        assert_eq!(decrypt_bytes(&[0u8; 32], "OSD"), Err(DecryptError::Empty));
    }

    #[test]
    fn output_paths_are_derived_from_extensions() {
        assert_eq!(
            output_path_for(Path::new("device/a.x")).as_deref(),
            Some(Path::new("device/a.xml"))
        );
        assert_eq!(
            output_path_for(Path::new("device/a.t")).as_deref(),
            Some(Path::new("device/a.txt"))
        );
        // Case-insensitive matching, lowercase output extension.
        assert_eq!(
            output_path_for(Path::new("device/A.X")).as_deref(),
            Some(Path::new("device/A.xml"))
        );
        // Non-matching extensions and extension-less files are ignored.
        assert_eq!(output_path_for(Path::new("device/a.bin")), None);
        assert_eq!(output_path_for(Path::new("device/README")), None);
    }

    #[test]
    fn directory_decrypt_recurses_and_reports_failures() {
        let root = std::env::temp_dir().join(format!("lmnflash-decrypt-test-{}", uuid()));
        fs::create_dir_all(root.join("sub")).expect("create test dirs");

        let good = b"<root><ok/></root>";
        fs::write(root.join("good.x"), encrypt_payload(good, "OSD")).expect("write good.x");
        fs::write(root.join("sub/also.t"), encrypt_payload(b"note", "OSD"))
            .expect("write also.t");
        // Not an encrypted file, but matches the extension: it must be
        // reported as failed instead of aborting the batch.
        fs::write(root.join("bad.x"), b"garbage").expect("write bad.x");
        // Unrelated extension: ignored entirely.
        fs::write(root.join("skip.bin"), b"ignore").expect("write skip.bin");

        let summary = decrypt_directory(&root, "OSD").expect("directory scan");
        fs::remove_dir_all(&root).expect("clean up test dirs");

        assert_eq!(summary.total, 3);
        assert_eq!(summary.succeeded, 2);
        assert_eq!(summary.failed.len(), 1);
        assert_eq!(summary.failed[0].0, root.join("bad.x"));
    }

    /// Unique-enough suffix for temp dirs, without pulling in a UUID crate.
    fn uuid() -> String {
        format!("{:?}", std::time::SystemTime::now())
            .replace([' ', ':', '.'], "_")
            .trim_start_matches('_')
            .to_string()
    }
}
