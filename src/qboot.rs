//! The "single image" container Motorola's blankflash packages are built in.
//!
//! A blankflash package is a folder of images plus three XML files — `index.xml`
//! names the board, `pkg.xml` names the programmer and the recipe, and the
//! recipe names the partitions to flash. The same package is also shipped as
//! *one* file: `singleimage.bin` holds those XMLs and every image in a flat
//! container, which is what `qboot` opens when it is handed the whole thing.
//!
//! The format is a 256-byte header and then, for each entry, a 256-byte slot
//! holding the name (NUL-padded) and the size as a little-endian `u64` at offset
//! 248, followed by the data padded up to a 4 KiB boundary:
//!
//! ```text
//! "SINGLE_N_LONELY\0"  (16 bytes, zero-filled to 256)
//! [ name ][ … ][ size: u64 @248 ][ data, padded to 4096 ]
//! …
//! [ "LONELY_N_SINGLE" ]   (the last slot, when the file has one)
//! ```
//!
//! Nothing here talks to a phone: this only reads the container, so the
//! programmer inside one can be used without unpacking it by hand.

/// The magic every container starts with: 16 bytes, then zeros to 256.
pub(crate) const MAGIC: &[u8; 16] = b"SINGLE_N_LONELY\0";

/// The name of the slot that closes a container.
const END_MARKER: &str = "LONELY_N_SINGLE";

/// Where the first slot starts: the container's header is one 256-byte block.
const FIRST_SLOT: usize = 256;

/// How long one slot, and the alignment of the data behind it, are.
const SLOT: usize = 256;
const ALIGNMENT: u64 = 4096;

/// Where in a slot the size of the entry is written.
const SIZE_AT: usize = 248;

/// One file inside a container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    /// Where its bytes start in the container.
    pub offset: u64,
    pub size: u64,
}

/// Whether these bytes are a single-image container.
pub fn is_container(data: &[u8]) -> bool {
    data.len() >= FIRST_SLOT && &data[..MAGIC.len()] == MAGIC
}

/// The files a container holds, in the order they are stored.
pub fn entries(data: &[u8]) -> Result<Vec<Entry>, String> {
    if !is_container(data) {
        return Err("the file is not a Motorola single image".to_owned());
    }

    let mut entries = Vec::new();
    let mut at = FIRST_SLOT;

    while at + SLOT <= data.len() {
        let slot = &data[at..at + SLOT];
        let name = slot_name(slot);

        // The last slot of a container is only a marker; a file that was cut
        // short ends without one, which the loop condition handles.
        if name == END_MARKER || name.is_empty() {
            break;
        }

        let size = u64::from_le_bytes(
            slot[SIZE_AT..SIZE_AT + 8]
                .try_into()
                .expect("eight bytes of a slot"),
        );

        let offset = (at + SLOT) as u64;
        let end = offset.saturating_add(size);
        if end > data.len() as u64 {
            return Err(format!(
                "the single image ends inside `{name}`, which claims {size} bytes"
            ));
        }

        entries.push(Entry { name, offset, size });

        at = (offset + size.div_ceil(ALIGNMENT) * ALIGNMENT) as usize;
    }

    if entries.is_empty() {
        return Err("the single image holds no files".to_owned());
    }

    Ok(entries)
}

/// One entry's bytes.
pub fn bytes<'a>(data: &'a [u8], entry: &Entry) -> &'a [u8] {
    let start = entry.offset as usize;
    let end = start.saturating_add(entry.size as usize);

    &data[start..end.min(data.len())]
}

/// The programmer a container carries, with the name it is stored under.
///
/// The package says which file that is: `pkg.xml` is a one-line document,
/// `<package><programmer filename="programmer.elf"/>…</package>`. A container
/// without one falls back to the first entry that looks like a programmer.
pub fn programmer(data: &[u8]) -> Result<(&[u8], String), String> {
    let entries = entries(data)?;

    let named = entries
        .iter()
        .find(|entry| entry.name.eq_ignore_ascii_case("pkg.xml"))
        .and_then(|entry| {
            let text = String::from_utf8_lossy(bytes(data, entry));
            programmer_of(&text)
        });

    let entry = match named {
        Some(name) => entries
            .iter()
            .find(|entry| entry.name.eq_ignore_ascii_case(&name))
            .ok_or_else(|| {
                format!("the single image names `{name}` as its programmer, but does not hold it")
            })?,
        None => entries
            .iter()
            .find(|entry| looks_like_programmer(&entry.name))
            .ok_or_else(|| "the single image holds no programmer".to_owned())?,
    };

    Ok((bytes(data, entry), entry.name.clone()))
}

/// The `filename` of the `<programmer>` element of a `pkg.xml`.
fn programmer_of(text: &str) -> Option<String> {
    let start = text.find("<programmer")?;
    let rest = &text[start..];
    let end = rest.find('>')?;
    let element = &rest[..end];

    let name = element.split("filename=").nth(1)?;
    let name = name.trim_start().strip_prefix(['"', '\''])?;
    let close = name.find(['"', '\''])?;

    Some(name[..close].to_owned())
}

/// Whether an entry name looks like a Firehose programmer.
fn looks_like_programmer(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();

    ["elf", "mbn", "melf"]
        .iter()
        .any(|extension| lower.ends_with(&format!(".{extension}")))
}

/// The name in a container slot: everything before the first NUL, and never
/// as long as the size field behind it.
fn slot_name(slot: &[u8]) -> String {
    let end = slot[..SIZE_AT].iter().position(|byte| *byte == 0).unwrap_or(SIZE_AT);

    String::from_utf8_lossy(&slot[..end]).trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a container the way `qboot` writes one.
    fn container(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(MAGIC);
        data.resize(FIRST_SLOT, 0);

        for (name, body) in entries {
            let mut slot = vec![0_u8; SLOT];
            slot[..name.len()].copy_from_slice(name.as_bytes());
            slot[SIZE_AT..SIZE_AT + 8].copy_from_slice(&(body.len() as u64).to_le_bytes());
            data.extend_from_slice(&slot);
            data.extend_from_slice(body);

            let padded = (body.len() as u64).div_ceil(ALIGNMENT) * ALIGNMENT;
            data.resize(data.len() + (padded - body.len() as u64) as usize, 0);
        }

        let mut slot = vec![0_u8; SLOT];
        slot[..END_MARKER.len()].copy_from_slice(END_MARKER.as_bytes());
        data.extend_from_slice(&slot);

        data
    }

    /// A container is read as the files it holds, and the programmer is the one
    /// `pkg.xml` names.
    #[test]
    fn a_single_image_is_read_as_the_files_it_holds() {
        let pkg = br#"<?xml version="1.0"?>
<package>
    <programmer filename="programmer.elf"/>
    <recipe filename="default.xml"/>
</package>"#
            .to_vec();

        let image = container(&[
            ("pkg.xml", pkg),
            ("programmer.elf", b"the programmer".to_vec()),
            ("abl.elf", b"the bootloader".to_vec()),
        ]);

        assert!(is_container(&image));

        let entries = entries(&image).expect("the files");
        assert_eq!(
            entries.iter().map(|entry| entry.name.as_str()).collect::<Vec<_>>(),
            ["pkg.xml", "programmer.elf", "abl.elf"]
        );
        assert_eq!(bytes(&image, &entries[2]), b"the bootloader");

        // `pkg.xml` fills the first slot and its data is padded to 4 KiB, so the
        // second entry's slot starts at 4608 and its bytes at 4864.
        assert_eq!(entries[1].offset, 4864);

        let (programmer, name) = programmer(&image).expect("a programmer");
        assert_eq!(programmer, b"the programmer");
        assert_eq!(name, "programmer.elf");
    }

    /// A container that names a programmer it does not hold says so, and a
    /// container with no `pkg.xml` falls back to the first programmer-shaped
    /// file.
    #[test]
    fn a_container_names_its_programmer() {
        let image = container(&[(
            "pkg.xml",
            br#"<package><programmer filename="missing.elf"/></package>"#.to_vec(),
        )]);
        let error = programmer(&image).expect_err("nothing to use");
        assert!(error.contains("missing.elf"), "{error}");

        let image = container(&[("abl.elf", b"not it".to_vec())]);
        let (_, name) = programmer(&image).expect("the first programmer-shaped file");
        assert_eq!(name, "abl.elf");
    }

    /// Anything else is refused, and a file cut short inside an entry is not
    /// read past its end.
    #[test]
    fn a_broken_container_is_refused() {
        assert!(!is_container(b"not a container at all"));
        assert!(entries(b"nope").is_err());

        let mut image = container(&[("pkg.xml", vec![7_u8; 8192])]);
        image.truncate(image.len() - 4096);
        let error = entries(&image).expect_err("a truncated entry");
        assert!(error.contains("pkg.xml"), "{error}");
    }
}
