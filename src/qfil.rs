//! The XML a Qualcomm package is flashed from.
//!
//! A "flat build" is the pair of files QFIL flashes from: `rawprogram<N>.xml`,
//! which says where every image on disk goes (`<program>`) and what to erase
//! first (`<erase>`), and `patch<N>.xml`, which writes a few bytes back into a
//! sector once the images are down (the backup GPT header, mostly). A "meta
//! build" is one `contents.xml` that names the programmer and those files
//! itself, so the user only picks the one file.
//!
//! Lenovo's L tablets ship the same files encrypted — `contents.x`,
//! `rawprogram0.x`, `patch0.x` — which read exactly like their `.xml` twins
//! once they are decrypted (see [`crate::decrypt`]).
//!
//! Nothing here talks to a phone: this module reads the files, works out what
//! would be flashed, and says which of the files a flash needs are not there.

use std::path::{Path, PathBuf};

use roxmltree::Document;

/// Which kind of package the user picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Build {
    /// `contents.xml` (or `contents.x`), naming everything else.
    Meta,
    /// `rawprogram*.xml` and `patch*.xml`, picked by hand.
    Flat,
}

/// The magic an Android sparse image starts with, little-endian.
const SPARSE_MAGIC: [u8; 4] = [0x3a, 0xff, 0x26, 0xed];

/// A sector as the packages spell it.
///
/// Most are plain numbers. A partition table is written at both ends of the
/// LUN, though, so its backup copy names the LUN's own length:
/// `start_sector="NUM_DISK_SECTORS-5."` is the fifth sector from the end. That
/// length is only known once the phone is asked for its geometry, so the
/// expression is carried as written and worked out at flash time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sector {
    /// A sector the XML names outright.
    At(u64),
    /// An expression over the LUN's sector count, as written.
    Relative(String),
}

impl Sector {
    /// The sector an attribute spells, or `None` when it is neither a number
    /// nor an expression this module can work out later.
    fn of(value: &str) -> Option<Self> {
        let text = value.trim();

        if let Some(sector) = number_of(text) {
            return Some(Self::At(sector));
        }

        // A probe of zero is enough to tell an expression from a typo: the
        // arithmetic saturates, so every well-formed one answers.
        expression_value(text, 0).ok().map(|_| Self::Relative(text.to_owned()))
    }

    /// Whether the value has to wait for the phone's answer.
    pub fn needs_geometry(&self) -> bool {
        matches!(self, Self::Relative(_))
    }

    /// The sector it stands for, given how many sectors the LUN has.
    pub fn at(&self, disk_sectors: u64) -> Result<u64, String> {
        match self {
            Self::At(sector) => Ok(*sector),
            Self::Relative(text) => expression_value(text, disk_sectors),
        }
    }
}

impl std::fmt::Display for Sector {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::At(sector) => write!(formatter, "{sector}"),
            Self::Relative(text) => write!(formatter, "{text}"),
        }
    }
}

/// The number an expression a package carries stands for.
///
/// The packages describe a few values against the device's own geometry —
/// `NUM_DISK_SECTORS-1.` is how the backup GPT header is told where the last
/// sector of the storage is. The trailing dot is part of the syntax: the
/// reference tools write `0x10-10.` for six and `0x10-10` for zero.
pub fn expression_value(expression: &str, disk_sectors: u64) -> Result<u64, String> {
    let text = expression.trim();

    let Some(rest) = text.strip_prefix("NUM_DISK_SECTORS") else {
        return number_of(text).ok_or_else(|| format!("`{expression}` is not a number"));
    };

    let rest = rest.trim().trim_end_matches('.').trim();

    if rest.is_empty() {
        Ok(disk_sectors)
    } else if let Some(offset) = rest.strip_prefix('-') {
        let offset = offset_number(offset)?;
        Ok(disk_sectors.saturating_sub(offset))
    } else if let Some(offset) = rest.strip_prefix('+') {
        let offset = offset_number(offset)?;
        Ok(disk_sectors.saturating_add(offset))
    } else {
        Err(format!(
            "`{expression}` is not a value this page understands"
        ))
    }
}

fn offset_number(text: &str) -> Result<u64, String> {
    let text = text.trim();

    number_of(text).ok_or_else(|| format!("`{text}` is not a number"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The partition name the XML gives it (`label`).
    pub label: String,
    /// The file as the XML names it.
    pub file: String,
    /// Where that file is, resolved next to the XML.
    pub path: PathBuf,
    /// Whether it is on disk. A flash cannot start without it.
    pub exists: bool,
    /// The physical partition (LUN) it lands on.
    pub lun: u8,
    pub sector_size: u32,
    pub start_sector: Sector,
    /// How many sectors the image takes.
    pub sectors: u64,
    /// Where in the file the first byte of the image is.
    pub file_sector_offset: u64,
    /// Whether the file is an Android sparse image — a container of chunks that
    /// the flash expands as it writes, so only the blocks it stores travel.
    pub sparse: bool,
}

/// One `erase` of a `rawprogram` file: sectors cleared as the images go down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Erase {
    pub label: String,
    pub lun: u8,
    pub sector_size: u32,
    pub start_sector: Sector,
    pub sectors: u64,
    /// Whether the range is written with zeroes rather than erased, which is
    /// what a `<zeroout>` element asks for — the reference packages use it for
    /// the places a stale value would be read back as a command or a size
    /// (`misc`, both copies of the partition table, the DDR training data).
    pub zero: bool,
}

/// One `patch`: a few bytes written into a sector once the images are down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patch {
    pub label: String,
    pub lun: u8,
    pub sector_size: u32,
    pub start_sector: Sector,
    /// How far into the sector the value goes.
    pub byte_offset: u64,
    pub size: u64,
    /// The value as the XML spells it — a number, or an expression over the
    /// device's sector count (see [`crate::firehose::patch_value`]).
    pub value: String,
    /// What the patch does, as the XML says it.
    pub what: String,
}

/// What one element of a `rawprogram` file does, in the order the file lists
/// it: a program and the erase that clears its sectors for it are two elements
/// of one list, and swapping them would wipe what the program wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// The image at this index of [`Plan::entries`].
    Program(usize),
    /// The erase at this index of [`Plan::erases`].
    Erase(usize),
}

/// Everything one package flash needs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// The programmer the meta build names, when there is one.
    pub programmer: Option<PathBuf>,
    /// The `rawprogram` files the plan was read from.
    pub rawprograms: Vec<PathBuf>,
    /// The `patch` files it was read from.
    pub patches_files: Vec<PathBuf>,
    /// The images, in the order the XMLs list them.
    pub entries: Vec<Entry>,
    /// The erases, in the same order.
    pub erases: Vec<Erase>,
    /// The programs and erases in the order the files list them.
    pub actions: Vec<Action>,
    /// The patches, in the order the patch files list them.
    pub patches: Vec<Patch>,
}

impl Plan {
    /// The images that are not on disk, by the name the XML gives them.
    pub fn missing(&self) -> Vec<&str> {
        self.entries
            .iter()
            .filter(|entry| !entry.exists)
            .map(|entry| entry.file.as_str())
            .collect()
    }

    /// Whether a partition is one the packages are flashed without by default:
    /// the two the boot ROM keeps its keys in, and the fuse backup.
    pub fn skipped_by_default(label: &str) -> bool {
        matches!(
            label.to_ascii_lowercase().as_str(),
            "sec" | "secdata" | "efusebackup" | "efuse_backup"
        )
    }
}

/// Reads a package file, decrypting it when it is one of the encrypted `.x`
/// files Lenovo's L tablets ship.
pub fn read_package_file(path: &Path) -> Result<Vec<u8>, String> {
    let data = std::fs::read(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;

    let encrypted = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("x"));

    if !encrypted {
        return Ok(data);
    }

    crate::decrypt::decrypt_bytes(&data, crate::decrypt::DEFAULT_PASSWORD)
        .map_err(|error| format!("could not decrypt {}: {error}", path.display()))
}

/// Reads the files a flat build was picked from.
///
/// The files are told apart by what is inside them rather than by their names:
/// a file whose root holds `program` or `erase` elements is a `rawprogram`, one
/// that holds `patch` elements is a patch file. A package folder usually holds
/// other XMLs as well — `poweroff.xml` and the like — so a file that neither
/// flashes nor patches is passed over, unless nothing was read at all.
pub fn flat(files: &[PathBuf]) -> Result<Plan, String> {
    let mut plan = Plan::default();
    let mut other: Vec<String> = Vec::new();

    for path in files {
        let text = text_of(path)?;

        if has_element(&text, "patch")? {
            plan.patches.extend(read_patch(&text, path)?);
            plan.patches_files.push(path.clone());
        } else if has_element(&text, "program")?
            || has_element(&text, "erase")?
            || has_element(&text, "zeroout")?
        {
            read_rawprogram(&text, path, &mut plan)?;
            plan.rawprograms.push(path.clone());
        } else {
            other.push(file_name(path));
        }
    }

    if plan.entries.is_empty() && plan.erases.is_empty() {
        return Err(if other.is_empty() {
            "the files picked flash no images".to_owned()
        } else {
            format!(
                "{} holds neither images to flash nor patches",
                other.join(", ")
            )
        });
    }

    Ok(plan)
}

/// Reads a meta build's `contents.xml` (or `contents.x`).
///
/// The file names the programmer and the `rawprogram`/`patch` files it is made
/// of; a name is looked for next to `contents.xml` itself, and its encrypted
/// `.x` twin is taken when the `.xml` is not there.
pub fn meta(contents: &Path) -> Result<Plan, String> {
    let text = text_of(contents)?;
    let directory = contents.parent().unwrap_or_else(|| Path::new("."));
    let document = Document::parse(&text)
        .map_err(|error| format!("{} is not XML: {error}", file_name(contents)))?;

    let mut plan = Plan::default();
    let mut seen: Vec<PathBuf> = Vec::new();

    for value in values_of(&document) {
        let name = value.trim();

        if let Some(programmer) = programmer_of(name) {
            // Named whether or not it is in the package: the card shows it, so
            // the user knows which programmer to inject.
            if plan.programmer.is_none() {
                plan.programmer = beside(directory, programmer, None)
                    .or_else(|| Some(directory.join(last_segment(programmer))));
            }
            continue;
        }

        if !is_flash_file(name) {
            continue;
        }

        // A `rawprogram` or `patch` file: its encrypted twin is taken when the
        // plain one is not in the package.
        let Some(path) = beside(directory, name, Some(("xml", "x"))) else {
            continue;
        };
        if seen.contains(&path) {
            continue;
        }
        seen.push(path.clone());

        let text = text_of(&path)?;
        if has_element(&text, "patch")? {
            plan.patches.extend(read_patch(&text, &path)?);
            plan.patches_files.push(path);
        } else {
            read_rawprogram(&text, &path, &mut plan)?;
            plan.rawprograms.push(path);
        }
    }

    if plan.entries.is_empty() {
        return Err(format!(
            "{} names no rawprogram file that is in the package",
            file_name(contents)
        ));
    }

    Ok(plan)
}

/// The text of a package file, decrypted when it needs to be.
fn text_of(path: &Path) -> Result<String, String> {
    let data = read_package_file(path)?;

    String::from_utf8(data)
        .map_err(|_| format!("{} is not text", file_name(path)))
}

/// Reads one `rawprogram` file into `plan`.
fn read_rawprogram(text: &str, path: &Path, plan: &mut Plan) -> Result<(), String> {
    let document = parse(text, path)?;
    let directory = path.parent().unwrap_or_else(|| Path::new("."));

    // A `<zeroout>` element names no sector size of its own, so the file's is
    // taken from the first element that does name one.
    let file_sector_size = document
        .descendants()
        .filter(|node| node.is_element())
        .find_map(|node| attribute(&node, "SECTOR_SIZE_IN_BYTES"))
        .and_then(|value| number_of(&value))
        .and_then(|size| u32::try_from(size).ok())
        .unwrap_or(4096);

    for node in document.descendants().filter(|node| node.is_element()) {
        match node.tag_name().name() {
            "program" => {
                let file = attribute(&node, "filename").unwrap_or_default();

                // A `program` with no file is a partition the package keeps no
                // image for: the reference packages list every partition of
                // the table and name a file only for the ones they flash, so
                // there is nothing here to write.
                if file.is_empty() {
                    continue;
                }

                let resolved = beside(directory, &file, None)
                    .unwrap_or_else(|| directory.join(&file));
                let exists = resolved.is_file();

                plan.actions.push(Action::Program(plan.entries.len()));
                plan.entries.push(Entry {
                    label: attribute(&node, "label").unwrap_or_default(),
                    exists,
                    // A package does not always say so, so the file itself is
                    // asked: a sparse image written raw would scatter its
                    // chunks over the storage.
                    sparse: exists
                        && (attribute(&node, "sparse")
                            .is_some_and(|value| value.eq_ignore_ascii_case("true"))
                            || is_sparse(&resolved)),
                    path: resolved,
                    file,
                    lun: number(&node, "physical_partition_number")? as u8,
                    sector_size: sized(&node, "SECTOR_SIZE_IN_BYTES")?,
                    start_sector: sector(&node, "start_sector")?,
                    sectors: number(&node, "num_partition_sectors")?,
                    file_sector_offset: number(&node, "file_sector_offset").unwrap_or(0),
                });
            }
            "erase" => {
                plan.actions.push(Action::Erase(plan.erases.len()));
                plan.erases.push(Erase {
                    label: attribute(&node, "label").unwrap_or_default(),
                    lun: number(&node, "physical_partition_number")? as u8,
                    sector_size: sized(&node, "SECTOR_SIZE_IN_BYTES")?,
                    start_sector: sector(&node, "start_sector")?,
                    sectors: number(&node, "num_partition_sectors")?,
                    zero: false,
                });
            }
            "zeroout" => {
                // A `zeroout` names no sector size of its own, so the one the
                // file's programs use is taken.
                plan.actions.push(Action::Erase(plan.erases.len()));
                plan.erases.push(Erase {
                    label: attribute(&node, "label").unwrap_or_default(),
                    lun: number(&node, "physical_partition_number")? as u8,
                    sector_size: sized(&node, "SECTOR_SIZE_IN_BYTES").unwrap_or(file_sector_size),
                    start_sector: sector(&node, "start_sector")?,
                    sectors: number(&node, "num_partition_sectors")?,
                    zero: true,
                });
            }
            _ => {}
        }
    }

    Ok(())
}

/// Reads one `patch` file.
///
/// Only the patches that edit the storage are kept. A patch file names either
/// `DISK` — the storage itself — or one of the package's own images, which is
/// the same edit made to the file before it is written. The images are written
/// as the package ships them, so the file copies have nothing to do here: every
/// one of them has its `DISK` twin, and writing the file copy to the LUN would
/// land in the wrong sector (the header of `gpt_backup0.bin` sits at sector 4 of
/// that file, not at sector 4 of the storage).
fn read_patch(text: &str, path: &Path) -> Result<Vec<Patch>, String> {
    let document = parse(text, path)?;
    let mut patches = Vec::new();

    for node in document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "patch")
    {
        let file = attribute(&node, "filename").unwrap_or_else(|| "DISK".to_owned());
        if !file.eq_ignore_ascii_case("DISK") {
            continue;
        }

        patches.push(Patch {
            label: file,
            lun: number(&node, "physical_partition_number")? as u8,
            sector_size: sized(&node, "SECTOR_SIZE_IN_BYTES")?,
            start_sector: sector(&node, "start_sector")?,
            byte_offset: number(&node, "byte_offset")?,
            size: number(&node, "size_in_bytes")?,
            value: attribute(&node, "value").unwrap_or_default(),
            what: attribute(&node, "what").unwrap_or_default(),
        });
    }

    Ok(patches)
}

fn parse<'a>(text: &'a str, path: &Path) -> Result<Document<'a>, String> {
    Document::parse(text)
        .map_err(|error| format!("{} is not XML: {error}", file_name(path)))
}

/// Whether the document holds at least one element called `name`.
fn has_element(text: &str, name: &str) -> Result<bool, String> {
    let document = Document::parse(text).map_err(|error| format!("it is not XML: {error}"))?;

    Ok(document
        .descendants()
        .any(|node| node.is_element() && node.tag_name().name() == name))
}

/// Every attribute value and text node of a meta build's contents file.
///
/// The file has been laid out several ways over the years — a `file_name` /
/// `file_path` pair, a bare `download_file`, an attribute — so what it names is
/// collected rather than one schema being assumed.
fn values_of(document: &Document<'_>) -> Vec<String> {
    let mut values = Vec::new();

    for node in document.descendants() {
        if node.is_text() {
            if let Some(text) = node.text() {
                values.push(text.to_owned());
            }
        }

        for attribute in node.attributes() {
            values.push(attribute.value().to_owned());
        }
    }

    values
}

/// The programmer a contents file names, if the value is one.
fn programmer_of(value: &str) -> Option<&str> {
    let name = last_segment(value);
    let lower = name.to_ascii_lowercase();

    ["elf", "mbn", "melf"]
        .iter()
        .any(|extension| lower.ends_with(&format!(".{extension}")))
        .then_some(name)
}

/// Whether a value names a `rawprogram` or `patch` file.
fn is_flash_file(value: &str) -> bool {
    let lower = last_segment(value).to_ascii_lowercase();

    (lower.starts_with("rawprogram") || lower.starts_with("patch"))
        && (lower.ends_with(".xml") || lower.ends_with(".x"))
}

/// The file name a path-like value ends with: contents files spell their
/// entries both as `rawprogram0.xml` and as `.\subdir\rawprogram0.xml`.
fn last_segment(value: &str) -> &str {
    let trimmed = value.trim();

    trimmed
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(trimmed)
}

/// The file `name` names inside `directory`, when it is there.
///
/// `twins` names the extension pair a file may be shipped encrypted as: a
/// `rawprogram0.xml` that is not in the package is looked for as
/// `rawprogram0.x`.
fn beside(directory: &Path, name: &str, twins: Option<(&str, &str)>) -> Option<PathBuf> {
    let name = last_segment(name);
    if name.is_empty() {
        return None;
    }

    let path = directory.join(name);
    if path.is_file() {
        return Some(path);
    }

    let (plain, encrypted) = twins?;
    let lower = name.to_ascii_lowercase();
    let wanted = format!(".{plain}");
    if !lower.ends_with(&wanted) {
        return None;
    }

    let twin = path.with_extension(encrypted);
    twin.is_file().then_some(twin)
}

fn attribute(node: &roxmltree::Node<'_, '_>, name: &str) -> Option<String> {
    node.attribute(name).map(|value| value.trim().to_owned())
}

/// A number the XML spells as the attribute `name`.
fn number(node: &roxmltree::Node<'_, '_>, name: &str) -> Result<u64, String> {
    let value = attribute(node, name)
        .ok_or_else(|| format!("a `{}` has no `{name}`", node.tag_name().name()))?;

    number_of(&value)
        .ok_or_else(|| format!("`{value}` is not a number (`{name}`)"))
}

/// A sector size, which every command needs and a flash cannot invent.
fn sized(node: &roxmltree::Node<'_, '_>, name: &str) -> Result<u32, String> {
    let size = number(node, name)?;

    u32::try_from(size).map_err(|_| format!("`{name}` is {size}, which is not a sector size"))
}

/// A sector the XML spells as the attribute `name`.
fn sector(node: &roxmltree::Node<'_, '_>, name: &str) -> Result<Sector, String> {
    let value = attribute(node, name)
        .ok_or_else(|| format!("a `{}` has no `{name}`", node.tag_name().name()))?;

    Sector::of(&value).ok_or_else(|| format!("`{value}` is not a sector (`{name}`)"))
}

/// A number as the files spell them: decimal, or hex with `0x`.
pub(crate) fn number_of(value: &str) -> Option<u64> {
    let text = value.trim();

    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        return u64::from_str_radix(hex.trim(), 16).ok();
    }

    text.parse::<u64>().ok()
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Whether a file is an Android sparse image.
fn is_sparse(path: &Path) -> bool {
    use std::io::Read;

    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };

    let mut magic = [0_u8; 4];
    file.read_exact(&mut magic).is_ok() && magic == SPARSE_MAGIC
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "lmnflash-qfil-{}-{}",
            std::process::id(),
            name
        ));
        std::fs::create_dir_all(&directory).expect("a scratch folder");
        directory
    }

    /// A rawprogram file and the images it names.
    fn rawprogram(directory: &Path) {
        std::fs::write(directory.join("xbl.elf"), b"xbl image").expect("an image");
        std::fs::write(directory.join("system.img"), b"system image").expect("an image");
        std::fs::write(
            directory.join("rawprogram0.xml"),
            r#"<?xml version="1.0" ?>
<data>
  <program SECTOR_SIZE_IN_BYTES="4096" file_sector_offset="0" filename="xbl.elf"
           label="xbl_a" num_partition_sectors="2" physical_partition_number="0"
           start_sector="1024" />
  <program SECTOR_SIZE_IN_BYTES="4096" file_sector_offset="0" filename=""
           label="splash_a" num_partition_sectors="8" physical_partition_number="0"
           start_sector="2048" />
  <erase SECTOR_SIZE_IN_BYTES="4096" filename="DISK" label="sec"
         num_partition_sectors="8" physical_partition_number="0" start_sector="2048" />
  <program SECTOR_SIZE_IN_BYTES="4096" filename="system.img" label="system_a"
           num_partition_sectors="4" physical_partition_number="2" start_sector="4096" />
</data>"#,
        )
        .expect("a rawprogram file");
    }

    /// A patch file.
    fn patch(directory: &Path) {
        std::fs::write(
            directory.join("patch0.xml"),
            r#"<?xml version="1.0" ?>
<patches>
  <patch SECTOR_SIZE_IN_BYTES="4096" byte_offset="16" filename="DISK"
         physical_partition_number="0" size_in_bytes="4" start_sector="4095"
         value="NUM_DISK_SECTORS-1." what="Update last sector number" />
</patches>"#,
        )
        .expect("a patch file");
    }

    /// A flat build is read from the files themselves: which is which comes
    /// from what they hold, not from what they are called.
    #[test]
    fn a_flat_build_is_read_from_the_files_it_is_given() {
        let directory = scratch("flat");
        rawprogram(&directory);
        patch(&directory);

        let plan = flat(&[
            directory.join("patch0.xml"),
            directory.join("rawprogram0.xml"),
        ])
        .expect("a plan");

        assert_eq!(plan.entries.len(), 2);
        assert_eq!(plan.erases.len(), 1);
        assert_eq!(plan.patches.len(), 1);

        let first = &plan.entries[0];
        assert_eq!(first.label, "xbl_a");
        assert_eq!(first.file, "xbl.elf");
        assert_eq!(first.lun, 0);
        assert_eq!(first.sector_size, 4096);
        assert_eq!(first.start_sector, Sector::At(1024));
        assert_eq!(first.sectors, 2);
        assert!(first.exists);
        assert!(!first.sparse);

        let second = &plan.entries[1];
        assert_eq!(second.label, "system_a");
        assert_eq!(second.lun, 2);

        assert_eq!(plan.erases[0].label, "sec");
        assert_eq!(plan.erases[0].sectors, 8);

        // The file's own order, so the erase stays between the two programs.
        assert_eq!(
            plan.actions,
            [
                Action::Program(0),
                Action::Erase(0),
                Action::Program(1)
            ]
        );

        let patch = &plan.patches[0];
        assert_eq!(patch.start_sector, Sector::At(4095));
        assert_eq!(patch.byte_offset, 16);
        assert_eq!(patch.size, 4);
        assert_eq!(patch.value, "NUM_DISK_SECTORS-1.");
        assert_eq!(patch.what, "Update last sector number");

        assert!(plan.missing().is_empty());
        // The partition the package keeps no image for is not a row: there is
        // nothing a flash could write to it.
        assert!(
            plan.entries.iter().all(|entry| !entry.file.is_empty()),
            "{plan:?}"
        );
        std::fs::remove_dir_all(&directory).expect("clean up");
    }

    /// An image that is not in the package is reported, and the plan is not
    /// complete because of it.
    #[test]
    fn a_missing_image_is_reported() {
        let directory = scratch("missing");
        rawprogram(&directory);
        std::fs::remove_file(directory.join("system.img")).expect("remove the image");

        let plan = flat(&[directory.join("rawprogram0.xml")]).expect("a plan");

        assert_eq!(plan.entries.len(), 2);
        assert!(!plan.entries[1].exists);
        assert_eq!(plan.missing(), ["system.img"]);

        std::fs::remove_dir_all(&directory).expect("clean up");
    }

    /// A file that is neither a rawprogram nor a patch file says so.
    #[test]
    fn a_file_that_flashes_nothing_says_so() {
        let directory = scratch("empty");
        std::fs::write(directory.join("other.xml"), "<data><config /></data>")
            .expect("a file");

        let error = flat(&[directory.join("other.xml")]).expect_err("nothing to flash");
        assert!(error.contains("neither images to flash nor patches"), "{error}");

        std::fs::remove_dir_all(&directory).expect("clean up");
    }

    /// A package folder holds files that are not part of the flash —
    /// `poweroff.xml` only asks the phone to switch off — and a pick that takes
    /// them along still reads the package.
    #[test]
    fn a_file_that_is_not_part_of_the_flash_is_passed_over() {
        let directory = scratch("extra");
        rawprogram(&directory);
        std::fs::write(
            directory.join("poweroff.xml"),
            r#"<?xml version="1.0" ?><data><power DelayInSeconds="5" value="off" /></data>"#,
        )
        .expect("a power file");

        let plan = flat(&[
            directory.join("rawprogram0.xml"),
            directory.join("poweroff.xml"),
        ])
        .expect("a plan");

        assert_eq!(plan.entries.len(), 2);
        assert_eq!(plan.rawprograms.len(), 1);

        std::fs::remove_dir_all(&directory).expect("clean up");
    }

    /// A `<zeroout>` names a range to fill with zeroes, and no sector size of
    /// its own: the file's own is what it is read with.
    #[test]
    fn a_zeroout_is_a_range_to_fill_with_zeroes() {
        let directory = scratch("zeroout");
        std::fs::write(directory.join("xbl.elf"), b"xbl image").expect("an image");
        std::fs::write(
            directory.join("rawprogram0.xml"),
            r#"<?xml version="1.0" ?>
<data>
  <zeroout label="PrimaryGPT" num_partition_sectors="34" physical_partition_number="0" start_sector="0"/>
  <zeroout label="BackupGPT" num_partition_sectors="33" physical_partition_number="0" start_sector="NUM_DISK_SECTORS-33."/>
  <program SECTOR_SIZE_IN_BYTES="512" file_sector_offset="0" filename="xbl.elf"
           label="xbl" num_partition_sectors="2" physical_partition_number="0" start_sector="6"/>
</data>"#,
        )
        .expect("a rawprogram file");

        let plan = flat(&[directory.join("rawprogram0.xml")]).expect("a plan");

        assert_eq!(plan.entries.len(), 1);
        assert_eq!(plan.erases.len(), 2);
        assert_eq!(
            plan.actions,
            [
                Action::Erase(0),
                Action::Erase(1),
                Action::Program(0)
            ]
        );

        // The file's sector size, not the 4096 a table would fall back to.
        assert_eq!(plan.erases[0].label, "PrimaryGPT");
        assert_eq!(plan.erases[0].sector_size, 512);
        assert_eq!(plan.erases[0].sectors, 34);
        assert!(plan.erases[0].zero);

        assert_eq!(
            plan.erases[1].start_sector,
            Sector::Relative("NUM_DISK_SECTORS-33.".to_owned())
        );

        std::fs::remove_dir_all(&directory).expect("clean up");
    }

    /// The backup copy of a partition table sits at the far end of the storage,
    /// so the packages name its sector against the storage's own length instead
    /// of numbering it: `start_sector="NUM_DISK_SECTORS-5."`.
    #[test]
    fn a_sector_can_name_the_end_of_the_storage() {
        let directory = scratch("relative");
        std::fs::write(directory.join("gpt_backup4.bin"), vec![0_u8; 5 * 4096])
            .expect("an image");
        std::fs::write(
            directory.join("rawprogram4.xml"),
            r#"<?xml version="1.0" ?>
<data>
  <program SECTOR_SIZE_IN_BYTES="4096" file_sector_offset="0" filename="gpt_backup4.bin"
           label="BackupGPT" num_partition_sectors="5" partofsingleimage="true"
           physical_partition_number="4" readbackverify="false" size_in_KB="20.0"
           sparse="false" start_byte_hex="(4096*NUM_DISK_SECTORS)-20480."
           start_sector="NUM_DISK_SECTORS-5." />
</data>"#,
        )
        .expect("a rawprogram file");

        let plan = flat(&[directory.join("rawprogram4.xml")]).expect("a plan");
        let entry = &plan.entries[0];

        assert_eq!(
            entry.start_sector,
            Sector::Relative("NUM_DISK_SECTORS-5.".to_owned())
        );
        assert!(entry.start_sector.needs_geometry());
        // A 128 GB UFS LUN of 4096-byte sectors ends at 30 535 680, so the
        // backup table starts five sectors before it.
        assert_eq!(entry.start_sector.at(30_535_680).expect("a sector"), 30_535_675);

        std::fs::remove_dir_all(&directory).expect("clean up");
    }

    /// The expressions the packages use are arithmetic over the phone's own
    /// geometry, with the trailing dot the reference tools write.
    #[test]
    fn an_expression_is_worked_out_against_the_devices_length() {
        assert_eq!(expression_value("NUM_DISK_SECTORS", 1000), Ok(1000));
        assert_eq!(expression_value("NUM_DISK_SECTORS-1.", 1000), Ok(999));
        assert_eq!(expression_value("NUM_DISK_SECTORS-5.", 1000), Ok(995));
        assert_eq!(expression_value("NUM_DISK_SECTORS+2.", 1000), Ok(1002));
        assert_eq!(expression_value("NUM_DISK_SECTORS-1", 1000), Ok(999));
        assert_eq!(expression_value("32", 1000), Ok(32));
        assert_eq!(expression_value("0x20", 1000), Ok(32));

        assert!(expression_value("NUM_DISK_SECTORS*2", 1000).is_err());
        assert!(expression_value("thirty", 1000).is_err());
    }

    /// A patch file names either the storage or one of the package's own
    /// images. The images are written as they ship, so only the `DISK` patches
    /// are the ones a flash runs.
    #[test]
    fn only_the_patches_meant_for_the_storage_are_kept() {
        let text = r#"<?xml version="1.0" ?>
<patches>
  <patch SECTOR_SIZE_IN_BYTES="4096" byte_offset="48" filename="gpt_main0.bin"
         physical_partition_number="0" size_in_bytes="8" start_sector="1"
         value="NUM_DISK_SECTORS-6." what="primary header" />
  <patch SECTOR_SIZE_IN_BYTES="4096" byte_offset="48" filename="DISK"
         physical_partition_number="0" size_in_bytes="8" start_sector="1"
         value="NUM_DISK_SECTORS-6." what="primary header" />
  <patch SECTOR_SIZE_IN_BYTES="4096" byte_offset="48" filename="gpt_backup0.bin"
         physical_partition_number="0" size_in_bytes="8" start_sector="4"
         value="NUM_DISK_SECTORS-6." what="backup header" />
  <patch SECTOR_SIZE_IN_BYTES="4096" byte_offset="48" filename="DISK"
         physical_partition_number="0" size_in_bytes="8"
         start_sector="NUM_DISK_SECTORS-1." value="NUM_DISK_SECTORS-6."
         what="backup header" />
</patches>"#;

        let patches = read_patch(text, Path::new("patch0.xml")).expect("the patches");

        // Two of the four edit files that are written as they are; their
        // sector numbers are positions inside those files, not on the storage.
        assert_eq!(patches.len(), 2);
        assert_eq!(patches[0].label, "DISK");
        assert_eq!(patches[0].start_sector, Sector::At(1));
        assert_eq!(
            patches[1].start_sector,
            Sector::Relative("NUM_DISK_SECTORS-1.".to_owned())
        );
        assert_eq!(patches[1].start_sector.at(1000).expect("a sector"), 999);
    }

    /// `sec`, `secdata` and the fuse backup are the three the packages are
    /// flashed without.
    #[test]
    fn the_restricted_partitions_are_skipped_by_default() {
        for label in ["sec", "secdata", "efuseBackup", "EFUSEBACKUP"] {
            assert!(Plan::skipped_by_default(label), "{label}");
        }

        for label in ["xbl_a", "modemst1", "system_a", "userdata"] {
            assert!(!Plan::skipped_by_default(label), "{label}");
        }
    }

    /// A meta build names its programmer and its files, and the files are
    /// found next to `contents.xml`.
    #[test]
    fn a_meta_build_names_the_files_it_is_made_of() {
        let directory = scratch("meta");
        rawprogram(&directory);
        patch(&directory);
        std::fs::write(
            directory.join("contents.xml"),
            r#"<?xml version="1.0" ?>
<contents>
  <builds_flat>
    <build>
      <description>a package</description>
      <download_file>prog_firehose_ddr.elf</download_file>
    </build>
  </builds_flat>
  <builds_meta>
    <build>
      <file_ref>
        <file_name>prog_firehose_ddr.elf</file_name>
        <file_path>prog_firehose_ddr.elf</file_path>
      </file_ref>
      <file_ref>
        <file_name>rawprogram0.xml</file_name>
        <file_path>.\rawprogram0.xml</file_path>
      </file_ref>
      <file_ref>
        <file_name>patch0.xml</file_name>
        <file_path>.\patch0.xml</file_path>
      </file_ref>
    </build>
  </builds_meta>
</contents>"#,
        )
        .expect("a contents file");
        std::fs::write(directory.join("prog_firehose_ddr.elf"), b"the programmer")
            .expect("a programmer");

        let plan = meta(&directory.join("contents.xml")).expect("a plan");

        assert_eq!(
            plan.programmer,
            Some(directory.join("prog_firehose_ddr.elf"))
        );
        assert_eq!(plan.rawprograms.len(), 1);
        assert_eq!(plan.patches_files.len(), 1);
        assert_eq!(plan.entries.len(), 2);
        assert_eq!(plan.patches.len(), 1);

        std::fs::remove_dir_all(&directory).expect("clean up");
    }

    /// A number is read as decimal or as hex, the way the files spell them.
    #[test]
    fn numbers_are_decimal_or_hex() {
        assert_eq!(number_of("4096"), Some(4096));
        assert_eq!(number_of("0x1000"), Some(4096));
        assert_eq!(number_of(" 0X10 "), Some(16));
        assert_eq!(number_of("nope"), None);
    }
}
