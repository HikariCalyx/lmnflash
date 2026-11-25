//! Merging the `*_sparsechunk.N` files of a Motorola firmware package.
//!
//! A factory package does not carry `super.img` as one file: it is split into
//! `super.img_sparsechunk.0`, `.1`, … because a bootloader can only take so
//! much at once. Every chunk is a complete Android sparse image of the *whole*
//! partition — same block size, same block count — in which only its own slice
//! carries data and everything else is a `DONT_CARE` run. That is why
//! `flashfile.xml` writes all of them to the same `super` partition without the
//! later ones wiping the earlier ones.
//!
//! Because each chunk already says where its blocks belong, merging them is a
//! matter of putting their runs back into one image:
//!
//! ```text
//! chunk.0  [ data 0…50   | ---- hole ---- ]
//! chunk.1  [ -- hole --- | data 50…100    ]
//! merged   [ data 0…50   | data 50…100    ]
//! ```
//!
//! Two shapes come out of that, which is what [`Format`] picks between:
//!
//! * [`Format::Sparse`] — one sparse image again (`super.img`), the runs of
//!   every chunk behind a single header. It is the shape a bootloader takes.
//! * [`Format::Raw`] — the image the runs describe (`super.img.unsparse`),
//!   block for block, holes written as zeros. That is what a partition tool
//!   (`lpunpack`, a loop mount) wants.
//!
//! The format is the one `libsparse` writes: a 28-byte file header, then one
//! 12-byte header per chunk.
//!
//! ```text
//! magic u32 = 0xED26FF3A | major u16 | minor u16 | file_hdr_sz u16 |
//! chunk_hdr_sz u16 | blk_sz u32 | total_blks u32 | total_chunks u32 |
//! image_checksum u32
//!
//! chunk_type u16 | reserved u16 | chunk_sz u32 (blocks) | total_sz u32 (bytes)
//! ```

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// The magic a sparse image starts with, little-endian.
const MAGIC: [u8; 4] = [0x3a, 0xff, 0x26, 0xed];

/// The header sizes this module writes; an image it reads may declare bigger
/// ones, whose extra bytes are skipped over.
const FILE_HEADER_SIZE: u16 = 28;
const CHUNK_HEADER_SIZE: u16 = 12;

/// The chunk kinds. `RAW` carries its blocks, `FILL` a four-byte pattern every
/// block of the run repeats, `DONT_CARE` nothing at all (a hole), and `CRC32`
/// a checksum of what came before it rather than blocks of its own.
const CHUNK_RAW: u16 = 0xcac1;
const CHUNK_FILL: u16 = 0xcac2;
const CHUNK_DONT_CARE: u16 = 0xcac3;
const CHUNK_CRC32: u16 = 0xcac4;

/// What a chunk's name carries in front of its number.
const MARKER: &str = "_sparsechunk";

/// Bytes copied at a time while a run is written out.
const COPY_BUFFER: usize = 1024 * 1024;

/// Which image the chunks are merged into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    /// One sparse image again, the way the chunks themselves are stored
    /// (`super.img`). This is what a bootloader takes.
    #[default]
    Sparse,
    /// The image the chunks describe, block for block, with every hole written
    /// as zeros (`super.img.unsparse`).
    Raw,
}

/// Why a set of chunks could not be read or merged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeError {
    /// No file was given.
    NoFiles,
    /// A file could not be read.
    Read { file: String, error: String },
    /// A file does not start with the sparse magic: not a chunk.
    NotSparse(String),
    /// The file is a sparse image this module cannot read.
    Unsupported { file: String, detail: String },
    /// A file's header or chunk table does not add up.
    Malformed { file: String, detail: String },
    /// Two files disagree about the block size, so they are not slices of one
    /// image.
    BlockSize { file: String, found: u32, expected: u32 },
    /// Two files carry data for the same block, which no two slices of one
    /// image do: they are chunks of different images (or the same chunk twice).
    Overlap { file: String, other: String, block: u64 },
    /// The merged image would be longer than a sparse header can describe.
    TooLarge,
    /// The merged image would be written over one of the chunks it is made of.
    OutputIsInput(String),
    /// The merged image could not be written.
    Write(String),
}

impl std::fmt::Display for MergeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoFiles => write!(f, "no sparsechunk file was selected"),
            Self::Read { file, error } => write!(f, "failed to read {file}: {error}"),
            Self::NotSparse(file) => write!(f, "{file} is not an Android sparse image"),
            Self::Unsupported { file, detail } => {
                write!(f, "{file} is a sparse image this tool cannot read: {detail}")
            }
            Self::Malformed { file, detail } => write!(f, "{file} is malformed: {detail}"),
            Self::BlockSize {
                file,
                found,
                expected,
            } => write!(
                f,
                "{file} uses {found}-byte blocks while the first file uses \
                 {expected}-byte blocks, so they are not parts of one image"
            ),
            Self::Overlap { file, other, block } => write!(
                f,
                "{file} and {other} both carry block {block}, so they are not \
                 parts of one image"
            ),
            Self::TooLarge => write!(f, "the merged image is too long for a sparse header"),
            Self::OutputIsInput(file) => {
                write!(f, "the merged image would be written over {file}")
            }
            Self::Write(error) => write!(f, "failed to write the merged image: {error}"),
        }
    }
}

impl std::error::Error for MergeError {}

/// One chunk file of a plan, as the dialog lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkFile {
    /// Where the file is.
    pub path: PathBuf,
    /// Its file name, which is what the dialog shows.
    pub name: String,
    /// Its own size on disk.
    pub size: u64,
    /// The number behind `_sparsechunk`, when the name carries one.
    pub index: Option<u64>,
    /// The first block it carries data for, and the block after its last one.
    /// Both are 0 for a chunk that is all holes.
    pub first_block: u64,
    pub end_block: u64,
    /// How many blocks of the merged image it fills.
    pub data_blocks: u64,
}

/// What merging a set of chunks will produce, read from their headers before
/// anything is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The chunks, in the order they are merged: by the block they start at,
    /// so the merged image is written front to back.
    pub files: Vec<ChunkFile>,
    /// The block size all of them share.
    pub block_size: u32,
    /// How many blocks the merged image covers, holes included.
    pub total_blocks: u64,
    /// How many of those blocks carry data.
    pub data_blocks: u64,
    /// Size of the merged sparse image.
    pub sparse_bytes: u64,
    /// Size of the unsparsed raw image.
    pub raw_bytes: u64,
    /// The name the merged image is given, before the format's suffix.
    pub stem: String,
    /// Chunk numbers that are missing from the selection (`.0` and `.2` picked
    /// but not `.1`). The merge runs anyway — the gap simply stays a hole — so
    /// this is something the dialog warns about, not an error.
    pub missing: Vec<u64>,
    /// Payload bytes a merged sparse image writes, and expanded bytes a raw
    /// one writes. Progress is counted against one of them.
    payload_bytes: u64,
    expanded_bytes: u64,
}

impl Plan {
    /// The file name the merged image gets.
    pub fn output_name(&self, format: Format) -> String {
        match format {
            Format::Sparse => self.stem.clone(),
            Format::Raw => format!("{}.unsparse", self.stem),
        }
    }

    /// How long the merged image will be.
    pub fn output_bytes(&self, format: Format) -> u64 {
        match format {
            Format::Sparse => self.sparse_bytes,
            Format::Raw => self.raw_bytes,
        }
    }

    /// The bytes the run writes, which is what its progress is measured
    /// against. A sparse image carries the payloads as they are; a raw one
    /// writes every block a run stands for.
    fn progress_total(&self, format: Format) -> u64 {
        match format {
            Format::Sparse => self.payload_bytes,
            Format::Raw => self.expanded_bytes,
        }
    }

    /// Total size of the chunk files themselves, for the list's heading.
    pub fn input_bytes(&self) -> u64 {
        self.files.iter().map(|file| file.size).sum()
    }
}

/// What a successful merge produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    /// How many chunks went into it.
    pub files: usize,
    /// The image that was written.
    pub output: PathBuf,
    /// How long it is.
    pub bytes: u64,
}

/// Progress reported while the chunks are merged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeEvent {
    /// A chunk is about to be read (`index` counts from 0).
    File { index: usize, name: String },
    /// `bytes` of the merged image have been written so far.
    Progress { bytes: u64, total: u64 },
    /// The run is over. Always the last event: the worker reports its outcome
    /// itself, so the dialog can never wait for an event that does not come.
    Finished(Result<Summary, MergeError>),
}

/// The name the merged image is given: what stands in front of `_sparsechunk`,
/// which is the image the chunks were split from.
///
/// A name without the marker is taken as it is, minus a trailing numeric
/// extension (`super.img.0`), so a set that was named by hand still merges
/// into something sensible.
pub fn image_stem(name: &str) -> String {
    if let Some(cut) = find_marker(name) {
        let stem = name[..cut].trim_end_matches(['.', '_', '-']);

        if !stem.is_empty() {
            return stem.to_owned();
        }
    }

    match name.rsplit_once('.') {
        Some((stem, digits)) if !stem.is_empty() && digits.parse::<u64>().is_ok() => {
            stem.to_owned()
        }
        _ => name.to_owned(),
    }
}

/// The number behind `_sparsechunk` in a chunk's name, when it carries one.
pub fn chunk_index(name: &str) -> Option<u64> {
    let cut = find_marker(name)?;
    let rest = name[cut + MARKER.len()..].trim_start_matches(['.', '_', '-']);

    rest.parse().ok()
}

/// Where `_sparsechunk` starts in a name, whatever its case.
fn find_marker(name: &str) -> Option<usize> {
    name.to_ascii_lowercase().find(MARKER)
}

/// The header of a sparse image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Header {
    block_size: u32,
    total_blocks: u32,
    total_chunks: u32,
    /// How long this image says its chunk headers are: the extra bytes a newer
    /// writer may have added are skipped rather than guessed at.
    chunk_header_size: u16,
}

impl Header {
    /// Reads the header a sparse image starts with, leaving the reader on its
    /// first chunk.
    fn read(reader: &mut BufReader<File>, name: &str) -> Result<Self, MergeError> {
        let mut bytes = [0u8; FILE_HEADER_SIZE as usize];

        reader.read_exact(&mut bytes).map_err(|error| {
            // A file too short to hold a header cannot be a chunk.
            if error.kind() == io::ErrorKind::UnexpectedEof {
                MergeError::NotSparse(name.to_owned())
            } else {
                read_error(name, &error)
            }
        })?;

        if bytes[..4] != MAGIC {
            return Err(MergeError::NotSparse(name.to_owned()));
        }

        let major = u16::from_le_bytes([bytes[4], bytes[5]]);
        if major != 1 {
            return Err(MergeError::Unsupported {
                file: name.to_owned(),
                detail: format!("its format version is {major}, not 1"),
            });
        }

        let file_header_size = u16::from_le_bytes([bytes[8], bytes[9]]);
        let chunk_header_size = u16::from_le_bytes([bytes[10], bytes[11]]);

        if file_header_size < FILE_HEADER_SIZE || chunk_header_size < CHUNK_HEADER_SIZE {
            return Err(MergeError::Malformed {
                file: name.to_owned(),
                detail: format!(
                    "it declares a {file_header_size}-byte file header and a \
                     {chunk_header_size}-byte chunk header"
                ),
            });
        }

        let block_size = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);

        if block_size == 0 || block_size % 4 != 0 {
            return Err(MergeError::Malformed {
                file: name.to_owned(),
                detail: format!("its block size is {block_size}"),
            });
        }

        // Anything a newer writer added behind the fields we know is skipped.
        skip(reader, u64::from(file_header_size - FILE_HEADER_SIZE), name)?;

        Ok(Self {
            block_size,
            total_blocks: u32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]),
            total_chunks: u32::from_le_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]),
            chunk_header_size,
        })
    }
}

/// One run of blocks a chunk carries data for, at the place in the merged image
/// the chunk itself puts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Run {
    /// The first block of the run.
    block: u64,
    /// How many blocks it covers.
    blocks: u32,
    /// What fills them.
    body: Body,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Body {
    /// The payload the reader is positioned at, in bytes.
    Raw(u64),
    /// The four-byte pattern every block of the run repeats.
    Fill([u8; 4]),
}

/// What [`walk`] hands every run of a chunk table to: the reader, positioned
/// on the run's payload, and the run itself. It answers with the number of
/// payload bytes it read.
type Visit<'a> = dyn FnMut(&mut BufReader<File>, &Run) -> Result<u64, MergeError> + 'a;

/// Walks the chunk table of one sparse image, handing every run that carries
/// data to `visit` with the reader positioned on its payload.
///
/// `visit` returns how many payload bytes it read, so the walker can skip
/// whatever is left and reach the next chunk header either way. Holes
/// (`DONT_CARE`) only move the block cursor, and a `CRC32` chunk is skipped:
/// it checksums the chunk file it came in, which says nothing about the merged
/// image.
///
/// Returns the block the image ends on, which is the bigger of its declared
/// length and what its chunks actually cover.
fn walk(
    reader: &mut BufReader<File>,
    name: &str,
    header: &Header,
    visit: &mut Visit<'_>,
) -> Result<u64, MergeError> {
    let mut block = 0u64;

    for index in 0..header.total_chunks {
        let mut bytes = [0u8; CHUNK_HEADER_SIZE as usize];

        reader.read_exact(&mut bytes).map_err(|error| {
            if error.kind() == io::ErrorKind::UnexpectedEof {
                MergeError::Malformed {
                    file: name.to_owned(),
                    detail: format!(
                        "it declares {} chunks but ends after {index}",
                        header.total_chunks
                    ),
                }
            } else {
                read_error(name, &error)
            }
        })?;

        let kind = u16::from_le_bytes([bytes[0], bytes[1]]);
        let blocks = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let declared = u64::from(u32::from_le_bytes([
            bytes[8], bytes[9], bytes[10], bytes[11],
        ]));

        // Extra bytes of a longer chunk header, before the payload.
        skip(
            reader,
            u64::from(header.chunk_header_size - CHUNK_HEADER_SIZE),
            name,
        )?;

        let payload = declared
            .checked_sub(u64::from(header.chunk_header_size))
            .ok_or_else(|| MergeError::Malformed {
                file: name.to_owned(),
                detail: format!("chunk {index} is {declared} bytes long, header included"),
            })?;

        let span = u64::from(blocks) * u64::from(header.block_size);
        let mut consumed = 0u64;

        match kind {
            CHUNK_RAW => {
                if payload != span {
                    return Err(MergeError::Malformed {
                        file: name.to_owned(),
                        detail: format!(
                            "chunk {index} carries {payload} bytes for {blocks} blocks"
                        ),
                    });
                }

                let run = Run {
                    block,
                    blocks,
                    body: Body::Raw(payload),
                };

                consumed = visit(reader, &run)?;
                block += u64::from(blocks);
            }
            CHUNK_FILL => {
                if payload != 4 {
                    return Err(MergeError::Malformed {
                        file: name.to_owned(),
                        detail: format!("fill chunk {index} carries {payload} bytes, not 4"),
                    });
                }

                let mut value = [0u8; 4];
                reader
                    .read_exact(&mut value)
                    .map_err(|error| read_error(name, &error))?;
                consumed = 4;

                let run = Run {
                    block,
                    blocks,
                    body: Body::Fill(value),
                };

                consumed += visit(reader, &run)?;
                block += u64::from(blocks);
            }
            CHUNK_DONT_CARE => block += u64::from(blocks),
            CHUNK_CRC32 => {}
            other => {
                return Err(MergeError::Unsupported {
                    file: name.to_owned(),
                    detail: format!("chunk {index} is of kind {other:#06x}"),
                });
            }
        }

        let left = payload.checked_sub(consumed).ok_or_else(|| MergeError::Malformed {
            file: name.to_owned(),
            detail: format!("chunk {index} was read past its payload"),
        })?;

        skip(reader, left, name)?;
    }

    Ok(block.max(u64::from(header.total_blocks)))
}

/// Moves the reader forward without reading, keeping its buffer where it can.
fn skip(reader: &mut BufReader<File>, bytes: u64, name: &str) -> Result<(), MergeError> {
    if bytes == 0 {
        return Ok(());
    }

    // `seek_relative` keeps the buffered bytes when the target is still inside
    // them, which a table of many small chunks leans on heavily.
    let mut left = bytes;

    while left > 0 {
        let step = i64::try_from(left.min(i64::MAX as u64)).unwrap_or(i64::MAX);

        reader
            .seek_relative(step)
            .map_err(|error| read_error(name, &error))?;

        left -= step as u64;
    }

    Ok(())
}

fn read_error(name: &str, error: &io::Error) -> MergeError {
    MergeError::Read {
        file: name.to_owned(),
        error: error.to_string(),
    }
}

fn write_error(error: &io::Error) -> MergeError {
    MergeError::Write(error.to_string())
}

/// The file name of a path, for a message and for the list.
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn open(path: &Path, name: &str) -> Result<BufReader<File>, MergeError> {
    let file = File::open(path).map_err(|error| read_error(name, &error))?;

    Ok(BufReader::with_capacity(COPY_BUFFER, file))
}

/// Reads the headers of every chunk and works out what merging them produces,
/// without writing (or even reading the payload of) anything.
///
/// This is what the dialog shows once files were picked: the chunks it will
/// merge, the size of either output, and the chunk numbers that are missing
/// from the selection.
pub fn inspect(paths: &[PathBuf]) -> Result<Plan, MergeError> {
    if paths.is_empty() {
        return Err(MergeError::NoFiles);
    }

    let mut files: Vec<ChunkFile> = Vec::with_capacity(paths.len());
    let mut block_size = 0u32;
    let mut total_blocks = 0u64;
    let mut data_blocks = 0u64;
    let mut payload_bytes = 0u64;
    let mut expanded_bytes = 0u64;
    // Runs that carry data, and the holes *inside* a chunk between them; the
    // holes between chunks are counted once they are in block order below.
    let mut data_runs = 0u64;
    let mut inner_gaps = 0u64;

    for path in paths {
        let name = file_name(path);
        let mut reader = open(path, &name)?;
        let header = Header::read(&mut reader, &name)?;

        if block_size == 0 {
            block_size = header.block_size;
        } else if header.block_size != block_size {
            return Err(MergeError::BlockSize {
                file: name.clone(),
                found: header.block_size,
                expected: block_size,
            });
        }

        let mut first_block = None;
        let mut end_block = 0u64;
        let mut blocks = 0u64;
        let mut cursor = 0u64;

        let covered = walk(&mut reader, &name, &header, &mut |_, run| {
            if first_block.is_none() {
                first_block = Some(run.block);
            } else if run.block > cursor {
                inner_gaps += 1;
            }

            data_runs += 1;
            blocks += u64::from(run.blocks);
            cursor = run.block + u64::from(run.blocks);
            end_block = cursor;

            payload_bytes += match run.body {
                Body::Raw(bytes) => bytes,
                Body::Fill(_) => 4,
            };
            expanded_bytes += u64::from(run.blocks) * u64::from(block_size);

            // Nothing of the payload is read here: only the table is walked.
            Ok(0)
        })?;

        total_blocks = total_blocks.max(covered);
        data_blocks += blocks;

        files.push(ChunkFile {
            path: path.clone(),
            name: name.clone(),
            size: std::fs::metadata(path).map(|data| data.len()).unwrap_or(0),
            index: chunk_index(&name),
            first_block: first_block.unwrap_or(0),
            end_block,
            data_blocks: blocks,
        });
    }

    if total_blocks > u64::from(u32::MAX) {
        return Err(MergeError::TooLarge);
    }

    // Merged front to back, which is the order the runs have to be written in.
    // A chunk that is all holes carries nothing, so where it lands is moot.
    files.sort_by_key(|file| (file.first_block, file.name.clone()));

    // Two chunks of one image never describe the same block. Two that do are
    // either the same file twice or chunks of different images, and merging
    // them would quietly produce an image that is neither.
    let mut previous: Option<&ChunkFile> = None;
    let mut outer_gaps = 0u64;

    for file in &files {
        if file.data_blocks == 0 {
            continue;
        }

        // Where the chunk in front of this one ended.
        let cursor = previous.map_or(0, |other| other.end_block);

        if file.first_block < cursor {
            return Err(MergeError::Overlap {
                file: file.name.clone(),
                // Only a chunk that came before can have taken those blocks,
                // which is also the only way `cursor` got past zero.
                other: previous.map_or_else(String::new, |other| other.name.clone()),
                block: file.first_block,
            });
        }

        if file.first_block > cursor {
            outer_gaps += 1;
        }

        previous = Some(file);
    }

    // The hole the merged image ends on, when its chunks stop short of the
    // length they declare.
    let covered = previous.map_or(0, |other| other.end_block);
    let trailing = u64::from(covered < total_blocks);

    let chunks = data_runs + inner_gaps + outer_gaps + trailing;
    let sparse_bytes = u64::from(FILE_HEADER_SIZE)
        + payload_bytes
        + chunks * u64::from(CHUNK_HEADER_SIZE);

    let stem = files
        .first()
        .map(|file| image_stem(&file.name))
        .unwrap_or_else(|| "merged.img".to_owned());

    Ok(Plan {
        block_size,
        total_blocks,
        data_blocks,
        sparse_bytes,
        raw_bytes: total_blocks * u64::from(block_size),
        stem,
        missing: missing_chunks(&files),
        payload_bytes,
        expanded_bytes,
        files,
    })
}

/// The chunk numbers between the lowest and the highest picked one that were
/// not picked.
///
/// A set that is numbered at all is numbered from 0 without a break, so a
/// missing number is a chunk the user forgot — and a hole in the merged image.
/// Names without a number say nothing, so they are left out of the check.
fn missing_chunks(files: &[ChunkFile]) -> Vec<u64> {
    let mut numbers: Vec<u64> = files.iter().filter_map(|file| file.index).collect();

    if numbers.len() < 2 {
        return Vec::new();
    }

    numbers.sort_unstable();

    let lowest = numbers[0];
    let highest = numbers[numbers.len() - 1];

    (lowest..=highest)
        .filter(|number| !numbers.contains(number))
        .collect()
}

/// Counts the bytes written and reports them at most once per permille, the
/// way the other jobs report their progress.
struct Progress<'a> {
    emit: &'a dyn Fn(MergeEvent),
    total: u64,
    written: u64,
    reported: u64,
}

impl Progress<'_> {
    fn add(&mut self, bytes: u64) {
        self.written += bytes;

        if self.total == 0 {
            return;
        }

        let permille = self.written * 1000 / self.total;

        if permille != self.reported {
            self.reported = permille;
            (self.emit)(MergeEvent::Progress {
                bytes: self.written,
                total: self.total,
            });
        }
    }

    fn finish(&mut self) {
        (self.emit)(MergeEvent::Progress {
            bytes: self.total,
            total: self.total,
        });
    }
}

/// Merges `paths` into one image in `destination`.
///
/// The name is the chunks' own (`super.img_sparsechunk.0` → `super.img`, or
/// `super.img.unsparse` for [`Format::Raw`]). Progress is reported through
/// `emit`, which always receives [`MergeEvent::Finished`] last.
pub fn merge(
    paths: &[PathBuf],
    destination: &Path,
    format: Format,
    emit: &dyn Fn(MergeEvent),
) -> Result<Summary, MergeError> {
    let result = merge_inner(paths, destination, format, emit);

    // The outcome is reported in exactly one place, so no early return can
    // skip it and leave the dialog waiting.
    emit(MergeEvent::Finished(result.clone()));

    result
}

fn merge_inner(
    paths: &[PathBuf],
    destination: &Path,
    format: Format,
    emit: &dyn Fn(MergeEvent),
) -> Result<Summary, MergeError> {
    let plan = inspect(paths)?;
    let output = destination.join(plan.output_name(format));

    // Writing the merged image over a chunk it is made of would destroy the
    // input halfway through reading it.
    for file in &plan.files {
        if same_file(&output, &file.path) {
            return Err(MergeError::OutputIsInput(file.name.clone()));
        }
    }

    std::fs::create_dir_all(destination).map_err(|error| write_error(&error))?;

    let mut progress = Progress {
        emit,
        total: plan.progress_total(format),
        written: 0,
        reported: 0,
    };

    match format {
        Format::Sparse => write_sparse(&plan, &output, emit, &mut progress)?,
        Format::Raw => write_raw(&plan, &output, emit, &mut progress)?,
    }

    progress.finish();

    let bytes = std::fs::metadata(&output)
        .map(|data| data.len())
        .unwrap_or_else(|_| plan.output_bytes(format));

    Ok(Summary {
        files: plan.files.len(),
        output,
        bytes,
    })
}

/// Whether two paths name the same file, as far as the file system will say.
/// A path that does not exist yet (the output, normally) cannot be one of the
/// inputs, so an answer of "no" is the right one for it.
fn same_file(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

/// Writes the chunks back into one sparse image: their runs behind a single
/// header, with a hole for every block none of them covers.
fn write_sparse(
    plan: &Plan,
    output: &Path,
    emit: &dyn Fn(MergeEvent),
    progress: &mut Progress<'_>,
) -> Result<(), MergeError> {
    let file = File::create(output).map_err(|error| write_error(&error))?;
    let mut writer = BufWriter::with_capacity(COPY_BUFFER, file);

    // The chunk count is only known once every table has been walked, so the
    // header is written again at the end with the final number in it.
    let total_blocks = u32::try_from(plan.total_blocks).map_err(|_| MergeError::TooLarge)?;

    writer
        .write_all(&header_bytes(plan.block_size, total_blocks, 0))
        .map_err(|error| write_error(&error))?;

    let mut cursor = 0u64;
    let mut chunks = 0u32;
    let mut buffer = vec![0u8; COPY_BUFFER];

    for (index, chunk) in plan.files.iter().enumerate() {
        emit(MergeEvent::File {
            index,
            name: chunk.name.clone(),
        });

        let mut reader = open(&chunk.path, &chunk.name)?;
        let header = Header::read(&mut reader, &chunk.name)?;

        walk(&mut reader, &chunk.name, &header, &mut |reader, run| {
            // The hole in front of the run: inside one chunk as well as
            // between two of them, every block nobody covers is a hole.
            if run.block > cursor {
                write_hole(&mut writer, run.block - cursor, &mut chunks)?;
            }

            match run.body {
                Body::Raw(bytes) => {
                    writer
                        .write_all(&chunk_bytes(CHUNK_RAW, run.blocks, bytes)?)
                        .map_err(|error| write_error(&error))?;

                    copy(reader, &chunk.name, &mut writer, bytes, &mut buffer, progress)?;
                }
                Body::Fill(value) => {
                    writer
                        .write_all(&chunk_bytes(CHUNK_FILL, run.blocks, 4)?)
                        .map_err(|error| write_error(&error))?;
                    writer
                        .write_all(&value)
                        .map_err(|error| write_error(&error))?;

                    progress.add(4);
                }
            }

            chunks += 1;
            cursor = run.block + u64::from(run.blocks);

            // The payload was consumed by the copy above; a fill chunk's four
            // bytes were read by the walker itself.
            Ok(match run.body {
                Body::Raw(bytes) => bytes,
                Body::Fill(_) => 0,
            })
        })?;
    }

    // The image covers every block it declares, so a selection that stops
    // short of the end finishes with one more hole.
    if cursor < plan.total_blocks {
        write_hole(&mut writer, plan.total_blocks - cursor, &mut chunks)?;
    }

    let mut file = writer
        .into_inner()
        .map_err(|error| MergeError::Write(error.to_string()))?;

    file.seek(SeekFrom::Start(0))
        .map_err(|error| write_error(&error))?;
    file.write_all(&header_bytes(plan.block_size, total_blocks, chunks))
        .map_err(|error| write_error(&error))?;
    file.flush().map_err(|error| write_error(&error))?;

    Ok(())
}

/// Writes the image the chunks describe, block for block.
///
/// The file is given its final length up front and every run is written where
/// it belongs, so the holes cost no reading and no writing: the file system
/// leaves them as the zeros a `DONT_CARE` run stands for.
fn write_raw(
    plan: &Plan,
    output: &Path,
    emit: &dyn Fn(MergeEvent),
    progress: &mut Progress<'_>,
) -> Result<(), MergeError> {
    let mut file = File::create(output).map_err(|error| write_error(&error))?;
    let block_size = u64::from(plan.block_size);

    file.set_len(plan.total_blocks * block_size)
        .map_err(|error| write_error(&error))?;

    let mut buffer = vec![0u8; COPY_BUFFER];

    for (index, chunk) in plan.files.iter().enumerate() {
        emit(MergeEvent::File {
            index,
            name: chunk.name.clone(),
        });

        let mut reader = open(&chunk.path, &chunk.name)?;
        let header = Header::read(&mut reader, &chunk.name)?;

        walk(&mut reader, &chunk.name, &header, &mut |reader, run| {
            file.seek(SeekFrom::Start(run.block * block_size))
                .map_err(|error| write_error(&error))?;

            let span = u64::from(run.blocks) * block_size;

            match run.body {
                Body::Raw(bytes) => {
                    copy(reader, &chunk.name, &mut file, bytes, &mut buffer, progress)?;

                    Ok(bytes)
                }
                Body::Fill(value) => {
                    // The pattern is written a bufferful at a time, so a run
                    // of a whole partition does not need a buffer of one.
                    let pattern: Vec<u8> = value
                        .iter()
                        .copied()
                        .cycle()
                        .take(COPY_BUFFER)
                        .collect();

                    let mut left = span;

                    while left > 0 {
                        let step = left.min(pattern.len() as u64) as usize;

                        file.write_all(&pattern[..step])
                            .map_err(|error| write_error(&error))?;

                        left -= step as u64;
                        progress.add(step as u64);
                    }

                    Ok(0)
                }
            }
        })?;
    }

    file.flush().map_err(|error| write_error(&error))?;

    Ok(())
}

/// Copies `bytes` of a run's payload into the merged image.
fn copy(
    reader: &mut BufReader<File>,
    name: &str,
    writer: &mut dyn Write,
    bytes: u64,
    buffer: &mut [u8],
    progress: &mut Progress<'_>,
) -> Result<(), MergeError> {
    let mut left = bytes;

    while left > 0 {
        let step = left.min(buffer.len() as u64) as usize;

        reader
            .read_exact(&mut buffer[..step])
            .map_err(|error| read_error(name, &error))?;
        writer
            .write_all(&buffer[..step])
            .map_err(|error| write_error(&error))?;

        left -= step as u64;
        progress.add(step as u64);
    }

    Ok(())
}

/// Writes a `DONT_CARE` chunk for `blocks` blocks, splitting a hole that is
/// longer than one chunk can describe.
fn write_hole(
    writer: &mut dyn Write,
    blocks: u64,
    chunks: &mut u32,
) -> Result<(), MergeError> {
    let mut left = blocks;

    while left > 0 {
        let step = u32::try_from(left.min(u64::from(u32::MAX))).unwrap_or(u32::MAX);

        writer
            .write_all(&chunk_bytes(CHUNK_DONT_CARE, step, 0)?)
            .map_err(|error| write_error(&error))?;

        *chunks += 1;
        left -= u64::from(step);
    }

    Ok(())
}

/// The 28-byte header of the merged image.
///
/// The checksum field stays zero: it describes the whole image, and the ones
/// the chunks carried describe only themselves. `libsparse` only verifies it
/// when it is asked to, which is why Motorola's own chunks leave it at zero as
/// well.
fn header_bytes(block_size: u32, total_blocks: u32, total_chunks: u32) -> [u8; 28] {
    let mut bytes = [0u8; FILE_HEADER_SIZE as usize];

    bytes[..4].copy_from_slice(&MAGIC);
    bytes[4..6].copy_from_slice(&1u16.to_le_bytes());
    bytes[6..8].copy_from_slice(&0u16.to_le_bytes());
    bytes[8..10].copy_from_slice(&FILE_HEADER_SIZE.to_le_bytes());
    bytes[10..12].copy_from_slice(&CHUNK_HEADER_SIZE.to_le_bytes());
    bytes[12..16].copy_from_slice(&block_size.to_le_bytes());
    bytes[16..20].copy_from_slice(&total_blocks.to_le_bytes());
    bytes[20..24].copy_from_slice(&total_chunks.to_le_bytes());

    bytes
}

/// The 12-byte header of one chunk of the merged image.
fn chunk_bytes(kind: u16, blocks: u32, payload: u64) -> Result<[u8; 12], MergeError> {
    let total = u64::from(CHUNK_HEADER_SIZE) + payload;
    let total = u32::try_from(total).map_err(|_| MergeError::TooLarge)?;

    let mut bytes = [0u8; CHUNK_HEADER_SIZE as usize];

    bytes[..2].copy_from_slice(&kind.to_le_bytes());
    bytes[4..8].copy_from_slice(&blocks.to_le_bytes());
    bytes[8..12].copy_from_slice(&total.to_le_bytes());

    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOCK: u32 = 4096;

    /// What a chunk file holds, the way `libsparse` would write it.
    enum Piece {
        Raw(Vec<u8>),
        Fill([u8; 4], u32),
        Hole(u32),
    }

    /// Writes one sparse image: a header declaring `total_blocks`, then the
    /// pieces.
    fn sparse_image(path: &Path, total_blocks: u32, pieces: &[Piece]) {
        let mut bytes = header_bytes(BLOCK, total_blocks, pieces.len() as u32).to_vec();

        for piece in pieces {
            match piece {
                Piece::Raw(payload) => {
                    let blocks = (payload.len() as u32) / BLOCK;
                    bytes.extend_from_slice(
                        &chunk_bytes(CHUNK_RAW, blocks, payload.len() as u64).unwrap(),
                    );
                    bytes.extend_from_slice(payload);
                }
                Piece::Fill(value, blocks) => {
                    bytes.extend_from_slice(&chunk_bytes(CHUNK_FILL, *blocks, 4).unwrap());
                    bytes.extend_from_slice(value);
                }
                Piece::Hole(blocks) => {
                    bytes.extend_from_slice(&chunk_bytes(CHUNK_DONT_CARE, *blocks, 0).unwrap());
                }
            }
        }

        std::fs::write(path, bytes).expect("write a chunk");
    }

    fn block_of(byte: u8, blocks: u32) -> Vec<u8> {
        vec![byte; (BLOCK * blocks) as usize]
    }

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "lmnflash-sparse-test-{}-{name}",
            std::process::id()
        ));

        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("create temp dir");

        directory
    }

    /// Two chunks of a four-block image, each a complete sparse image of the
    /// whole thing with only its own half filled in — which is how Motorola
    /// splits `super.img`.
    fn two_chunks(directory: &Path) -> Vec<PathBuf> {
        let first = directory.join("super.img_sparsechunk.0");
        let second = directory.join("super.img_sparsechunk.1");

        sparse_image(
            &first,
            4,
            &[Piece::Raw(block_of(0xaa, 2)), Piece::Hole(2)],
        );
        sparse_image(
            &second,
            4,
            &[Piece::Hole(2), Piece::Raw(block_of(0xbb, 2))],
        );

        vec![first, second]
    }

    fn events() -> std::cell::RefCell<Vec<MergeEvent>> {
        std::cell::RefCell::new(Vec::new())
    }

    #[test]
    fn the_plan_describes_both_outputs_before_anything_is_written() {
        let directory = scratch("plan");
        let paths = two_chunks(&directory);

        let plan = inspect(&paths).expect("a plan");

        assert_eq!(plan.block_size, BLOCK);
        assert_eq!(plan.total_blocks, 4);
        assert_eq!(plan.data_blocks, 4);
        assert_eq!(plan.stem, "super.img");
        assert_eq!(plan.output_name(Format::Sparse), "super.img");
        assert_eq!(plan.output_name(Format::Raw), "super.img.unsparse");
        assert_eq!(plan.raw_bytes, u64::from(BLOCK) * 4);
        // A header, two raw chunk headers and the four blocks behind them: the
        // merged image needs no hole, since the chunks cover it end to end.
        assert_eq!(
            plan.sparse_bytes,
            u64::from(FILE_HEADER_SIZE) + 2 * u64::from(CHUNK_HEADER_SIZE) + u64::from(BLOCK) * 4
        );
        assert!(plan.missing.is_empty());
        assert_eq!(plan.files[0].name, "super.img_sparsechunk.0");
        assert_eq!(plan.files[1].first_block, 2);

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// The merged sparse image is one image again: the runs of both chunks
    /// behind a single header, and it expands to exactly what the chunks
    /// described.
    #[test]
    fn merging_into_a_sparse_image_keeps_every_run() {
        let directory = scratch("sparse");
        let paths = two_chunks(&directory);
        let output = directory.join("out");

        let log = events();
        let summary = merge(&paths, &output, Format::Sparse, &|event| {
            log.borrow_mut().push(event)
        })
        .expect("a merge");

        assert_eq!(summary.files, 2);
        assert_eq!(summary.output, output.join("super.img"));

        let merged = summary.output.clone();
        let plan = inspect(&[merged.clone()]).expect("the merged image is sparse");
        assert_eq!(plan.total_blocks, 4);
        assert_eq!(plan.data_blocks, 4);

        // The same bytes the two chunks stood for, in one file.
        let mut expected = block_of(0xaa, 2);
        expected.extend(block_of(0xbb, 2));
        assert_eq!(expand(&merged), expected);

        assert!(matches!(
            log.borrow().last(),
            Some(MergeEvent::Finished(Ok(_)))
        ));

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// The raw image is the partition itself: every run where it belongs, and
    /// zeros for the holes.
    #[test]
    fn merging_into_a_raw_image_writes_the_holes_as_zeros() {
        let directory = scratch("raw");
        let first = directory.join("super.img_sparsechunk.0");
        let second = directory.join("super.img_sparsechunk.1");

        // Block 0 raw, block 1 a hole, block 2 filled with a pattern, block 3
        // raw again — split over two chunks the way a package carries them.
        sparse_image(
            &first,
            4,
            &[Piece::Raw(block_of(0x11, 1)), Piece::Hole(3)],
        );
        sparse_image(
            &second,
            4,
            &[
                Piece::Hole(2),
                Piece::Fill([1, 2, 3, 4], 1),
                Piece::Raw(block_of(0x22, 1)),
            ],
        );

        let output = directory.join("out");
        let log = events();
        let summary = merge(&[first, second], &output, Format::Raw, &|event| {
            log.borrow_mut().push(event)
        })
        .expect("a merge");

        assert_eq!(summary.output, output.join("super.img.unsparse"));

        let raw = std::fs::read(&summary.output).expect("the raw image");
        assert_eq!(raw.len(), (BLOCK * 4) as usize);
        assert_eq!(&raw[..BLOCK as usize], &block_of(0x11, 1)[..]);
        assert!(raw[BLOCK as usize..(BLOCK * 2) as usize]
            .iter()
            .all(|byte| *byte == 0));
        assert_eq!(
            &raw[(BLOCK * 2) as usize..(BLOCK * 3) as usize],
            &[1u8, 2, 3, 4].repeat((BLOCK / 4) as usize)[..]
        );
        assert_eq!(&raw[(BLOCK * 3) as usize..], &block_of(0x22, 1)[..]);

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// The order the files were picked in does not matter: they are merged by
    /// the block they start at.
    #[test]
    fn chunks_are_merged_in_block_order_however_they_were_picked() {
        let directory = scratch("order");
        let mut paths = two_chunks(&directory);
        paths.reverse();

        let output = directory.join("out");
        let summary = merge(&paths, &output, Format::Raw, &|_| {}).expect("a merge");

        let mut expected = block_of(0xaa, 2);
        expected.extend(block_of(0xbb, 2));
        assert_eq!(std::fs::read(&summary.output).unwrap(), expected);

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// Two files that describe the same block are not two chunks of one image,
    /// and merging them would quietly produce a third one.
    #[test]
    fn chunks_of_different_images_are_refused() {
        let directory = scratch("overlap");
        let paths = two_chunks(&directory);
        let twice = vec![paths[0].clone(), paths[0].clone()];

        assert!(matches!(
            inspect(&twice),
            Err(MergeError::Overlap { block: 0, .. })
        ));

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_file_that_is_not_sparse_is_reported_as_such() {
        let directory = scratch("not-sparse");
        let path = directory.join("super.img_sparsechunk.0");
        std::fs::write(&path, vec![0u8; 4096]).expect("write");

        assert_eq!(
            inspect(&[path]),
            Err(MergeError::NotSparse(
                "super.img_sparsechunk.0".to_owned()
            ))
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// Chunks of two different images have different block sizes often enough
    /// that it is worth saying so plainly.
    #[test]
    fn chunks_with_different_block_sizes_are_refused() {
        let directory = scratch("block-size");
        let first = directory.join("super.img_sparsechunk.0");
        let second = directory.join("super.img_sparsechunk.1");

        sparse_image(&first, 1, &[Piece::Raw(block_of(0xaa, 1))]);

        // The same image with a 512-byte block size, written by hand.
        let mut bytes = header_bytes(512, 1, 1).to_vec();
        bytes.extend_from_slice(&chunk_bytes(CHUNK_RAW, 1, 512).unwrap());
        bytes.extend(vec![0xbb; 512]);
        std::fs::write(&second, bytes).expect("write");

        assert!(matches!(
            inspect(&[first, second]),
            Err(MergeError::BlockSize {
                found: 512,
                expected: 4096,
                ..
            })
        ));

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// A chunk the user forgot leaves a hole in the merged image, so the plan
    /// says which numbers are missing instead of letting it pass unnoticed.
    #[test]
    fn a_missing_chunk_number_is_reported_in_the_plan() {
        let directory = scratch("missing");
        let first = directory.join("super.img_sparsechunk.0");
        let third = directory.join("super.img_sparsechunk.2");

        sparse_image(&first, 6, &[Piece::Raw(block_of(0xaa, 2)), Piece::Hole(4)]);
        sparse_image(
            &third,
            6,
            &[Piece::Hole(4), Piece::Raw(block_of(0xcc, 2))],
        );

        let plan = inspect(&[first, third]).expect("a plan");

        assert_eq!(plan.missing, vec![1]);
        // The hole between them is still part of the image.
        assert_eq!(plan.total_blocks, 6);
        assert_eq!(plan.data_blocks, 4);

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// Writing the merged image over one of its own chunks would destroy the
    /// input halfway through reading it.
    #[test]
    fn the_merged_image_is_never_written_over_a_chunk() {
        let directory = scratch("in-place");
        let path = directory.join("super.img");

        sparse_image(&path, 1, &[Piece::Raw(block_of(0xaa, 1))]);

        let error = merge(&[path], &directory, Format::Sparse, &|_| {})
            .expect_err("the output is the input");

        assert_eq!(error, MergeError::OutputIsInput("super.img".to_owned()));

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// The outcome is the last event even when the job fails early, so a
    /// dialog that waits for it can never hang.
    #[test]
    fn a_failed_merge_still_reports_its_outcome() {
        let log = events();
        let result = merge(&[], Path::new("."), Format::Sparse, &|event| {
            log.borrow_mut().push(event)
        });

        assert_eq!(result, Err(MergeError::NoFiles));
        assert_eq!(
            log.into_inner().last(),
            Some(&MergeEvent::Finished(Err(MergeError::NoFiles)))
        );
    }

    #[test]
    fn a_chunks_name_tells_the_image_and_the_number_it_belongs_to() {
        assert_eq!(image_stem("super.img_sparsechunk.0"), "super.img");
        assert_eq!(image_stem("system.img_sparsechunk.12"), "system.img");
        assert_eq!(image_stem("SUPER.IMG_SparseChunk.3"), "SUPER.IMG");
        assert_eq!(image_stem("super.img.0"), "super.img");
        assert_eq!(image_stem("super.img"), "super.img");

        assert_eq!(chunk_index("super.img_sparsechunk.0"), Some(0));
        assert_eq!(chunk_index("super.img_sparsechunk.12"), Some(12));
        assert_eq!(chunk_index("super.img"), None);
    }

    /// What the chunks of a sparse image add up to, for a test that compares
    /// the merged image with the bytes it stands for.
    fn expand(path: &Path) -> Vec<u8> {
        let name = file_name(path);
        let mut reader = open(path, &name).expect("open");
        let header = Header::read(&mut reader, &name).expect("a header");
        let mut bytes = Vec::new();

        walk(&mut reader, &name, &header, &mut |reader, run| {
            bytes.resize((run.block * u64::from(header.block_size)) as usize, 0);

            match run.body {
                Body::Raw(len) => {
                    let mut payload = vec![0u8; len as usize];
                    reader.read_exact(&mut payload).expect("a payload");
                    bytes.extend_from_slice(&payload);

                    Ok(len)
                }
                Body::Fill(value) => {
                    let span = u64::from(run.blocks) * u64::from(header.block_size);
                    bytes.extend(
                        value
                            .iter()
                            .copied()
                            .cycle()
                            .take(span as usize),
                    );

                    Ok(0)
                }
            }
        })
        .expect("a table");

        bytes.resize(
            (u64::from(header.total_blocks) * u64::from(header.block_size)) as usize,
            0,
        );

        bytes
    }
}
