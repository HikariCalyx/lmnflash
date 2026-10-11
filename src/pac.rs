//! Unisoc (Spreadtrum) PAC firmware packages.
//!
//! A PAC ("packet") is the container the Unisoc download tools flash a device
//! from: one fixed header, a table of file records, and the payloads those
//! records point at. The layout is the one the reference tool's own C
//! structures declare — `BIN_PACKET_HEADER` and `FILE_INFO` in
//! `reference/unisoc-download/Source/Common/Include/def/packet_def.h` — and it
//! was verified against a real package
//! (`reference/TYH201H_HNR552T_N1770.6.12.09.00.pac`: 54 records, 42 of them
//! files, the first payload starting right behind the table):
//!
//! ```text
//! header (2124 bytes)
//!   version[22] | size u64 | product[256] | version[256] | record count |
//!   table offset | … | encrypted flag | … | magic 0xFFFAFFFA | crc16 | crc16
//! table (record count × 2580 bytes), right behind the header
//!   record size | id[256] | name[256] | version[252] | size u64 |
//!   kind | required | data offset u64 | omit | address count | addr[5] | …
//! payloads (every record names the ABSOLUTE offset of its file)
//! ```
//!
//! Strings are UTF-16LE and numbers are little-endian. Only the plain form is
//! read: a "safe mode" package keeps its file table encrypted (its
//! `wEncrypted`/`dwFTEncryptedSize` fields are set) and is refused with
//! [`PacError::Encrypted`] instead of being misread as garbage.
//!
//! A package can also be PARTIAL — an interrupted download or copy, which is
//! how a damaged one usually reaches this tool. The header and the file table
//! sit at the very start, so the list of files survives the missing tail:
//! [`list`] keeps a payload that is cut short with the bytes that are there and
//! marks it with [`Entry::complete`] = `false`, leaves out a payload that is
//! missing entirely, and reports the package itself as incomplete through
//! [`Package::complete`]. Only a package whose header or table is damaged is
//! refused, since nothing can be listed from one.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

/// Size of the fixed package header.
pub const HEADER_SIZE: u64 = 2124;

/// Size of one file-table record.
pub const ENTRY_SIZE: usize = 2580;

/// The value stored at the end of the header, and the only marker of the
/// format.
const MAGIC: u32 = 0xFFFA_FFFA;
const MAGIC_OFFSET: usize = 2116;

/// Where the header's own fields sit. The package size is stored as a high and
/// a low 32-bit half, in that order.
const SIZE_HIGH_OFFSET: usize = 44;
const SIZE_LOW_OFFSET: usize = 48;
const VERSION_OFFSET: usize = 0;
const VERSION_LEN: usize = 22;
const PRODUCT_OFFSET: usize = 52;
const PRODUCT_LEN: usize = 256;
const PRODUCT_VERSION_OFFSET: usize = 564;
const PRODUCT_VERSION_LEN: usize = 256;
const RECORD_COUNT_OFFSET: usize = 1076;
const TABLE_OFFSET: usize = 1080;
const MODE_OFFSET: usize = 1084;
const FLASH_TYPE_OFFSET: usize = 1088;
const NAND_STRATEGY_OFFSET: usize = 1092;
const NV_BACKUP_OFFSET: usize = 1096;
const NAND_PAGE_TYPE_OFFSET: usize = 1100;
const PRODUCT_ALIAS_OFFSET: usize = 1104;
const PRODUCT_ALIAS_LEN: usize = 100;
const OMA_DM_PRODUCT_FLAG_OFFSET: usize = 1304;
const OMA_DM_OFFSET: usize = 1308;
const IS_PRELOAD_OFFSET: usize = 1312;
const ENCRYPTED_OFFSET: usize = 1316;

/// Where a record's fields sit.
const RECORD_SIZE_OFFSET: usize = 0;
const ID_OFFSET: usize = 4;
const ID_LEN: usize = 256;
const NAME_OFFSET: usize = 516;
const NAME_LEN: usize = 256;
const ENTRY_SIZE_HIGH_OFFSET: usize = 1532;
const ENTRY_OFFSET_HIGH_OFFSET: usize = 1536;
const ENTRY_SIZE_LOW_OFFSET: usize = 1540;
const ENTRY_KIND_OFFSET: usize = 1544;
const ENTRY_FLAG_OFFSET: usize = 1548;
const ENTRY_OFFSET_LOW_OFFSET: usize = 1552;
const ENTRY_OMIT_OFFSET: usize = 1556;
const ENTRY_ADDR_COUNT_OFFSET: usize = 1560;
const ENTRY_ADDR_OFFSET: usize = 1564;

/// Where the header's two checksums sit, and how much of the header the first
/// one covers.
const HEAD_CRC_OFFSET: usize = 2120;
const DATA_CRC_OFFSET: usize = 2122;
const HEAD_CRC_LEN: usize = HEAD_CRC_OFFSET;

/// Most records a package may declare. A real package holds dozens; the cap
/// keeps a damaged header from asking for a multi-gigabyte table.
const MAX_RECORDS: usize = 4096;

/// Bytes copied at a time while a payload is written out.
const COPY_BUFFER: usize = 64 * 1024;

/// Name of the flash configuration written next to the unpacked files. It is
/// the name the Unisoc tools use.
pub const CONFIG_FILE: &str = "flash.cfg";

/// What the package says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// Version of the header structure itself, e.g. `BP_R2.0.1`.
    pub version: String,
    /// Product the package is built for, e.g. `HNR552T`.
    pub product_name: String,
    /// The package's own version, e.g. `N1770.6.12.09.00`.
    pub product_version: String,
    /// The product alias, which many packages fill with the product name.
    pub product_alias: String,
    /// The rest of the `[Setting]` block of the flash configuration. They are
    /// the download tool's own switches, kept as the package spells them.
    pub mode: u32,
    pub flash_type: u32,
    pub nand_strategy: u32,
    pub nand_page_type: u32,
    pub nv_backup: u32,
    pub oma_dm_product_flag: u32,
    pub oma_dm: u32,
    pub is_preload: u32,
}

/// A file stored in a package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The id of the record it belongs to, e.g. `FDL`, `VBMETA` or `Modem_LTE`.
    /// It is what a scatter XML names a record by, so it is what ties a file to
    /// the partition it is written to.
    pub id: String,
    /// The file name, as it will be written to the output directory.
    pub name: String,
    /// Payload size in bytes. For a file of a partial package this is the
    /// number of bytes that are actually there.
    pub size: u64,
    /// Absolute offset of the payload inside the package file.
    pub offset: u64,
    /// Whether the whole payload is present. `false` only for a file of a
    /// partial package whose payload is cut short.
    pub complete: bool,
}

/// One record of a package's file table, as the flash configuration lists it.
///
/// Unlike [`Entry`] this keeps the records that carry no file as well — the
/// pure operations such as `EraseMisc` — and keeps the table's order, which is
/// what the flash configuration describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// The record's own id, e.g. `FDL`, `VBMETA` or `EraseMisc`.
    pub id: String,
    /// The file it names, empty for a pure operation.
    pub name: String,
    /// The record's download flag: the number the flash configuration writes
    /// before the `@` (the record's `nCheckFlag` field).
    pub flag: u32,
    /// The record's `nFileFlag`: 0 = a pure operation, 1 = a file, 2 = the
    /// package's own configuration XML.
    pub kind: u32,
    /// The record's `dwCanOmitFlag`: 1 = the download tool leaves it out of
    /// its "all files" selection.
    pub omit: u32,
    /// The block base addresses the record is written to (`dwAddr`), at most
    /// five.
    pub addresses: Vec<u32>,
}

/// A package's header and the files it holds.
///
/// Records that carry no file — the pure operations a package also stores,
/// such as `FLASH` or `EraseSPL` — are left out of `files`, so every entry can
/// be extracted; `records` holds the whole table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub header: Header,
    /// The files that can be extracted.
    pub files: Vec<Entry>,
    /// The whole file table, in the order it is stored.
    pub records: Vec<Record>,
    /// Size the header records for the whole package.
    pub expected_size: u64,
    /// Size of the package file on disk. Smaller than `expected_size` for a
    /// partial package.
    pub actual_size: u64,
    /// Whether the file is at least as long as that size says. A partial
    /// package still lists its files; it only says that some of them are
    /// missing or cut short.
    pub complete: bool,
}

/// Why a package could not be read or unpacked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PacError {
    /// The package could not be read.
    Read(String),
    /// The header does not carry the expected magic: not this format.
    NotAPac,
    /// The package's file table is encrypted ("safe mode"), which this tool
    /// cannot read.
    Encrypted,
    /// The file table is truncated, or points outside the package.
    BadTable,
    /// An entry names something that is not a plain file.
    BadName(String),
    /// A file could not be written.
    Write(String),
}

impl std::fmt::Display for PacError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read(error) => write!(f, "failed to read the package: {error}"),
            Self::NotAPac => write!(f, "not a Unisoc PAC package"),
            Self::Encrypted => write!(f, "the package's file table is encrypted"),
            Self::BadTable => write!(f, "the package's file table is truncated or malformed"),
            Self::BadName(name) => write!(f, "`{name}` is not a usable file name"),
            Self::Write(error) => write!(f, "failed to write the files: {error}"),
        }
    }
}

impl std::error::Error for PacError {}

/// What a successful run produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    /// Number of files written.
    pub files: usize,
    /// Total number of payload bytes written.
    pub bytes: u64,
}

/// Progress reported while a package is unpacked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PacEvent {
    /// A file is about to be written.
    File { name: String, size: u64 },
    /// `bytes` payload bytes have been written so far (across all files).
    Progress { bytes: u64, total: u64 },
    /// The run is over. Always the last event: the worker reports its outcome
    /// itself, so the dialog can never wait for an event that does not come.
    Finished(Result<Summary, PacError>),
}

/// A little-endian `u32` at an offset of a buffer that is known to be long
/// enough (header and record buffers are fixed sizes).
fn u32_at(buffer: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(buffer[offset..offset + 4].try_into().expect("4 bytes"))
}

/// A NUL-terminated UTF-16LE string in a fixed-size field.
fn wide_string(buffer: &[u8], offset: usize, units: usize) -> String {
    let units: Vec<u16> = (0..units)
        .map(|index| {
            let start = offset + index * 2;
            u16::from_le_bytes([buffer[start], buffer[start + 1]])
        })
        .collect();

    let end = units
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(units.len());

    String::from_utf16_lossy(&units[..end])
}

/// Joins the high and low half of a 64-bit value.
fn combine(high: u32, low: u32) -> u64 {
    ((high as u64) << 32) | low as u64
}

/// What the header says about the package's file table.
struct Table {
    /// Number of records, files and operations alike.
    record_count: usize,
    /// Absolute offset of the first record.
    offset: u64,
    /// Size the header records for the whole package.
    expected_size: u64,
}

/// Reads the fixed header of a package, together with what it says about the
/// package's file table.
fn parse_header(bytes: &[u8]) -> Result<(Header, Table), PacError> {
    if u32_at(bytes, MAGIC_OFFSET) != MAGIC {
        return Err(PacError::NotAPac);
    }

    // "Safe mode" packages keep their file table encrypted: the flag and the
    // size of the encrypted table are both set. Reading the table anyway would
    // only produce garbage names.
    if u16::from_le_bytes(
        bytes[ENCRYPTED_OFFSET..ENCRYPTED_OFFSET + 2]
            .try_into()
            .expect("2 bytes"),
    ) != 0
    {
        return Err(PacError::Encrypted);
    }

    let record_count = u32_at(bytes, RECORD_COUNT_OFFSET) as usize;

    if record_count > MAX_RECORDS {
        return Err(PacError::BadTable);
    }

    let header = Header {
        version: wide_string(bytes, VERSION_OFFSET, VERSION_LEN),
        product_name: wide_string(bytes, PRODUCT_OFFSET, PRODUCT_LEN),
        product_version: wide_string(bytes, PRODUCT_VERSION_OFFSET, PRODUCT_VERSION_LEN),
        product_alias: wide_string(bytes, PRODUCT_ALIAS_OFFSET, PRODUCT_ALIAS_LEN),
        mode: u32_at(bytes, MODE_OFFSET),
        flash_type: u32_at(bytes, FLASH_TYPE_OFFSET),
        nand_strategy: u32_at(bytes, NAND_STRATEGY_OFFSET),
        nand_page_type: u32_at(bytes, NAND_PAGE_TYPE_OFFSET),
        nv_backup: u32_at(bytes, NV_BACKUP_OFFSET),
        oma_dm_product_flag: u32_at(bytes, OMA_DM_PRODUCT_FLAG_OFFSET),
        oma_dm: u32_at(bytes, OMA_DM_OFFSET),
        is_preload: u32_at(bytes, IS_PRELOAD_OFFSET),
    };

    let table = Table {
        record_count,
        offset: u32_at(bytes, TABLE_OFFSET) as u64,
        expected_size: combine(u32_at(bytes, SIZE_HIGH_OFFSET), u32_at(bytes, SIZE_LOW_OFFSET)),
    };

    Ok((header, table))
}

/// Reads one file-table record.
fn parse_entry(record: &[u8]) -> Entry {
    let name = wide_string(record, NAME_OFFSET, NAME_LEN);
    let size = combine(
        u32_at(record, ENTRY_SIZE_HIGH_OFFSET),
        u32_at(record, ENTRY_SIZE_LOW_OFFSET),
    );
    let offset = combine(
        u32_at(record, ENTRY_OFFSET_HIGH_OFFSET),
        u32_at(record, ENTRY_OFFSET_LOW_OFFSET),
    );

    Entry {
        id: wide_string(record, ID_OFFSET, ID_LEN),
        name,
        size,
        offset,
        // The caller clips this when the payload is not fully there.
        complete: true,
    }
}

/// Reads one record's id, file name, download flag and the fields the flash
/// tool needs to write it out again.
fn parse_record(record: &[u8]) -> Record {
    Record {
        id: wide_string(record, ID_OFFSET, ID_LEN),
        name: wide_string(record, NAME_OFFSET, NAME_LEN),
        flag: u32_at(record, ENTRY_FLAG_OFFSET),
        kind: u32_at(record, ENTRY_KIND_OFFSET),
        omit: u32_at(record, ENTRY_OMIT_OFFSET),
        addresses: (0..5)
            .map(|index| u32_at(record, ENTRY_ADDR_OFFSET + index * 4))
            .take(u32_at(record, ENTRY_ADDR_COUNT_OFFSET).min(5) as usize)
            .collect(),
    }
}

impl Entry {
    /// Whether the record carries an actual file. Operations store neither a
    /// name nor a payload, and have nothing to extract.
    fn has_payload(&self) -> bool {
        !self.name.is_empty() && self.size > 0
    }
}

/// Reads the table of a package: the files that can be extracted, and the whole
/// table the way the flash configuration describes it.
///
/// The tail of a partial package is missing, so a payload that is not fully
/// there is kept with the bytes that are present and marked incomplete instead
/// of failing the read: recovering what a damaged dump does hold is the whole
/// point of listing it. Only a table that is not completely there is an error,
/// since nothing can be listed from one.
fn read_table(
    file: &mut File,
    table: &Table,
    length: u64,
) -> Result<(Vec<Entry>, Vec<Record>), PacError> {
    let end = table
        .offset
        .checked_add(table.record_count as u64 * ENTRY_SIZE as u64)
        .ok_or(PacError::BadTable)?;

    // The records live behind the header, and the whole table has to fit in
    // the package.
    if table.offset < HEADER_SIZE || end > length {
        return Err(PacError::BadTable);
    }

    let mut record = vec![0u8; ENTRY_SIZE];
    let mut files = Vec::new();
    let mut records = Vec::new();

    for index in 0..table.record_count {
        let start = table.offset + index as u64 * ENTRY_SIZE as u64;

        file.seek(SeekFrom::Start(start))
            .map_err(|error| PacError::Read(error.to_string()))?;
        file.read_exact(&mut record).map_err(|error| match error.kind() {
            io::ErrorKind::UnexpectedEof => PacError::BadTable,
            _ => PacError::Read(error.to_string()),
        })?;

        // Every record is described, the operations included …
        records.push(parse_record(&record));

        // … but only the ones that carry a file can be extracted.
        let mut entry = parse_entry(&record);

        if !entry.has_payload() {
            continue;
        }

        // A payload lives behind the header and the table. A record pointing
        // into the header holds nothing usable, and one that starts behind the
        // end of the file has nothing left to write.
        if entry.offset < HEADER_SIZE || entry.offset >= length {
            continue;
        }

        let available = length - entry.offset;

        if entry.size > available {
            entry.size = available;
            entry.complete = false;
        }

        files.push(entry);
    }

    Ok((files, records))
}

/// Whether a file name matches what the search box contains.
///
/// The query is looked for, without case, in the file's name; an empty query
/// (or one that is only spaces) matches every file.
pub fn matches(name: &str, query: &str) -> bool {
    let query = query.trim();

    if query.is_empty() {
        return true;
    }

    name.to_lowercase().contains(&query.to_lowercase())
}

/// Reads a package's header and file table without copying any payload.
///
/// This is what the dialog shows after a file was picked: it both validates
/// the package and lists what is inside before anything is written.
pub fn list(package: &Path) -> Result<Package, PacError> {
    let mut file = File::open(package).map_err(|error| PacError::Read(error.to_string()))?;
    let length = file
        .metadata()
        .map_err(|error| PacError::Read(error.to_string()))?
        .len();

    let mut header = [0u8; HEADER_SIZE as usize];
    file.read_exact(&mut header).map_err(|error| match error.kind() {
        // A file shorter than the header cannot be one of ours.
        io::ErrorKind::UnexpectedEof => PacError::NotAPac,
        _ => PacError::Read(error.to_string()),
    })?;

    let (header, table) = parse_header(&header)?;

    // The length of the file — not the size the header records — is what a
    // payload has to fit into.
    let (files, records) = read_table(&mut file, &table, length)?;

    Ok(Package {
        header,
        files,
        records,
        expected_size: table.expected_size,
        actual_size: length,
        // A package that does not say how long it should be cannot be called
        // incomplete.
        complete: table.expected_size == 0 || length >= table.expected_size,
    })
}

/// Reads the payload of one of a package's files.
///
/// The tools that need a file the user may not have extracted — the
/// configuration XML, which names the partitions a `spd_dump` command writes
/// to — read it from the package itself.
pub(crate) fn payload(package: &Path, entry: &Entry) -> Result<Vec<u8>, PacError> {
    let mut file = File::open(package).map_err(|error| PacError::Read(error.to_string()))?;

    file.seek(SeekFrom::Start(entry.offset))
        .map_err(|error| PacError::Read(error.to_string()))?;

    // `size` is what is really there for a file of a partial package.
    let mut buffer = vec![0u8; entry.size as usize];

    file.read_exact(&mut buffer)
        .map_err(|error| PacError::Read(error.to_string()))?;

    Ok(buffer)
}

/// The name of the package's own configuration XML, the file the flash
/// configuration points at with `PAC_CONFILE`. Empty when the package carries
/// none, which is what the tools that can only work from it go by.
pub(crate) fn config_name(package: &Package) -> &str {
    package
        .records
        .iter()
        .map(|record| record.name.as_str())
        .find(|name| name.to_ascii_lowercase().ends_with(".xml"))
        .unwrap_or("")
}

/// Whether the package carries that configuration XML.
pub(crate) fn has_configuration(package: &Package) -> bool {
    !config_name(package).is_empty()
}

/// Renders the `flash.cfg` the Unisoc tools write next to the files they
/// unpack.
///
/// It is an INI-style description of the package: a `[Setting]` block taken
/// from the header, and one `[FlashParam]` line per file-table record —
/// `ID=<flag>@<file>`, with an empty file for the pure operations
/// (`EraseMisc=1@`) — in the order the table holds them. The layout follows
/// `reference/scattered_unisoc_fw/flash.cfg`, the file the official tool left
/// next to a scattered firmware: a blank line before EVERY section header (the
/// first one included, which is why the file starts with a newline), LF line
/// endings, and a trailing newline.
///
/// A record whose `ID` is empty is left out: the id is the line's key, and an
/// INI file cannot have an empty one. A package that leaves the id of its
/// configuration XML empty is normal (the reference `HNR552T` package does),
/// and that file is still named by `PAC_CONFILE` above — and still unpacked.
///
/// A package does not store this file: the Unisoc download tool GENERATES it
/// while it unpacks one, so it is generated here as well. It describes the
/// whole package, not only the files the user checked.
pub fn flash_cfg(package: &Package) -> String {
    let header = &package.header;
    let mut config = String::new();

    config.push('\n');
    config.push_str("[Setting]\n");

    for (key, value) in [
        ("PacVer", header.version.as_str()),
        ("PAC_PRODUCT", header.product_name.as_str()),
        ("ProductAlias", header.product_alias.as_str()),
        ("ProductVer", header.product_version.as_str()),
    ] {
        config.push_str(key);
        config.push('=');
        config.push_str(value);
        config.push('\n');
    }

    for (key, value) in [
        ("Mode", header.mode),
        ("FlashType", header.flash_type),
        ("NandStrategy", header.nand_strategy),
        ("NandPageType", header.nand_page_type),
        ("NvBackup", header.nv_backup),
        ("OmaDmProductFlag", header.oma_dm_product_flag),
        ("OmaDM", header.oma_dm),
        ("IsPreload", header.is_preload),
    ] {
        config.push_str(key);
        config.push('=');
        config.push_str(&value.to_string());
        config.push('\n');
    }

    config.push_str("PAC_CONFILE=");
    config.push_str(config_name(package));
    config.push('\n');

    config.push('\n');
    config.push_str("[FlashParam]\n");

    for record in &package.records {
        // The id is the key, so a record that has none cannot be written.
        if record.id.is_empty() {
            continue;
        }

        config.push_str(&record.id);
        config.push('=');
        config.push_str(&record.flag.to_string());
        config.push('@');
        config.push_str(&record.name);
        config.push('\n');
    }

    config
}

/// A file name that can safely be written into the output directory.
///
/// Entry names come from the package, so anything that could escape the chosen
/// directory (a path separator, `.`, `..`) is refused instead of followed.
fn safe_file_name(name: &str) -> Result<&str, PacError> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\'])
        || name.contains(':')
    {
        return Err(PacError::BadName(name.to_string()));
    }

    Ok(name)
}

/// A name that is not taken yet in this run.
///
/// A package may hold the same file name twice (the reference sample does, for
/// `fbootlogo.bin`), and both have to end up on disk.
fn unique_name(used: &mut HashSet<String>, name: &str) -> String {
    if used.insert(name.to_string()) {
        return name.to_string();
    }

    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, Some(extension)),
        _ => (name, None),
    };

    for number in 2.. {
        let candidate = match extension {
            Some(extension) => format!("{stem} ({number}).{extension}"),
            None => format!("{name} ({number})"),
        };

        if used.insert(candidate.clone()) {
            return candidate;
        }
    }

    unreachable!("an unused name is always found")
}

/// Copies one entry's payload into `output`, reporting progress as it goes.
#[allow(clippy::too_many_arguments)]
fn copy_payload(
    file: &mut File,
    entry: &Entry,
    output: &Path,
    written: &mut u64,
    total: u64,
    emit: &dyn Fn(PacEvent),
    last_reported: &mut u64,
) -> Result<(), PacError> {
    let mut output = File::create(output).map_err(|error| PacError::Write(error.to_string()))?;

    let mut remaining = entry.size;
    let mut buffer = vec![0u8; COPY_BUFFER.min(entry.size.max(1) as usize)];

    while remaining > 0 {
        let chunk = remaining.min(buffer.len() as u64) as usize;
        file.read_exact(&mut buffer[..chunk])
            .map_err(|error| PacError::Read(error.to_string()))?;
        output
            .write_all(&buffer[..chunk])
            .map_err(|error| PacError::Write(error.to_string()))?;

        remaining -= chunk as u64;
        *written += chunk as u64;

        // Report at most every permille of the payload, like the flash engine
        // reports its own progress.
        if total > 0 {
            let permille = *written * 1000 / total;
            if permille != *last_reported {
                *last_reported = permille;
                emit(PacEvent::Progress {
                    bytes: *written,
                    total,
                });
            }
        }
    }

    output
        .flush()
        .map_err(|error| PacError::Write(error.to_string()))?;

    Ok(())
}

/// Whether a run writes the flash configuration next to the files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Configuration {
    /// Write it, the way the Unisoc tools leave it in a scattered firmware.
    Write,
    /// Leave it out: a run that takes nothing but the two download agents is
    /// not a scattered firmware, and a configuration listing files that are
    /// not there only misleads the tools that read it.
    Skip,
}

/// Extracts the selected files of a package into `destination`.
///
/// `selected` holds indexes into [`Package::files`]. The directory is created
/// when missing and progress is reported through `emit`, which always receives
/// [`PacEvent::Finished`] last. Next to the files, the flash configuration
/// ([`flash_cfg`], named [`CONFIG_FILE`]) is written unless the run is not a
/// scattered firmware and says so with [`Configuration::Skip`].
pub fn unpack(
    package: &Path,
    destination: &Path,
    selected: &[usize],
    configuration: Configuration,
    emit: &dyn Fn(PacEvent),
) -> Result<Summary, PacError> {
    let result = unpack_inner(package, destination, selected, configuration, emit);

    // The outcome is reported in exactly one place, so no early return can
    // skip it and leave the dialog waiting.
    emit(PacEvent::Finished(result.clone()));

    result
}

fn unpack_inner(
    package: &Path,
    destination: &Path,
    selected: &[usize],
    configuration: Configuration,
    emit: &dyn Fn(PacEvent),
) -> Result<Summary, PacError> {
    let parsed = list(package)?;
    let chosen: Vec<&Entry> = selected
        .iter()
        .filter_map(|&index| parsed.files.get(index))
        .collect();

    let total: u64 = chosen.iter().map(|entry| entry.size).sum();

    fs::create_dir_all(destination).map_err(|error| PacError::Write(error.to_string()))?;

    let mut file = File::open(package).map_err(|error| PacError::Read(error.to_string()))?;

    let mut used = HashSet::new();
    let mut written = 0u64;
    let mut reported = 0u64;
    let mut files = 0usize;

    // Reserve the configuration's name first, so a file the package happens to
    // call `flash.cfg` cannot overwrite it (that file becomes `flash (2).cfg`).
    // A run without a configuration has no name to reserve.
    if configuration == Configuration::Write {
        used.insert(CONFIG_FILE.to_string());
    }

    for entry in chosen {
        safe_file_name(&entry.name)?;

        emit(PacEvent::File {
            name: entry.name.clone(),
            size: entry.size,
        });

        let name = unique_name(&mut used, &entry.name);
        let output = destination.join(name);

        file.seek(SeekFrom::Start(entry.offset))
            .map_err(|error| PacError::Read(error.to_string()))?;

        copy_payload(
            &mut file,
            entry,
            &output,
            &mut written,
            total,
            emit,
            &mut reported,
        )?;

        files += 1;
    }

    // Make sure the bar reaches its end even for a tiny payload.
    emit(PacEvent::Progress { bytes: total, total });

    // What the files are and where they belong, for the tools that flash a
    // scattered firmware. A run of nothing but the download agents is not one,
    // and leaves no configuration behind.
    if configuration == Configuration::Write {
        fs::write(destination.join(CONFIG_FILE), flash_cfg(&parsed))
            .map_err(|error| PacError::Write(error.to_string()))?;
    }

    Ok(Summary {
        files,
        bytes: total,
    })
}

/// A record to write, with the file its payload comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payload {
    pub record: Record,
    /// The file to copy into the package, or `None` for a record that carries
    /// no payload (a pure operation).
    pub source: Option<std::path::PathBuf>,
}

/// Writes a little-endian `u32` into a fixed-size buffer.
fn write_u32(buffer: &mut [u8], offset: usize, value: u32) {
    buffer[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

/// Writes a little-endian `u16` into a fixed-size buffer.
fn write_u16(buffer: &mut [u8], offset: usize, value: u16) {
    buffer[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

/// Writes a NUL-padded UTF-16LE string into a fixed-size field.
fn write_wide(buffer: &mut [u8], offset: usize, units: usize, value: &str) {
    for (index, unit) in value.encode_utf16().take(units).enumerate() {
        write_u16(buffer, offset + index * 2, unit);
    }
}

/// The lookup table of the reflected checksum below, built once.
fn crc_table() -> &'static [u16; 256] {
    static TABLE: std::sync::OnceLock<[u16; 256]> = std::sync::OnceLock::new();

    TABLE.get_or_init(|| {
        let mut table = [0u16; 256];

        for (index, slot) in table.iter_mut().enumerate() {
            let mut value = index as u16;

            for _ in 0..8 {
                value = if value & 1 != 0 {
                    (value >> 1) ^ 0xA001
                } else {
                    value >> 1
                };
            }

            *slot = value;
        }

        table
    })
}

/// CRC-16/ARC, the reflected checksum with polynomial `0xA001` that a package
/// stores in its header: once over the header itself, once over everything
/// behind it.
///
/// Verified against the reference package (its stored head checksum is
/// `0x4242` and this computes the same) and against the standard check value
/// of the algorithm.
fn crc16(mut crc: u16, data: &[u8]) -> u16 {
    let table = crc_table();

    for &byte in data {
        crc = (crc >> 8) ^ table[((crc ^ byte as u16) & 0xFF) as usize];
    }

    crc
}

/// Writes a package: the header, the file table, and the payloads of `records`
/// right behind it, with both checksums the format stores.
///
/// Everything the format records about the package — the record count, the
/// table offset, the payload offsets, the package size and the checksums — is
/// computed here, so a caller only has to fill in the header fields and say
/// which file belongs to which record. Progress is reported through `emit`,
/// which always receives [`PacEvent::Finished`] last.
pub fn write(
    header: &Header,
    records: &[Payload],
    destination: &Path,
    emit: &dyn Fn(PacEvent),
) -> Result<Summary, PacError> {
    let result = write_inner(header, records, destination, emit);

    // The outcome is reported in exactly one place, so no early return can
    // skip it and leave the dialog waiting.
    emit(PacEvent::Finished(result.clone()));

    result
}

fn write_inner(
    header: &Header,
    records: &[Payload],
    destination: &Path,
    emit: &dyn Fn(PacEvent),
) -> Result<Summary, PacError> {
    if records.is_empty() || records.len() > MAX_RECORDS {
        return Err(PacError::Write(format!(
            "a package cannot hold {} records",
            records.len()
        )));
    }

    // The sizes come first: they decide where every payload goes.
    let mut sizes = Vec::with_capacity(records.len());
    let mut total = 0u64;

    for payload in records {
        let size = match &payload.source {
            Some(source) => fs::metadata(source)
                .map_err(|error| PacError::Read(format!("{}: {error}", source.display())))?
                .len(),
            None => 0,
        };

        sizes.push(size);
        total += size;
    }

    let table_size = records.len() as u64 * ENTRY_SIZE as u64;
    let mut offsets = Vec::with_capacity(records.len());
    let mut offset = HEADER_SIZE + table_size;

    for size in &sizes {
        offsets.push(offset);
        offset += size;
    }

    let package_size = offset;

    let header_bytes = build_header(header, records.len(), package_size);
    let table = build_table(records, &sizes, &offsets);
    let mut file = File::create(destination).map_err(|error| PacError::Write(error.to_string()))?;

    file.write_all(&header_bytes)
        .and_then(|()| file.write_all(&table))
        .map_err(|error| PacError::Write(error.to_string()))?;

    // Everything behind the header — the table included — is covered by the
    // second checksum.
    let mut data_crc = crc16(0, &table);
    let mut buffer = vec![0u8; COPY_BUFFER];
    let mut written = 0u64;
    let mut reported = 0u64;
    let mut files = 0usize;

    for (index, payload) in records.iter().enumerate() {
        let Some(source) = &payload.source else {
            continue;
        };

        emit(PacEvent::File {
            name: payload.record.name.clone(),
            size: sizes[index],
        });

        let mut input =
            File::open(source).map_err(|error| PacError::Read(error.to_string()))?;
        let mut remaining = sizes[index];

        while remaining > 0 {
            let chunk = remaining.min(buffer.len() as u64) as usize;

            input
                .read_exact(&mut buffer[..chunk])
                .map_err(|error| PacError::Read(error.to_string()))?;
            file.write_all(&buffer[..chunk])
                .map_err(|error| PacError::Write(error.to_string()))?;

            data_crc = crc16(data_crc, &buffer[..chunk]);
            remaining -= chunk as u64;
            written += chunk as u64;

            // Report at most every permille of the payload, like the other
            // long-running jobs do.
            if total > 0 {
                let permille = written * 1000 / total;

                if permille != reported {
                    reported = permille;
                    emit(PacEvent::Progress {
                        bytes: written,
                        total,
                    });
                }
            }
        }

        files += 1;
    }

    // Now that it is known, the second checksum goes into the header it was
    // left blank for.
    file.seek(SeekFrom::Start(DATA_CRC_OFFSET as u64))
        .and_then(|_| file.write_all(&data_crc.to_le_bytes()))
        .and_then(|()| file.flush())
        .map_err(|error| PacError::Write(error.to_string()))?;

    emit(PacEvent::Progress { bytes: total, total });

    Ok(Summary {
        files,
        bytes: total,
    })
}

/// The fixed header of a package, checksums included (the second one is only
/// known once the data behind the header is written, so it stays zero here).
fn build_header(header: &Header, record_count: usize, package_size: u64) -> Vec<u8> {
    let mut bytes = vec![0u8; HEADER_SIZE as usize];

    write_wide(&mut bytes, VERSION_OFFSET, VERSION_LEN, &header.version);
    write_wide(&mut bytes, PRODUCT_OFFSET, PRODUCT_LEN, &header.product_name);
    write_wide(
        &mut bytes,
        PRODUCT_VERSION_OFFSET,
        PRODUCT_VERSION_LEN,
        &header.product_version,
    );
    write_wide(
        &mut bytes,
        PRODUCT_ALIAS_OFFSET,
        PRODUCT_ALIAS_LEN,
        &header.product_alias,
    );
    write_u32(&mut bytes, SIZE_HIGH_OFFSET, (package_size >> 32) as u32);
    write_u32(&mut bytes, SIZE_LOW_OFFSET, package_size as u32);
    write_u32(&mut bytes, MODE_OFFSET, header.mode);
    write_u32(&mut bytes, FLASH_TYPE_OFFSET, header.flash_type);
    write_u32(&mut bytes, NAND_STRATEGY_OFFSET, header.nand_strategy);
    write_u32(&mut bytes, NV_BACKUP_OFFSET, header.nv_backup);
    write_u32(&mut bytes, NAND_PAGE_TYPE_OFFSET, header.nand_page_type);
    write_u32(
        &mut bytes,
        OMA_DM_PRODUCT_FLAG_OFFSET,
        header.oma_dm_product_flag,
    );
    write_u32(&mut bytes, OMA_DM_OFFSET, header.oma_dm);
    write_u32(&mut bytes, IS_PRELOAD_OFFSET, header.is_preload);
    write_u32(&mut bytes, RECORD_COUNT_OFFSET, record_count as u32);
    write_u32(&mut bytes, TABLE_OFFSET, HEADER_SIZE as u32);
    write_u32(&mut bytes, MAGIC_OFFSET, MAGIC);

    // The first checksum covers the header without the two checksum fields.
    let head_crc = crc16(0, &bytes[..HEAD_CRC_LEN]);
    write_u16(&mut bytes, HEAD_CRC_OFFSET, head_crc);

    bytes
}

/// The file table of a package: one fixed-size record per payload, in order.
fn build_table(payloads: &[Payload], sizes: &[u64], offsets: &[u64]) -> Vec<u8> {
    let mut table = vec![0u8; payloads.len() * ENTRY_SIZE];

    for (index, payload) in payloads.iter().enumerate() {
        let entry = &payload.record;
        let record = &mut table[index * ENTRY_SIZE..(index + 1) * ENTRY_SIZE];

        write_u32(record, RECORD_SIZE_OFFSET, ENTRY_SIZE as u32);
        write_wide(record, ID_OFFSET, ID_LEN, &entry.id);
        write_wide(record, NAME_OFFSET, NAME_LEN, &entry.name);
        write_u32(record, ENTRY_SIZE_HIGH_OFFSET, (sizes[index] >> 32) as u32);
        write_u32(
            record,
            ENTRY_OFFSET_HIGH_OFFSET,
            (offsets[index] >> 32) as u32,
        );
        write_u32(record, ENTRY_SIZE_LOW_OFFSET, sizes[index] as u32);
        write_u32(record, ENTRY_KIND_OFFSET, entry.kind);
        write_u32(record, ENTRY_FLAG_OFFSET, entry.flag);
        write_u32(record, ENTRY_OFFSET_LOW_OFFSET, offsets[index] as u32);
        write_u32(record, ENTRY_OMIT_OFFSET, entry.omit);

        // The block count is simply how many blocks the record has: the
        // reference package stores one for the records its product XML gives a
        // `<Block>` to (an empty base included), and none for the record that
        // carries the configuration XML itself.
        let addresses: Vec<u32> = entry.addresses.iter().copied().take(5).collect();

        write_u32(record, ENTRY_ADDR_COUNT_OFFSET, addresses.len() as u32);

        for (slot, address) in addresses.iter().enumerate() {
            write_u32(record, ENTRY_ADDR_OFFSET + slot * 4, *address);
        }
    }

    table
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a package the way the reference tool writes one: header, table,
    /// then the payloads the table points at. An entry with an empty name and
    /// an empty payload becomes an operation record, like the ones a real
    /// package stores next to its files.
    fn build_pac(files: &[(&str, &str, &[u8])]) -> Vec<u8> {
        let table_size = files.len() * ENTRY_SIZE;
        let mut table = vec![0u8; table_size];
        let mut payloads = Vec::new();

        for (index, (id, name, payload)) in files.iter().enumerate() {
            let record = &mut table[index * ENTRY_SIZE..(index + 1) * ENTRY_SIZE];
            let offset = HEADER_SIZE as usize + table_size + payloads.len();

            write_u32(record, RECORD_SIZE_OFFSET, ENTRY_SIZE as u32);
            write_wide(record, ID_OFFSET, ID_LEN, id);
            write_wide(record, NAME_OFFSET, NAME_LEN, name);
            write_u32(record, ENTRY_SIZE_LOW_OFFSET, payload.len() as u32);
            write_u32(record, ENTRY_OFFSET_LOW_OFFSET, offset as u32);
            // One is what a real record carries for a file to download.
            write_u32(record, ENTRY_FLAG_OFFSET, 1);

            payloads.extend_from_slice(payload);
        }

        let mut pac = vec![0u8; HEADER_SIZE as usize];

        write_wide(&mut pac, VERSION_OFFSET, VERSION_LEN, "BP_R2.0.1");
        write_u32(
            &mut pac,
            SIZE_LOW_OFFSET,
            (HEADER_SIZE as usize + table_size + payloads.len()) as u32,
        );
        write_wide(&mut pac, PRODUCT_OFFSET, PRODUCT_LEN, "HNR552T");
        write_wide(
            &mut pac,
            PRODUCT_VERSION_OFFSET,
            PRODUCT_VERSION_LEN,
            "N1770.6.12.09.00",
        );
        write_u32(&mut pac, RECORD_COUNT_OFFSET, files.len() as u32);
        write_u32(&mut pac, TABLE_OFFSET, HEADER_SIZE as u32);
        write_wide(&mut pac, PRODUCT_ALIAS_OFFSET, PRODUCT_ALIAS_LEN, "HNR552T");
        write_u32(&mut pac, MAGIC_OFFSET, MAGIC);

        pac.extend_from_slice(&table);
        pac.extend_from_slice(&payloads);

        pac
    }

    fn write_pac(directory: &Path, bytes: &[u8]) -> std::path::PathBuf {
        let path = directory.join("firmware.pac");
        std::fs::write(&path, bytes).expect("write package");
        path
    }

    fn temp_directory(name: &str) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "lmnflash-pac-test-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("create temp dir");
        directory
    }

    #[test]
    fn the_search_box_matches_file_names_without_case() {
        assert!(matches("fdl1-sign.bin", ""));
        assert!(matches("fdl1-sign.bin", "   "));
        assert!(matches("fdl1-sign.bin", "FDL1"));
        assert!(matches("system.img.ext4", "img"));
        assert!(matches("system.img.ext4", "  IMG  "));
        assert!(matches("HNR552T.xml", "hnr552t"));
        assert!(!matches("fdl1-sign.bin", "fdl2"));

        // A non-ASCII name is matched the same way.
        assert!(matches("引导程序.bin", "程序"));
        assert!(!matches("引导程序.bin", "内核"));
    }

    /// The checksum the format stores is CRC-16/ARC.
    #[test]
    fn the_checksum_matches_the_standard_check_value() {
        assert_eq!(crc16(0, b"123456789"), 0xBB3D);
        assert_eq!(crc16(0, b""), 0);
    }

    /// A package that was written reads back with the same header, the same
    /// records and its payloads where the table says they are.
    #[test]
    fn a_written_package_reads_back() {
        let directory = temp_directory("written");
        let source = directory.join("files");
        std::fs::create_dir_all(&source).expect("create the source folder");
        std::fs::write(source.join("fdl1-sign.bin"), b"one").expect("write a file");
        std::fs::write(source.join("boot.img"), b"seven!!").expect("write a file");

        let header = Header {
            version: "BP_R2.0.1".to_string(),
            product_name: "TEST".to_string(),
            product_version: "1.0.0".to_string(),
            product_alias: "TEST".to_string(),
            mode: 0,
            flash_type: 1,
            nand_strategy: 0,
            nand_page_type: 0,
            nv_backup: 1,
            oma_dm_product_flag: 0,
            oma_dm: 1,
            is_preload: 1,
        };

        let record = |id: &str, name: &str, kind: u32, omit: u32, addresses: Vec<u32>| Record {
            id: id.to_string(),
            name: name.to_string(),
            flag: 1,
            kind,
            omit,
            addresses,
        };

        let payloads = vec![
            Payload {
                record: record("FDL", "fdl1-sign.bin", 1, 0, vec![0x5500]),
                source: Some(source.join("fdl1-sign.bin")),
            },
            Payload {
                record: record("EraseSPL", "", 0, 1, vec![]),
                source: None,
            },
            Payload {
                record: record("BOOT", "boot.img", 1, 0, vec![]),
                source: Some(source.join("boot.img")),
            },
        ];

        let destination = directory.join("out.pac");
        let summary = write(&header, &payloads, &destination, &|_| {}).expect("write the package");

        assert_eq!(summary.files, 2);
        assert_eq!(summary.bytes, 3 + 7);

        let package = list(&destination).expect("read the package");

        assert_eq!(package.header, header);
        assert!(package.complete);
        assert_eq!(package.expected_size, package.actual_size);
        assert_eq!(package.records.len(), 3);
        assert_eq!(package.records[0].id, "FDL");
        assert_eq!(package.records[0].addresses, vec![0x5500]);
        assert_eq!(package.records[1].name, "");
        assert_eq!(package.records[1].kind, 0);
        assert_eq!(package.records[1].omit, 1);
        // A record has as many blocks as it was given.
        assert!(package.records[1].addresses.is_empty());

        assert_eq!(package.files.len(), 2);
        assert_eq!(package.files[0].size, 3);
        assert_eq!(package.files[0].offset, HEADER_SIZE + 3 * ENTRY_SIZE as u64);
        assert_eq!(
            package.files[1].offset,
            HEADER_SIZE + 3 * ENTRY_SIZE as u64 + 3
        );

        // The reader does not check the checksums, so they are checked here.
        let bytes = std::fs::read(&destination).expect("read the package");
        assert_eq!(
            u16::from_le_bytes(
                bytes[HEAD_CRC_OFFSET..HEAD_CRC_OFFSET + 2]
                    .try_into()
                    .expect("2 bytes")
            ),
            crc16(0, &bytes[..HEAD_CRC_LEN])
        );
        assert_eq!(
            u16::from_le_bytes(
                bytes[DATA_CRC_OFFSET..DATA_CRC_OFFSET + 2]
                    .try_into()
                    .expect("2 bytes")
            ),
            crc16(0, &bytes[HEADER_SIZE as usize..])
        );

        // The payloads come out again as they went in.
        let unpacked = directory.join("unpacked");
        unpack(
            &destination,
            &unpacked,
            &[0, 1],
            Configuration::Write,
            &|_| {},
        )
        .expect("unpack");
        assert_eq!(
            std::fs::read(unpacked.join("fdl1-sign.bin")).expect("read a file"),
            b"one"
        );
        assert_eq!(
            std::fs::read(unpacked.join("boot.img")).expect("read a file"),
            b"seven!!"
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// The header and the file table this module writes have to be the layout
    /// the reference package uses, field for field.
    #[test]
    #[ignore = "needs reference/TYH201H_HNR552T_N1770.6.12.09.00.pac"]
    fn the_written_layout_matches_the_reference_package() {
        let path = Path::new("reference/TYH201H_HNR552T_N1770.6.12.09.00.pac");
        let package = list(path).expect("read the reference package");

        let mut file = File::open(path).expect("open the package");
        let mut real = vec![0u8; HEADER_SIZE as usize + package.records.len() * ENTRY_SIZE];
        file.read_exact(&mut real)
            .expect("read the header and the table");
        let real_table = &real[HEADER_SIZE as usize..];

        // The sizes and offsets come from the package itself: its payloads are
        // far too large to be written out here.
        let mut sizes = Vec::new();
        let mut offsets = Vec::new();

        for index in 0..package.records.len() {
            let record = &real_table[index * ENTRY_SIZE..(index + 1) * ENTRY_SIZE];

            sizes.push(combine(
                u32_at(record, ENTRY_SIZE_HIGH_OFFSET),
                u32_at(record, ENTRY_SIZE_LOW_OFFSET),
            ));
            offsets.push(combine(
                u32_at(record, ENTRY_OFFSET_HIGH_OFFSET),
                u32_at(record, ENTRY_OFFSET_LOW_OFFSET),
            ));
        }

        let payloads: Vec<Payload> = package
            .records
            .iter()
            .cloned()
            .map(|record| Payload {
                record,
                source: None,
            })
            .collect();

        let written = build_table(&payloads, &sizes, &offsets);

        // A byte-for-byte comparison of two 141 KB tables is unreadable, so
        // the first difference is what gets reported.
        if let Some(index) = written
            .iter()
            .zip(real_table)
            .position(|(ours, theirs)| ours != theirs)
        {
            panic!(
                "the file table differs at byte {index} (record {}, offset {}): wrote 0x{:02x}, expected 0x{:02x}",
                index / ENTRY_SIZE,
                index % ENTRY_SIZE,
                written[index],
                real_table[index]
            );
        }

        // The header too, apart from the second checksum: it covers the
        // payloads, which are not written here.
        let header = build_header(&package.header, package.records.len(), package.actual_size);

        if let Some(index) = header
            .iter()
            .zip(&real)
            .take(DATA_CRC_OFFSET)
            .position(|(ours, theirs)| ours != theirs)
        {
            panic!(
                "the header differs at byte {index}: wrote 0x{:02x}, expected 0x{:02x}",
                header[index], real[index]
            );
        }
    }

    #[test]
    fn reads_the_header_and_the_file_table() {
        let directory = temp_directory("list");
        let pac = build_pac(&[
            ("FDL", "fdl1-sign.bin", b"one"),
            ("FDL2", "fdl2-sign.bin", b"two-two"),
        ]);
        let path = write_pac(&directory, &pac);

        let package = list(&path).expect("read the package");

        assert_eq!(package.header.product_name, "HNR552T");
        assert_eq!(package.header.product_version, "N1770.6.12.09.00");
        assert_eq!(package.header.product_alias, "HNR552T");
        assert_eq!(package.files.len(), 2);

        // The first payload starts right behind the header and the table, and
        // the second one right behind the first.
        assert_eq!(package.files[0].name, "fdl1-sign.bin");
        assert_eq!(package.files[0].size, 3);
        assert_eq!(package.files[0].offset, HEADER_SIZE + 2 * ENTRY_SIZE as u64);
        assert!(package.files[0].complete);
        assert_eq!(package.files[1].name, "fdl2-sign.bin");
        assert_eq!(package.files[1].size, 7);
        assert_eq!(
            package.files[1].offset,
            HEADER_SIZE + 2 * ENTRY_SIZE as u64 + 3
        );

        // Nothing is missing from a file that is as long as its header says.
        assert!(package.complete);
        assert_eq!(package.expected_size, pac.len() as u64);
        assert_eq!(package.actual_size, pac.len() as u64);

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn operation_records_are_not_files() {
        let directory = temp_directory("operations");
        let pac = build_pac(&[
            ("FDL", "fdl1-sign.bin", b"one"),
            ("EraseSPL", "", b""),
        ]);
        let path = write_pac(&directory, &pac);

        let package = list(&path).expect("read the package");

        assert_eq!(package.files.len(), 1);
        assert_eq!(package.files[0].name, "fdl1-sign.bin");

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_file_that_is_not_a_pac_is_refused() {
        let directory = temp_directory("foreign");

        // A file of the right size whose header does not carry the magic …
        let mut pac = build_pac(&[("FDL", "fdl1-sign.bin", b"one")]);
        write_u32(&mut pac, MAGIC_OFFSET, 0x1234_5678);
        let path = write_pac(&directory, &pac);
        assert_eq!(list(&path), Err(PacError::NotAPac));

        // … and one that is shorter than the header.
        let path = write_pac(&directory, b"not a package at all");
        assert_eq!(list(&path), Err(PacError::NotAPac));

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn refuses_an_encrypted_package() {
        let directory = temp_directory("encrypted");
        let mut pac = build_pac(&[("FDL", "fdl1-sign.bin", b"one")]);
        pac[ENCRYPTED_OFFSET] = 1;
        let path = write_pac(&directory, &pac);

        assert_eq!(list(&path), Err(PacError::Encrypted));

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn rejects_a_table_that_runs_past_the_end_of_the_file() {
        let directory = temp_directory("truncated");
        let mut pac = build_pac(&[("FDL", "fdl1-sign.bin", b"one")]);
        write_u32(&mut pac, RECORD_COUNT_OFFSET, MAX_RECORDS as u32);
        let path = write_pac(&directory, &pac);

        assert_eq!(list(&path), Err(PacError::BadTable));

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_flash_config_describes_the_package() {
        let directory = temp_directory("config");
        let pac = build_pac(&[
            ("FDL", "fdl1-sign.bin", b"one"),
            ("CONFILE", "TEST.xml", b"two-two"),
            ("EraseMisc", "", b""),
        ]);
        let path = write_pac(&directory, &pac);

        let package = list(&path).expect("read the package");

        assert_eq!(
            flash_cfg(&package),
            concat!(
                "\n[Setting]\n",
                "PacVer=BP_R2.0.1\n",
                "PAC_PRODUCT=HNR552T\n",
                "ProductAlias=HNR552T\n",
                "ProductVer=N1770.6.12.09.00\n",
                "Mode=0\n",
                "FlashType=0\n",
                "NandStrategy=0\n",
                "NandPageType=0\n",
                "NvBackup=0\n",
                "OmaDmProductFlag=0\n",
                "OmaDM=0\n",
                "IsPreload=0\n",
                "PAC_CONFILE=TEST.xml\n",
                "\n[FlashParam]\n",
                "FDL=1@fdl1-sign.bin\n",
                "CONFILE=1@TEST.xml\n",
                // A pure operation has no file, and is still listed.
                "EraseMisc=1@\n",
            )
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_record_without_an_id_is_not_listed() {
        let directory = temp_directory("config-no-id");
        // A package may leave the id of its configuration XML empty.
        let pac = build_pac(&[
            ("FDL", "fdl1-sign.bin", b"one"),
            ("", "TEST.xml", b"two-two"),
            ("EraseMisc", "", b""),
        ]);
        let path = write_pac(&directory, &pac);

        let package = list(&path).expect("read the package");
        let config = flash_cfg(&package);

        // The line would have no key at all, which an INI file cannot hold …
        assert!(!config.contains("=0@TEST.xml"), "{config}");
        assert!(!config.contains("@TEST.xml"), "{config}");

        // … while the XML is still named by the setting above, and is still
        // one of the files that can be unpacked.
        assert!(config.contains("PAC_CONFILE=TEST.xml\n"), "{config}");
        assert!(package.files.iter().any(|entry| entry.name == "TEST.xml"));
        assert!(config.contains("FDL=1@fdl1-sign.bin\n"), "{config}");
        assert!(config.contains("EraseMisc=1@\n"), "{config}");

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn unpacking_writes_the_flash_config() {
        let directory = temp_directory("config-written");
        let pac = build_pac(&[("FDL", "fdl1-sign.bin", b"one")]);
        let path = write_pac(&directory, &pac);
        let output = directory.join("out");

        unpack(&path, &output, &[0], Configuration::Write, &|_| {}).expect("unpack");

        let config = std::fs::read_to_string(output.join(CONFIG_FILE)).expect("read the config");
        assert!(config.contains("PAC_PRODUCT=HNR552T\n"), "{config}");
        assert!(config.contains("FDL=1@fdl1-sign.bin\n"), "{config}");

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_run_of_nothing_but_the_download_agents_writes_no_configuration() {
        let directory = temp_directory("config-skipped");
        let pac = build_pac(&[
            ("FDL", "fdl1-sign.bin", b"one"),
            ("FDL2", "fdl2-sign.bin", b"two"),
        ]);
        let path = write_pac(&directory, &pac);
        let output = directory.join("out");

        let summary =
            unpack(&path, &output, &[0, 1], Configuration::Skip, &|_| {}).expect("unpack");

        assert_eq!(summary.files, 2);
        assert!(output.join("fdl1-sign.bin").is_file());
        assert!(
            !output.join(CONFIG_FILE).exists(),
            "a loaders-only run leaves no configuration behind"
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_package_file_called_like_the_config_keeps_its_name_without_one() {
        let directory = temp_directory("config-clash-skipped");
        let pac = build_pac(&[
            ("FDL", "fdl1-sign.bin", b"one"),
            ("CFG", "flash.cfg", b"package data"),
        ]);
        let path = write_pac(&directory, &pac);
        let output = directory.join("out");

        unpack(&path, &output, &[0, 1], Configuration::Skip, &|_| {}).expect("unpack");

        // Nothing else wants the name, so the package's own file keeps it.
        assert_eq!(
            std::fs::read(output.join(CONFIG_FILE)).expect("read the package file"),
            b"package data"
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_package_file_called_like_the_config_is_kept() {
        let directory = temp_directory("config-clash");
        let pac = build_pac(&[("CFG", "flash.cfg", b"package data")]);
        let path = write_pac(&directory, &pac);
        let output = directory.join("out");

        unpack(&path, &output, &[0], Configuration::Write, &|_| {}).expect("unpack");

        // The generated configuration keeps its name; the package's own file
        // is written beside it instead of being overwritten.
        let config = std::fs::read_to_string(output.join(CONFIG_FILE)).expect("read the config");
        assert!(config.contains("[FlashParam]"), "{config}");
        assert_eq!(
            std::fs::read(output.join("flash (2).cfg")).expect("read the package file"),
            b"package data"
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_partial_package_lists_the_files_that_are_present() {
        let directory = temp_directory("partial");
        let pac = build_pac(&[
            ("FDL", "fdl1-sign.bin", b"one"),
            ("NV", "nvitem.bin", b"two-two"),
            ("FDL2", "fdl2-sign.bin", b"three-three-three"),
        ]);

        // Cutting the file in the middle of the second payload leaves the
        // header and the whole file table intact, which is what an interrupted
        // download looks like.
        let cut = HEADER_SIZE as usize + 3 * ENTRY_SIZE + 3 + 3;
        let path = write_pac(&directory, &pac[..cut]);

        let package = list(&path).expect("read the partial package");

        assert!(!package.complete, "the file is shorter than its header says");
        assert_eq!(package.expected_size, pac.len() as u64);
        assert_eq!(package.actual_size, cut as u64);

        // The first file is whole, the second keeps the three bytes that are
        // there, and the third is missing entirely.
        assert_eq!(package.files.len(), 2);
        assert!(package.files[0].complete);
        assert_eq!(package.files[0].size, 3);
        assert_eq!(package.files[1].name, "nvitem.bin");
        assert!(!package.files[1].complete);
        assert_eq!(package.files[1].size, 3);

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn unpacking_a_partial_package_writes_what_is_there() {
        let directory = temp_directory("partial-unpack");
        let pac = build_pac(&[
            ("FDL", "fdl1-sign.bin", b"one"),
            ("NV", "nvitem.bin", b"two-two"),
        ]);
        let cut = HEADER_SIZE as usize + 2 * ENTRY_SIZE + 3 + 3;
        let path = write_pac(&directory, &pac[..cut]);
        let output = directory.join("out");

        let summary = unpack(&path, &output, &[0, 1], Configuration::Write, &|_| {}).expect("unpack");

        assert_eq!(summary.files, 2);
        assert_eq!(summary.bytes, 3 + 3);
        assert_eq!(
            std::fs::read(output.join("fdl1-sign.bin")).expect("read file"),
            b"one"
        );
        assert_eq!(
            std::fs::read(output.join("nvitem.bin")).expect("read file"),
            b"two"
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_table_that_is_cut_off_cannot_be_listed() {
        let directory = temp_directory("cut-table");
        let pac = build_pac(&[("FDL", "fdl1-sign.bin", b"one")]);

        // The header is there, the record that follows is not.
        let cut = HEADER_SIZE as usize + ENTRY_SIZE - 10;
        let path = write_pac(&directory, &pac[..cut]);

        assert_eq!(list(&path), Err(PacError::BadTable));

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn combines_the_64_bit_size_and_offset() {
        let mut record = vec![0u8; ENTRY_SIZE];
        write_u32(&mut record, RECORD_SIZE_OFFSET, ENTRY_SIZE as u32);
        write_wide(&mut record, NAME_OFFSET, NAME_LEN, "system.img.ext4");
        write_u32(&mut record, ENTRY_SIZE_HIGH_OFFSET, 2);
        write_u32(&mut record, ENTRY_SIZE_LOW_OFFSET, 5);
        write_u32(&mut record, ENTRY_OFFSET_HIGH_OFFSET, 3);
        write_u32(&mut record, ENTRY_OFFSET_LOW_OFFSET, 7);

        let entry = parse_entry(&record);

        assert_eq!(entry.name, "system.img.ext4");
        assert_eq!(entry.size, (2u64 << 32) | 5);
        assert_eq!(entry.offset, (3u64 << 32) | 7);
        assert!(entry.complete);
    }

    #[test]
    fn unpacks_the_selected_files_only() {
        let directory = temp_directory("unpack");
        let pac = build_pac(&[
            ("FDL", "fdl1-sign.bin", b"one"),
            ("NV", "nvitem.bin", b"two-two"),
            ("FDL2", "fdl2-sign.bin", b"three-three-three"),
        ]);
        let path = write_pac(&directory, &pac);
        let output = directory.join("out");

        let summary = unpack(&path, &output, &[0, 2], Configuration::Write, &|_| {}).expect("unpack");

        assert_eq!(summary.files, 2);
        assert_eq!(summary.bytes, 3 + b"three-three-three".len() as u64);
        assert_eq!(
            std::fs::read(output.join("fdl1-sign.bin")).expect("read file"),
            b"one"
        );
        assert_eq!(
            std::fs::read(output.join("fdl2-sign.bin")).expect("read file"),
            b"three-three-three"
        );
        assert!(!output.join("nvitem.bin").exists());

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_duplicate_name_does_not_overwrite_a_file() {
        let directory = temp_directory("duplicate");
        let pac = build_pac(&[
            ("LOGO", "fbootlogo.bin", b"first"),
            ("LOGO2", "fbootlogo.bin", b"second"),
        ]);
        let path = write_pac(&directory, &pac);
        let output = directory.join("out");

        unpack(&path, &output, &[0, 1], Configuration::Write, &|_| {}).expect("unpack");

        assert_eq!(
            std::fs::read(output.join("fbootlogo.bin")).expect("read file"),
            b"first"
        );
        assert_eq!(
            std::fs::read(output.join("fbootlogo (2).bin")).expect("read file"),
            b"second"
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn refuses_a_name_that_escapes_the_output_folder() {
        let directory = temp_directory("unsafe");
        let pac = build_pac(&[("FDL", "..\\evil.bin", b"one")]);
        let path = write_pac(&directory, &pac);
        let output = directory.join("out");

        assert_eq!(
            unpack(&path, &output, &[0], Configuration::Write, &|_| {}),
            Err(PacError::BadName("..\\evil.bin".to_string()))
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// Reads the package the format was verified against. It is not part of
    /// the repository (the whole `reference/` tree is ignored), so this test is
    /// skipped by default: run it with
    /// `cargo test -- --ignored reads_the_reference_package`.
    #[test]
    #[ignore = "needs reference/TYH201H_HNR552T_N1770.6.12.09.00.pac"]
    fn reads_the_reference_package() {
        let path = Path::new("reference/TYH201H_HNR552T_N1770.6.12.09.00.pac");
        let package = list(path).expect("read the reference package");

        assert_eq!(package.header.product_name, "HNR552T");
        assert_eq!(package.header.product_version, "N1770.6.12.09.00");
        assert_eq!(package.header.product_alias, "HNR552T");

        // The reference file is a complete package.
        assert!(package.complete);
        let length = std::fs::metadata(path).expect("read the file length").len();
        assert_eq!(package.expected_size, length);

        // 54 records, of which 12 are operations without a file.
        assert_eq!(package.files.len(), 42);

        // The first payload starts right behind the header and the table.
        let first = &package.files[0];
        assert_eq!(first.name, "fdl1-sign.bin");
        assert_eq!(first.size, 57156);
        assert_eq!(first.offset, HEADER_SIZE + 54 * ENTRY_SIZE as u64);

        // A real package may name the same file twice.
        let duplicates = package
            .files
            .iter()
            .filter(|entry| entry.name == "fbootlogo.bin")
            .count();
        assert_eq!(duplicates, 2);

        // The package carries its own flash definition XML as well.
        assert_eq!(package.files.last().expect("a last file").name, "HNR552T.xml");

        // The generated flash configuration matches what the header really
        // holds, and lists the table in its own order.
        let config = flash_cfg(&package);
        assert!(
            config.starts_with(concat!(
                "\n[Setting]\n",
                "PacVer=BP_R2.0.1\n",
                "PAC_PRODUCT=HNR552T\n",
                "ProductAlias=HNR552T\n",
                "ProductVer=N1770.6.12.09.00\n",
                "Mode=0\n",
                "FlashType=1\n",
                "NandStrategy=0\n",
                "NandPageType=0\n",
                "NvBackup=1\n",
                "OmaDmProductFlag=0\n",
                "OmaDM=1\n",
                "IsPreload=1\n",
                "PAC_CONFILE=HNR552T.xml\n",
                "\n[FlashParam]\n",
                "FDL=1@fdl1-sign.bin\n",
                "FDL2=1@fdl2-sign.bin\n",
            )),
            "{config}"
        );
        assert!(config.contains("EraseSPL=1@\n"), "{config}");

        // Every line of the file has a key: the package's own configuration
        // XML carries no id, and must not turn into a `=0@HNR552T.xml` line.
        assert!(
            config.lines().all(|line| !line.starts_with('=')),
            "{config}"
        );
        assert!(!config.contains("@HNR552T.xml\n"), "{config}");

        // Cut the file down to its header, its table and a bit of the second
        // payload: what an interrupted download leaves behind. The table is
        // intact, so the files before the cut are still there to take.
        let directory = temp_directory("reference-partial");
        let cut = HEADER_SIZE + 54 * ENTRY_SIZE as u64 + first.size + 100;
        let partial = directory.join("partial.pac");

        // Only the head is copied — the package itself is several gigabytes.
        let mut head = vec![0u8; cut as usize];
        File::open(path)
            .expect("open the package")
            .read_exact(&mut head)
            .expect("read the package head");
        std::fs::write(&partial, &head).expect("write the partial package");

        let package = list(&partial).expect("read the partial package");

        assert!(!package.complete);
        assert_eq!(package.actual_size, cut);
        assert_eq!(package.expected_size, length);

        assert_eq!(package.files.len(), 2);
        assert!(package.files[0].complete);
        assert_eq!(package.files[0].name, "fdl1-sign.bin");
        assert_eq!(package.files[0].size, 57156);
        assert!(!package.files[1].complete);
        assert_eq!(package.files[1].name, "fdl2-sign.bin");
        assert_eq!(package.files[1].size, 100);

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_run_always_ends_with_a_finished_event() {
        let directory = temp_directory("events");
        let pac = build_pac(&[("FDL", "fdl1-sign.bin", b"one")]);
        let path = write_pac(&directory, &pac);
        let output = directory.join("out");

        // A `Fn` closure cannot own the collection, so it is borrowed.
        let events = std::cell::RefCell::new(Vec::new());
        unpack(
            &path,
            &output,
            &[0],
            Configuration::Write,
            &|event| events.borrow_mut().push(event),
        )
        .expect("unpack");

        assert!(matches!(
            events.borrow().last(),
            Some(PacEvent::Finished(Ok(Summary { files: 1, .. })))
        ));

        // A failed run reports its outcome the same way instead of returning
        // early, which would leave the dialog waiting for it.
        let missing = directory.join("missing.pac");
        let events = std::cell::RefCell::new(Vec::new());

        assert!(unpack(
            &missing,
            &output,
            &[0],
            Configuration::Write,
            &|event| events.borrow_mut().push(event)
        )
        .is_err());
        assert!(matches!(
            events.borrow().last(),
            Some(PacEvent::Finished(Err(_)))
        ));

        let _ = std::fs::remove_dir_all(&directory);
    }
}
