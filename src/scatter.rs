//! Unisoc "scatter" product XML files.
//!
//! The Unisoc download tools describe a firmware with an XML product file
//! (the one a package stores as its `PAC_CONFILE`). It has two halves: a
//! `<ProductList>` with the product's switches and its partition table, and a
//! `<SchemeList>` whose `<Scheme>` holds one `<File>` entry per record of the
//! package the tool builds from it — with the entry's id, its display alias,
//! whether it carries a payload, whether the packer selects it, and the block
//! base addresses it is written to.
//!
//! The file documents its own fields, and the mapping was verified against the
//! XML inside `reference/TYH201H_HNR552T_N1770.6.12.09.00.pac` (53 entries,
//! whose order, `<Flag>`, `<CheckFlag>` and `<Block><Base>` values all line up
//! with that package's records):
//!
//! ```text
//! FlashTypeID: 0 = Nor Flash, 1 = Nand Flash
//! File-Flag:   0 = this file need not input file path, 1 = it does
//! File-CheckFlag: 0 = optional, 1 = must select this file, 2 = not checked in pack
//! Mode:        0 = not used
//! File-ID:      cannot be changed, it is used by the tools
//! ```

/// One `<File>` of a scheme: what a package record looks like before the files
/// are attached to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    /// The record's id, e.g. `FDL` or `EraseMisc`.
    pub id: String,
    /// The id the tools show in their GUI, which a product may spell
    /// differently (`FDL1` for `FDL`).
    pub alias: String,
    /// `File-Flag`: 0 = the record needs no file (a pure operation), 1 = it
    /// needs one.
    pub kind: u32,
    /// `File-CheckFlag`: 0 = optional, 1 = the packer must select it, 2 = the
    /// packer does not check it in the package. A package stores this as the
    /// record's "omit" flag (2 means the record is left out of the tool's
    /// "all files" selection).
    pub check_flag: u32,
    /// The block base addresses the record writes to, from its `<Block>`
    /// elements (a record stores up to five).
    pub addresses: Vec<u32>,
    /// The partition the record is written to, from the `id` of its first
    /// `<Block id="…">`. A block without an id writes to a raw address rather
    /// than to a partition — the two download agents (`FDL`, `FDL2`) are.
    pub partition: Option<String>,
}

/// What a product XML describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scatter {
    /// The product, from `<SchemeName>` (or the `<Product>` name).
    pub product: String,
    /// `<Mode>`.
    pub mode: u32,
    /// `<FlashTypeID>`.
    pub flash_type: u32,
    /// `<NVBackup backup="…">`.
    pub nv_backup: u32,
    /// The scheme's `<File>` entries, in the order the file lists them — which
    /// is the order the package's records are written in.
    pub files: Vec<File>,
}

/// Why a product XML could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScatterError;

impl std::fmt::Display for ScatterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("not a Unisoc scatter XML")
    }
}

impl std::error::Error for ScatterError {}

/// The text of a direct child element, trimmed.
fn child_text(node: &roxmltree::Node<'_, '_>, name: &str) -> Option<String> {
    node.children()
        .find(|child| child.has_tag_name(name))
        .and_then(|child| child.text())
        .map(|text| text.trim().to_string())
}

/// A number written either in hexadecimal (`0x…`) or in decimal.
fn number(text: &str) -> u32 {
    let text = text.trim();

    match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, 16).unwrap_or(0),
        None => text.parse().unwrap_or(0),
    }
}

/// A child element's number, or `default` when it is missing.
fn child_number(node: &roxmltree::Node<'_, '_>, name: &str) -> u32 {
    child_text(node, name)
        .map(|text| number(&text))
        .unwrap_or(0)
}

/// Reads a product XML from its text.
pub fn parse(text: &str) -> Result<Scatter, ScatterError> {
    let document = roxmltree::Document::parse(text).map_err(|_| ScatterError)?;
    let root = document.root_element();

    if !root.has_tag_name("BMAConfig") {
        return Err(ScatterError);
    }

    // The product block is where the switches live, the schedule is where the
    // records come from.
    let product = root
        .descendants()
        .find(|node| node.has_tag_name("Product"))
        .ok_or(ScatterError)?;
    let scheme = root
        .descendants()
        .find(|node| node.has_tag_name("Scheme"))
        .ok_or(ScatterError)?;

    let name = child_text(&product, "SchemeName")
        .filter(|name| !name.is_empty())
        .or_else(|| product.attribute("name").map(|name| name.trim().to_string()))
        .unwrap_or_default();

    let nv_backup = product
        .children()
        .find(|node| node.has_tag_name("NVBackup"))
        .and_then(|node| node.attribute("backup"))
        .map(number)
        .unwrap_or(0);

    let mut files = Vec::new();

    for entry in scheme.children().filter(|node| node.has_tag_name("File")) {
        // The id is what the tools key a record by; an entry without one is
        // not usable.
        let id = child_text(&entry, "ID").unwrap_or_default();

        if id.is_empty() {
            continue;
        }

        let blocks = || entry.children().filter(|node| node.has_tag_name("Block"));

        let addresses = blocks()
            .filter_map(|block| child_text(&block, "Base"))
            .take(5)
            .map(|base| number(&base))
            .collect();

        // What the record writes to, when the tools name a partition for it.
        let partition = blocks()
            .filter_map(|block| block.attribute("id"))
            .map(|id| id.trim())
            .find(|id| !id.is_empty())
            .map(str::to_string);

        files.push(File {
            alias: child_text(&entry, "IDAlias").unwrap_or_default(),
            kind: child_number(&entry, "Flag"),
            check_flag: child_number(&entry, "CheckFlag"),
            addresses,
            partition,
            id,
        });
    }

    Ok(Scatter {
        product: name,
        mode: child_number(&product, "Mode"),
        flash_type: child_number(&product, "FlashTypeID"),
        nv_backup,
        files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<BMAConfig>
  <ProductList>
    <Product name="TEST">
      <SchemeName>TEST</SchemeName>
      <FlashTypeID>1</FlashTypeID>
      <Mode>0</Mode>
      <NVBackup backup="1">
        <NVItem name="IMEI" />
      </NVBackup>
      <Partitions>
        <Partition id="boot" size="20" />
      </Partitions>
    </Product>
  </ProductList>
  <SchemeList>
    <Scheme name="TEST">
      <File>
        <ID>FDL</ID>
        <IDAlias>FDL1</IDAlias>
        <Type>FDL</Type>
        <Block><Base>0x00005500</Base><Size>0x0</Size></Block>
        <Flag>1</Flag>
        <CheckFlag>1</CheckFlag>
        <Description>First image</Description>
      </File>
      <File>
        <ID>EraseMisc</ID>
        <IDAlias>EraseMisc</IDAlias>
        <Type>EraseFlash2</Type>
        <Block id="misc"><Base>0x0</Base><Size>0x0</Size></Block>
        <Flag>0</Flag>
        <CheckFlag>2</CheckFlag>
      </File>
      <File>
        <ID>Super</ID>
        <IDAlias>SuperImage</IDAlias>
        <Block id="super"><Base>0x1000000</Base><Size>0x2000000</Size></Block>
        <Block id="super_b"><Base>0x3000000</Base></Block>
        <Flag>1</Flag>
        <CheckFlag>0</CheckFlag>
      </File>
    </Scheme>
  </SchemeList>
</BMAConfig>
"#;

    #[test]
    fn reads_a_scheme() {
        let scatter = parse(SAMPLE).expect("parse");

        assert_eq!(scatter.product, "TEST");
        assert_eq!(scatter.mode, 0);
        assert_eq!(scatter.flash_type, 1);
        assert_eq!(scatter.nv_backup, 1);
        assert_eq!(scatter.files.len(), 3);

        assert_eq!(scatter.files[0].id, "FDL");
        assert_eq!(scatter.files[0].alias, "FDL1");
        assert_eq!(scatter.files[0].kind, 1);
        assert_eq!(scatter.files[0].check_flag, 1);
        assert_eq!(scatter.files[0].addresses, vec![0x5500]);
        // A block without an id writes to an address, not to a partition.
        assert_eq!(scatter.files[0].partition, None);

        // A pure operation needs no file, and a check flag of two means the
        // packer leaves it out of its "all files" selection.
        assert_eq!(scatter.files[1].id, "EraseMisc");
        assert_eq!(scatter.files[1].kind, 0);
        assert_eq!(scatter.files[1].check_flag, 2);
        assert_eq!(scatter.files[1].partition.as_deref(), Some("misc"));

        // A file may carry several blocks: the partition is the first one that
        // names an id.
        assert_eq!(scatter.files[2].id, "Super");
        assert_eq!(scatter.files[2].addresses, vec![0x1000000, 0x3000000]);
        assert_eq!(scatter.files[2].partition.as_deref(), Some("super"));
    }

    #[test]
    fn something_else_is_refused() {
        assert_eq!(parse("<not xml").unwrap_err(), ScatterError);
        assert_eq!(parse("<html><body/></html>").unwrap_err(), ScatterError);

        // The right root, but no product or scheme to read.
        assert_eq!(parse("<BMAConfig/>").unwrap_err(), ScatterError);
    }

    /// The product XML the reference package stores, when it is there: it is
    /// not part of the repository.
    #[test]
    #[ignore = "needs reference/scattered_unisoc_fw/ums9620_2h10.xml"]
    fn reads_the_reference_scatter() {
        let text = std::fs::read_to_string("reference/scattered_unisoc_fw/ums9620_2h10.xml")
            .expect("read the scatter XML");
        let scatter = parse(&text).expect("parse the scatter XML");

        assert_eq!(scatter.product, "ums9620_2h10");
        assert_eq!(scatter.flash_type, 1);
        assert_eq!(scatter.nv_backup, 1);
        assert_eq!(scatter.files.len(), 49);

        // The first entries are the loader chain. The download agents are not
        // written to a partition, so only the rest carry one.
        assert_eq!(scatter.files[0].id, "FDL");
        assert_eq!(scatter.files[0].alias, "FDL1");
        assert_eq!(scatter.files[0].kind, 1);
        assert_eq!(scatter.files[0].check_flag, 1);
        assert_eq!(scatter.files[0].partition, None);
        assert_eq!(scatter.files[1].id, "FDL2");
        assert_eq!(scatter.files[1].partition, None);

        // An operation, and the flag that keeps it out of the pack.
        let erase = scatter
            .files
            .iter()
            .find(|file| file.id == "EraseMisc")
            .expect("EraseMisc");
        assert_eq!(erase.kind, 0);
        assert_eq!(erase.check_flag, 0);
        assert_eq!(erase.partition.as_deref(), Some("misc"));

        // Every record that writes to a partition names it in its block.
        let boots: Vec<Option<&str>> = scatter
            .files
            .iter()
            .filter(|file| ["BOOT", "SML", "Modem_NR", "PhaseCheck", "EraseSPL"].contains(&file.id.as_str()))
            .map(|file| file.partition.as_deref())
            .collect();

        assert_eq!(boots, vec![Some("splloader"), Some("miscdata"), Some("nr_modem_a"), Some("boot_a"), Some("sml_a")]);
    }
}
