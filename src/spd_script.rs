//! The `spd_dump` scripts written next to the files a package is unpacked into.
//!
//! `spd_dump` flashes a *scattered* firmware over USB: it loads the two
//! download agents into memory, has the phone repartition itself from the
//! scatter XML, and then writes one partition per command. A package stores
//! neither the partition names nor the addresses the agents are loaded at —
//! but the configuration XML it keeps does: the record the package calls its
//! configuration (`PAC_CONFILE`) names the block of every record
//! (`<Block id="l_modem">`), and its two loader records carry the addresses
//! (`<Base>0x00005000</Base>`). Both scripts are written from the package
//! together with that XML, so a package whose XML is missing has none.
//!
//! Comments are English, like every other generated file: they are part of the
//! script, not of the interface. The shell follows the host, the way the
//! Firmware Flash export does — a `.cmd` that ends in `pause` on Windows, a
//! `.sh` that ends in `read` anywhere else.

use std::collections::HashMap;

use crate::flash_script::{sanitize, Shell};
use crate::{pac, scatter};

/// The records whose files are never written: they carry what belongs to a
/// single unit (its radio calibration, its label) or what the phone makes
/// again on its own, so writing another dump's copy of them is at best useless
/// and at worst harmful.
const SKIPPED: &[&str] = &["prodnv", "elable", "elabel", "nv_lte", "userdata"];

/// The records the two download agents are loaded through. Their files are not
/// written to a partition: they are the `fdl` arguments of the command.
const LOADERS: [&str; 2] = ["FDL", "FDL2"];

/// The partitions the factory reset script erases.
const ERASED: [&str; 2] = ["persist", "userdata"];

/// How long the tool waits for the phone, and that it may not ask anything.
const OPTIONS: &str = "--wait 32767 skip_confirm 1";

/// The two scripts a run writes: the file name and the contents of each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scripts {
    /// The script that writes the whole firmware. It is only written when the
    /// run extracts both download agents *and* the configuration XML the phone
    /// repartitions itself from, and when something is left to write: with
    /// nothing to write it would only erase the partitions.
    pub flash: Option<(String, String)>,
    /// The script that erases the phone's own data. It needs the download
    /// agents and nothing else.
    pub reset: (String, String),
}

/// Why the scripts could not be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpdError {
    /// The configuration XML could not be read as a scatter XML, so the
    /// partition a record writes to is not known.
    Xml(String),
    /// A download agent is in neither the package or the XML, so the command
    /// would not load anything.
    Loader(String),
}

impl std::fmt::Display for SpdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Xml(error) => write!(f, "the configuration XML could not be read: {error}"),
            Self::Loader(id) => write!(f, "the package carries no `{id}` to load the firmware with"),
        }
    }
}

impl std::error::Error for SpdError {}

/// Whether a run of these files takes nothing but the two download agents.
///
/// Such a run prepares a factory reset rather than a scattered firmware, which
/// is what a flash configuration describes, so it leaves that file out.
pub(crate) fn only_loaders(package: &pac::Package, selected: &[usize]) -> bool {
    let loader = |index: usize| {
        package
            .files
            .get(index)
            .is_some_and(|entry| LOADERS.iter().any(|id| id.eq_ignore_ascii_case(&entry.id)))
    };

    !selected.is_empty() && selected.iter().all(|&index| loader(index))
}

/// Whether the scripts can be asked for at all.
///
/// The package has to carry its configuration XML (which names the partitions
/// and the addresses the download agents are loaded at), and both download
/// agents have to be part of what the run writes — the scripts name their
/// files, so they have to have been written out.
pub(crate) fn can_generate(package: &pac::Package, selected: &[bool]) -> bool {
    pac::has_configuration(package) && LOADERS.iter().all(|id| loader_selected(package, selected, id))
}

/// Whether the file of one download agent is checked.
fn loader_selected(package: &pac::Package, selected: &[bool], id: &str) -> bool {
    loader_index(package, id).is_some_and(|index| selected.get(index).copied().unwrap_or(false))
}

/// Where the file of a download agent sits in the package's file list.
fn loader_index(package: &pac::Package, id: &str) -> Option<usize> {
    package
        .files
        .iter()
        .position(|entry| entry.id.eq_ignore_ascii_case(id))
}

/// Renders the scripts of a package.
///
/// `selected` holds the indexes of the files the run writes (into
/// [`pac::Package::files`]), and is what the commands list: a script flashes
/// the scattered firmware the extraction just produced, so a file the user
/// left out is not written by it either. `base` is what the script files are
/// named after, which is the product when the package names one.
pub fn scripts(
    package: &pac::Package,
    selected: &[usize],
    xml: &str,
    shell: Shell,
    base: &str,
) -> Result<Scripts, SpdError> {
    let scatter = scatter::parse(xml).map_err(|error| SpdError::Xml(error.to_string()))?;

    // What the XML knows about a record: the partition it writes to, and the
    // address the loader records are loaded at.
    let blocks: HashMap<String, &scatter::File> = scatter
        .files
        .iter()
        .map(|file| (file.id.to_ascii_lowercase(), file))
        .collect();

    let loaders = loaders(package, &blocks)?;

    // The package's own XML is what the phone repartitions itself from, so the
    // command names the file the extraction wrote — which is only there when
    // the user checked it.
    let configuration = pac::config_name(package);

    let extracted = !configuration.is_empty()
        && package.files.iter().enumerate().any(|(index, entry)| {
            entry.name.eq_ignore_ascii_case(configuration) && selected.contains(&index)
        });

    let mut writes = String::new();

    for &index in selected {
        let Some(entry) = package.files.get(index) else {
            continue;
        };

        // The configuration XML is already named by `repartition`, and a
        // record without an id is the package's own description, not a file
        // that is written anywhere.
        if entry.id.is_empty() || entry.name.eq_ignore_ascii_case(configuration) {
            continue;
        }

        // A record the tool must not write is left out even when the dump
        // holds it.
        if SKIPPED
            .iter()
            .any(|skipped| skipped.eq_ignore_ascii_case(&entry.id))
        {
            continue;
        }

        // A record whose block carries no id is written to a raw address,
        // which this command cannot express.
        let Some(partition) = blocks
            .get(&entry.id.to_ascii_lowercase())
            .and_then(|file| file.partition.as_deref())
        else {
            continue;
        };

        writes.push_str(&format!(" w {partition} {}", entry.name));
    }

    let name = sanitize(base);
    let name = if name.is_empty() {
        "spd_dump".to_string()
    } else {
        name
    };

    // With nothing to write, the flash script would only repartition and
    // restart: the reset script is the one that describes such a run.
    let flash = (!writes.is_empty() && extracted).then(|| {
        let command = format!(
            "spd_dump {OPTIONS}{loaders} exec repartition {configuration}{writes} reset"
        );

        (
            format!("{name}_spd_dump.{}", shell.extension()),
            script(shell, &command, &format!("flash {name}")),
        )
    });

    let mut reset = format!("spd_dump {OPTIONS}{loaders} exec");

    for partition in ERASED {
        reset.push_str(&format!(" e {partition}"));
    }

    reset.push_str(" reset");

    Ok(Scripts {
        flash,
        reset: (
            format!("{name}_factory_reset.{}", shell.extension()),
            script(shell, &reset, &format!("erase the data of {name}")),
        ),
    })
}

/// The `fdl` arguments of the command: each download agent with the address its
/// block is loaded at, which only the configuration XML knows.
fn loaders(
    package: &pac::Package,
    blocks: &HashMap<String, &scatter::File>,
) -> Result<String, SpdError> {
    let mut arguments = String::new();

    for id in LOADERS {
        let Some(index) = loader_index(package, id) else {
            return Err(SpdError::Loader(id.to_string()));
        };

        let address = blocks
            .get(&id.to_ascii_lowercase())
            .and_then(|file| file.addresses.first());

        match address {
            Some(address) => {
                arguments.push_str(&format!(" fdl {} 0x{address:x}", package.files[index].name));
            }
            None => return Err(SpdError::Loader(id.to_string())),
        }
    }

    Ok(arguments)
}

/// One script around a single tool command: what it does, the command, and the
/// line that keeps the window open when it is started by double clicking it.
fn script(shell: Shell, command: &str, subject: &str) -> String {
    let comment = match shell {
        Shell::Cmd => "REM",
        Shell::Sh => "#",
    };

    let mut out = String::new();

    match shell {
        Shell::Cmd => out.push_str("@echo off\n"),
        Shell::Sh => out.push_str("#!/bin/sh\n"),
    }

    out.push_str(&format!("{comment} LMN Flash: {subject} with spd_dump.\n"));
    out.push_str(&format!(
        "{comment} Run this from the folder the package was unpacked into.\n"
    ));
    out.push('\n');
    out.push_str(command);
    out.push('\n');
    out.push('\n');
    out.push_str(&format!(
        "{comment} Wait for the last line before closing the window.\n"
    ));

    match shell {
        Shell::Cmd => out.push_str("pause\n"),
        Shell::Sh => out.push_str("printf 'Press Enter to close...'\nread -r _\n"),
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A package of a Unisoc Sharkle firmware, with the records a real one
    /// holds: the two download agents, the images, a record that must not be
    /// written, and the configuration XML itself.
    fn package() -> pac::Package {
        let file = |id: &str, name: &str| pac::Entry {
            id: id.to_string(),
            name: name.to_string(),
            size: 1024,
            offset: 0,
            complete: true,
        };

        let record = |id: &str, name: &str| pac::Record {
            id: id.to_string(),
            name: name.to_string(),
            flag: 1,
            kind: 1,
            omit: 0,
            addresses: Vec::new(),
        };

        pac::Package {
            header: pac::Header {
                version: "BP_R2.0.1".to_string(),
                product_name: "s9863a3c10".to_string(),
                product_version: String::new(),
                product_alias: "s9863a3c10".to_string(),
                mode: 0,
                flash_type: 1,
                nand_strategy: 0,
                nand_page_type: 0,
                nv_backup: 1,
                oma_dm_product_flag: 0,
                oma_dm: 1,
                is_preload: 1,
            },
            files: vec![
                file("FDL", "fdl1-sign.bin"),
                file("FDL2", "fdl2-sign.bin"),
                file("NV_LTE", "sharkl3_pubcp_customer_nvitem.bin"),
                file("ProdNV", "prodnv.img"),
                file("ELABLE", "elable.img"),
                file("Modem_LTE", "SC9600_sharkl3_pubcp_modem.dat"),
                file("DFS", "sharkl3_cm4.bin"),
                file("Modem_WCN", "PM_sharkl3_cm4_v2.bin"),
                file("BOOT", "boot.img"),
                file("Super", "super.img"),
                file("UserData", "userdata.img"),
                file("", "s9863a3c10_gamoraplus.xml"),
            ],
            records: vec![
                record("FDL", "fdl1-sign.bin"),
                record("FDL2", "fdl2-sign.bin"),
                record("", "s9863a3c10_gamoraplus.xml"),
            ],
            expected_size: 0,
            actual_size: 0,
            complete: true,
        }
    }

    /// The records of that package, the way the dialog passes them: every file
    /// is checked.
    fn all() -> Vec<usize> {
        (0..package().files.len()).collect()
    }

    /// The same, for a package that was built by a test of its own.
    fn all_of(package: &pac::Package) -> Vec<usize> {
        (0..package.files.len()).collect()
    }

    const XML: &str = r#"<?xml version="1.0"?>
<BMAConfig>
  <ProductList>
    <Product name="s9863a3c10"><SchemeName>s9863a3c10</SchemeName><FlashTypeID>1</FlashTypeID><Mode>0</Mode></Product>
  </ProductList>
  <SchemeList>
    <Scheme name="s9863a3c10">
      <File><ID>FDL</ID><Block><Base>0x00005000</Base></Block><Flag>1</Flag><CheckFlag>1</CheckFlag></File>
      <File><ID>FDL2</ID><Block><Base>0x9EFFFE00</Base></Block><Flag>1</Flag><CheckFlag>1</CheckFlag></File>
      <File><ID>NV_LTE</ID><Block id="l_fixnv1"><Base>0x0</Base></Block><Flag>1</Flag><CheckFlag>2</CheckFlag></File>
      <File><ID>Modem_LTE</ID><Block id="l_modem"><Base>0x0</Base></Block><Flag>1</Flag><CheckFlag>2</CheckFlag></File>
      <File><ID>DFS</ID><Block id="pm_sys"><Base>0x0</Base></Block><Flag>1</Flag><CheckFlag>2</CheckFlag></File>
      <File><ID>Modem_WCN</ID><Block id="wcnmodem"><Base>0x0</Base></Block><Flag>1</Flag><CheckFlag>2</CheckFlag></File>
      <File><ID>BOOT</ID><Block id="boot"><Base>0x0</Base></Block><Flag>1</Flag><CheckFlag>0</CheckFlag></File>
      <File><ID>Super</ID><Block id="super"><Base>0x0</Base></Block><Flag>1</Flag><CheckFlag>0</CheckFlag></File>
    </Scheme>
  </SchemeList>
</BMAConfig>
"#;

    /// The one line of a script that runs the tool.
    fn command(text: &str) -> String {
        text.lines()
            .find(|line| line.starts_with("spd_dump "))
            .expect("the command")
            .to_string()
    }

    /// The name and the command of a run's flash script.
    fn flash(scripts: &Scripts) -> (String, String) {
        let (name, text) = scripts.flash.as_ref().expect("the flash script");

        (name.clone(), command(text))
    }

    #[test]
    fn writes_the_firmware_the_configuration_describes() {
        let package = package();
        let scripts = scripts(&package, &all(), XML, Shell::Cmd, "s9863a3c10").expect("scripts");
        let (name, flash) = flash(&scripts);

        assert_eq!(
            flash,
            "spd_dump --wait 32767 skip_confirm 1 \
             fdl fdl1-sign.bin 0x5000 fdl fdl2-sign.bin 0x9efffe00 \
             exec repartition s9863a3c10_gamoraplus.xml \
             w l_modem SC9600_sharkl3_pubcp_modem.dat \
             w pm_sys sharkl3_cm4.bin \
             w wcnmodem PM_sharkl3_cm4_v2.bin \
             w boot boot.img \
             w super super.img reset"
        );

        // The files of the unit the dump came from, and the ones the phone
        // makes again on its own, are never written.
        for left_out in [
            "prodnv.img",
            "elable.img",
            "sharkl3_pubcp_customer_nvitem.bin",
            "userdata.img",
        ] {
            assert!(!flash.contains(left_out), "{left_out} is written");
        }

        // The configuration XML is named by `repartition`, which is not a
        // partition it could be written to.
        assert!(!flash.contains(" w s9863a3c10_gamoraplus.xml"));

        assert_eq!(name, "s9863a3c10_spd_dump.cmd");
        assert_eq!(scripts.reset.0, "s9863a3c10_factory_reset.cmd");

        let text = &scripts.flash.as_ref().expect("the flash script").1;
        assert!(text.starts_with("@echo off\n"));
        assert!(text.ends_with("pause\n"));
    }

    #[test]
    fn the_reset_script_erases_what_belongs_to_the_user() {
        let package = package();
        let scripts = scripts(&package, &all(), XML, Shell::Cmd, "s9863a3c10").expect("scripts");

        assert_eq!(
            command(&scripts.reset.1),
            "spd_dump --wait 32767 skip_confirm 1 \
             fdl fdl1-sign.bin 0x5000 fdl fdl2-sign.bin 0x9efffe00 \
             exec e persist e userdata reset"
        );
    }

    #[test]
    fn a_posix_script_is_a_shell_script() {
        let package = package();
        let scripts = scripts(&package, &all(), XML, Shell::Sh, "s9863a3c10").expect("scripts");
        let (name, text) = scripts.flash.as_ref().expect("the flash script");

        assert_eq!(name, "s9863a3c10_spd_dump.sh");
        assert!(text.starts_with("#!/bin/sh\n"));
        assert!(text.contains("# LMN Flash:"));
        assert!(text.ends_with("printf 'Press Enter to close...'\nread -r _\n"));
        assert!(!text.contains("pause"));
    }

    #[test]
    fn only_the_files_the_run_writes_are_in_the_command() {
        let package = package();

        // The user unchecked `super`, so the extraction does not write it and
        // the script does not flash it.
        let selected: Vec<usize> = all()
            .into_iter()
            .filter(|&index| package.files[index].id != "Super")
            .collect();

        let scripts = scripts(&package, &selected, XML, Shell::Cmd, "s9863a3c10").expect("scripts");
        let (_, flash) = flash(&scripts);

        assert!(!flash.contains("super.img"));
        assert!(flash.contains("w boot boot.img"));
    }

    /// With only the download agents checked there is nothing to write: the
    /// run can still reset the phone, so that is the only script it gets.
    #[test]
    fn the_flash_script_needs_something_to_write() {
        let package = package();

        // `FDL` and `FDL2` are the first two files of the package.
        let scripts = scripts(&package, &[0, 1], XML, Shell::Cmd, "s9863a3c10").expect("scripts");

        assert!(scripts.flash.is_none());
        assert_eq!(
            command(&scripts.reset.1),
            "spd_dump --wait 32767 skip_confirm 1 \
             fdl fdl1-sign.bin 0x5000 fdl fdl2-sign.bin 0x9efffe00 \
             exec e persist e userdata reset"
        );
    }

    /// The command repartitions from the package's configuration XML, which is
    /// only there when the user checked it.
    #[test]
    fn the_flash_script_needs_the_configuration_xml() {
        let package = package();

        let selected: Vec<usize> = all()
            .into_iter()
            .filter(|&index| !package.files[index].name.ends_with(".xml"))
            .collect();

        let scripts = scripts(&package, &selected, XML, Shell::Cmd, "s9863a3c10").expect("scripts");

        assert!(scripts.flash.is_none());
        assert!(scripts.reset.1.contains("exec e persist e userdata reset"));
    }

    /// The option can only be offered while both download agents are checked:
    /// the scripts name their files.
    #[test]
    fn the_scripts_need_both_download_agents_checked() {
        let package = package();

        let checked = |indexes: &[usize]| {
            let mut flags = vec![false; package.files.len()];

            for &index in indexes {
                flags[index] = true;
            }

            flags
        };

        assert!(can_generate(&package, &checked(&[0, 1])));
        assert!(!can_generate(&package, &checked(&[0])));
        assert!(!can_generate(&package, &checked(&[1])));

        // A package without a configuration XML has neither partitions nor
        // loader addresses, whatever is checked.
        let mut anonymous = package.clone();
        anonymous.records.clear();

        assert!(!can_generate(&anonymous, &checked(&[0, 1])));
    }

    /// A run of nothing but the download agents is the one the flash
    /// configuration is left out for.
    #[test]
    fn a_run_of_nothing_but_the_download_agents_is_recognised() {
        let package = package();

        // `FDL` and `FDL2` are the first two files of the package. Either one
        // of them on its own is still only a download agent.
        assert!(only_loaders(&package, &[0, 1]));
        assert!(only_loaders(&package, &[1, 0]));
        assert!(only_loaders(&package, &[0]));

        // Anything else is a normal extraction again, and an empty run has
        // nothing to describe either.
        assert!(!only_loaders(&package, &[]));
        assert!(!only_loaders(&package, &[0, 2]));
        assert!(!only_loaders(&package, &all_of(&package)));

        // An index the package does not have is not a download agent either.
        assert!(!only_loaders(&package, &[0, package.files.len()]));
    }

    #[test]
    fn a_record_the_xml_does_not_name_is_left_out() {
        let mut package = package();

        // `USERDATA` is skipped, but a record the XML says nothing about is
        // left out for the same reason: there is no partition to write it to.
        package.files.push(pac::Entry {
            id: "Obscure".to_string(),
            name: "obscure.bin".to_string(),
            size: 1,
            offset: 0,
            complete: true,
        });

        let scripts = scripts(&package, &all(), XML, Shell::Cmd, "s9863a3c10").expect("scripts");

        assert!(!command(&flash(&scripts).1).contains("obscure.bin"));
    }

    #[test]
    fn a_package_without_its_configuration_has_no_scripts() {
        let mut package = package();
        package.records.clear();

        let error = scripts(&package, &all(), "", Shell::Cmd, "s9863a3c10").expect_err("error");

        assert!(matches!(error, SpdError::Xml(_)), "{error:?}");
    }

    #[test]
    fn a_package_without_a_loader_has_no_scripts() {
        let mut package = package();
        package.files.retain(|entry| entry.id != "FDL2");

        let error = scripts(&package, &all(), XML, Shell::Cmd, "s9863a3c10").expect_err("error");

        assert_eq!(error, SpdError::Loader("FDL2".to_string()));
    }

    /// The example the feature was asked for: a scattered `s9863a3c10`
    /// firmware (the configuration of the reference tree and its XML), whose
    /// images are the files the configuration names.
    #[test]
    #[ignore = "needs reference/s9863a3c10_gamoraplus.xml"]
    fn writes_the_firmware_of_the_second_reference_product() {
        let xml = std::fs::read_to_string("reference/s9863a3c10_gamoraplus.xml").expect("read xml");
        let config = std::fs::read_to_string("reference/flash_gamoraplus.cfg").expect("read cfg");

        // The configuration names the file of every record: `ID=<flag>@<file>`.
        let mut files = Vec::new();
        let mut records = Vec::new();

        for line in config.lines() {
            let Some((id, value)) = line.split_once('=') else {
                continue;
            };

            let name = value.split_once('@').map(|(_, name)| name.trim()).unwrap_or("");

            if id.starts_with('[') || name.is_empty() {
                continue;
            }

            files.push(pac::Entry {
                id: id.to_string(),
                name: name.to_string(),
                size: 1024,
                offset: 0,
                complete: true,
            });

            records.push(pac::Record {
                id: id.to_string(),
                name: name.to_string(),
                flag: 1,
                kind: 1,
                omit: 0,
                addresses: Vec::new(),
            });
        }

        // The configuration XML is stored as a record of its own, and is one of
        // the files the run writes out.
        for (id, name) in [("", "s9863a3c10.xml")] {
            files.push(pac::Entry {
                id: id.to_string(),
                name: name.to_string(),
                size: 1024,
                offset: 0,
                complete: true,
            });

            records.push(pac::Record {
                id: id.to_string(),
                name: name.to_string(),
                flag: 1,
                kind: 2,
                omit: 0,
                addresses: Vec::new(),
            });
        }

        let package = pac::Package {
            header: package().header,
            files,
            records,
            expected_size: 0,
            actual_size: 0,
            complete: true,
        };

        let scripts = scripts(&package, &all_of(&package), &xml, Shell::Cmd, "s9863a3c10")
            .expect("scripts");
        let flash = command(&scripts.flash.as_ref().expect("the flash script").1);

        // The head is the loader chain and the repartition of the reference
        // XML, which the package stores as `s9863a3c10.xml`.
        assert!(
            flash.starts_with(
                "spd_dump --wait 32767 skip_confirm 1 \
                 fdl fdl1-sign.bin 0x5000 fdl fdl2-sign.bin 0x9efffe00 \
                 exec repartition s9863a3c10.xml "
            ),
            "{flash}"
        );

        // Every record of that firmware is written to the block its own XML
        // names, with the file the configuration gives it.
        for pair in [
            "w super super.img",
            "w vbmeta vbmeta-sign.img",
            "w vbmeta_system vbmeta_system.img",
            "w vbmeta_vendor vbmeta_vendor.img",
            "w vbmeta_custom1 vbmeta_custom1.img",
            "w l_modem SC9600_sharkl3_pubcp_modem.dat",
            "w l_deltanv sharkl3_pubcp_customer_deltanv.bin",
            "w l_ldsp sharkl3_pubcp_LTEA_DSP.bin",
            "w l_gdsp sharkl3_pubcp_DM_DSP.bin",
            "w pm_sys sharkl3_cm4.bin",
            "w gpsgl gnssmodem.bin",
            "w gpsbd gnssbdmodem.bin",
            "w wcnmodem PM_sharkl3_cm4_v2.bin",
            "w boot boot.img",
            "w dtbo dtbo.img",
            "w recovery recovery.img",
            "w logo logo.bin",
            "w fbootlogo LOGO.bmp",
            "w cache cache.img",
            "w trustos tos-sign.bin",
            "w teecfg teecfg-sign.bin",
            "w sml sml-sign.bin",
            "w persist persist.img",
            "w uboot u-boot-sign.bin",
            "w splloader u-boot-spl-16k-sign.bin",
            "w socko socko.img",
            "w odmko odmko.img",
        ] {
            assert!(flash.contains(pair), "{pair} is missing");
        }

        assert!(flash.ends_with(" reset"), "{flash}");
    }
}
