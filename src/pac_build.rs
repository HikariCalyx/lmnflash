//! Building a PAC package from a flash configuration or a scatter XML.
//!
//! The Unisoc tools describe a firmware either with the `flash.cfg` they leave
//! next to a scattered firmware (a `[Setting]` block with the package's own
//! header values and a `[FlashParam]` block listing every record), or with a
//! scatter XML (which lists the records but not the files). Both are read here
//! into one plan: what the package's header will say, which records it will
//! hold, and which file belongs to each of them.
//!
//! A `flash.cfg` names every file itself. A scatter XML does not — the tools
//! match a record's id to a file by a table of names, so the names this tool
//! knows are tried first (the ids of the packages it was built against), then
//! a `flash.cfg` sitting next to the XML (which is authoritative for that
//! firmware), and whatever is left over has to be picked by hand.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::pac::{self, Payload, Record};
use crate::scatter;

/// Version of the header structure a written package carries. Both reference
/// packages store this value.
const HEADER_VERSION: &str = "BP_R2.0.1";

/// How many block addresses a record can hold.
///
/// A product that writes more than one block takes its addresses from the
/// scatter XML, which is why a plan keeps at most this many per record.
const MAX_ADDRESSES: usize = 5;

/// The file names known for a record id, in the order they are tried.
///
/// Taken from the packages this tool was built against: an id like `BOOT` is
/// always `boot.img` or one of its siblings, while a product-specific id such
/// as `Modem_NR` names a file only that product knows.
const KNOWN_FILES: &[(&str, &[&str])] = &[
    // The loader chain.
    ("FDL", &["fdl1-sign.bin"]),
    ("FDL2", &["fdl2-sign.bin", "lk-fdl2-sign.bin"]),
    ("UBOOTLoader", &["lk-sign.bin", "u-boot-sign.bin"]),
    (
        "SPLLoader",
        &["u-boot-spl-16k-sign.bin", "u-boot-spl-16k-emmc-sign.bin"],
    ),
    ("SPLLoaderEMMC", &["u-boot-spl-16k-emmc-sign.bin"]),
    ("SPLLoaderUFS", &["u-boot-spl-16k-ufs-sign.bin"]),
    ("SML", &["sml-sign.bin"]),
    ("Teecfg", &["teecfg-sign.bin"]),
    ("Trustos", &["tos-sign.bin"]),
    // The Android images.
    ("BOOT", &["boot.img", "boot-gki.img", "boot-sign.img"]),
    ("VBMETA", &["vbmeta-sign.img", "vbmeta.img"]),
    ("VBMETA_SYSTEM", &["vbmeta_system.img"]),
    ("VBMETA_SYSTEM_EXT", &["vbmeta_system_ext.img"]),
    ("VBMETA_VENDOR", &["vbmeta_vendor.img"]),
    ("VBMETA_ODM", &["vbmeta_odm.img"]),
    ("VBMETA_PRODUCT", &["vbmeta_product.img"]),
    ("INIT_BOOT", &["init_boot.img"]),
    ("VENDOR_BOOT", &["vendor_boot.img"]),
    ("DTB", &["dtb.img"]),
    ("DTBO", &["dtbo.img"]),
    ("Recovery", &["recovery.img"]),
    ("Cache", &["cache.img"]),
    ("UserData", &["userdata.img"]),
    ("Persist", &["persist.img"]),
    ("System", &["system.img.ext4", "system.img"]),
    ("Vendor", &["vendor.img.ext4", "vendor.img"]),
    ("Product", &["product.img.ext4", "product.img"]),
    ("Super", &["super.img"]),
    // Both reference packages give these two the logo images of their own
    // product: `logo.bin` for the boot logo, and the fastboot logo beside it.
    ("BootLogo", &["logo.bin", "fbootlogo.bin"]),
    (
        "Fastboot_Logo",
        &[
            "logo.bin(1)",
            "LOGO.bmp",
            "logo.bin",
            "fbootlogo.bin",
            "hmd_hd_720_1600.bmp",
            "unisoc_FHD_1080_1920.bmp",
            "unisoc_FHD_1080_2400.bmp",
        ],
    ),
    ("ProdNV", &["prodnv.img"]),
    ("Flag", &["dummy_flag.bin"]),
    ("Tpf", &["tpf.bin"]),
    ("BlackBox", &["blackbox.img"]),
    ("Elabel", &["elabel.img", "elable.img"]),
    ("DSP_LTE_CDMA", &["sharkl5pro_pubcp_CDMA_DSP.bin", "l_cdsp.img"]),
    ("GPS_BD", &["gnssbdmodem.bin"]),
    ("GPS_GL", &["gnssmodem.bin"]),
    ("Socko", &["socko.img"]),
    ("Odmko", &["odmko.img"]),
    ("VBMETA_CUSTOM1", &["vbmeta_custom1.img"]),
    // The records whose file name differs from product to product: an id says
    // nothing about what its file is called, so every name the reference
    // packages give it is listed, the name a packaging step produces first. A
    // name no folder holds is simply not found, which is why listing several
    // is safe.
    //
    // The names come from the packages this tool was built against: the
    // scattered firmwares of the reference tree, each with a `flash.cfg` of its
    // own, and the Unisoc build system's own packing configuration (`pac.ini`),
    // which packs from a build tree and so names the modem images by their
    // place in it (`l_modem.img`) as well as by the names a packaging step
    // gives them.
    //
    // Two of those firmwares are NR-era products: their modem, PHY and channel
    // records are named after the chipset they were built for, with the
    // spelling of each product's own build (one carries `_SECU_`, the other
    // does not).
    (
        "Modem_NR",
        &[
            "SC9600_QogirN6Pro_PS_SECU_Customer_modem.bin",
            "SC9600_QogirN6Pro_PS_modem.bin",
        ],
    ),
    (
        "Modem_NR_PHY",
        &[
            "SC9600_QogirN6Pro_PHY_SECU_modem.bin",
            "SC9600_QogirN6Pro_PHY_modem.bin",
        ],
    ),
    (
        "Modem_NR_DELTANV",
        &[
            "QogirN6Pro_PS_SECU_Customer_deltanv.bin",
            "QogirN6Pro_PS_Customer_deltanv.bin",
        ],
    ),
    (
        "NV_NR",
        &[
            "QogirN6Pro_PS_SECU_Customer_nvitem.bin",
            "QogirN6Pro_PS_Customer_nvitem.bin",
        ],
    ),
    ("CH_SYS", &["QogirN6Pro_CH.bin"]),
    ("ELABLE", &["elable.img", "elabel.img"]),
    (
        "NV_LTE",
        &[
            "sharkl5pro_pubcp_nvitem.bin",
            "sharkl3_pubcp_customer_nvitem.bin",
            "l_fixnv1.img",
            "l_fixnv2.img",
            "qogirl6_pubcp_exLNA_customer_nvitem.bin",
            "qogirl6_pubcp_customer_nvitem.bin",
        ],
    ),
    (
        "Modem_LTE",
        &[
            "SC9600_sharkl5pro_pubcp_modem.dat",
            "SC9600_sharkl3_pubcp_modem.dat",
            "l_modem.img",
            "SC9600_sharkl3_pubcp_modem.bin",
            "SC9600_qogirl6_pubcp_modem.bin",
        ],
    ),
    (
        "Modem_LTE_DELTANV",
        &[
            "sharkl5pro_pubcp_deltanv.bin",
            "sharkl3_pubcp_customer_deltanv.bin",
            "l_deltanv.img",
            "qogirl6_pubcp_exLNA_customer_deltanv.bin",
            "qogirl6_pubcp_customer_deltanv.bin",
        ],
    ),
    (
        "DSP_LTE_LTE",
        &[
            "sharkl5pro_pubcp_LTEA_DSP.bin",
            "sharkl3_pubcp_LTEA_DSP.bin",
            "l_ldsp.img",
            "qogirl6_pubcp_LTEA_DSP.bin",
        ],
    ),
    (
        "DSP_LTE_GGE",
        &[
            "sharkl5pro_pubcp_DM_DSP.bin",
            "sharkl3_pubcp_DM_DSP.bin",
            "l_gdsp.img",
            "qogirl6_pubcp_DM_DSP.bin",
        ],
    ),
    (
        "DSP_LTE_AG",
        &[
            "sharkl5pro_pubcp_AGCP_DSP.bin",
            "QogirN6Pro_AUDCP_DSP_lit_dm.bin",
            "l_agdsp.img",
            "QogirL6_AUDCP_DSP_lit_dm.bin",
        ],
    ),
    (
        "DFS",
        &[
            "sharkl5pro_cm4.bin",
            "sharkl3_cm4.bin",
            "QogirN6Pro_SP.bin",
            "pm_sys.img",
            "qogirl6_cm4.bin",
        ],
    ),
    (
        "Modem_WCN",
        &[
            "EXEC_KERNEL_IMAGE.bin",
            "PM_sharkl3_cm4_v2.bin",
            "wcnmodem.img",
            "wcnmodem.bin",
            "9863_integration_cm4_v2.bin",
        ],
    ),
    ("Modem_GNSS", &["gnssmodem.bin", "gnssmodem.img"]),
];

/// The candidate names for a record id.
fn known_files(id: &str) -> &'static [&'static str] {
    KNOWN_FILES
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(id))
        .map(|(_, files)| *files)
        .unwrap_or(&[])
}

/// Which file a record will take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// Found next to the source, by the source's own table or by a known name.
    Found(PathBuf),
    /// The user picked it.
    Chosen(PathBuf),
    /// Nothing was found for it yet.
    Missing,
}

impl Choice {
    /// The file this choice refers to.
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::Found(path) | Self::Chosen(path) => Some(path),
            Self::Missing => None,
        }
    }

    /// Whether a file is set.
    pub fn is_resolved(&self) -> bool {
        self.path().is_some()
    }
}

/// One record of the package to build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The record's id.
    pub id: String,
    /// The id the source displays, which a product may spell differently.
    pub alias: String,
    /// `nFileFlag` the source asks for: 0 = operation, 1 = file, 2 = the
    /// package's configuration XML.
    pub kind: u32,
    /// `nCheckFlag`.
    pub check: u32,
    /// `dwCanOmitFlag`.
    pub omit: u32,
    /// The block base addresses the record writes to.
    pub addresses: Vec<u32>,
    /// The file for this record.
    pub choice: Choice,
    /// How large that file is, once something resolved it.
    pub size: Option<u64>,
    /// The partition the record is written to. Only a scatter XML knows it
    /// (from the record's `<Block id="…">`), which is why a plan built from a
    /// `flash.cfg` takes it from the XML that configuration names.
    pub partition: Option<String>,
    /// Whether the record goes into the package at all.
    pub include: bool,
}

impl Entry {
    /// Points the record at a file the user picked, and remembers its size.
    pub fn choose(&mut self, path: PathBuf) {
        self.size = file_size(&path);
        self.choice = Choice::Chosen(path);
    }

    /// Whether this record needs a file, and has none.
    pub fn is_missing(&self) -> bool {
        self.include && self.needs_file() && !self.choice.is_resolved()
    }

    /// Whether the record carries a payload at all.
    pub fn needs_file(&self) -> bool {
        self.kind != 0
    }

    /// Whether this record is the package's configuration XML: the source
    /// stored as a record of its own (with no id, and last). The tools read a
    /// package back through it, so it is always written — the user never gets
    /// to leave it out, and the dialog gives it no row of its own.
    pub fn is_configuration(&self) -> bool {
        self.kind == 2
    }
}

/// Which kind of file a plan was read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// A `flash.cfg`.
    Config,
    /// A scatter XML.
    Scatter,
}

/// Everything needed to write a package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The file the plan was read from.
    pub source: PathBuf,
    pub kind: SourceKind,
    /// The header the package will carry.
    pub header: pac::Header,
    /// The records, in the order the source lists them.
    pub entries: Vec<Entry>,
}

impl Plan {
    /// The records that need a file and do not have one.
    pub fn missing(&self) -> usize {
        self.entries.iter().filter(|entry| entry.is_missing()).count()
    }

    /// How many records carry a file.
    pub fn with_file(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.needs_file() && entry.include)
            .count()
    }

    /// How many records go into the package, the configuration XML included:
    /// it is written whatever the user does.
    pub fn included(&self) -> usize {
        self.entries.iter().filter(|entry| entry.include).count()
    }

    /// How many records the user puts in, which is what the table counts: the
    /// configuration XML is not one of them.
    pub fn chosen(&self) -> usize {
        self.included() - self.configuration_records()
    }

    /// How many of those records carry a file.
    pub fn chosen_with_file(&self) -> usize {
        self.with_file() - self.configuration_records()
    }

    /// The configuration records of the plan, which are written whatever the
    /// user does. A source becomes exactly one: the XML it is read from, stored
    /// as a record of its own.
    fn configuration_records(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.is_configuration() && entry.include)
            .count()
    }

    /// Takes the record at `index` into the package, or leaves it out. The
    /// configuration XML ignores the request: a package without it is not a
    /// package the tools can read back.
    pub fn set_included(&mut self, index: usize, included: bool) {
        if let Some(entry) = self.entries.get_mut(index) {
            if !entry.is_configuration() {
                entry.include = included;
            }
        }
    }

    /// Whether the source left something for the user to fill in: a scatter
    /// XML carries neither the product version nor the alias.
    pub fn needs_metadata(&self) -> bool {
        self.header.product_alias.trim().is_empty() || self.header.product_version.trim().is_empty()
    }

    /// A name to suggest for the package file.
    pub fn suggested_name(&self) -> String {
        let product = self.header.product_alias.trim();
        let version = self.header.product_version.trim();

        let name = match (product.is_empty(), version.is_empty()) {
            (false, false) => format!("{product}_{version}"),
            (false, true) => product.to_string(),
            (true, false) => version.to_string(),
            (true, true) => self
                .source
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_else(|| "package".to_string()),
        };

        let name: String = name
            .chars()
            .map(|letter| {
                if letter.is_ascii_alphanumeric() || "._-".contains(letter) {
                    letter
                } else {
                    '_'
                }
            })
            .collect();

        format!("{name}.pac")
    }

    /// The records to write, with the file names their payloads carry.
    pub fn payloads(&self) -> Vec<Payload> {
        self.entries
            .iter()
            .filter(|entry| entry.include)
            .map(|entry| {
                let source = entry.choice.path().map(Path::to_path_buf);
                let name = source
                    .as_deref()
                    .and_then(|path| path.file_name())
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();

                Payload {
                    record: Record {
                        id: entry.id.clone(),
                        name,
                        flag: entry.check,
                        // A record without a file is a pure operation.
                        kind: if source.is_some() { entry.kind } else { 0 },
                        omit: entry.omit,
                        addresses: entry.addresses.clone(),
                    },
                    source,
                }
            })
            .collect()
    }
}

/// Why a package could not be planned or built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildError {
    /// The source could not be read.
    Read(String),
    /// The file is neither a flash configuration nor a scatter XML.
    NotASource,
    /// The record pieces do not add up to a package.
    Pac(pac::PacError),
    /// A record still has no file.
    MissingFile(String),
    /// The product alias or version is missing.
    MissingMetadata,
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read(error) => write!(f, "failed to read the source: {error}"),
            Self::NotASource => write!(f, "not a flash configuration or a scatter XML"),
            Self::Pac(error) => write!(f, "{error}"),
            Self::MissingFile(id) => write!(f, "the record `{id}` has no file"),
            Self::MissingMetadata => write!(f, "the product alias and version are missing"),
        }
    }
}

impl std::error::Error for BuildError {}

impl From<pac::PacError> for BuildError {
    fn from(error: pac::PacError) -> Self {
        Self::Pac(error)
    }
}

/// A `[Section]`/`key=value` file, which is what a flash configuration is.
struct Ini(Vec<(String, Vec<(String, String)>)>);

impl Ini {
    fn parse(text: &str) -> Self {
        let mut sections: Vec<(String, Vec<(String, String)>)> = Vec::new();

        for line in text.lines() {
            let line = line.trim();

            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }

            if let Some(name) = line.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
                sections.push((name.trim().to_string(), Vec::new()));
                continue;
            }

            if let Some((key, value)) = line.split_once('=') {
                // A key before any section belongs to a nameless one.
                if sections.is_empty() {
                    sections.push((String::new(), Vec::new()));
                }

                sections
                    .last_mut()
                    .expect("a section was just ensured")
                    .1
                    .push((key.trim().to_string(), value.trim().to_string()));
            }
        }

        Self(sections)
    }

    /// The keys of a section, in file order.
    fn section(&self, name: &str) -> &[(String, String)] {
        self.0
            .iter()
            .find(|(section, _)| section.eq_ignore_ascii_case(name))
            .map(|(_, keys)| keys.as_slice())
            .unwrap_or(&[])
    }

    /// One key of a section.
    fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.section(section)
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value.as_str())
    }
}

/// A number from a configuration value, or zero.
fn number(text: &str) -> u32 {
    text.trim().parse().unwrap_or(0)
}

/// The file name of a record's value: `<flag>@<file>`.
fn flash_parameter(value: &str) -> (u32, String) {
    match value.split_once('@') {
        Some((flag, file)) => (number(flag), file.trim().to_string()),
        None => (0, value.trim().to_string()),
    }
}

/// Whether a path holds a file with this name.
fn file_in(folder: &Path, name: &str) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }

    let path = folder.join(name);

    path.is_file().then_some(path)
}

/// How large a file is, when it can be read.
fn file_size(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|data| data.len())
}

/// How large the file a choice holds is.
fn choice_size(choice: &Choice) -> Option<u64> {
    choice.path().and_then(file_size)
}

/// The file a record takes, looked for by its id.
fn resolve(
    id: &str,
    folder: &Path,
    from_config: &HashMap<String, String>,
) -> Choice {
    for candidate in known_files(id) {
        if let Some(path) = file_in(folder, candidate) {
            return Choice::Found(path);
        }
    }

    // A flash configuration next to a scatter XML names the files the tool
    // matches to the ids, which beats guessing.
    if let Some(name) = from_config.get(id) {
        if let Some(path) = file_in(folder, name) {
            return Choice::Found(path);
        }
    }

    Choice::Missing
}

/// The names a flash configuration gives the ids, for resolving a scatter XML
/// that has one beside it.
fn names_beside(folder: &Path) -> HashMap<String, String> {
    let mut names = HashMap::new();

    let Ok(text) = std::fs::read_to_string(folder.join("flash.cfg")) else {
        return names;
    };

    for (key, value) in Ini::parse(&text).section("FlashParam") {
        let (_, file) = flash_parameter(value);

        if !file.is_empty() {
            names.insert(key.clone(), file);
        }
    }

    names
}

/// The partition each id is written to, which only the scatter XML a flash
/// configuration names knows: a `flash.cfg` lists file names, not blocks.
fn partitions_beside(folder: &Path, confile: &str) -> HashMap<String, String> {
    let mut partitions = HashMap::new();

    let Ok(text) = std::fs::read_to_string(folder.join(confile)) else {
        return partitions;
    };

    let Ok(scatter) = scatter::parse(&text) else {
        return partitions;
    };

    for file in scatter.files {
        if let Some(partition) = file.partition {
            partitions.insert(file.id.to_lowercase(), partition);
        }
    }

    partitions
}

/// Reads a source file into a plan.
pub fn plan(source: &Path) -> Result<Plan, BuildError> {
    let text =
        std::fs::read_to_string(source).map_err(|error| BuildError::Read(error.to_string()))?;

    // The extension is a hint at best, so the content decides.
    if text.contains("<BMAConfig") {
        plan_from_scatter(source, &text)
    } else if text.contains("[Setting]") || text.contains("[FlashParam]") {
        plan_from_config(source, &text)
    } else {
        Err(BuildError::NotASource)
    }
}

fn plan_from_config(source: &Path, text: &str) -> Result<Plan, BuildError> {
    let ini = Ini::parse(text);

    let folder = source.parent().unwrap_or(Path::new("."));
    let setting = |key: &str| ini.get("Setting", key).unwrap_or_default().to_string();
    let switch = |key: &str| number(ini.get("Setting", key).unwrap_or("0"));

    let confile = setting("PAC_CONFILE");

    // A flash configuration says nothing about partitions; the XML it names
    // does.
    let partitions = partitions_beside(folder, &confile);

    let mut entries = Vec::new();
    let mut config_taken = false;

    for (id, value) in ini.section("FlashParam") {
        let (check, name) = flash_parameter(value);
        let choice = match file_in(folder, &name) {
            Some(path) => Choice::Found(path),
            None => Choice::Missing,
        };

        // The record that carries the configuration XML is the one the tools
        // mark as such.
        let kind = if !name.is_empty() && name.eq_ignore_ascii_case(&confile) {
            config_taken = true;
            2
        } else if name.is_empty() {
            0
        } else {
            1
        };

        entries.push(Entry {
            size: choice_size(&choice),
            alias: id.clone(),
            partition: partitions.get(&id.to_lowercase()).cloned(),
            id: id.clone(),
            kind,
            check,
            omit: 0,
            addresses: Vec::new(),
            choice,
            include: true,
        });
    }

    // A flash configuration does not list the configuration XML itself: the
    // package stores it as a record of its own, with no id.
    if !config_taken {
        if let Some(path) = file_in(folder, &confile) {
            entries.push(Entry {
                id: String::new(),
                alias: String::new(),
                kind: 2,
                check: 1,
                omit: 0,
                addresses: Vec::new(),
                size: file_size(&path),
                partition: None,
                choice: Choice::Found(path),
                include: true,
            });
        }
    }

    let header = pac::Header {
        version: setting("PacVer"),
        product_name: setting("PAC_PRODUCT"),
        product_version: setting("ProductVer"),
        product_alias: setting("ProductAlias"),
        mode: switch("Mode"),
        flash_type: switch("FlashType"),
        nand_strategy: switch("NandStrategy"),
        nand_page_type: switch("NandPageType"),
        nv_backup: switch("NvBackup"),
        oma_dm_product_flag: switch("OmaDmProductFlag"),
        oma_dm: switch("OmaDM"),
        is_preload: switch("IsPreload"),
    };

    Ok(Plan {
        source: source.to_path_buf(),
        kind: SourceKind::Config,
        header,
        entries,
    })
}

fn plan_from_scatter(source: &Path, text: &str) -> Result<Plan, BuildError> {
    let scatter = scatter::parse(text).map_err(|error| BuildError::Read(error.to_string()))?;

    let folder = source.parent().unwrap_or(Path::new("."));
    let names = names_beside(folder);

    let mut entries: Vec<Entry> = scatter
        .files
        .iter()
        .map(|file| {
            let choice = if file.kind == 0 {
                Choice::Missing
            } else {
                resolve(&file.id, folder, &names)
            };

            Entry {
                id: file.id.clone(),
                alias: file.alias.clone(),
                partition: file.partition.clone(),
                kind: file.kind,
                // A package built from a product XML selects everything.
                check: 1,
                // "Not checked in the pack" is what the record stores as its omit
                // flag.
                omit: u32::from(file.check_flag == 2),
                addresses: file.addresses.iter().copied().take(MAX_ADDRESSES).collect(),
                size: choice_size(&choice),
                choice,
                include: true,
            }
        })
        .collect();

    // The XML itself becomes the package's configuration record, which is how
    // a real package carries it (with no id, and last).
    entries.push(Entry {
        id: String::new(),
        alias: String::new(),
        kind: 2,
        check: 1,
        omit: 0,
        addresses: Vec::new(),
        size: file_size(source),
        partition: None,
        choice: Choice::Found(source.to_path_buf()),
        include: true,
    });

    let header = pac::Header {
        version: HEADER_VERSION.to_string(),
        product_name: scatter.product.clone(),
        // A product XML carries neither of these: the user has to say.
        product_version: String::new(),
        product_alias: String::new(),
        mode: scatter.mode,
        flash_type: scatter.flash_type,
        nand_strategy: 0,
        nand_page_type: 0,
        nv_backup: scatter.nv_backup,
        oma_dm_product_flag: 0,
        oma_dm: 0,
        is_preload: 0,
    };

    Ok(Plan {
        source: source.to_path_buf(),
        kind: SourceKind::Scatter,
        header,
        entries,
    })
}

/// Writes the package a plan describes.
pub fn build(
    plan: &Plan,
    destination: &Path,
    emit: &dyn Fn(pac::PacEvent),
) -> Result<pac::Summary, BuildError> {
    if plan.needs_metadata() {
        return Err(BuildError::MissingMetadata);
    }

    if let Some(entry) = plan.entries.iter().find(|entry| entry.is_missing()) {
        return Err(BuildError::MissingFile(entry.id.clone()));
    }

    let summary = pac::write(&plan.header, &plan.payloads(), destination, emit)?;

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_directory(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "lmnflash-pac-build-test-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("create temp dir");
        directory
    }

    fn write(directory: &Path, name: &str, contents: &str) -> PathBuf {
        let path = directory.join(name);
        std::fs::write(&path, contents).expect("write the file");
        path
    }

    const CONFIG: &str = "\
[Setting]
PacVer=BP_R2.0.1
PAC_PRODUCT=TEST
ProductAlias=TEST
ProductVer=V1.0.0
Mode=0
FlashType=1
NandStrategy=0
NandPageType=0
NvBackup=1
OmaDmProductFlag=0
OmaDM=1
IsPreload=1
PAC_CONFILE=TEST.xml

[FlashParam]
FDL=1@fdl1-sign.bin
EraseSPL=1@
BOOT=1@boot.img
";

    #[test]
    fn plans_from_a_flash_configuration() {
        let directory = temp_directory("config-plan");
        write(&directory, "fdl1-sign.bin", "one");
        write(&directory, "boot.img", "seven!!");
        write(&directory, "TEST.xml", "<BMAConfig/>");

        let source = write(&directory, "flash.cfg", CONFIG);
        let plan = plan(&source).expect("plan");

        assert_eq!(plan.kind, SourceKind::Config);
        assert_eq!(plan.header.product_name, "TEST");
        assert_eq!(plan.header.product_version, "V1.0.0");
        assert_eq!(plan.header.flash_type, 1);
        assert_eq!(plan.header.nv_backup, 1);
        assert!(!plan.needs_metadata());

        // The records keep the order of the configuration, and the operation
        // carries no file.
        assert_eq!(plan.entries.len(), 4);
        assert_eq!(plan.entries[0].id, "FDL");
        assert_eq!(plan.entries[1].id, "EraseSPL");
        assert_eq!(plan.entries[1].kind, 0);
        assert_eq!(plan.entries[2].id, "BOOT");
        assert_eq!(plan.missing(), 0);

        // The configuration XML is not in the table, so it is added as the
        // record a real package stores it in.
        let config = plan.entries.last().expect("the configuration record");
        assert_eq!(config.id, "");
        assert_eq!(config.kind, 2);
        assert!(config.choice.is_resolved());

        assert_eq!(plan.suggested_name(), "TEST_V1.0.0.pac");

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_configuration_record_is_not_the_users_to_drop() {
        let directory = temp_directory("configuration-record");
        write(&directory, "TEST.xml", "<BMAConfig/>");
        let source = write(&directory, "flash.cfg", CONFIG);

        let mut plan = plan(&source).expect("plan");
        let total = plan.entries.len();

        // The table lists the records the user chooses from. The configuration
        // XML is stored in the package but is not one of them.
        let configuration = plan
            .entries
            .iter()
            .position(Entry::is_configuration)
            .expect("the configuration record");

        assert_eq!(plan.chosen(), total - 1);
        assert_eq!(plan.chosen_with_file(), plan.with_file() - 1);

        // Nothing the dialog can send leaves it out.
        plan.set_included(configuration, false);
        assert!(plan.entries[configuration].include);
        assert_eq!(plan.chosen(), total - 1);

        // Every other record follows the checks.
        plan.set_included(0, false);
        assert!(!plan.entries[0].include);
        assert_eq!(plan.chosen(), total - 2);
        assert_eq!(plan.included(), total - 1);

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn plans_from_a_scatter_xml() {
        let directory = temp_directory("scatter-plan");
        write(&directory, "fdl1-sign.bin", "one");
        write(&directory, "boot-gki.img", "boot");
        write(&directory, "unknown-modem.bin", "modem");

        let xml = write(
            &directory,
            "TEST.xml",
            r#"<?xml version="1.0"?>
<BMAConfig>
  <ProductList>
    <Product name="TEST"><SchemeName>TEST</SchemeName><FlashTypeID>1</FlashTypeID><Mode>0</Mode></Product>
  </ProductList>
  <SchemeList>
    <Scheme name="TEST">
      <File><ID>FDL</ID><IDAlias>FDL1</IDAlias><Block><Base>0x5500</Base></Block><Flag>1</Flag><CheckFlag>1</CheckFlag></File>
      <File><ID>BOOT</ID><IDAlias>BOOT</IDAlias><Flag>1</Flag><CheckFlag>0</CheckFlag></File>
      <File><ID>Modem_NR</ID><IDAlias>Modem_NR</IDAlias><Flag>1</Flag><CheckFlag>2</CheckFlag></File>
      <File><ID>EraseMisc</ID><IDAlias>EraseMisc</IDAlias><Flag>0</Flag><CheckFlag>2</CheckFlag></File>
    </Scheme>
  </SchemeList>
</BMAConfig>
"#,
        );

        let plan = plan(&xml).expect("plan");

        assert_eq!(plan.kind, SourceKind::Scatter);
        assert_eq!(plan.header.product_name, "TEST");
        assert_eq!(plan.header.flash_type, 1);
        // A scatter XML needs the version and the alias from the user.
        assert!(plan.needs_metadata());
        assert_eq!(plan.header.version, HEADER_VERSION);

        // A known name is found, a product-specific one is not.
        assert!(plan.entries[0].choice.is_resolved());
        assert!(plan.entries[1].choice.is_resolved());
        assert_eq!(plan.entries[2].id, "Modem_NR");
        assert_eq!(plan.entries[2].choice, Choice::Missing);
        // "Not checked in the pack" becomes the record's omit flag.
        assert_eq!(plan.entries[2].omit, 1);
        // An operation needs no file at all.
        assert_eq!(plan.entries[3].kind, 0);
        assert!(!plan.entries[3].needs_file());
        assert!(!plan.entries[3].is_missing());

        // Plus the XML itself as the configuration record.
        assert_eq!(plan.entries.len(), 5);
        assert_eq!(plan.missing(), 1);

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn resolves_a_sharkl3_scatter_xml_by_its_known_names() {
        let directory = temp_directory("sharkl3-names");

        // The files a Unisoc Sharkle firmware ships, spelled the way its own
        // flash.cfg spells them.
        for name in [
            "fdl1-sign.bin",
            "sml-sign.bin",
            "u-boot-spl-16k-sign.bin",
            "u-boot-sign.bin",
            "sharkl3_pubcp_customer_nvitem.bin",
            "SC9600_sharkl3_pubcp_modem.dat",
            "sharkl3_pubcp_customer_deltanv.bin",
            "sharkl3_pubcp_LTEA_DSP.bin",
            "sharkl3_pubcp_DM_DSP.bin",
            "sharkl3_cm4.bin",
            "gnssbdmodem.bin",
            "gnssmodem.bin",
            "elable.img",
            "socko.img",
            "odmko.img",
            "vbmeta_custom1.img",
            "PM_sharkl3_cm4_v2.bin",
            "logo.bin",
            "LOGO.bmp",
        ] {
            write(&directory, name, "x");
        }

        // No flash.cfg goes with it, so every file is found by name alone.
        let xml = write(
            &directory,
            "s9863a3c10_gamoraplus.xml",
            r#"<?xml version="1.0"?>
<BMAConfig>
  <ProductList>
    <Product name="s9863a3c10"><SchemeName>s9863a3c10</SchemeName><FlashTypeID>1</FlashTypeID><Mode>0</Mode></Product>
  </ProductList>
  <SchemeList>
    <Scheme name="s9863a3c10">
      <File><ID>FDL</ID><Flag>1</Flag></File>
      <File><ID>SML</ID><Flag>1</Flag></File>
      <File><ID>SPLLoader</ID><Flag>1</Flag></File>
      <File><ID>UBOOTLoader</ID><Flag>1</Flag></File>
      <File><ID>NV_LTE</ID><Flag>1</Flag></File>
      <File><ID>Modem_LTE</ID><Flag>1</Flag></File>
      <File><ID>Modem_LTE_DELTANV</ID><Flag>1</Flag></File>
      <File><ID>DSP_LTE_LTE</ID><Flag>1</Flag></File>
      <File><ID>DSP_LTE_GGE</ID><Flag>1</Flag></File>
      <File><ID>DFS</ID><Flag>1</Flag></File>
      <File><ID>GPS_BD</ID><Flag>1</Flag></File>
      <File><ID>GPS_GL</ID><Flag>1</Flag></File>
      <File><ID>ELABLE</ID><Flag>1</Flag></File>
      <File><ID>Modem_WCN</ID><Flag>1</Flag></File>
      <File><ID>Socko</ID><Flag>1</Flag></File>
      <File><ID>Odmko</ID><Flag>1</Flag></File>
      <File><ID>VBMETA_CUSTOM1</ID><Flag>1</Flag></File>
      <File><ID>BootLogo</ID><Flag>1</Flag></File>
      <File><ID>Fastboot_Logo</ID><Flag>1</Flag></File>
      <File><ID>EraseMisc</ID><Flag>0</Flag></File>
    </Scheme>
  </SchemeList>
</BMAConfig>
"#,
        );

        let plan = plan(&xml).expect("plan");

        for (id, name) in [
            ("SML", "sml-sign.bin"),
            ("SPLLoader", "u-boot-spl-16k-sign.bin"),
            ("UBOOTLoader", "u-boot-sign.bin"),
            ("NV_LTE", "sharkl3_pubcp_customer_nvitem.bin"),
            ("Modem_LTE", "SC9600_sharkl3_pubcp_modem.dat"),
            ("Modem_LTE_DELTANV", "sharkl3_pubcp_customer_deltanv.bin"),
            ("DSP_LTE_LTE", "sharkl3_pubcp_LTEA_DSP.bin"),
            ("DSP_LTE_GGE", "sharkl3_pubcp_DM_DSP.bin"),
            ("DFS", "sharkl3_cm4.bin"),
            ("GPS_BD", "gnssbdmodem.bin"),
            ("GPS_GL", "gnssmodem.bin"),
            ("ELABLE", "elable.img"),
            ("Modem_WCN", "PM_sharkl3_cm4_v2.bin"),
            ("Socko", "socko.img"),
            ("Odmko", "odmko.img"),
            ("VBMETA_CUSTOM1", "vbmeta_custom1.img"),
            ("BootLogo", "logo.bin"),
            ("Fastboot_Logo", "LOGO.bmp"),
        ] {
            let entry = plan
                .entries
                .iter()
                .find(|entry| entry.id == id)
                .unwrap_or_else(|| panic!("{id} is not in the plan"));

            let file = entry
                .choice
                .path()
                .and_then(|path| path.file_name())
                .map(|name| name.to_string_lossy().into_owned());

            assert_eq!(file.as_deref(), Some(name), "{id}");
        }

        // The operation still needs no file, and the XML is its own record.
        assert_eq!(plan.missing(), 0);
        assert_eq!(plan.entries.len(), 21);
        assert_eq!(plan.entries.last().expect("the XML").kind, 2);

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// The Unisoc build system packs from a build tree, so its own packing
    /// configuration names the modem images by their place in it rather than by
    /// the names the packaging step gives them.
    #[test]
    fn resolves_a_build_trees_modem_images() {
        let directory = temp_directory("build-tree-names");

        for name in [
            "l_modem.img",
            "l_deltanv.img",
            "l_ldsp.img",
            "l_gdsp.img",
            "l_agdsp.img",
            "l_cdsp.img",
            "l_fixnv1.img",
            "l_fixnv2.img",
            "pm_sys.img",
            "wcnmodem.img",
            "gnssmodem.img",
        ] {
            write(&directory, name, "x");
        }

        let xml = write(
            &directory,
            "TEST.xml",
            r#"<?xml version="1.0"?>
<BMAConfig>
  <ProductList>
    <Product name="ums512_1h10"><SchemeName>ums512_1h10</SchemeName><FlashTypeID>1</FlashTypeID><Mode>0</Mode></Product>
  </ProductList>
  <SchemeList>
    <Scheme name="ums512_1h10">
      <File><ID>Modem_LTE</ID><Flag>1</Flag></File>
      <File><ID>Modem_LTE_DELTANV</ID><Flag>1</Flag></File>
      <File><ID>DSP_LTE_LTE</ID><Flag>1</Flag></File>
      <File><ID>DSP_LTE_GGE</ID><Flag>1</Flag></File>
      <File><ID>DSP_LTE_AG</ID><Flag>1</Flag></File>
      <File><ID>DSP_LTE_CDMA</ID><Flag>1</Flag></File>
      <File><ID>DFS</ID><Flag>1</Flag></File>
      <File><ID>Modem_WCN</ID><Flag>1</Flag></File>
      <File><ID>Modem_GNSS</ID><Flag>1</Flag></File>
      <File><ID>NV_LTE</ID><Flag>1</Flag></File>
    </Scheme>
  </SchemeList>
</BMAConfig>
"#,
        );

        let plan = plan(&xml).expect("plan");

        for (id, name) in [
            ("Modem_LTE", "l_modem.img"),
            ("Modem_LTE_DELTANV", "l_deltanv.img"),
            ("DSP_LTE_LTE", "l_ldsp.img"),
            ("DSP_LTE_GGE", "l_gdsp.img"),
            ("DSP_LTE_AG", "l_agdsp.img"),
            ("DSP_LTE_CDMA", "l_cdsp.img"),
            ("DFS", "pm_sys.img"),
            ("Modem_WCN", "wcnmodem.img"),
            ("Modem_GNSS", "gnssmodem.img"),
            // The build system lists two of them; the first is the one taken.
            ("NV_LTE", "l_fixnv1.img"),
        ] {
            let entry = plan
                .entries
                .iter()
                .find(|entry| entry.id == id)
                .unwrap_or_else(|| panic!("{id} is not in the plan"));

            let file = entry
                .choice
                .path()
                .and_then(|path| path.file_name())
                .map(|name| name.to_string_lossy().into_owned());

            assert_eq!(file.as_deref(), Some(name), "{id}");
        }

        assert_eq!(plan.missing(), 0);

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// The three newest reference firmwares — a Sharkle Go one and two HMD ones
    /// — whose own `flash.cfg` files name the same records differently again.
    #[test]
    fn resolves_the_names_of_the_hmd_firmwares() {
        let directory = temp_directory("hmd-names");

        for name in [
            "SC9600_sharkl3_pubcp_modem.bin",
            "SC9600_qogirl6_pubcp_modem.bin",
            "9863_integration_cm4_v2.bin",
            "wcnmodem.bin",
            "qogirl6_cm4.bin",
            "qogirl6_pubcp_exLNA_customer_nvitem.bin",
            "qogirl6_pubcp_exLNA_customer_deltanv.bin",
            "qogirl6_pubcp_LTEA_DSP.bin",
            "qogirl6_pubcp_DM_DSP.bin",
            "QogirL6_AUDCP_DSP_lit_dm.bin",
            "hmd_hd_720_1600.bmp",
            "unisoc_FHD_1080_1920.bmp",
            "unisoc_FHD_1080_2400.bmp",
        ] {
            write(&directory, name, "x");
        }

        let xml = write(
            &directory,
            "T19681AA1.xml",
            r#"<?xml version="1.0"?>
<BMAConfig>
  <ProductList>
    <Product name="T19681AA1"><SchemeName>T19681AA1</SchemeName><FlashTypeID>0</FlashTypeID><Mode>0</Mode></Product>
  </ProductList>
  <SchemeList>
    <Scheme name="T19681AA1">
      <File><ID>Modem_LTE</ID><Flag>1</Flag></File>
      <File><ID>Modem_WCN</ID><Flag>1</Flag></File>
      <File><ID>NV_LTE</ID><Flag>1</Flag></File>
      <File><ID>Modem_LTE_DELTANV</ID><Flag>1</Flag></File>
      <File><ID>DFS</ID><Flag>1</Flag></File>
      <File><ID>DSP_LTE_LTE</ID><Flag>1</Flag></File>
      <File><ID>DSP_LTE_GGE</ID><Flag>1</Flag></File>
      <File><ID>DSP_LTE_AG</ID><Flag>1</Flag></File>
      <File><ID>Fastboot_Logo</ID><Flag>1</Flag></File>
    </Scheme>
  </SchemeList>
</BMAConfig>
"#,
        );

        let plan = plan(&xml).expect("plan");

        for (id, name) in [
            // Several names of the record are in the folder: the first one the
            // table lists is the one taken.
            ("Modem_LTE", "SC9600_sharkl3_pubcp_modem.bin"),
            ("Modem_WCN", "wcnmodem.bin"),
            ("NV_LTE", "qogirl6_pubcp_exLNA_customer_nvitem.bin"),
            ("Modem_LTE_DELTANV", "qogirl6_pubcp_exLNA_customer_deltanv.bin"),
            ("DFS", "qogirl6_cm4.bin"),
            ("DSP_LTE_LTE", "qogirl6_pubcp_LTEA_DSP.bin"),
            ("DSP_LTE_GGE", "qogirl6_pubcp_DM_DSP.bin"),
            ("DSP_LTE_AG", "QogirL6_AUDCP_DSP_lit_dm.bin"),
            ("Fastboot_Logo", "hmd_hd_720_1600.bmp"),
        ] {
            let entry = plan
                .entries
                .iter()
                .find(|entry| entry.id == id)
                .unwrap_or_else(|| panic!("{id} is not in the plan"));

            let file = entry
                .choice
                .path()
                .and_then(|path| path.file_name())
                .map(|name| name.to_string_lossy().into_owned());

            assert_eq!(file.as_deref(), Some(name), "{id}");
        }

        assert_eq!(plan.missing(), 0);

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn builds_a_package_from_a_configuration() {
        let directory = temp_directory("config-build");
        write(&directory, "fdl1-sign.bin", "one");
        write(&directory, "boot.img", "seven!!");
        write(&directory, "TEST.xml", "<BMAConfig/>");
        let source = write(&directory, "flash.cfg", CONFIG);

        let plan = plan(&source).expect("plan");
        let destination = directory.join("out.pac");
        let summary = build(&plan, &destination, &|_| {}).expect("build");

        assert_eq!(summary.files, 3);

        // The package reads back with the records of the configuration, and
        // describes itself the way the tools expect.
        let package = pac::list(&destination).expect("read the package");

        assert_eq!(package.header, plan.header);
        assert_eq!(package.records[0].id, "FDL");
        assert_eq!(package.records[0].name, "fdl1-sign.bin");
        assert_eq!(package.records[0].flag, 1);
        assert_eq!(package.records[1].name, "");
        assert_eq!(package.records[1].kind, 0);
        assert_eq!(package.records[3].kind, 2);
        assert_eq!(package.records[3].name, "TEST.xml");

        let config = pac::flash_cfg(&package);
        assert!(config.contains("PAC_PRODUCT=TEST\n"), "{config}");
        assert!(config.contains("FDL=1@fdl1-sign.bin\n"), "{config}");
        assert!(config.contains("EraseSPL=1@\n"), "{config}");

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn an_unresolved_file_stops_the_build() {
        let directory = temp_directory("unresolved");
        let xml = write(
            &directory,
            "TEST.xml",
            r#"<BMAConfig><ProductList><Product name="TEST"><SchemeName>TEST</SchemeName></Product></ProductList>
<SchemeList><Scheme name="TEST"><File><ID>Modem_NR</ID><Flag>1</Flag></File></Scheme></SchemeList></BMAConfig>"#,
        );

        let mut plan = plan(&xml).expect("plan");
        plan.header.product_alias = "TEST".to_string();
        plan.header.product_version = "1.0.0".to_string();

        assert_eq!(plan.missing(), 1);
        assert_eq!(
            build(&plan, &directory.join("out.pac"), &|_| {}),
            Err(BuildError::MissingFile("Modem_NR".to_string()))
        );

        // Leaving it out is allowed, and writes the record as an operation.
        plan.entries[0].include = false;
        build(&plan, &directory.join("out.pac"), &|_| {}).expect("build without it");

        let package = pac::list(&directory.join("out.pac")).expect("read the package");
        assert!(package.records.iter().all(|record| record.id != "Modem_NR"));

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn metadata_is_required_for_a_scatter_xml() {
        let directory = temp_directory("metadata");
        let xml = write(&directory, "TEST.xml", "<BMAConfig/>");

        // Not a usable product XML either way.
        assert!(plan(&xml).is_err());

        let source = write(&directory, "flash.cfg", "[FlashParam]\nFDL=1@\n");
        let mut plan = plan(&source).expect("plan");
        plan.header.product_alias.clear();
        plan.header.product_version.clear();

        assert!(plan.needs_metadata());
        assert_eq!(
            build(&plan, &directory.join("out.pac"), &|_| {}),
            Err(BuildError::MissingMetadata)
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn something_else_is_not_a_source() {
        let directory = temp_directory("foreign");
        let path = write(&directory, "notes.txt", "hello");

        assert_eq!(plan(&path), Err(BuildError::NotASource));

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn names_the_partition_a_record_writes_to() {
        let directory = temp_directory("partitions");
        let source = write(
            &directory,
            "flash.cfg",
            "[Setting]\nPAC_CONFILE=product.xml\n\n[FlashParam]\nBOOT=1@boot-gki.img\nFDL=1@fdl1-sign.bin\n",
        );
        write(
            &directory,
            "product.xml",
            "<BMAConfig><ProductList><Product name=\"P\"><SchemeName>P</SchemeName>\
             </Product></ProductList><SchemeList><Scheme><File><ID>BOOT</ID><Block id=\"boot_a\">\
             <Base>0x0</Base></Block><Flag>1</Flag></File><File><ID>FDL</ID>\
             <Block><Base>0x65000800</Base></Block><Flag>1</Flag></File>\
             </Scheme></SchemeList></BMAConfig>",
        );
        write(&directory, "boot-gki.img", "kernel");
        write(&directory, "fdl1-sign.bin", "loader");

        // The configuration says nothing about partitions: the XML it names is
        // where they come from.
        let plan = plan(&source).expect("plan");
        let partition = |id: &str| -> Option<String> {
            plan.entries
                .iter()
                .find(|entry| entry.id == id)
                .expect("record")
                .partition
                .clone()
        };

        assert_eq!(partition("BOOT").as_deref(), Some("boot_a"));
        // The two download agents write to an address, not to a partition.
        assert_eq!(partition("FDL"), None);

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// The second reference product (a Unisoc Sharkle firmware), which ships
    /// its scatter XML and a `flash.cfg` named after the product instead of
    /// `flash.cfg`: the names its records take have to be known by heart, or
    /// they could not be packed without a configuration to read.
    #[test]
    #[ignore = "needs reference/s9863a3c10_gamoraplus.xml"]
    fn knows_the_files_of_the_second_reference_product() {
        let plan = plan(Path::new("reference/s9863a3c10_gamoraplus.xml")).expect("plan");

        // Every record that carries a file has a name this tool knows.
        for entry in plan
            .entries
            .iter()
            .filter(|entry| entry.needs_file() && !entry.is_configuration())
        {
            assert!(
                !known_files(&entry.id).is_empty(),
                "no known name for {}",
                entry.id
            );
        }

        // And the XML itself says which partition each record writes to.
        let partition = |id: &str| {
            plan.entries
                .iter()
                .find(|entry| entry.id == id)
                .and_then(|entry| entry.partition.clone())
        };

        assert_eq!(plan.header.product_name, "s9863a3c10");
        assert_eq!(partition("SPLLoader"), Some("splloader".to_string()));
        assert_eq!(partition("UBOOTLoader"), Some("uboot".to_string()));
        assert_eq!(partition("Modem_LTE"), Some("l_modem".to_string()));
        assert_eq!(partition("DFS"), Some("pm_sys".to_string()));
        assert_eq!(partition("ELABLE"), Some("elable".to_string()));
        assert_eq!(partition("BootLogo"), Some("logo".to_string()));
        assert_eq!(partition("Fastboot_Logo"), Some("fbootlogo".to_string()));
    }

    /// The Unisoc build system's own packing configuration, which names the
    /// records with the paths of the build tree it packs from. Its names are
    /// the ones this tool knows by heart, so a scattered firmware of that kind
    /// needs no configuration beside it.
    #[test]
    #[ignore = "needs reference/pac.ini"]
    fn knows_the_names_of_the_build_systems_configuration() {
        let text = std::fs::read_to_string("reference/pac.ini").expect("read pac.ini");
        let ini = Ini::parse(&text);

        // Every record its packaging list carries has a file this tool names.
        for (id, value) in ini.section("pac_list") {
            let (_, path) = flash_parameter(value);

            // The first entry names the configuration XML, which is the plan's
            // own record rather than a file a record takes.
            if path.is_empty() || id.eq_ignore_ascii_case("CONFILE") {
                continue;
            }

            let name = file_of(&path);

            assert!(
                known_files(id).contains(&name.as_str()),
                "{id} takes {name}, which is not a known name"
            );
        }

        // So has every modem image its partition list points at in the tree.
        for line in text.lines() {
            let Some((id, path)) = line.trim().split_once(':') else {
                continue;
            };

            if !path.ends_with(".img") {
                continue;
            }

            let name = file_of(path);

            assert!(
                known_files(id).contains(&name.as_str()),
                "{id} takes {name}, which is not a known name"
            );
        }
    }

    /// The file a build-tree path names.
    fn file_of(path: &str) -> String {
        path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
    }

    /// Every name the reference flash configurations give their records is one
    /// this tool knows, so a scattered firmware of any of those products can be
    /// resolved without a configuration beside it.
    #[test]
    #[ignore = "needs the reference flash configurations"]
    fn knows_every_name_of_the_reference_configurations() {
        let folder = Path::new("reference");

        let mut sources = vec![folder.join("scattered_unisoc_fw").join("flash.cfg")];

        // The configurations the samples sit in, named after their product.
        sources.extend(
            std::fs::read_dir(folder)
                .expect("read the reference folder")
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("cfg"))
                }),
        );

        let mut configurations = 0;
        let mut records = 0;

        for source in sources {
            let Ok(text) = std::fs::read_to_string(&source) else {
                continue;
            };

            configurations += 1;

            for (id, value) in Ini::parse(&text).section("FlashParam") {
                let (_, name) = flash_parameter(value);

                // An operation carries no file.
                if name.is_empty() {
                    continue;
                }

                assert!(
                    known_files(id).contains(&name.as_str()),
                    "{}: {id} takes {name}, which is not a known name",
                    source.display()
                );

                records += 1;
            }
        }

        assert!(configurations >= 5, "only {configurations} were read");
        assert!(records > 150, "only {records} records were checked");
    }

    /// The scattered firmware in the reference tree: a scatter XML, the
    /// `flash.cfg` beside it and the images they name.
    #[test]
    #[ignore = "needs reference/scattered_unisoc_fw"]
    fn plans_from_the_reference_folder() {
        let scatter_plan =
            plan(Path::new("reference/scattered_unisoc_fw/ums9620_2h10.xml")).expect("plan");

        assert_eq!(scatter_plan.header.product_name, "ums9620_2h10");
        assert_eq!(scatter_plan.header.flash_type, 1);
        assert_eq!(scatter_plan.header.nv_backup, 1);
        assert!(scatter_plan.needs_metadata());

        // 49 records from the XML, plus the XML as the configuration record.
        assert_eq!(scatter_plan.entries.len(), 50);
        assert_eq!(scatter_plan.entries[0].id, "FDL");
        assert_eq!(scatter_plan.entries[0].addresses, vec![0x6500_0800]);
        assert_eq!(scatter_plan.entries[1].id, "FDL2");
        // The download agents write to an address, not to a partition.
        assert_eq!(scatter_plan.entries[0].partition, None);
        assert_eq!(scatter_plan.entries[1].partition, None);
        assert_eq!(scatter_plan.entries[2].partition.as_deref(), Some("splloader"));

        // The partition comes from the record's own block, which is what the
        // table's "actual partition" column shows.
        let partition = |id: &str| -> Option<&str> {
            scatter_plan
                .entries
                .iter()
                .find(|entry| entry.id == id)
                .and_then(|entry| entry.partition.as_deref())
        };

        assert_eq!(partition("BOOT"), Some("boot_a"));
        assert_eq!(partition("SML"), Some("sml_a"));
        assert_eq!(partition("Modem_NR"), Some("nr_modem_a"));
        assert_eq!(partition("PhaseCheck"), Some("miscdata"));
        assert_eq!(partition("SPLLoaderUFS"), Some("splloader"));

        // "Not checked in the pack" is the record's omit flag, and an
        // operation is a record that needs no file.
        let erase = scatter_plan
            .entries
            .iter()
            .find(|entry| entry.id == "EraseSPL")
            .expect("EraseSPL");
        assert_eq!(erase.kind, 0);
        assert_eq!(erase.omit, 1);
        assert_eq!(scatter_plan.entries[0].omit, 0);

        // Every record that needs a file is resolved except the five whose
        // names are not known and cannot be guessed; those are the ones the
        // user has to point at by hand.
        assert_eq!(scatter_plan.with_file(), 40);
        let waiting: Vec<&str> = scatter_plan
            .entries
            .iter()
            .filter(|entry| entry.is_missing())
            .map(|entry| entry.id.as_str())
            .collect();
        assert_eq!(
            waiting,
            [
                "HYPERVSIOR",
                "AVBMETA_RS",
                "COMMON_RS1",
                "COMMON_RS2",
                "NV_CALI_NR"
            ]
        );

        // The configuration beside it names every file itself, so nothing is
        // left to guess.
        let config_plan = plan(Path::new("reference/scattered_unisoc_fw/flash.cfg")).expect("plan");

        assert_eq!(config_plan.kind, SourceKind::Config);
        // 46 parameters, plus the configuration XML as a record of its own.
        assert_eq!(config_plan.entries.len(), 47);
        assert_eq!(config_plan.entries[0].id, "FDL2");
        assert_eq!(config_plan.with_file(), 37);
        assert_eq!(config_plan.missing(), 0);

        // Every record that has a file knows how large it is, which is what the
        // table's size column shows.
        for entry in config_plan.entries.iter().filter(|entry| entry.needs_file()) {
            assert!(entry.size.is_some(), "{} has no size", entry.id);
        }

        // The configuration names the XML to read the partitions from.
        let boot = config_plan
            .entries
            .iter()
            .find(|entry| entry.id == "BOOT")
            .expect("BOOT");

        assert_eq!(boot.partition.as_deref(), Some("boot_a"));
    }

    /// Builds a package out of the reference folder and reads it back. Only the
    /// images under a megabyte are written, so this goes through the whole path
    /// without copying the 7.8 GB of firmware the folder holds.
    #[test]
    #[ignore = "needs reference/scattered_unisoc_fw"]
    fn the_reference_folder_builds_into_a_package_of_its_own() {
        let mut plan = plan(Path::new("reference/scattered_unisoc_fw/flash.cfg")).expect("plan");

        plan.entries.retain(|entry| match entry.choice.path() {
            Some(path) => path
                .metadata()
                .map(|data| data.len() < 1_000_000)
                .unwrap_or(false),
            // An operation, and the configuration record of a scatter XML,
            // carry no file of their own.
            None => true,
        });

        assert_eq!(plan.missing(), 0);
        assert!(plan.with_file() > 0);

        plan.header.product_alias = "Reference".to_string();
        plan.header.product_version = "1.0".to_string();

        let directory = temp_directory("reference");
        let destination = directory.join(plan.suggested_name());

        assert_eq!(plan.suggested_name(), "Reference_1.0.pac");

        let summary = build(&plan, &destination, &|_| {}).expect("build");

        assert_eq!(summary.files, plan.with_file());

        let expected: u64 = plan
            .payloads()
            .iter()
            .filter_map(|payload| payload.source.as_deref())
            .filter_map(|path| std::fs::metadata(path).ok())
            .map(|data| data.len())
            .sum();

        assert_eq!(summary.bytes, expected);

        let package = pac::list(&destination).expect("list");

        assert!(package.complete);
        assert_eq!(package.header.product_name, plan.header.product_name);
        assert_eq!(package.header.product_alias, "Reference");
        assert_eq!(package.header.product_version, "1.0");
        assert_eq!(package.files.len(), plan.with_file());
        assert_eq!(package.records.len(), plan.included());

        // The names come from the files that went in.
        let names: Vec<&str> = package
            .files
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();

        assert!(names.contains(&"prodnv.img"));
        assert!(names.contains(&"ums9620_2h10.xml"));

        let _ = std::fs::remove_dir_all(&directory);
    }
}
