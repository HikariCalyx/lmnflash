//! Motorola `flashfile.xml` support for the "Firmware Flash" feature
//! (Mode 2, Smartphone Flash).
//!
//! A firmware package is either the `flashfile.xml` itself (with the image
//! files sitting next to it) or the ZIP archive the factory firmware is
//! shipped in (which contains the `flashfile.xml`, usually at the root).
//! The XML describes an ordered list of `<step>` elements:
//!
//! ```xml
//! <flashing>
//!   <header>
//!     <phone_model model="SM8650_ARCFOX_IFLASH"/>
//!     <software_version version="arcfox_factory-userdebug 14 …"/>
//!   </header>
//!   <steps interface="AP">
//!     <step operation="flash" partition="boot" filename="boot.img" MD5="d73e…"/>
//!     <step operation="erase" partition="userdata"/>
//!     <step operation="oem" var="config bootmode fastboot"/>
//!   </steps>
//! </flashing>
//!
//! This module only parses those steps and unpacks the ZIP; running them is
//! the job of `crate::flash_engine`.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// A single flashing step, normalized from one `<step>` element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlashOp {
    /// `operation="flash"`: write `file` (relative to the package directory)
    /// to `partition`.
    Flash {
        partition: String,
        file: String,
        /// Expected MD5 from the XML, when the step carries one.
        md5: Option<String>,
    },
    /// `operation="erase"`: erase `partition`.
    Erase { partition: String },
    /// `operation="oem"`: run `fastboot oem <var>`.
    Oem { var: String },
    /// `operation="getvar"`: query a variable (`fastboot getvar:<var>`).
    /// Informational — a bootloader that does not know the variable must not
    /// abort the flash.
    Getvar { var: String },
}

/// The firmware part a step belongs to.
///
/// Factory firmware is flashed in three parts, and the scripts Motorola ships
/// with it (`reference/tiny-fastboot-script`) offer one menu per part:
///
/// * [`FlashPart::Ap`] — the Android OS (boot, vbmeta, system/vendor/super, …),
/// * [`FlashPart::Bp`] — the baseband (radio/modem, fsg, md1img),
/// * [`FlashPart::Bl`] — the bootloader (GPT, bootloader/preloader, …).
///
/// A flashed image whose partition is not named in either explicit list counts
/// as AP. `erase` and `oem` commands are not an image of any part, so
/// [`FlashOp::part`] reports `None` for them and the part buttons leave them
/// untouched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlashPart {
    Ap,
    Bp,
    Bl,
}

impl FlashPart {
    /// The short name the selection buttons are labelled with.
    pub fn name(self) -> &'static str {
        match self {
            Self::Ap => "AP",
            Self::Bp => "BP",
            Self::Bl => "BL",
        }
    }
}

/// Partitions flashed with the baseband part (`do_flash_bp_*.cmd`).
const BP_PARTITIONS: &[&str] = &["radio", "modem", "fsg", "md1img", "md1img2"];

/// Partitions flashed with the bootloader part (`do_flash_bl_*.cmd`).
const BL_PARTITIONS: &[&str] = &[
    // GPT and the bootloader images of the Qualcomm / generic flow.
    "partition",
    "gpt",
    "bootloader",
    "motoboot",
    "preloader",
    "pit",
    "diskmap",
    "pi_img",
    // The MediaTek bootloader chain.
    "lk",
    "scp",
    "sspm",
    "mcupm",
    "gpueb",
    "spmfw",
    "gz",
    "tee",
    "dpm",
    "vcp",
    "efuse",
    "efusebackup",
    "keystorage",
    "fwbl1",
];

impl FlashOp {
    /// The partition this step names, spelled the way the flashfile does.
    /// `oem` and `getvar` steps name none.
    fn raw_partition(&self) -> Option<&str> {
        match self {
            Self::Flash { partition, .. } | Self::Erase { partition } => Some(partition),
            Self::Oem { .. } | Self::Getvar { .. } => None,
        }
    }

    /// The partition this step names, normalized (lower-cased, A/B slot suffix
    /// dropped) so it can be compared with a known partition name.
    fn partition(&self) -> Option<String> {
        self.raw_partition().map(normalize_partition)
    }

    /// Which firmware part this step is an image of, when it is one.
    ///
    /// `erase` and `oem` commands are part of no group (they are not images),
    /// so they answer `None` and stay exactly as the user left them.
    pub fn part(&self) -> Option<FlashPart> {
        // Only a flashed image belongs to a part.
        if !matches!(self, Self::Flash { .. }) {
            return None;
        }

        let partition = self.partition()?;

        if BP_PARTITIONS.contains(&partition.as_str()) {
            Some(FlashPart::Bp)
        } else if BL_PARTITIONS.contains(&partition.as_str()) {
            Some(FlashPart::Bl)
        } else {
            Some(FlashPart::Ap)
        }
    }

    /// A short, tool-agnostic description of the step, used for the progress
    /// line and the log (e.g. `flash boot (boot.img)`).
    pub fn describe(&self) -> String {
        match self {
            Self::Flash { partition, file, .. } => format!("flash {partition} ({file})"),
            Self::Erase { partition } => format!("erase {partition}"),
            Self::Oem { var } => format!("oem {var}"),
            Self::Getvar { var } => format!("getvar {var}"),
        }
    }
}

/// Partitions a firmware package must not write.
///
/// `persist` holds per-unit calibration data and `cid`/`secdata`/
/// `secdataBackup` hold carrier and secure-element data: writing the copies
/// from another unit breaks the phone, so the steps that touch them are
/// dropped when a package is loaded.
const IGNORED_PARTITIONS: &[&str] = &["cid", "persist", "secdata", "secdatabackup"];

/// Whether a step touches a partition that is ignored.
pub fn is_ignored(step: &FlashOp) -> bool {
    step.partition()
        .is_some_and(|partition| IGNORED_PARTITIONS.contains(&partition.as_str()))
}

/// Drops the steps that touch an ignored partition, reporting the partitions
/// left out (in the order they appear, spelled the way the flashfile does).
fn without_ignored_steps(steps: Vec<FlashOp>) -> (Vec<FlashOp>, Vec<String>) {
    let mut kept = Vec::with_capacity(steps.len());
    let mut ignored: Vec<String> = Vec::new();

    for step in steps {
        if !is_ignored(&step) {
            kept.push(step);
            continue;
        }

        if let Some(name) = step.raw_partition() {
            let name = name.to_owned();

            if !ignored.contains(&name) {
                ignored.push(name);
            }
        }
    }

    (kept, ignored)
}

/// A parsed `flashfile.xml` plus the directory its image files live in.
#[derive(Debug, Clone)]
pub struct FlashPackage {
    /// Directory the `filename` attributes are resolved against.
    pub directory: PathBuf,
    /// Path of the `flashfile.xml` the steps were read from.
    pub flashfile: PathBuf,
    /// `<phone_model model="…">`, when present.
    pub model: Option<String>,
    /// `<software_version version="…">`, when present.
    pub software_version: Option<String>,
    /// The CID (carrier/region code) of the package: the `<cid_value>` of the
    /// flashfile, or the one embedded in its `vbmeta` image.
    pub cid: Option<String>,
    /// Partitions whose steps were dropped because a package must not write
    /// them (see `IGNORED_PARTITIONS`), spelled the way the flashfile does.
    pub ignored_partitions: Vec<String>,
    /// The `flash` / `erase` / `oem` / `getvar` steps, in document order.
    pub steps: Vec<FlashOp>,
}

/// What the `<header>` of a `flashfile.xml` declares.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlashHeader {
    /// `<phone_model model="…">`.
    pub model: Option<String>,
    /// `<software_version version="…">`.
    pub software_version: Option<String>,
    /// `<cid_value value="0x0032">`: the carrier/region the package is for.
    pub cid: Option<String>,
}

/// Parses a `flashfile.xml` document.
///
/// Unknown `<step operation="…">` values are rejected: silently skipping a
/// step could leave the phone with a half-written firmware.
pub fn parse(xml: &str) -> Result<(FlashHeader, Vec<FlashOp>), String> {
    // A UTF-8 byte order mark is not part of the document.
    let xml = xml.strip_prefix('\u{feff}').unwrap_or(xml);
    let document = roxmltree::Document::parse(xml).map_err(|error| error.to_string())?;
    let root = document.root_element();

    if !root.has_tag_name("flashing") {
        return Err(format!(
            "expected a <flashing> root element, found <{}>",
            root.tag_name().name()
        ));
    }

    let mut header_values = FlashHeader::default();

    if let Some(header) = children_named(root, "header") {
        if let Some(node) = header.children().find(|node| node.has_tag_name("phone_model")) {
            header_values.model = attribute(&node, "model");
        }

        if let Some(node) = header
            .children()
            .find(|node| node.has_tag_name("software_version"))
        {
            header_values.software_version = attribute(&node, "version");
        }

        if let Some(node) = header.children().find(|node| node.has_tag_name("cid_value")) {
            header_values.cid = attribute(&node, "value").map(|value| normalize_cid(&value));
        }
    }

    let mut steps = Vec::new();

    for block in root.children().filter(|node| node.has_tag_name("steps")) {
        for step in block.children().filter(|node| node.has_tag_name("step")) {
            steps.push(parse_step(&step)?);
        }
    }

    if steps.is_empty() {
        return Err("the flashfile contains no <step> elements".to_owned());
    }

    Ok((header_values, steps))
}

fn parse_step(step: &roxmltree::Node<'_, '_>) -> Result<FlashOp, String> {
    let operation = attribute(step, "operation")
        .ok_or_else(|| "a <step> element has no `operation` attribute".to_owned())?;

    match operation.as_str() {
        "flash" => {
            let partition = attribute(step, "partition")
                .ok_or("a `flash` step has no `partition` attribute")?;
            let file = attribute(step, "filename")
                .ok_or("a `flash` step has no `filename` attribute")?;
            // The attribute is spelled `MD5` in the factory files; accept the
            // lower-case spelling as well.
            let md5 = attribute(step, "MD5").or_else(|| attribute(step, "md5"));

            Ok(FlashOp::Flash {
                partition,
                file,
                md5,
            })
        }
        "erase" => {
            let partition = attribute(step, "partition")
                .ok_or("an `erase` step has no `partition` attribute")?;

            Ok(FlashOp::Erase { partition })
        }
        "oem" => {
            let var = attribute(step, "var").ok_or("an `oem` step has no `var` attribute")?;

            Ok(FlashOp::Oem { var })
        }
        "getvar" => {
            // Informational: the value is written to the log.
            let var = attribute(step, "var").ok_or("a `getvar` step has no `var` attribute")?;

            Ok(FlashOp::Getvar { var })
        }
        other => Err(format!("unsupported step operation `{other}`")),
    }
}

/// Reads an attribute (XML attribute names are case-sensitive, and Motorola
/// spells the checksum `MD5`).
fn attribute(node: &roxmltree::Node<'_, '_>, name: &str) -> Option<String> {
    node.attribute(name).map(str::to_owned)
}

/// A group of `erase` commands that can be added on top of a package, ported
/// from `do_erase_data.cmd` and `do_erase_modem_cache.cmd`.
///
/// Firmware packages do not always erase these partitions themselves (a
/// package never contains the baseband cache, for instance), so they are
/// offered as standalone erases — the same two entries the flashing scripts
/// have in their menus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EraseGroup {
    /// User data and the caches/leftovers a factory reset clears.
    Userdata,
    /// The baseband (modem) NV cache.
    NvCache,
}

impl EraseGroup {
    /// The partitions this group erases, in the order the reference scripts
    /// erase them.
    pub fn partitions(self) -> &'static [&'static str] {
        match self {
            // `do_erase_data.cmd`. `DDR` (the pre-GPT spelling) is left out;
            // matching is case-insensitive anyway.
            Self::Userdata => &[
                "carrier",
                "cache",
                "userdata",
                "customize",
                "clogo",
                "metadata",
                "ddr",
            ],
            // `do_erase_modem_cache.cmd`: the Qualcomm modem cache partitions,
            // or the single MediaTek `nvdata` partition.
            Self::NvCache => &[
                "modemst1",
                "modemst2",
                "mdmddr",
                "mdm1m9kefs1",
                "mdm1m9kefs2",
                "nvdata",
            ],
        }
    }

    /// FTL id of the button that adds this group.
    pub fn label_id(self) -> &'static str {
        match self {
            Self::Userdata => "firmware-flash-erase-userdata",
            Self::NvCache => "firmware-flash-erase-nv-cache",
        }
    }
}

/// Lower-cases a partition name and drops an A/B slot suffix, so `boot_a` or
/// `vbmeta_b` are recognized like `boot` and `vbmeta`.
pub(crate) fn normalize_partition(partition: &str) -> String {
    let partition = partition.trim().to_ascii_lowercase();

    match partition
        .strip_suffix("_a")
        .or_else(|| partition.strip_suffix("_b"))
    {
        Some(base) if !base.is_empty() => base.to_owned(),
        _ => partition,
    }
}

fn children_named<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    name: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    node.children().find(|child| child.has_tag_name(name))
}

/// The directory firmware packages are unpacked into. Everything inside it is
/// disposable and is cleared before every extraction.
pub fn work_directory() -> PathBuf {
    std::env::temp_dir()
        .join("lmnflash-firmware-flash")
        .join("package")
}

/// Deletes the unpacked package (best effort; the directory is re-created on
/// the next extraction anyway).
pub fn cleanup_work_directory() {
    let _ = std::fs::remove_dir_all(work_directory());
}

/// Loads a package from `path`: a ZIP archive or a `flashfile.xml`.
///
/// `on_progress` reports the ZIP extraction as `(bytes_written, total_bytes)`.
/// The result carries the package directory the image files are read from.
pub fn load(
    path: &Path,
    on_progress: &mut dyn FnMut(u64, u64),
) -> Result<FlashPackage, String> {
    if path.is_dir() {
        return Err("select the firmware ZIP or its flashfile.xml, not a directory".to_owned());
    }

    if !path.is_file() {
        return Err(format!("{} does not exist", path.display()));
    }

    let is_zip = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"));

    let directory = if is_zip {
        let destination = work_directory();
        cleanup_work_directory();
        std::fs::create_dir_all(&destination)
            .map_err(|error| format!("could not create {}: {error}", destination.display()))?;

        extract_zip(path, &destination, on_progress)?;
        destination
    } else {
        path.parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    };

    let flashfile = if is_zip {
        find_flashfile(&directory).ok_or_else(|| {
            "no flashfile.xml found in the selected ZIP archive".to_owned()
        })?
    } else {
        path.to_path_buf()
    };

    let xml = read_flashfile(&flashfile)?;
    let (header, steps) =
        parse(&xml).map_err(|error| format!("{}: {error}", flashfile.display()))?;

    // Steps a package must not run (they would write another unit's
    // calibration or carrier data) are dropped before anything can flash or
    // even show them.
    let (steps, ignored_partitions) = without_ignored_steps(steps);

    let directory = flashfile
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| directory.clone());

    // The CID is declared by the flashfile, or embedded in the `vbmeta` image
    // when it does not declare one.
    let cid = header
        .cid
        .clone()
        .or_else(|| cid_from_vbmeta(&directory, &steps));

    Ok(FlashPackage {
        directory,
        flashfile,
        model: header.model,
        software_version: header.software_version,
        cid,
        ignored_partitions,
        steps,
    })
}

/// Reads the CID out of the package's `vbmeta` image, used when the
/// `flashfile.xml` declares no `<cid_value>`.
///
/// The image carries the code Motorola puts in `vbmeta` as plain text,
/// followed by the device codename and the CID in decimal — a real image has
/// `HAB_META\0arcfox_50`, i.e. CID `0x0032` (`reference/M_cid.md` documents
/// the same string without the NUL: `HAB_METAeqs_50`).
fn cid_from_vbmeta(directory: &Path, steps: &[FlashOp]) -> Option<String> {
    /// How much of the image is searched. `vbmeta` is a few hundred KiB; the
    /// cap only keeps a mis-declared `filename` from pulling in a huge file.
    const MAX_SCAN: u64 = 8 << 20;
    const MAGIC: &[u8] = b"HAB_META";

    let path = vbmeta_file(directory, steps)?;

    let mut bytes = Vec::new();
    std::fs::File::open(&path)
        .ok()?
        .take(MAX_SCAN)
        .read_to_end(&mut bytes)
        .ok()?;

    let mut search_from = 0;

    while let Some(offset) = bytes[search_from..]
        .windows(MAGIC.len())
        .position(|window| window == MAGIC)
    {
        let position = search_from + offset;

        if let Some(cid) = cid_after_marker(&bytes[position + MAGIC.len()..]) {
            return Some(cid);
        }

        search_from = position + MAGIC.len();
    }

    None
}

/// Reads the CID that follows a `HAB_META` marker: `<codename>_<cid>`, with
/// the decimal code after the last `_` (`arcfox_50` → `50`).
fn cid_after_marker(rest: &[u8]) -> Option<String> {
    // A NUL byte separates the marker from the codename in a real image; the
    // codename is then ended by the NUL that follows it.
    let identifier: String = rest
        .iter()
        .take(64)
        .skip_while(|byte| **byte == 0)
        .take_while(|byte| byte.is_ascii_alphanumeric() || **byte == b'_')
        .map(|byte| char::from(*byte))
        .collect();

    let (_, value) = identifier.rsplit_once('_')?;

    value.parse::<u32>().ok().map(format_cid)
}

/// The `vbmeta` image of the package: the file its `flash` step names, or a
/// plain `vbmeta.img` next to the flashfile.
fn vbmeta_file(directory: &Path, steps: &[FlashOp]) -> Option<PathBuf> {
    let named = steps.iter().find_map(|step| match step {
        FlashOp::Flash {
            partition, file, ..
        } if normalize_partition(partition) == "vbmeta" => Some(directory.join(file)),
        _ => None,
    });

    match named {
        Some(path) if path.is_file() => Some(path),
        _ => {
            let fallback = directory.join("vbmeta.img");
            fallback.is_file().then_some(fallback)
        }
    }
}

/// Whether a device CID accepts firmware from any region: `0x0000` is the
/// super CID and `0x00FF` the factory super CID (`reference/M_cid.md`).
fn cid_allows_any_region(cid: &str) -> bool {
    matches!(cid.to_ascii_uppercase().as_str(), "0X0000" | "0X00FF")
}

/// Whether a package and a device disagree about the CID in a way worth
/// warning about. Both sides are compared in their normalized form, and a
/// device with a super CID takes firmware of any region.
pub(crate) fn cid_mismatch(package: &str, device: &str) -> bool {
    !package.eq_ignore_ascii_case(device) && !cid_allows_any_region(device)
}

/// Formats a CID the way the flashfiles write it, e.g. `0x0032`.
fn format_cid(cid: u32) -> String {
    format!("0x{cid:04X}")
}

/// Normalizes a CID written in a flashfile or reported by a device: `0x0032`,
/// `0x32` and `50` all mean the same code. A value that cannot be read is
/// kept as it is.
pub(crate) fn normalize_cid(value: &str) -> String {
    let value = value.trim();

    let parsed = match value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        // The code inside `vbmeta` is decimal, but a bare hex value (`DEAD`)
        // is accepted as well.
        None => value
            .parse::<u32>()
            .ok()
            .or_else(|| u32::from_str_radix(value, 16).ok()),
    };

    match parsed {
        Some(cid) => format_cid(cid),
        None => value.to_owned(),
    }
}

/// Reads a `flashfile.xml` as text.
///
/// Factory packages are UTF-8, but a file edited with a Windows tool is
/// easily saved as UTF-16 with a byte order mark; both are accepted, and a
/// UTF-8 BOM (which the XML parser would choke on) is stripped.
fn read_flashfile(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;

    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        if bytes.len() % 2 != 0 {
            return Err(format!("{} is not a valid UTF-16 file", path.display()));
        }

        let little_endian = bytes[0] == 0xFF;
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|pair| {
                if little_endian {
                    u16::from_le_bytes([pair[0], pair[1]])
                } else {
                    u16::from_be_bytes([pair[0], pair[1]])
                }
            })
            .collect();

        return String::from_utf16(&units)
            .map_err(|error| format!("{} is not valid UTF-16 text: {error}", path.display()));
    }

    String::from_utf8(bytes)
        .map_err(|error| format!("{} is not valid UTF-8 text: {error}", path.display()))
}

/// Finds the `flashfile.xml` of an extracted package: at the root of the
/// archive, or a few directory levels down (some packages wrap everything in
/// a per-model folder).
pub fn find_flashfile(directory: &Path) -> Option<PathBuf> {
    fn search(directory: &Path, depth: usize) -> Option<PathBuf> {
        let mut subdirectories = Vec::new();

        let entries = std::fs::read_dir(directory).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();

            if path.is_dir() {
                subdirectories.push(path);
            } else if path
                .file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("flashfile.xml"))
            {
                return Some(path);
            }
        }

        if depth == 0 {
            return None;
        }

        // Depth-first, in a stable order so the same package always loads the
        // same way.
        subdirectories.sort();
        for subdirectory in subdirectories {
            if let Some(found) = search(&subdirectory, depth - 1) {
                return Some(found);
            }
        }

        None
    }

    search(directory, 3)
}

/// Extracts every file of a ZIP archive into `destination`.
///
/// Entry names are sanitized (`ZipFile::enclosed_name`), so an archive can
/// never write outside `destination`.
pub fn extract_zip(
    archive: &Path,
    destination: &Path,
    on_progress: &mut dyn FnMut(u64, u64),
) -> Result<(), String> {
    let file = std::fs::File::open(archive)
        .map_err(|error| format!("could not open {}: {error}", archive.display()))?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|error| format!("{} is not a readable ZIP archive: {error}", archive.display()))?;

    // Summing the entry sizes needs the archive itself (each `by_index` call
    // borrows it mutably), so it cannot be written as a lazy iterator.
    let mut total = 0u64;
    for index in 0..zip.len() {
        if let Ok(entry) = zip.by_index(index) {
            if !entry.is_dir() {
                total += entry.size();
            }
        }
    }

    let mut written = 0u64;
    let mut buffer = vec![0u8; 1 << 20];

    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|error| format!("could not read entry #{index}: {error}"))?;

        // `enclosed_name` rejects absolute paths and `..` components.
        let Some(relative) = entry.enclosed_name() else {
            continue;
        };
        let target = destination.join(relative);

        if entry.is_dir() {
            std::fs::create_dir_all(&target)
                .map_err(|error| format!("could not create {}: {error}", target.display()))?;
            continue;
        }

        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
        }

        let mut output = std::fs::File::create(&target)
            .map_err(|error| format!("could not create {}: {error}", target.display()))?;

        loop {
            let read = entry
                .read(&mut buffer)
                .map_err(|error| format!("could not read {}: {error}", target.display()))?;
            if read == 0 {
                break;
            }

            output
                .write_all(&buffer[..read])
                .map_err(|error| format!("could not write {}: {error}", target.display()))?;
            written += read as u64;
            on_progress(written, total.max(written));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" ?>
<flashing>
  <header>
    <phone_model model="SM8650_ARCFOX_IFLASH"/>
    <software_version version="arcfox_factory-userdebug 14 U3UX34.30-F.15"/>
    <sparsing enabled="true" max-sparse-size="536870912"/>
    <interfaces>
      <interface name="AP"/>
    </interfaces>
  </header>
  <steps interface="AP">
    <step MD5="251f6137c4ba2e99470bfcc646c9116e" filename="gpt.bin" operation="flash" partition="partition"/>
    <step operation="oem" var="config bootmode fastboot"/>
    <step MD5="A0C2E5D4E34F845A49B9261FA2491618" filename="sec.elf" operation="flash" partition="secdataBackup"/>
    <step operation="erase" partition="userdata"/>
  </steps>
</flashing>
"#;

    #[test]
    fn parses_header_and_steps_in_order() {
        let (header, steps) = parse(SAMPLE).unwrap();

        assert_eq!(header.model.as_deref(), Some("SM8650_ARCFOX_IFLASH"));
        assert_eq!(
            header.software_version.as_deref(),
            Some("arcfox_factory-userdebug 14 U3UX34.30-F.15")
        );
        assert_eq!(header.cid, None);
        assert_eq!(steps.len(), 4);

        assert_eq!(
            steps[0],
            FlashOp::Flash {
                partition: "partition".to_owned(),
                file: "gpt.bin".to_owned(),
                md5: Some("251f6137c4ba2e99470bfcc646c9116e".to_owned()),
            }
        );
        assert_eq!(
            steps[1],
            FlashOp::Oem {
                var: "config bootmode fastboot".to_owned(),
            }
        );
        // Attributes are case-sensitive: the checksum keeps its upper-case
        // digits as written.
        assert_eq!(
            steps[2],
            FlashOp::Flash {
                partition: "secdataBackup".to_owned(),
                file: "sec.elf".to_owned(),
                md5: Some("A0C2E5D4E34F845A49B9261FA2491618".to_owned()),
            }
        );
        assert_eq!(
            steps[3],
            FlashOp::Erase {
                partition: "userdata".to_owned(),
            }
        );
    }

    #[test]
    fn flash_without_checksum_is_allowed() {
        let xml = r#"<flashing><steps>
            <step operation="flash" partition="boot" filename="boot.img"/>
        </steps></flashing>"#;

        let (_, steps) = parse(xml).unwrap();

        assert_eq!(
            steps[0],
            FlashOp::Flash {
                partition: "boot".to_owned(),
                file: "boot.img".to_owned(),
                md5: None,
            }
        );
    }

    #[test]
    fn steps_of_several_blocks_are_concatenated() {
        let xml = r#"<flashing>
            <steps interface="AP"><step operation="erase" partition="apdp"/></steps>
            <steps interface="BP"><step operation="erase" partition="modem"/></steps>
        </flashing>"#;

        let (_, steps) = parse(xml).unwrap();

        assert_eq!(steps.len(), 2);
        assert_eq!(
            steps[1],
            FlashOp::Erase {
                partition: "modem".to_owned(),
            }
        );
    }

    #[test]
    fn unknown_operations_are_rejected() {
        let xml = r#"<flashing><steps><step operation="md5"/></steps></flashing>"#;

        let error = parse(xml).unwrap_err();

        assert!(error.contains("md5"), "unexpected error: {error}");
    }

    #[test]
    fn missing_root_element_is_rejected() {
        assert!(parse("<not-flashing/>").is_err());
        assert!(parse("not xml at all").is_err());
    }

    #[test]
    fn empty_step_list_is_rejected() {
        assert!(parse("<flashing><steps/></flashing>").is_err());
    }

    #[test]
    fn firmware_parts_follow_the_reference_scripts() {
        let flash = |partition: &str| FlashOp::Flash {
            partition: partition.to_owned(),
            file: "image.img".to_owned(),
            md5: None,
        };
        let erase = |partition: &str| FlashOp::Erase {
            partition: partition.to_owned(),
        };

        // Bootloader: the GPT plus the bootloader images.
        assert_eq!(flash("partition").part(), Some(FlashPart::Bl));
        assert_eq!(flash("bootloader").part(), Some(FlashPart::Bl));
        // The MediaTek bootloader chain.
        assert_eq!(flash("lk").part(), Some(FlashPart::Bl));
        assert_eq!(flash("efuseBackup").part(), Some(FlashPart::Bl));

        // Baseband.
        assert_eq!(flash("radio").part(), Some(FlashPart::Bp));
        assert_eq!(flash("modem").part(), Some(FlashPart::Bp));
        assert_eq!(flash("fsg").part(), Some(FlashPart::Bp));
        assert_eq!(flash("md1img").part(), Some(FlashPart::Bp));

        // Everything else is the Android OS part.
        assert_eq!(flash("super").part(), Some(FlashPart::Ap));
        assert_eq!(flash("boot").part(), Some(FlashPart::Ap));
        assert_eq!(flash("dtbo").part(), Some(FlashPart::Ap));
        assert_eq!(flash("secdataBackup").part(), Some(FlashPart::Ap));

        // `erase` and `oem` commands are an image of no part, so the part
        // buttons never touch them — not even when they name the partition of
        // a part (`erase preloader`).
        assert_eq!(erase("userdata").part(), None);
        assert_eq!(erase("preloader").part(), None);
        assert_eq!(
            FlashOp::Oem {
                var: "config bootmode fastboot".to_owned(),
            }
            .part(),
            None
        );
    }

    #[test]
    fn slot_suffixes_and_case_do_not_matter() {
        let flash = |partition: &str| FlashOp::Flash {
            partition: partition.to_owned(),
            file: "image.img".to_owned(),
            md5: None,
        };

        assert_eq!(flash("boot_a").part(), Some(FlashPart::Ap));
        assert_eq!(flash("MODEM_B").part(), Some(FlashPart::Bp));
        assert_eq!(flash(" Bootloader ").part(), Some(FlashPart::Bl));
        assert_eq!(flash("vbmeta_system").part(), Some(FlashPart::Ap));
        assert_eq!(flash("_b").part(), Some(FlashPart::Ap));
    }

    #[test]
    fn cid_mismatch_ignores_super_cids_and_casing() {
        // The package CID and the device CID are compared in the same form.
        assert!(!cid_mismatch("0x0032", "0x0032"));
        assert!(!cid_mismatch("0x0032", "0x0032".to_ascii_lowercase().as_str()));
        assert!(cid_mismatch("0x0032", "0x0033"));

        // A device with a super (or factory super) CID flashes any region.
        assert!(!cid_mismatch("0x0032", "0x0000"));
        assert!(!cid_mismatch("0x0032", "0x00FF"));
        // A corrupt CID is still a mismatch.
        assert!(cid_mismatch("0x0032", "0xDEAD"));
    }

    #[test]
    fn ignored_partitions_are_dropped_from_the_steps() {
        let xml = r#"<flashing><steps>
            <step operation="flash" partition="boot" filename="boot.img"/>
            <step operation="flash" partition="persist" filename="persist.img"/>
            <step operation="erase" partition="secdata"/>
            <step operation="erase" partition="secdataBackup"/>
            <step operation="flash" partition="cid" filename="cid.bin"/>
            <step operation="oem" var="fb_mode_set"/>
        </steps></flashing>"#;

        // The parser reports the file as it is; loading is what drops the
        // steps that must never run.
        let (_, steps) = parse(xml).unwrap();
        assert_eq!(steps.len(), 6);

        let (kept, ignored) = without_ignored_steps(steps);

        assert_eq!(kept.len(), 2);
        assert!(matches!(kept[0], FlashOp::Flash { .. }));
        assert!(matches!(kept[1], FlashOp::Oem { .. }));
        // The partitions are reported the way the flashfile spells them.
        assert_eq!(
            ignored,
            vec![
                "persist".to_owned(),
                "secdata".to_owned(),
                "secdataBackup".to_owned(),
                "cid".to_owned(),
            ]
        );
    }

    #[test]
    fn ignored_partitions_match_exactly() {
        // Only the listed partitions: `prodpersist` is a partition of its own.
        assert!(is_ignored(&FlashOp::Erase {
            partition: "persist".to_owned(),
        }));
        // ...and neither case nor a slot suffix matters.
        assert!(is_ignored(&FlashOp::Erase {
            partition: "SECDATA".to_owned(),
        }));
        assert!(is_ignored(&FlashOp::Flash {
            partition: "cid_a".to_owned(),
            file: "cid.bin".to_owned(),
            md5: None,
        }));

        assert!(!is_ignored(&FlashOp::Erase {
            partition: "prodpersist".to_owned(),
        }));
        assert!(!is_ignored(&FlashOp::Erase {
            partition: "userdata".to_owned(),
        }));
        // An `oem` command names no partition.
        assert!(!is_ignored(&FlashOp::Oem {
            var: "persist".to_owned(),
        }));
    }

    #[test]
    fn erase_groups_are_disjoint_lower_case_names() {
        let userdata = EraseGroup::Userdata.partitions();
        let nv_cache = EraseGroup::NvCache.partitions();

        assert!(userdata.contains(&"userdata"));
        assert!(userdata.contains(&"metadata"));
        assert!(userdata.contains(&"ddr"));
        assert!(nv_cache.contains(&"modemst1"));
        assert!(nv_cache.contains(&"mdm1m9kefs1"));
        assert!(nv_cache.contains(&"nvdata"));

        // A partition is erased by one group only, so enabling both cannot
        // erase the same partition twice.
        assert!(!userdata.iter().any(|name| nv_cache.contains(name)));

        // Names are compared against normalized partitions, so they are plain
        // lower-case names.
        assert!(
            userdata
                .iter()
                .chain(nv_cache.iter())
                .all(|name| *name == name.to_ascii_lowercase())
        );
    }

    #[test]
    fn parses_getvar_steps() {
        let xml = r#"<flashing><steps>
            <step operation="getvar" var="max-sparse-size"/>
        </steps></flashing>"#;

        let (_, steps) = parse(xml).unwrap();

        assert_eq!(
            steps[0],
            FlashOp::Getvar {
                var: "max-sparse-size".to_owned(),
            }
        );
        assert_eq!(steps[0].describe(), "getvar max-sparse-size");
        // A query is no image of a part.
        assert_eq!(steps[0].part(), None);
    }

    #[test]
    fn reads_the_cid_declared_by_the_flashfile() {
        let xml = r#"<flashing>
            <header><cid_value value="0x0032"/></header>
            <steps><step operation="erase" partition="userdata"/></steps>
        </flashing>"#;

        let (header, _) = parse(xml).unwrap();

        assert_eq!(header.cid.as_deref(), Some("0x0032"));
        // The same code written differently is normalized.
        assert_eq!(normalize_cid("50"), "0x0032");
        assert_eq!(normalize_cid("0x32"), "0x0032");
        assert_eq!(normalize_cid("0xdead"), "0xDEAD");
        assert_eq!(normalize_cid("not a cid"), "not a cid");
    }

    #[test]
    fn falls_back_to_the_cid_inside_vbmeta() {
        let directory = std::env::temp_dir().join("lmnflash-cid-test");
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();

        let steps = vec![FlashOp::Flash {
            partition: "vbmeta".to_owned(),
            file: "vbmeta.img".to_owned(),
            md5: None,
        }];

        // Without a `vbmeta` there is nothing to read.
        assert_eq!(cid_from_vbmeta(&directory, &steps), None);

        // A real image stores the marker, a NUL and `<codename>_<cid in
        // decimal>` — `reference/vbmeta.img` contains `HAB_META\0arcfox_50`.
        let mut image = vec![0x00u8; 512];
        image.extend_from_slice(b"HAB_META\0arcfox_50\0");
        image.extend_from_slice(&[0xFFu8; 64]);
        std::fs::write(directory.join("vbmeta.img"), &image).unwrap();

        assert_eq!(cid_from_vbmeta(&directory, &steps).as_deref(), Some("0x0032"));

        // The spelling documented in `M_cid.md` (no separator) works as well,
        // and a marker without a code is skipped rather than guessed at.
        assert_eq!(
            cid_after_marker(b"HAB_METAeqs_50\0").as_deref(),
            Some("0x0032")
        );
        assert_eq!(cid_after_marker(b"HAB_META\0nomarker\0"), None);
        assert_eq!(cid_after_marker(b"HAB_META\0arcfox\0"), None);

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn byte_order_marks_are_ignored() {
        let (_, steps) = parse(&format!("\u{feff}{SAMPLE}")).unwrap();

        assert_eq!(steps.len(), 4);
    }

    #[test]
    fn utf16_flashfiles_are_read() {
        let path = std::env::temp_dir().join("lmnflash-flashfile-utf16-test.xml");

        let mut bytes = vec![0xFF, 0xFE]; // UTF-16 LE byte order mark
        for unit in SAMPLE.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        std::fs::write(&path, &bytes).unwrap();

        let text = read_flashfile(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        assert_eq!(parse(&text).unwrap().1.len(), 4);
    }

    #[test]
    fn describe_mentions_partition_and_file() {
        assert_eq!(
            FlashOp::Flash {
                partition: "boot".to_owned(),
                file: "boot.img".to_owned(),
                md5: None,
            }
            .describe(),
            "flash boot (boot.img)"
        );
        assert_eq!(
            FlashOp::Oem {
                var: "config bootmode fastboot".to_owned(),
            }
            .describe(),
            "oem config bootmode fastboot"
        );
    }
}
