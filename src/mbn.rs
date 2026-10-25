//! What a Qualcomm programmer is signed against.
//!
//! A programmer — the `xbl_s_devprg_ns.melf`, `prog_firehose_ddr.elf` or
//! `prog_firehose_ddr.mbn` a phone in EDL is given — is a signed image: a hash
//! segment header, the metadata that names the SoC and the OEM, the code, the
//! signature, and then the three certificates of its chain, DER-encoded one
//! after another and padded with `0xFF` to the end of the segment. The last of
//! them is the OEM's **root** certificate, and the SHA-256 (or SHA-384) of its
//! bytes is the value the phone reports as its `PK_HASH`: the two have to be
//! equal for the phone to accept the programmer.
//!
//! Nothing here signs or checks a signature — the boot ROM does that. This
//! reads the root certificate out of the chain so the page can compare it with
//! what the boot ROM said, which is what tells a user that the programmer they
//! picked belongs to the phone in front of them.

use sha2::{Digest, Sha256, Sha384};

/// The magic a bare MBN starts with, little-endian.
const MBN_MAGIC: u32 = 0x844b_dcd1;

/// How a hash segment marks itself in an ELF: no `p_type`, and the "NAND /
/// demand page / execute-only" flag word the boot ROM looks for.
const HASH_SEGMENT_MASK: u32 = 0x0f00_0000;
const HASH_SEGMENT_FLAGS: u32 = 0x0200_0000;

/// What the certificate chain is padded with.
const PADDING: u8 = 0xff;

/// Which hash the phone's own `PK_HASH` is: a phone with a 48-byte hash signs
/// with SHA-384, every other one with SHA-256.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hash {
    Sha256,
    Sha384,
}

impl Hash {
    /// The hash an image of `length` bytes stands for.
    pub fn of_length(length: usize) -> Self {
        if length == 48 { Self::Sha384 } else { Self::Sha256 }
    }

    fn digest(self, bytes: &[u8]) -> Vec<u8> {
        match self {
            Self::Sha256 => Sha256::digest(bytes).to_vec(),
            Self::Sha384 => Sha384::digest(bytes).to_vec(),
        }
    }
}

/// Reads the hash of the root certificate `image` is signed against.
///
/// `image` is the programmer as the page has it — a `.elf` or `.mbn` on its own,
/// or the `.melf` container that several platforms ship: an ELF whose *inner*
/// images are ELFs of their own, and only some of which carry a chain. Every
/// ELF in the buffer is tried in turn, and the first one whose chain can be
/// read answers.
pub fn root_hash(image: &[u8], hash: Hash) -> Result<Vec<u8>, String> {
    let mut reason = String::from("it is neither an ELF nor an MBN");
    let mut found = false;

    for base in elf_offsets(image) {
        found = true;

        let segment = match hash_segment(image, base) {
            Ok(segment) => segment,
            Err(error) => {
                reason = error;
                continue;
            }
        };

        match segment_root(image, segment, hash) {
            Ok(root) => return Ok(root),
            Err(error) => reason = error,
        }
    }

    // A bare MBN has no ELF around it: the file itself is the hash segment.
    if !found && image.len() >= 4 && le_u32(image, 0)? == MBN_MAGIC {
        return segment_root(image, 0, hash);
    }

    Err(reason)
}

/// Every ELF header in `image`, at the offset it starts at.
///
/// The first is the file's own; a container carries the images it holds behind
/// it, and the chains live in those.
fn elf_offsets(image: &[u8]) -> Vec<usize> {
    let mut offsets = Vec::new();
    let mut from = 0;

    while from + 4 <= image.len() {
        let Some(index) = image[from..]
            .windows(4)
            .position(|window| window == b"\x7fELF")
        else {
            break;
        };

        offsets.push(from + index);
        from += index + 1;
    }

    offsets
}

/// Where the hash segment of the ELF at `base` begins, as an offset into
/// `image`.
fn hash_segment(image: &[u8], base: usize) -> Result<usize, String> {
    if image.len() < base + 0x34 {
        return Err("its ELF header is cut short".to_owned());
    }

    // A 64-bit ELF header is laid out differently from a 32-bit one: the
    // offsets and the flag word of the program headers move.
    let wide = image[base + 4] == 2;
    let (table, entry_size, count) = if wide {
        (
            le_u64(image, base + 0x20)? as usize,
            le_u16(image, base + 0x36)? as usize,
            le_u16(image, base + 0x38)? as usize,
        )
    } else {
        (
            le_u32(image, base + 0x1c)? as usize,
            le_u16(image, base + 0x2a)? as usize,
            le_u16(image, base + 0x2c)? as usize,
        )
    };

    for index in 0..count {
        let entry = base + table + index * entry_size;
        let (kind, flags, offset) = if wide {
            (
                le_u32(image, entry)?,
                le_u32(image, entry + 4)?,
                le_u64(image, entry + 8)? as usize,
            )
        } else {
            (
                le_u32(image, entry)?,
                le_u32(image, entry + 0x18)?,
                le_u32(image, entry + 4)? as usize,
            )
        };

        if kind == 0 && flags & HASH_SEGMENT_MASK == HASH_SEGMENT_FLAGS {
            // The segment's offset is measured from its own ELF's start.
            return Ok(base + offset);
        }
    }

    Err("its ELF carries no hash segment".to_owned())
}

/// The header a hash segment begins with: its version, the sizes its own
/// offsets are worked out from, and four values the newer layouts add.
struct Header {
    version: u32,
    /// Three sizes the version 7 layout measures the chain with.
    hdr1: u32,
    hdr2: u32,
    hdr3: u32,
    /// The hash table's size in the version 7 layout.
    hdr4: u32,
    code_size: usize,
    signature_size: usize,
}

impl Header {
    fn of(image: &[u8], at: usize) -> Result<Self, String> {
        Ok(Self {
            version: le_u32(image, at + 0x04)?,
            hdr1: le_u32(image, at + 0x08)?,
            hdr2: le_u32(image, at + 0x0c)?,
            hdr3: le_u32(image, at + 0x10)?,
            code_size: le_u32(image, at + 0x14)? as usize,
            hdr4: le_u32(image, at + 0x18)?,
            signature_size: le_u32(image, at + 0x1c)? as usize,
        })
    }

    /// Where the certificate chain begins, in the file.
    ///
    /// The three layouts measured it differently as the format grew: the old
    /// one puts it right after the header, the code and the signature; version
    /// 6 adds the metadata block the header now names; version 7 sizes it from
    /// its own fields as well.
    fn chain_offset(&self, image: &[u8], at: usize) -> Result<usize, String> {
        let offset = match self.version {
            0..=5 => at + 0x28 + self.code_size + self.signature_size,
            6 => {
                let metadata = le_u32(image, at + 0x2c)? as usize;
                at + 0x30 + metadata + self.code_size + self.signature_size
            }
            7 => {
                let metadata = le_u32(image, at + 0x2c)? as usize;
                at + 0x28
                    + self.hdr1 as usize
                    + self.hdr2 as usize
                    + self.hdr3 as usize
                    + metadata
                    + self.code_size
                    + self.hdr4 as usize
            }
            other => return Err(format!("its hash segment has version {other}, which is not known")),
        };

        if offset >= image.len() {
            return Err(format!(
                "its certificate chain would start at {offset:#x}, past the {} bytes it holds",
                image.len()
            ));
        }

        Ok(offset)
    }
}

/// The hash of the root certificate of the chain that follows the hash segment
/// at `at`.
fn segment_root(image: &[u8], at: usize, hash: Hash) -> Result<Vec<u8>, String> {
    let chain = certificate_chain(image, at)?;

    Ok(hash.digest(roots(chain)))
}

/// The three certificates of the chain, from its first byte on.
///
/// Every certificate is one DER `SEQUENCE`, so the length in its own header
/// says where the next one starts. The root is the third.
fn certificate_chain(image: &[u8], at: usize) -> Result<&[u8], String> {
    let header = Header::of(image, at)?;
    let offset = header.chain_offset(image, at)?;
    let chain = &image[offset..];

    let first = sequence_length(chain, 0)?;
    let second_at = first;
    let second = sequence_length(chain, second_at)?;
    let root_at = second_at + second;

    if root_at >= chain.len() {
        return Err("its chain ends before the root certificate".to_owned());
    }

    Ok(&chain[root_at..])
}

/// The length of one DER `SEQUENCE` — its header included — in `chain` at
/// `at`.
fn sequence_length(chain: &[u8], at: usize) -> Result<usize, String> {
    if chain.get(at) != Some(&0x30) {
        return Err(format!(
            "the certificate at {at:#x} is not a DER sequence"
        ));
    }

    let first = *chain
        .get(at + 1)
        .ok_or_else(|| "a certificate header is cut short".to_owned())?;

    // A length below 0x80 is the length itself; above it, the low seven bits
    // say how many bytes carry it.
    if first & 0x80 == 0 {
        return Ok(2 + first as usize);
    }

    let count = (first & 0x7f) as usize;
    if count == 0 || count > 4 {
        return Err(format!("a certificate has a {count}-byte length, which is not DER"));
    }

    let mut length = 0_usize;
    for index in 0..count {
        let byte = *chain
            .get(at + 2 + index)
            .ok_or_else(|| "a certificate length is cut short".to_owned())?;
        length = (length << 8) | byte as usize;
    }

    Ok(2 + count + length)
}

/// The root certificate's bytes: everything after the second certificate, up
/// to the `0xFF` the segment is padded with.
fn roots(after_second: &[u8]) -> &[u8] {
    match after_second
        .windows(9)
        .position(|window| window.iter().all(|byte| *byte == PADDING))
    {
        Some(end) => &after_second[..end],
        None => after_second,
    }
}

fn le_u16(image: &[u8], at: usize) -> Result<u16, String> {
    let bytes = image
        .get(at..at + 2)
        .ok_or_else(|| format!("the programmer ends before offset {at:#x}"))?;

    Ok(u16::from_le_bytes(bytes.try_into().unwrap()))
}

fn le_u32(image: &[u8], at: usize) -> Result<u32, String> {
    let bytes = image
        .get(at..at + 4)
        .ok_or_else(|| format!("the programmer ends before offset {at:#x}"))?;

    Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
}

fn le_u64(image: &[u8], at: usize) -> Result<u64, String> {
    let bytes = image
        .get(at..at + 8)
        .ok_or_else(|| format!("the programmer ends before offset {at:#x}"))?;

    Ok(u64::from_le_bytes(bytes.try_into().unwrap()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One DER sequence: the tag, the short-form length and the body.
    fn sequence(body: &[u8]) -> Vec<u8> {
        assert!(body.len() < 0x80, "the tests only need short forms");

        let mut out = vec![0x30, body.len() as u8];
        out.extend_from_slice(body);
        out
    }

    /// An ELF of the test's own: one hash segment, carrying `chain`.
    fn elf(chain: &[u8], code_size: u32, signature_size: u32) -> Vec<u8> {
        let segment = 0x100;
        // The headers, then the segment the chain follows.
        let mut image =
            vec![0_u8; segment + 0x28 + code_size as usize + signature_size as usize];

        // The ELF header names one program header, at 0x34.
        image[..4].copy_from_slice(b"\x7fELF");
        image[4] = 1; // 32-bit
        image[0x1c..0x20].copy_from_slice(&0x34_u32.to_le_bytes());
        image[0x2a..0x2c].copy_from_slice(&0x20_u16.to_le_bytes());
        image[0x2c..0x2e].copy_from_slice(&1_u16.to_le_bytes());

        // Which is the hash segment: no type, and the flag word the boot ROM
        // looks for, its bytes at 0x100.
        image[0x34..0x38].copy_from_slice(&0_u32.to_le_bytes());
        image[0x38..0x3c].copy_from_slice(&(segment as u32).to_le_bytes());
        image[0x4c..0x50].copy_from_slice(&HASH_SEGMENT_FLAGS.to_le_bytes());

        // A version 5 hash segment: the chain follows the header, the code and
        // the signature.
        image[0x100 + 0x04..0x100 + 0x08].copy_from_slice(&5_u32.to_le_bytes());
        image[0x100 + 0x14..0x100 + 0x18].copy_from_slice(&code_size.to_le_bytes());
        image[0x100 + 0x1c..0x100 + 0x20].copy_from_slice(&signature_size.to_le_bytes());

        image.extend_from_slice(chain);
        image.extend_from_slice(&[PADDING; 16]);

        image
    }

    /// An ELF of the test's own that carries no hash segment — the container a
    /// `.melf` begins with, whose images live behind it.
    fn plain_elf(size: usize) -> Vec<u8> {
        let mut image = vec![0_u8; size];

        image[..4].copy_from_slice(b"\x7fELF");
        image[4] = 1; // 32-bit
        image[0x1c..0x20].copy_from_slice(&0x34_u32.to_le_bytes());
        image[0x2a..0x2c].copy_from_slice(&0x20_u16.to_le_bytes());
        image[0x2c..0x2e].copy_from_slice(&1_u16.to_le_bytes());

        // One loadable segment, which is not the hash segment.
        image[0x34..0x38].copy_from_slice(&1_u32.to_le_bytes());
        image[0x38..0x3c].copy_from_slice(&(size as u32).to_le_bytes());
        image[0x4c..0x50].copy_from_slice(&0x1000_0007_u32.to_le_bytes());

        image
    }

    /// A `.melf` is a container: its own ELF has no hash segment at all, and
    /// the images behind it — one ELF after another — carry the chains.
    #[test]
    fn the_chain_of_a_container_is_read_from_the_image_behind_it() {
        let root = b"the root certificate";

        let mut container = plain_elf(0x400);
        container.extend(elf(&chain(root), 16, 16));

        let hash = root_hash(&container, Hash::Sha256).expect("the root hash");
        assert_eq!(hash, Sha256::digest(sequence(root)).to_vec());
    }

    /// The chain of a programmer: two certificates, then the root.
    fn chain(root: &[u8]) -> Vec<u8> {
        let mut chain = sequence(b"the signing certificate");
        chain.extend(sequence(b"the intermediate"));
        chain.extend(sequence(root));
        chain
    }

    /// The root certificate is the third of the chain, and its hash is what the
    /// phone reports as its `PK_HASH`.
    #[test]
    fn the_root_certificate_is_the_third_of_the_chain() {
        let root = b"the root certificate";
        let image = elf(&chain(root), 16, 16);

        let sha256 = root_hash(&image, Hash::Sha256).expect("the root hash");
        assert_eq!(sha256, Sha256::digest(sequence(root)).to_vec());

        let sha384 = root_hash(&image, Hash::Sha384).expect("the root hash");
        assert_eq!(sha384, Sha384::digest(sequence(root)).to_vec());
        assert_eq!(sha384.len(), 48);
    }

    /// A chain whose certificates are longer than 0x7F bytes carries their
    /// lengths in DER's long form, which is what a real root key does.
    #[test]
    fn a_long_certificate_is_measured_in_its_long_form() {
        // A 300-byte certificate: `0x82` says two length bytes follow, and
        // 0x012C is 300.
        let mut certificate = vec![0x30, 0x82, 0x01, 0x2c];
        certificate.extend_from_slice(&[0x5a_u8; 300]);

        let mut chain = sequence(b"the signing certificate");
        chain.extend(sequence(b"the intermediate"));
        chain.extend_from_slice(&certificate);

        let image = elf(&chain, 16, 16);
        let hash = root_hash(&image, Hash::Sha256).expect("the root hash");
        assert_eq!(hash, Sha256::digest(&certificate).to_vec());
    }

    /// A bare MBN is a hash segment on its own, with no ELF around it.
    #[test]
    fn a_bare_mbn_is_read_as_its_own_hash_segment() {
        let root = b"the root certificate";

        let mut image = vec![0_u8; 0x28];
        image[..4].copy_from_slice(&MBN_MAGIC.to_le_bytes());
        image[0x04..0x08].copy_from_slice(&3_u32.to_le_bytes());
        image[0x14..0x18].copy_from_slice(&8_u32.to_le_bytes());
        image[0x1c..0x20].copy_from_slice(&8_u32.to_le_bytes());
        image.resize(0x28 + 8 + 8, 0x77);
        image.extend_from_slice(&chain(root));
        image.extend_from_slice(&[PADDING; 16]);

        let hash = root_hash(&image, Hash::Sha256).expect("the root hash");
        assert_eq!(hash, Sha256::digest(sequence(root)).to_vec());
    }

    /// A file that is neither an ELF nor an MBN says so, and the page falls
    /// back to asking without a comparison.
    #[test]
    fn a_file_that_is_not_a_programmer_says_why() {
        let error = root_hash(b"not a programmer at all", Hash::Sha256)
            .expect_err("it is not a programmer");
        assert!(error.contains("neither an ELF nor an MBN"), "{error}");

        // A chain that starts with something other than a DER sequence: the
        // segment is 0x28 bytes of header, then the code and the signature.
        let mut image = elf(&chain(b"the root certificate"), 16, 16);
        let chain_at = 0x100 + 0x28 + 16 + 16;
        image[chain_at] = 0x31;

        let error = root_hash(&image, Hash::Sha256).expect_err("the chain is broken");
        assert!(error.contains("not a DER sequence"), "{error}");
    }

    /// Which hash is used follows what the phone reported: a 48-byte `PK_HASH`
    /// is a SHA-384 one.
    #[test]
    fn the_hash_follows_the_length_of_the_phones() {
        assert_eq!(Hash::of_length(32), Hash::Sha256);
        assert_eq!(Hash::of_length(48), Hash::Sha384);
        assert_eq!(Hash::of_length(0), Hash::Sha256);
    }
}
