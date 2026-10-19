//! The `disable-verity` / `disable-verification` flags of an AVB image.
//!
//! Google's `fastboot` does not send these two flags to the phone: it writes
//! them into the vbmeta struct of the image it is about to flash
//! (`--disable-verity`, `--disable-verification`), so the phone comes up with
//! dm-verity hashtrees and AVB verification turned off. Only the 32-bit
//! big-endian `flags` field at offset 120 of that struct changes — bit 0
//! disables the hashtree, bit 1 disables verification — while the rest of the
//! image, its hash and its signature included, is left exactly as it was.
//!
//! This is a port of `rewrite_vbmeta_buffer()` from AOSP's `fastboot`, so the
//! built-in fastboot writes what Motorola's `mfastboot` writes when it is
//! handed the same two flags. The struct sits at the start of the image, or
//! wherever the AVB footer the image carries points (`vbmeta_offset`).

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Magic a vbmeta struct starts with (`AVB_MAGIC`).
pub(crate) const AVB_MAGIC: [u8; 4] = *b"AVB0";

/// Magic the footer of an AVB image starts with (`AVB_FOOTER_MAGIC`).
pub(crate) const AVB_FOOTER_MAGIC: [u8; 4] = *b"AVBF";

/// How long that footer is (`AVB_FOOTER_SIZE`), sitting at the end of the
/// image.
pub(crate) const AVB_FOOTER_SIZE: usize = 64;

/// How long a vbmeta struct is (`AVB_VBMETA_IMAGE_HEADER_SIZE`); an image
/// shorter than this cannot carry one.
const VBMETA_SIZE: usize = 256;

/// Offset of the `flags` field inside the vbmeta struct.
const FLAGS_OFFSET: usize = 120;

/// The field is 32-bit big endian, so the two bits that matter are in its last
/// byte (`uint64_t flags_offset = 123 + vbmeta_offset` in `fastboot`).
const FLAGS_LAST_BYTE: usize = FLAGS_OFFSET + 3;

/// `AVB_VBMETA_IMAGE_FLAGS_HASHTREE_DISABLED`: dm-verity is not started.
const HASHTREE_DISABLED: u8 = 0x01;

/// `AVB_VBMETA_IMAGE_FLAGS_VERIFICATION_DISABLED`: nothing is verified.
const VERIFICATION_DISABLED: u8 = 0x02;

/// Where the vbmeta struct of an image starts, or `None` when it has none.
///
/// A `vbmeta.img` carries the struct at offset 0. An image the AVB tools gave
/// a footer to (a `boot.img`, for instance) records where its struct sits in
/// that footer, which is the last [`AVB_FOOTER_SIZE`] bytes of the file.
pub(crate) fn vbmeta_offset(data: &[u8]) -> Option<usize> {
    if data.len() >= VBMETA_SIZE && data[..AVB_MAGIC.len()] == AVB_MAGIC {
        return Some(0);
    }

    if data.len() < AVB_FOOTER_SIZE {
        return None;
    }

    let footer = &data[data.len() - AVB_FOOTER_SIZE..];

    if footer[..AVB_FOOTER_MAGIC.len()] != AVB_FOOTER_MAGIC {
        return None;
    }

    // `original_image_size`, `vbmeta_offset` and `vbmeta_size` are 64-bit big
    // endian fields behind the magic; the offset is the second of them.
    let offset = u64::from_be_bytes(footer[16..24].try_into().ok()?) as usize;

    // The footer only says where the struct should be: it has to be there for
    // the flags to be written anywhere sensible.
    let end = offset.checked_add(AVB_MAGIC.len())?;
    (end <= data.len() && data[offset..end] == AVB_MAGIC).then_some(offset)
}

/// Reads where an image carries its vbmeta struct without loading it, so the
/// dialog can tell whether the two flags would apply to it.
///
/// A file that cannot be read has no struct as far as this is concerned: the
/// check only decides whether the checkboxes are offered.
pub(crate) fn image_vbmeta_offset(path: &Path) -> Option<u64> {
    let mut file = std::fs::File::open(path).ok()?;
    let length = file.metadata().ok()?.len();

    if length < VBMETA_SIZE as u64 {
        return None;
    }

    let mut magic = [0u8; 4];

    if read_exact_at(&mut file, 0, &mut magic) && magic == AVB_MAGIC {
        return Some(0);
    }

    if length < AVB_FOOTER_SIZE as u64 {
        return None;
    }

    let mut footer = [0u8; AVB_FOOTER_SIZE];

    if !read_exact_at(&mut file, length - AVB_FOOTER_SIZE as u64, &mut footer) {
        return None;
    }

    if footer[..AVB_FOOTER_MAGIC.len()] != AVB_FOOTER_MAGIC {
        return None;
    }

    let offset = u64::from_be_bytes(footer[16..24].try_into().ok()?);

    if offset + AVB_MAGIC.len() as u64 > length {
        return None;
    }

    let mut magic = [0u8; 4];

    if !read_exact_at(&mut file, offset, &mut magic) || magic != AVB_MAGIC {
        return None;
    }

    Some(offset)
}

/// Reads `buffer.len()` bytes at `offset`, reporting whether it worked.
fn read_exact_at(file: &mut std::fs::File, offset: u64, buffer: &mut [u8]) -> bool {
    file.seek(SeekFrom::Start(offset)).is_ok()
        && file.read_exact(buffer).is_ok()
}

/// Writes the two flags into the image's vbmeta struct, returning the offset
/// the struct was found at — or `None` when the image has no struct, in which
/// case it is left untouched.
///
/// The bits are only ever set, never cleared: the flags an image already
/// carries are the ones whoever built it asked for.
pub(crate) fn set_flags(
    data: &mut [u8],
    disable_verity: bool,
    disable_verification: bool,
) -> Option<usize> {
    let offset = vbmeta_offset(data)?;

    if !disable_verity && !disable_verification {
        return Some(offset);
    }

    let flags = offset + FLAGS_LAST_BYTE;

    if flags >= data.len() {
        return None;
    }

    if disable_verity {
        data[flags] |= HASHTREE_DISABLED;
    }

    if disable_verification {
        data[flags] |= VERIFICATION_DISABLED;
    }

    Some(offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A vbmeta struct with `AVB0` at the front and no flag set.
    fn vbmeta(len: usize) -> Vec<u8> {
        let mut data = vec![0u8; len.max(VBMETA_SIZE)];
        data[..4].copy_from_slice(&AVB_MAGIC);

        data
    }

    /// `image` with an AVB footer added that points at `offset`.
    fn with_footer(mut image: Vec<u8>, offset: u64) -> Vec<u8> {
        let mut footer = vec![0u8; AVB_FOOTER_SIZE];
        footer[..4].copy_from_slice(&AVB_FOOTER_MAGIC);
        footer[8..16].copy_from_slice(&(image.len() as u64).to_be_bytes());
        footer[16..24].copy_from_slice(&offset.to_be_bytes());
        footer[24..32].copy_from_slice(&(VBMETA_SIZE as u64).to_be_bytes());

        image.extend_from_slice(&footer);
        image
    }

    #[test]
    fn a_struct_at_the_start_is_found() {
        assert_eq!(vbmeta_offset(&vbmeta(256)), Some(0));
    }

    #[test]
    fn the_footer_points_at_the_struct() {
        // A boot image: a payload, then the vbmeta struct, then the footer.
        let mut image = vec![0xAAu8; 1024];
        let offset = image.len();
        image.extend_from_slice(&vbmeta(256));
        let image = with_footer(image, offset as u64);

        assert_eq!(vbmeta_offset(&image), Some(offset));
    }

    #[test]
    fn an_image_without_a_struct_has_no_offset() {
        assert_eq!(vbmeta_offset(&[]), None);
        assert_eq!(vbmeta_offset(&[0u8; 4096]), None);
        // A footer that points at something that is not a struct is not one
        // either: the magic has to be there.
        assert_eq!(vbmeta_offset(&with_footer(vec![0u8; 512], 512)), None);
        // Nor is a footer that points outside the file.
        assert_eq!(vbmeta_offset(&with_footer(vec![0u8; 512], 4096)), None);
    }

    #[test]
    fn the_flags_are_or_ed_into_the_big_endian_field() {
        let mut data = vbmeta(256);
        assert_eq!(set_flags(&mut data, true, false), Some(0));
        // Only the last byte of the 32-bit big-endian field is written, so the
        // three bytes in front of it stay zero.
        assert_eq!(&data[FLAGS_OFFSET..FLAGS_OFFSET + 3], &[0, 0, 0]);
        assert_eq!(data[FLAGS_LAST_BYTE], HASHTREE_DISABLED);

        let mut data = vbmeta(256);
        assert_eq!(set_flags(&mut data, false, true), Some(0));
        assert_eq!(data[FLAGS_LAST_BYTE], VERIFICATION_DISABLED);

        let mut data = vbmeta(256);
        assert_eq!(set_flags(&mut data, true, true), Some(0));
        assert_eq!(
            data[FLAGS_LAST_BYTE],
            HASHTREE_DISABLED | VERIFICATION_DISABLED
        );
    }

    #[test]
    fn the_flags_of_an_image_are_kept() {
        // Whatever the image already carried is still there afterwards: the
        // bits are set, never cleared.
        let mut data = vbmeta(256);
        data[FLAGS_LAST_BYTE] = 0x10;

        assert_eq!(set_flags(&mut data, true, false), Some(0));
        assert_eq!(data[FLAGS_LAST_BYTE], 0x10 | HASHTREE_DISABLED);
    }

    #[test]
    fn an_image_without_a_struct_is_left_alone() {
        let mut data = vec![0x11u8; 256];
        assert_eq!(set_flags(&mut data, true, true), None);
        assert!(data.iter().all(|byte| *byte == 0x11));
    }

    #[test]
    fn the_struct_of_a_file_is_found_without_reading_it() {
        let directory = std::env::temp_dir().join(format!(
            "lmnflash-avb-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("the test directory");
        let path = directory.join("vbmeta.img");

        std::fs::write(&path, vbmeta(256)).expect("the image");
        assert_eq!(image_vbmeta_offset(&path), Some(0));

        // The same through a footer, in a file that holds more than the struct.
        let mut image = vec![0xAAu8; 1024];
        let offset = image.len();
        image.extend_from_slice(&vbmeta(256));
        std::fs::write(&path, with_footer(image, offset as u64)).expect("the image");
        assert_eq!(image_vbmeta_offset(&path), Some(offset as u64));

        std::fs::write(&path, vec![0u8; 4096]).expect("the image");
        assert_eq!(image_vbmeta_offset(&path), None);

        // A file that is not there has no struct either.
        std::fs::remove_file(&path).expect("removing the image");
        assert_eq!(image_vbmeta_offset(&path), None);

        let _ = std::fs::remove_dir_all(&directory);
    }
}
