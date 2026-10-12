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

/// Partitions flashed with the bootloader part on a MediaTek device only.
///
/// MediaTek's bootloader chain (`do_flash_bl_mtk.cmd`) flashes `dtbo`
/// alongside `lk`/`tee`/…, while Qualcomm packages put it in the AP part
/// (which is why `dtbo` is not part of [`BL_PARTITIONS`]).
const MTK_BL_PARTITIONS: &[&str] = &["dtbo"];

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
    ///
    /// `mediatek` says whether the package is for a MediaTek device: there
    /// `dtbo` is flashed as part of the bootloader (see [`MTK_BL_PARTITIONS`]).
    pub fn part(&self, mediatek: bool) -> Option<FlashPart> {
        // Only a flashed image belongs to a part.
        if !matches!(self, Self::Flash { .. }) {
            return None;
        }

        let partition = self.partition()?;

        if BP_PARTITIONS.contains(&partition.as_str()) {
            Some(FlashPart::Bp)
        } else if BL_PARTITIONS.contains(&partition.as_str())
            || (mediatek && MTK_BL_PARTITIONS.contains(&partition.as_str()))
        {
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
    /// The project code of the package, read from the `vbmeta` image it
    /// carries: the codename in front of the CID, without the `_<cid>` that
    /// follows it (`arcfox` of `arcfox_50`). `None` when the package has no
    /// `vbmeta` image — that is not an error, there is simply nothing to
    /// report.
    pub project_code: Option<String>,
    /// Partitions whose steps were dropped because a package must not write
    /// them (see `IGNORED_PARTITIONS`), spelled the way the flashfile does.
    pub ignored_partitions: Vec<String>,
    /// The `flash` / `erase` / `oem` / `getvar` steps, in document order.
    pub steps: Vec<FlashOp>,
}

impl FlashPackage {
    /// Whether the package is for a MediaTek device.
    ///
    /// The bootloader image names the chip vendor: a Qualcomm package flashes
    /// `bootloader.img`, a MediaTek one a `preloader`, and a Unisoc one
    /// `u-boot-spl-16k`. On MediaTek the `dtbo` image is part of the
    /// bootloader, so the part classification depends on this.
    pub fn is_mediatek(&self) -> bool {
        is_mediatek(&self.steps)
    }
}

/// Whether a step list belongs to a MediaTek package (see
/// [`FlashPackage::is_mediatek`]).
///
/// The steps on their own are enough to tell, so the Multi Flashing preset
/// list — which holds an identified package, not a loaded one — can classify
/// its procedures the same way the single-device dialog does.
pub fn is_mediatek(steps: &[FlashOp]) -> bool {
    steps.iter().any(|step| match step {
        FlashOp::Flash { partition, file, .. } => {
            normalize_partition(partition) == "preloader"
                || file.to_ascii_lowercase().contains("preloader")
        }
        _ => false,
    })
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

/// Whether `path` is a firmware ZIP archive rather than a `flashfile.xml`
/// sitting next to its image files.
///
/// The extension decides, which is what both dialogs' file pickers filter for
/// and what the Multi Flashing presets ask the user to unpack.
pub fn is_zip(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
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

/// Why a package could not be loaded.
///
/// Most of what the loader reports is technical text — from the ZIP reader,
/// the XML parser or the file system — which the dialog shows as it is. The
/// one message the user has to act on is a value instead, so the dialog can
/// translate it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// The ZIP archive holds no `flashfile.xml`, so it is not a firmware
    /// package at all.
    NotAPackage,
    /// Anything else the loader ran into.
    Other(String),
}

impl LoadError {
    /// The English wording, for the places that need a message but have no
    /// translation at hand.
    pub fn message(&self) -> &str {
        match self {
            Self::NotAPackage => {
                "The selected ZIP archive is not a valid firmware package (no flashfile.xml inside)"
            }
            Self::Other(message) => message,
        }
    }
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl From<String> for LoadError {
    fn from(message: String) -> Self {
        Self::Other(message)
    }
}

impl From<&str> for LoadError {
    fn from(message: &str) -> Self {
        Self::Other(message.to_owned())
    }
}

/// Loads a package from `path`: a ZIP archive or a `flashfile.xml`.
///
/// `on_progress` reports the ZIP extraction as `(bytes_written, total_bytes)`.
/// The result carries the package directory the image files are read from.
///
/// A ZIP is unpacked into [`work_directory`], which is cleared first: use
/// [`load_into`] to unpack it into a directory of the caller's own (a
/// directory that must not be wiped, such as the one a Multi Flashing preset
/// unpacks into).
pub fn load(
    path: &Path,
    on_progress: &mut dyn FnMut(u64, u64),
) -> Result<FlashPackage, LoadError> {
    // Everything inside the work directory is disposable, so what an earlier
    // run left there is dropped before this package is written.
    cleanup_work_directory();

    load_into(path, &work_directory(), on_progress)
}

/// Loads a package from `path`, unpacking a ZIP archive into `destination`.
///
/// A `flashfile.xml` (a package that is already unpacked next to its image
/// files) is read where it lies and ignores `destination`.
///
/// The archive is checked for a `flashfile.xml` before anything is written, so
/// a file that is not a firmware package never costs an extraction.
pub fn load_into(
    path: &Path,
    destination: &Path,
    on_progress: &mut dyn FnMut(u64, u64),
) -> Result<FlashPackage, LoadError> {
    if path.is_dir() {
        return Err("select the firmware ZIP or its flashfile.xml, not a directory".into());
    }

    if !path.is_file() {
        return Err(format!("{} does not exist", path.display()).into());
    }

    let (directory, flashfile) = if is_zip(path) {
        // A ZIP without a `flashfile.xml` is not a firmware package. Saying so
        // here means a file that was picked by mistake never gets unpacked
        // (a factory package is several gigabytes of images), and the dialog
        // reports it as invalid instead of failing halfway through.
        let Some(name) = zip_flashfile_name(path)? else {
            return Err(LoadError::NotAPackage);
        };

        std::fs::create_dir_all(destination)
            .map_err(|error| format!("could not create {}: {error}", destination.display()))?;

        extract_zip(path, destination, on_progress)?;

        // The file the archive itself brought, not whatever a search finds
        // first: a preset unpacks into a folder of the user's own, which may
        // already hold an older package that must not be read in its place.
        let flashfile = find_flashfile(destination)
            .filter(|found| found.ends_with(&name))
            .or_else(|| {
                let extracted = destination.join(&name);
                extracted.is_file().then_some(extracted)
            })
            .ok_or_else(|| "no flashfile.xml found in the selected ZIP archive".to_owned())?;

        let directory = flashfile
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| destination.to_path_buf());

        (directory, flashfile)
    } else {
        let directory = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));

        (directory, path.to_path_buf())
    };

    let xml = read_flashfile(&flashfile)?;
    let (header, steps) =
        parse(&xml).map_err(|error| format!("{}: {error}", flashfile.display()))?;

    // Steps a package must not run (they would write another unit's
    // calibration or carrier data) are dropped before anything can flash or
    // even show them.
    let (steps, ignored_partitions) = without_ignored_steps(steps);

    // The CID is declared by the flashfile, or embedded in the `vbmeta` image
    // when it does not declare one; the project code only ever comes from that
    // image, and a package without one simply has neither.
    let identifier = vbmeta_identifier(&directory, &steps);
    let project_code = identifier.as_deref().and_then(project_codename);
    let cid = header
        .cid
        .clone()
        .or_else(|| identifier.as_deref().and_then(cid_from_project_code));

    Ok(FlashPackage {
        directory,
        flashfile,
        model: header.model,
        software_version: header.software_version,
        cid,
        project_code,
        ignored_partitions,
        steps,
    })
}

/// What a package says about itself, read without unpacking its images.
///
/// [`load`] has to write every image of a package out before it can be read,
/// which costs gigabytes; the Multi Flashing preset list wants this for every
/// entry it holds, so [`identify`] reads only the files the answer lives in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackageIdentity {
    pub model: Option<String>,
    pub software_version: Option<String>,
    /// The CID the package is made for: the one its flashfile declares, or the
    /// one encoded in its `vbmeta` image.
    pub cid: Option<String>,
    /// The device codename the package is for (`arcfox` of `arcfox_50`).
    pub codename: Option<String>,
    /// The procedures the package runs, in document order, with the steps of
    /// ignored partitions already dropped — exactly what a checklist may
    /// offer (and what [`load`] would flash).
    pub steps: Vec<FlashOp>,
    /// The partitions those dropped steps would have written.
    pub ignored_partitions: Vec<String>,
}

/// Reads what a package says about itself without unpacking it.
///
/// A ZIP is only looked into — its `flashfile.xml` and the `vbmeta` image that
/// file names are read straight out of the archive — so this stays fast
/// whatever the size of the package. A file that is no package at all fails
/// exactly as [`load`] would.
pub fn identify(path: &Path) -> Result<PackageIdentity, LoadError> {
    if path.is_dir() {
        return Err("select the firmware ZIP or its flashfile.xml, not a directory".into());
    }

    if !path.is_file() {
        return Err(format!("{} does not exist", path.display()).into());
    }

    let is_zip = is_zip(path);

    // A `flashfile.xml` next to its images: the images are read from that
    // directory, exactly as `load` reads them after unpacking a ZIP.
    if !is_zip {
        let directory = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let xml = read_flashfile(path)?;
        let (header, steps) =
            parse(&xml).map_err(|error| format!("{}: {error}", path.display()))?;

        return Ok(identity_from(
            header,
            vbmeta_identifier(&directory, &steps),
            steps,
        ));
    }

    let file = std::fs::File::open(path)
        .map_err(|error| format!("could not open {}: {error}", path.display()))?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|error| format!("{} is not a readable ZIP archive: {error}", path.display()))?;

    // The same depth `find_flashfile` searches, so a package this refuses is
    // one `load` could not have read either.
    let bytes = zip_read_named(&mut zip, "flashfile.xml", Some(FINDFILE_DEPTH))?
        .ok_or(LoadError::NotAPackage)?;
    let xml = decode_flashfile(&bytes, path)?;
    let (header, steps) = parse(&xml).map_err(|error| format!("{}: {error}", path.display()))?;

    // A package whose `vbmeta` is missing (or carries no marker) simply has no
    // codename; that is not a failure.
    let identifier = vbmeta_name(&steps)
        .map(|name| zip_read_named(&mut zip, &name, None))
        .transpose()?
        .flatten()
        .and_then(|bytes| identifier_in_bytes(&bytes));

    Ok(identity_from(header, identifier, steps))
}

/// What the dialog shows, from a parsed header and the `vbmeta` identifier.
fn identity_from(
    header: FlashHeader,
    identifier: Option<String>,
    steps: Vec<FlashOp>,
) -> PackageIdentity {
    let cid = header
        .cid
        .or_else(|| identifier.as_deref().and_then(cid_from_project_code));

    // Steps a package must not run are dropped here as well, so the checklist
    // a preset offers is exactly what a run would do.
    let (steps, ignored_partitions) = without_ignored_steps(steps);

    PackageIdentity {
        model: header.model,
        software_version: header.software_version,
        cid,
        codename: identifier.as_deref().and_then(project_codename),
        steps,
        ignored_partitions,
    }
}

/// The project code a package reports: the codename in front of the CID
/// (`arcfox` of `arcfox_50`).
fn project_codename(identifier: &str) -> Option<String> {
    let (codename, _) = identifier.rsplit_once('_')?;

    (!codename.is_empty()).then(|| codename.to_owned())
}

/// The CID a project code encodes: the decimal number after the last `_`
/// (`arcfox_50` → `0x0032`).
fn cid_from_project_code(identifier: &str) -> Option<String> {
    let (_, value) = identifier.rsplit_once('_')?;

    value.parse::<u32>().ok().map(format_cid)
}

/// How much of a `vbmeta` image (or of any other file read out of a ZIP) is
/// taken. `vbmeta` is a few hundred KiB; the cap only keeps a mis-declared
/// `filename` from pulling in a huge file.
const MAX_SCAN: u64 = 8 << 20;

/// Reads the identifier out of the package's `vbmeta` image.
///
/// The image carries the code Motorola puts in `vbmeta` as plain text,
/// followed by the device codename and the CID in decimal — a real image has
/// `HAB_META\0arcfox_50` (`reference/M_cid.md` documents the same string
/// without the NUL: `HAB_METAeqs_50`).
///
/// A package without a `vbmeta` image (or with one that carries no marker) has
/// no identifier: that is not a failure, the dialog just has nothing to show.
fn vbmeta_identifier(directory: &Path, steps: &[FlashOp]) -> Option<String> {
    let path = vbmeta_file(directory, steps)?;

    let mut bytes = Vec::new();
    std::fs::File::open(&path)
        .ok()?
        .take(MAX_SCAN)
        .read_to_end(&mut bytes)
        .ok()?;

    identifier_in_bytes(&bytes)
}

/// Finds the `<codename>_<cid>` identifier a `vbmeta` image carries.
///
/// The marker can appear more than once (and the bare byte sequence can appear
/// by chance), so every occurrence is tried until one parses.
fn identifier_in_bytes(bytes: &[u8]) -> Option<String> {
    const MAGIC: &[u8] = b"HAB_META";

    let mut search_from = 0;

    while let Some(offset) = bytes[search_from..]
        .windows(MAGIC.len())
        .position(|window| window == MAGIC)
    {
        let position = search_from + offset;

        if let Some(identifier) = identifier_after_marker(&bytes[position + MAGIC.len()..]) {
            return Some(identifier);
        }

        search_from = position + MAGIC.len();
    }

    None
}

/// Reads the identifier that follows a `HAB_META` marker: `<codename>_<cid>`,
/// with the decimal code after the last `_` (`arcfox_50`). Anything else after
/// a marker — an image can contain the byte sequence by chance — is rejected.
fn identifier_after_marker(rest: &[u8]) -> Option<String> {
    // A NUL byte separates the marker from the codename in a real image; the
    // codename is then ended by the NUL that follows it.
    let identifier: String = rest
        .iter()
        .take(64)
        .skip_while(|byte| **byte == 0)
        .take_while(|byte| byte.is_ascii_alphanumeric() || **byte == b'_')
        .map(|byte| char::from(*byte))
        .collect();

    let (codename, value) = identifier.rsplit_once('_')?;

    if codename.is_empty() || value.parse::<u32>().is_err() {
        return None;
    }

    Some(identifier)
}

/// The `vbmeta` image file a package's steps name, if any.
fn vbmeta_name(steps: &[FlashOp]) -> Option<String> {
    steps.iter().find_map(|step| match step {
        FlashOp::Flash {
            partition, file, ..
        } if normalize_partition(partition) == "vbmeta" => Some(file.clone()),
        _ => None,
    })
}

/// The `vbmeta` image of the package: the file its `flash` step names, or a
/// plain `vbmeta.img` next to the flashfile.
fn vbmeta_file(directory: &Path, steps: &[FlashOp]) -> Option<PathBuf> {
    let named = vbmeta_name(steps).map(|name| directory.join(name));

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
///
/// The Multi Flashing conflict check asks the same question about the CIDs it
/// compares, which is why this is reachable outside the module.
pub(crate) fn cid_allows_any_region(cid: &str) -> bool {
    matches!(cid.to_ascii_uppercase().as_str(), "0X0000" | "0X00FF")
}

/// Whether a package and a device disagree about the CID in a way worth
/// warning about. Both sides are compared in their normalized form, and a
/// device with a super CID takes firmware of any region.
pub(crate) fn cid_mismatch(package: &str, device: &str) -> bool {
    !package.eq_ignore_ascii_case(device) && !cid_allows_any_region(device)
}

/// Whether a package and a device disagree about the project code, i.e. whether
/// they are made for different phones.
///
/// A missing code on either side is not a mismatch: there is simply nothing to
/// compare (a package without a `vbmeta` image, or a bootloader that does not
/// report `product`), and guessing would be worse than saying nothing.
pub(crate) fn project_code_mismatch(package: Option<&str>, device: Option<&str>) -> bool {
    let (Some(package), Some(device)) = (package, device) else {
        return false;
    };

    let package = package.trim();
    let device = device.trim();

    !package.is_empty() && !device.is_empty() && !package.eq_ignore_ascii_case(device)
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

    decode_flashfile(&bytes, path)
}

/// Decodes the bytes of a `flashfile.xml`, wherever they were read from (the
/// file itself, or the ZIP archive that holds it).
fn decode_flashfile(bytes: &[u8], path: &Path) -> Result<String, String> {
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

    String::from_utf8(bytes.to_vec())
        .map_err(|error| format!("{} is not valid UTF-8 text: {error}", path.display()))
}

/// How many directory levels a package's `flashfile.xml` may sit under, both
/// when it is looked for on disk and when it is looked for in a ZIP.
const FINDFILE_DEPTH: usize = 3;

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

    search(directory, FINDFILE_DEPTH)
}

/// Reads one file out of a ZIP archive by name, without unpacking the rest.
///
/// The name is matched against the entry's whole path and against its file
/// name, so a file the flashfile names (`images/vbmeta.img`) is found either
/// way. `max_parents` limits how many directories it may sit under: the
/// package's `flashfile.xml` keeps to [`FINDFILE_DEPTH`], the depth
/// [`find_flashfile`] searches; a file the flashfile itself names takes any
/// directory.
fn zip_read_named(
    zip: &mut zip::ZipArchive<std::fs::File>,
    name: &str,
    max_parents: Option<usize>,
) -> Result<Option<Vec<u8>>, String> {
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|error| format!("could not read entry #{index}: {error}"))?;

        // The name as the archive spells it, e.g. `XT2451/arcfox/flashfile.xml`.
        let path = entry.name().replace('\\', "/");
        let file_name = path.rsplit('/').next().unwrap_or(&path);
        let parents = path
            .split('/')
            .filter(|part| !part.is_empty())
            .count()
            .saturating_sub(1);

        let matches = (path.eq_ignore_ascii_case(name) || file_name.eq_ignore_ascii_case(name))
            && max_parents.is_none_or(|max| parents <= max);

        if !matches {
            continue;
        }

        let mut bytes = Vec::new();
        entry
            .by_ref()
            .take(MAX_SCAN)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("could not read {path}: {error}"))?;

        return Ok(Some(bytes));
    }

    Ok(None)
}

/// The name of the `flashfile.xml` a ZIP archive holds, at the same depth
/// [`find_flashfile`] searches; `None` when the archive holds none.
///
/// This runs before the archive is unpacked, so a file that is not a firmware
/// package is rejected without writing anything to disk. The name it returns
/// is also how [`load_into`] finds the file the archive brought.
fn zip_flashfile_name(archive: &Path) -> Result<Option<String>, String> {
    /// Directory levels [`find_flashfile`] searches; the two have to agree,
    /// or a package would pass this check only to fail that search.
    const DEPTH: usize = FINDFILE_DEPTH;

    let file = std::fs::File::open(archive)
        .map_err(|error| format!("could not open {}: {error}", archive.display()))?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|error| format!("{} is not a readable ZIP archive: {error}", archive.display()))?;

    for index in 0..zip.len() {
        // An entry that cannot be read is left to the extraction to complain
        // about; only the names matter here.
        let Ok(entry) = zip.by_index(index) else {
            continue;
        };

        // The name as the archive spells it, e.g. `XT2451/arcfox/flashfile.xml`.
        let name = entry.name().replace('\\', "/");
        let mut parts: Vec<&str> = name.split('/').filter(|part| !part.is_empty()).collect();

        let Some(file_name) = parts.pop() else {
            continue;
        };

        if parts.len() <= DEPTH && file_name.eq_ignore_ascii_case("flashfile.xml") {
            return Ok(Some(name));
        }
    }

    Ok(None)
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
        assert_eq!(flash("partition").part(false), Some(FlashPart::Bl));
        assert_eq!(flash("bootloader").part(false), Some(FlashPart::Bl));
        // The MediaTek bootloader chain.
        assert_eq!(flash("lk").part(false), Some(FlashPart::Bl));
        assert_eq!(flash("efuseBackup").part(false), Some(FlashPart::Bl));

        // Baseband.
        assert_eq!(flash("radio").part(false), Some(FlashPart::Bp));
        assert_eq!(flash("modem").part(false), Some(FlashPart::Bp));
        assert_eq!(flash("fsg").part(false), Some(FlashPart::Bp));
        assert_eq!(flash("md1img").part(false), Some(FlashPart::Bp));

        // Everything else is the Android OS part.
        assert_eq!(flash("super").part(false), Some(FlashPart::Ap));
        assert_eq!(flash("boot").part(false), Some(FlashPart::Ap));
        assert_eq!(flash("dtbo").part(false), Some(FlashPart::Ap));
        assert_eq!(flash("secdataBackup").part(false), Some(FlashPart::Ap));

        // `erase` and `oem` commands are an image of no part, so the part
        // buttons never touch them — not even when they name the partition of
        // a part (`erase preloader`).
        assert_eq!(erase("userdata").part(false), None);
        assert_eq!(erase("preloader").part(false), None);
        assert_eq!(
            FlashOp::Oem {
                var: "config bootmode fastboot".to_owned(),
            }
            .part(false),
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

        assert_eq!(flash("boot_a").part(false), Some(FlashPart::Ap));
        assert_eq!(flash("MODEM_B").part(false), Some(FlashPart::Bp));
        assert_eq!(flash(" Bootloader ").part(false), Some(FlashPart::Bl));
        assert_eq!(flash("vbmeta_system").part(false), Some(FlashPart::Ap));
        assert_eq!(flash("_b").part(false), Some(FlashPart::Ap));
    }

    #[test]
    fn mediatek_packages_flash_dtbo_with_the_bootloader() {
        let flash = |partition: &str| FlashOp::Flash {
            partition: partition.to_owned(),
            file: "image.img".to_owned(),
            md5: None,
        };

        // A Qualcomm or Unisoc package keeps `dtbo` in the AP part.
        assert_eq!(flash("dtbo").part(false), Some(FlashPart::Ap));

        // The MediaTek bootloader chain flashes it next to `lk`.
        assert_eq!(flash("dtbo").part(true), Some(FlashPart::Bl));
        assert_eq!(flash("lk").part(true), Some(FlashPart::Bl));

        // Everything else is classified the same on both.
        assert_eq!(flash("boot").part(true), Some(FlashPart::Ap));
        assert_eq!(flash("radio").part(true), Some(FlashPart::Bp));
    }

    #[test]
    fn a_preloader_image_marks_a_mediatek_package() {
        let flash = |partition: &str, file: &str| FlashOp::Flash {
            partition: partition.to_owned(),
            file: file.to_owned(),
            md5: None,
        };
        let package = |steps: Vec<FlashOp>| FlashPackage {
            directory: PathBuf::from("."),
            flashfile: PathBuf::from("flashfile.xml"),
            model: None,
            software_version: None,
            cid: None,
            project_code: None,
            ignored_partitions: Vec::new(),
            steps,
        };

        // Qualcomm names its bootloader image `bootloader.img`, Unisoc
        // `u-boot-spl-16k`.
        assert!(!package(vec![flash("bootloader", "bootloader.img")]).is_mediatek());
        assert!(!package(vec![flash("pre", "u-boot-spl-16k")]).is_mediatek());

        // A MediaTek package carries a `preloader` image.
        assert!(package(vec![flash("preloader", "preloader.img")]).is_mediatek());
        assert!(package(vec![flash("pre", "preloader.bin")]).is_mediatek());
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
    fn project_code_mismatch_needs_both_sides() {
        // Two phones of the same project agree, whatever the casing.
        assert!(!project_code_mismatch(Some("arcfox"), Some("arcfox")));
        assert!(!project_code_mismatch(Some("arcfox"), Some("ARCFOX")));
        assert!(!project_code_mismatch(Some(" arcfox "), Some("arcfox")));

        // Another project means another phone.
        assert!(project_code_mismatch(Some("arcfox"), Some("eqs")));

        // Nothing to compare is not a mismatch: a package without a `vbmeta`
        // image, or a bootloader that reports no `product`, is not a warning.
        assert!(!project_code_mismatch(None, Some("arcfox")));
        assert!(!project_code_mismatch(Some("arcfox"), None));
        assert!(!project_code_mismatch(None, None));
        assert!(!project_code_mismatch(Some(""), Some("arcfox")));
        assert!(!project_code_mismatch(Some("arcfox"), Some("   ")));
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
        assert_eq!(steps[0].part(false), None);
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
    fn reads_the_project_code_inside_vbmeta() {
        let directory = std::env::temp_dir().join("lmnflash-cid-test");
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();

        let steps = vec![FlashOp::Flash {
            partition: "vbmeta".to_owned(),
            file: "vbmeta.img".to_owned(),
            md5: None,
        }];

        // Without a `vbmeta` there is nothing to read.
        assert_eq!(vbmeta_identifier(&directory, &steps), None);

        // A real image stores the marker, a NUL and `<codename>_<cid in
        // decimal>` — `reference/vbmeta.img` contains `HAB_META\0arcfox_50`.
        let mut image = vec![0x00u8; 512];
        image.extend_from_slice(b"HAB_META\0arcfox_50\0");
        image.extend_from_slice(&[0xFFu8; 64]);
        std::fs::write(directory.join("vbmeta.img"), &image).unwrap();

        assert_eq!(
            vbmeta_identifier(&directory, &steps).as_deref(),
            Some("arcfox_50")
        );
        // The project code is the codename in front of the CID …
        assert_eq!(
            project_codename("arcfox_50").as_deref(),
            Some("arcfox")
        );
        assert_eq!(project_codename("arcfox"), None);
        // … and the CID is the decimal number in it.
        assert_eq!(cid_from_project_code("arcfox_50").as_deref(), Some("0x0032"));
        assert_eq!(cid_from_project_code("arcfox").as_deref(), None);
        assert_eq!(cid_from_project_code("arcfox_DEADX").as_deref(), None);

        // The spelling documented in `M_cid.md` (no separator between the
        // marker and the codename) works as well, and a marker without a code
        // is skipped rather than guessed at.
        assert_eq!(
            identifier_after_marker(b"eqs_50\0").as_deref(),
            Some("eqs_50")
        );
        assert_eq!(identifier_after_marker(b"\0nomarker\0"), None);
        assert_eq!(identifier_after_marker(b"\0arcfox\0"), None);
        assert_eq!(identifier_after_marker(b"\0_50\0"), None);

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

    /// A ZIP archive built in memory out of raw entries (a `vbmeta` image is
    /// not text).
    fn zip_archive_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::Write;

        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));

        for (name, contents) in entries {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(contents).unwrap();
        }

        writer.finish().unwrap().into_inner()
    }

    /// Writes such an archive into a file of its own.
    fn zip_file_bytes(name: &str, entries: &[(&str, &[u8])]) -> PathBuf {
        let path = std::env::temp_dir().join(format!("lmnflash-flashfile-{name}.zip"));
        std::fs::write(&path, zip_archive_bytes(entries)).unwrap();

        path
    }

    /// The same for an archive of text entries.
    fn zip_file(name: &str, entries: &[(&str, &str)]) -> PathBuf {
        zip_file_bytes(
            name,
            &entries
                .iter()
                .map(|(name, contents)| (*name, contents.as_bytes()))
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn a_zip_without_a_flashfile_is_not_a_package() {
        let path = zip_file(
            "no-flashfile",
            &[("gpt.bin", "binary"), ("boot.img", "binary")],
        );

        assert!(zip_flashfile_name(&path).unwrap().is_none());
        // The package is rejected before anything is unpacked, and with the
        // error the dialog knows to translate rather than print.
        assert!(matches!(
            load(&path, &mut |_, _| {}),
            Err(LoadError::NotAPackage)
        ));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_zip_with_a_flashfile_passes_the_pre_check() {
        let root = zip_file("flashfile-root", &[("flashfile.xml", "xml")]);
        assert_eq!(zip_flashfile_name(&root).unwrap().as_deref(), Some("flashfile.xml"));
        let _ = std::fs::remove_file(&root);

        // A factory package wraps everything in a per-model folder, and the
        // name the check reports is the one the loader reads afterwards.
        let nested = zip_file("flashfile-nested", &[("XT2451/arcfox/flashfile.xml", "xml")]);
        assert_eq!(
            zip_flashfile_name(&nested).unwrap().as_deref(),
            Some("XT2451/arcfox/flashfile.xml")
        );
        let _ = std::fs::remove_file(&nested);

        // The name is matched without regard to case, like `find_flashfile`.
        let upper = zip_file("flashfile-upper", &[("FLASHFILE.XML", "xml")]);
        assert!(zip_flashfile_name(&upper).unwrap().is_some());
        let _ = std::fs::remove_file(&upper);
    }

    #[test]
    fn a_flashfile_beyond_the_searched_depth_does_not_count() {
        // `find_flashfile` searches three directories down at most, and this
        // check has to agree with it: a package it cannot find would otherwise
        // be unpacked and then reported as having no flashfile after all.
        let found = zip_file("flashfile-at-depth", &[("a/b/c/flashfile.xml", "xml")]);
        assert!(zip_flashfile_name(&found).unwrap().is_some());
        let _ = std::fs::remove_file(&found);

        let too_deep = zip_file("flashfile-too-deep", &[("a/b/c/d/flashfile.xml", "xml")]);
        assert!(zip_flashfile_name(&too_deep).unwrap().is_none());
        let _ = std::fs::remove_file(&too_deep);
    }

    /// A package can be unpacked into a directory of the caller's own — a
    /// Multi Flashing preset unpacks into a folder the user picked — which is
    /// NOT cleared, and the package read afterwards is the one that was just
    /// unpacked even when the folder holds an older one.
    #[test]
    fn a_package_can_be_unpacked_into_a_chosen_directory() {
        let directory = std::env::temp_dir().join("lmnflash-load-into");
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();

        // What an earlier extraction (or the user) left behind: a flashfile a
        // search would find before the nested one of this package.
        std::fs::write(directory.join("flashfile.xml"), "<flashing/>").unwrap();
        std::fs::write(directory.join("keep.txt"), "keep").unwrap();

        let archive = zip_file(
            "load-into",
            &[
                (
                    "XT2451/arcfox/flashfile.xml",
                    r#"<flashing><header><phone_model model="arcfox"/></header><steps>
                       <step operation="flash" partition="boot" filename="boot.img"/>
                       </steps></flashing>"#,
                ),
                ("XT2451/arcfox/boot.img", "boot"),
            ],
        );

        let package = load_into(&archive, &directory, &mut |_, _| {}).unwrap();

        // The file the archive brought is the one that was read …
        assert_eq!(package.directory, directory.join("XT2451/arcfox"));
        assert_eq!(package.steps.len(), 1);
        assert_eq!(package.model.as_deref(), Some("arcfox"));
        // … and nothing of the user's own folder was touched.
        assert!(directory.join("keep.txt").is_file());
        assert!(directory.join("flashfile.xml").is_file());

        let _ = std::fs::remove_file(&archive);
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// A `vbmeta` image carrying the marker the way a real one does
    /// (`HAB_META\0arcfox_50\0`, see `reference/M_cid.md`).
    fn vbmeta_image(code: &str) -> Vec<u8> {
        let mut bytes = b"HAB_META\0".to_vec();
        bytes.extend_from_slice(code.as_bytes());
        bytes.push(0);
        // A real image is a few hundred KiB of padding; only the marker reads.
        bytes.resize(4096, 0);

        bytes
    }

    /// A flashfile whose steps name a `vbmeta` image, so the identifier can be
    /// read out of it.
    const SAMPLE_WITH_VBMETA: &str = r#"<flashing>
  <header>
    <phone_model model="SM8650_ARCFOX_IFLASH"/>
    <software_version version="arcfox_factory-userdebug 14 U3UX34.30-F.15"/>
  </header>
  <steps>
    <step operation="flash" partition="vbmeta" filename="vbmeta.img"/>
    <step operation="erase" partition="userdata"/>
  </steps>
</flashing>"#;

    #[test]
    fn a_zip_package_is_identified_without_unpacking_it() {
        // Only the flashfile and the `vbmeta` image are read: the images
        // themselves are not even in the archive.
        let path = zip_file_bytes(
            "identify",
            &[
                ("arcfox/flashfile.xml", SAMPLE_WITH_VBMETA.as_bytes()),
                ("arcfox/vbmeta.img", vbmeta_image("arcfox_50").as_slice()),
            ],
        );

        let identity = identify(&path).unwrap();

        assert_eq!(identity.model.as_deref(), Some("SM8650_ARCFOX_IFLASH"));
        assert_eq!(identity.codename.as_deref(), Some("arcfox"));
        assert_eq!(identity.cid.as_deref(), Some("0x0032"));
        // The steps come back too, so a preset can offer the checklist.
        assert_eq!(identity.steps.len(), 2);
        assert_eq!(identity.steps[1].describe(), "erase userdata");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_unpacked_package_is_identified_too() {
        let directory = std::env::temp_dir().join("lmnflash-flashfile-identify");
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();

        std::fs::write(directory.join("flashfile.xml"), SAMPLE_WITH_VBMETA).unwrap();
        std::fs::write(directory.join("vbmeta.img"), vbmeta_image("arcfox_50")).unwrap();

        let identity = identify(&directory.join("flashfile.xml")).unwrap();

        assert_eq!(identity.codename.as_deref(), Some("arcfox"));
        assert_eq!(identity.cid.as_deref(), Some("0x0032"));

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn identifying_a_zip_without_a_flashfile_fails() {
        let path = zip_file("identify-no-flashfile", &[("gpt.bin", "binary")]);

        assert!(matches!(identify(&path), Err(LoadError::NotAPackage)));

        let _ = std::fs::remove_file(&path);
    }
}
