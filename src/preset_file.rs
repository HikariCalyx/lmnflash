//! Saving and loading the Multi Flashing preset list as JSON.
//!
//! The preset list is the one part of the page a user builds up by hand, so it
//! can be written to a file and read back later. The file holds what a preset
//! is *made of*, not what its package says about itself: the package is
//! identified again while the file is read, so firmware that was replaced,
//! moved or deleted is caught here instead of during a run, and the checklist
//! the entry offers is always the one the package really has.
//!
//! ```json
//! {
//!   "version": 1,
//!   "presets": [
//!     {
//!       "path": "D:\\firmware\\arcfox.zip",
//!       "extract_to": "D:\\firmware\\arcfox",
//!       "enabled": "0x5",
//!       "reboot": true,
//!       "verify_checksums": false,
//!       "tool": { "kind": "shipped", "version": "34.0.4" },
//!       "conditions": {
//!         "kind": "custom",
//!         "cids": ["0x0032"],
//!         "states": ["flashing_unlocked"]
//!       }
//!     }
//!   ]
//! }
//! ```
//!
//! `enabled` is a hex bit mask rather than one flag per step: the binary value
//! is the checklist read BACKWARDS, i.e. the first step is the lowest bit, so
//! the `0x5` of the example (`0b101`) means steps 1 and 3 run and step 2 does
//! not.
//!
//! Bootloader unlock states are spelled the way a bootloader reports them
//! (`oem_locked`, `flashing_unlocked`, …), so the file stays readable without
//! knowing this program's internals.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fastboot_info::SecureState;
use crate::flashfile;
use crate::{FlashConditions, MultiFlashPreset, PresetTool};

/// The layout this release writes. A file of a higher version is refused
/// instead of being misread.
pub(crate) const VERSION: u32 = 1;

/// What the save dialog suggests as the file name.
pub(crate) const DEFAULT_FILE_NAME: &str = "multi-flash-presets.json";

/// The extension both preset file dialogs filter for.
pub(crate) const EXTENSIONS: [&str; 1] = ["json"];

/// Why a preset file could not be read or written.
///
/// The two cases a user has to act on are told apart from the technical text
/// the file system and the JSON parser produce, so the dialog can translate
/// them (like [`flashfile::LoadError`] does for a firmware package).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FileError {
    /// Not JSON, or not the shape this program writes.
    NotAPresetFile,
    /// Written by a newer release, whose entries may mean other things.
    Newer(u32),
    /// The parser or the file system said no.
    Other(String),
}

impl FileError {
    /// The English wording, for the log and for a file that is not ours.
    pub(crate) fn message(&self) -> String {
        match self {
            Self::NotAPresetFile => "not a preset file written by LMN Flash".to_owned(),
            Self::Newer(version) => {
                format!("written by a newer version of LMN Flash (file version {version})")
            }
            Self::Other(message) => message.clone(),
        }
    }
}

impl std::fmt::Display for FileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl From<String> for FileError {
    fn from(message: String) -> Self {
        Self::Other(message)
    }
}

/// The presets a file turned into.
#[derive(Debug, Clone)]
pub(crate) struct Loaded {
    /// The file they came from.
    pub path: PathBuf,
    pub presets: Vec<MultiFlashPreset>,
    /// The entries that were left out: their firmware package could not be
    /// read any more, or they name something this release does not know.
    pub skipped: Vec<PathBuf>,
}

/// Writes the list to `path`.
pub(crate) fn save(presets: &[MultiFlashPreset], path: &Path) -> Result<(), FileError> {
    let file = PresetFile {
        version: VERSION,
        presets: presets.iter().map(SavedPreset::from).collect(),
    };

    let text = serde_json::to_string_pretty(&file)
        .map_err(|error| FileError::Other(error.to_string()))?;

    std::fs::write(path, text)
        .map_err(|error| FileError::Other(format!("{}: {error}", path.display())))
}

/// Reads a list back, identifying every firmware package again.
pub(crate) fn load(path: &Path) -> Result<Loaded, FileError> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| FileError::Other(format!("{}: {error}", path.display())))?;

    let file: PresetFile = serde_json::from_str(&text).map_err(|error| {
        // Not JSON, or JSON of another shape: either way this is not one of
        // our files. A file that is cut short keeps the parser's own wording,
        // which says more than "not a preset file" would.
        if error.is_syntax() || error.is_data() {
            FileError::NotAPresetFile
        } else {
            FileError::Other(error.to_string())
        }
    })?;

    if file.version > VERSION {
        return Err(FileError::Newer(file.version));
    }

    let mut presets = Vec::with_capacity(file.presets.len());
    let mut skipped = Vec::new();

    for saved in file.presets {
        let path = saved.path.clone();

        match saved.into_preset() {
            Some(preset) => presets.push(preset),
            None => skipped.push(path),
        }
    }

    Ok(Loaded {
        path: path.to_path_buf(),
        presets,
        skipped,
    })
}

/// The whole file: a version, and the presets it holds.
#[derive(Debug, Serialize, Deserialize)]
struct PresetFile {
    version: u32,
    presets: Vec<SavedPreset>,
}

/// One preset as it is stored: where its package is and what to do with it.
///
/// Every field that was added after the first release carries a default, so a
/// file written by an older build still reads.
#[derive(Debug, Serialize, Deserialize)]
struct SavedPreset {
    /// The firmware package: the factory ZIP, or the `flashfile.xml` of a
    /// package unpacked next to its images.
    path: PathBuf,
    /// Where a ZIP package's images are unpacked (nothing for a package that
    /// is already unpacked).
    #[serde(default)]
    extract_to: Option<PathBuf>,
    /// Which procedures run, as a hex bit mask (see [`EnabledMask`]).
    #[serde(default)]
    enabled: EnabledMask,
    /// Reboot the device once this preset finished.
    #[serde(default)]
    reboot: bool,
    /// Verify the `MD5` of every image before writing it.
    #[serde(default = "yes")]
    verify_checksums: bool,
    /// The tool this preset's run uses.
    #[serde(default)]
    tool: SavedTool,
    /// Which devices this firmware may be flashed to.
    #[serde(default)]
    conditions: SavedConditions,
    /// Whether the default rule takes an engineering unit as well. A file
    /// written before the choice existed carries no field for it, which means
    /// it did — the way the rule always worked.
    #[serde(default)]
    no_engineering: bool,
}

/// The default of [`SavedPreset::verify_checksums`]: a preset that says
/// nothing about it is verified, like the single-device dialog does.
fn yes() -> bool {
    true
}

impl SavedPreset {
    /// The preset this entry describes, or `None` when it cannot be loaded.
    ///
    /// The package is read again here — a saved path is only a hint, and the
    /// checklist has to fit the procedures the package really has now.
    fn into_preset(self) -> Option<MultiFlashPreset> {
        let identity = flashfile::identify(&self.path).ok()?;
        // A mask that is not a number is not something this program wrote, so
        // the entry is left out rather than being read as "run everything".
        let enabled = self.enabled.flags()?;

        Some(MultiFlashPreset {
            path: self.path,
            enabled: fit(enabled, identity.steps.len()),
            identity,
            reboot: self.reboot,
            verify_checksums: self.verify_checksums,
            tool: self.tool.into(),
            // The directory is taken as it stands: whether the files are still
            // in it is only known when a run reads them, and the card offers
            // to pick it again.
            extract_to: self.extract_to.filter(|directory| directory.is_dir()),
            conditions: self.conditions.into(),
            no_engineering: self.no_engineering,
        })
    }
}

impl From<&MultiFlashPreset> for SavedPreset {
    fn from(preset: &MultiFlashPreset) -> Self {
        Self {
            path: preset.path.clone(),
            enabled: EnabledMask::of(&preset.enabled),
            reboot: preset.reboot,
            verify_checksums: preset.verify_checksums,
            tool: (&preset.tool).into(),
            extract_to: preset.extract_to.clone(),
            // A condition too incomplete to match anything is still written as
            // it stands: it is what the entry in the list says.
            conditions: (&preset.conditions).into(),
            no_engineering: preset.no_engineering,
        }
    }
}

/// Which of a package's procedures run, as the file spells it.
///
/// The mask is as short as the checklist needs (`0x1F` for five checked
/// steps, `0x03` when the last three of five are skipped), so a long procedure
/// list does not fill the file with one word per step.
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum EnabledMask {
    /// The hex mask: the binary value is the checklist read backwards, i.e.
    /// the first step is the lowest bit.
    Mask(String),
    /// The list of booleans this file used before the mask; still read, so a
    /// file saved that way is not lost.
    Flags(Vec<bool>),
}

impl Default for EnabledMask {
    /// A mask with no digits describes no steps at all, which the checklist
    /// reads as "every step runs" (its rule for a step the file does not
    /// mention).
    fn default() -> Self {
        Self::Mask(String::new())
    }
}

impl EnabledMask {
    /// The mask of a checklist: one hex digit per four steps, the first step
    /// of each group being that digit's lowest bit.
    fn of(enabled: &[bool]) -> Self {
        Self::Mask(encode_mask(enabled))
    }

    /// The checklist it holds, or `None` when it is not a mask this program
    /// writes.
    fn flags(self) -> Option<Vec<bool>> {
        match self {
            // `Vec<bool>` from a plain list needs no checks: serde already
            // made sure every entry is a boolean.
            Self::Flags(flags) => Some(flags),
            Self::Mask(mask) => decode_mask(&mask),
        }
    }
}

/// The hex mask of a checklist, padded to the digits the list needs.
///
/// Step `i` is bit `i`, so the digits are built four steps at a time from the
/// front and then written back to front: `[true, false, true]` is `0b101` =
/// `0x5`, and a list of six steps always takes two digits (`0x3F` for all
/// checked), so the width says how long the checklist was.
fn encode_mask(enabled: &[bool]) -> String {
    let digits: Vec<u8> = enabled
        .chunks(4)
        .map(|group| {
            group
                .iter()
                .enumerate()
                .fold(0u8, |digit, (bit, flag)| {
                    digit | u8::from(*flag) << bit
                })
        })
        .collect();

    let digits: String = digits
        .iter()
        .rev()
        .map(|digit| format!("{digit:X}"))
        .collect();

    format!("0x{digits}")
}

/// The checklist a hex mask describes, or `None` when it is not one.
///
/// The digits are read back to front, four steps each, so a mask written by
/// [`encode_mask`] returns exactly the list it came from. The mask covers a
/// whole number of digits, so the bit above the saved checklist's last step
/// are described as unchecked; [`fit`] later gives those steps the checklist's
/// own rule (a step the file does not mention runs) for the package as it is
/// now.
fn decode_mask(mask: &str) -> Option<Vec<bool>> {
    let mask = mask.trim();
    let digits = mask
        .strip_prefix("0x")
        .or_else(|| mask.strip_prefix("0X"))
        .unwrap_or(mask);

    let mut enabled: Vec<bool> = Vec::with_capacity(digits.len() * 4);

    for digit in digits.chars().rev() {
        let value = digit.to_digit(16)?;

        for bit in 0..4 {
            enabled.push(value & (1 << bit) != 0);
        }
    }

    Some(enabled)
}

/// Makes saved step flags fit the package as it is now: flags for steps that
/// are gone are dropped, and a step the package gained since counts as
/// checked (the rule the checklist itself follows).
fn fit(mut enabled: Vec<bool>, steps: usize) -> Vec<bool> {
    enabled.truncate(steps);
    enabled.resize(steps, true);
    enabled
}

/// The tool a preset runs with, as the file names it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SavedTool {
    /// The built-in `fastboot` crate.
    #[default]
    Builtin,
    /// A shipped `mfastboot` build, named by the version it reports.
    Shipped { version: String },
    /// Google's `fastboot` from the Android platform-tools.
    PlatformTools,
}

impl From<&PresetTool> for SavedTool {
    fn from(tool: &PresetTool) -> Self {
        match tool {
            PresetTool::Builtin => Self::Builtin,
            PresetTool::Shipped(version) => Self::Shipped {
                version: version.clone(),
            },
            PresetTool::PlatformTools => Self::PlatformTools,
        }
    }
}

impl From<SavedTool> for PresetTool {
    fn from(tool: SavedTool) -> Self {
        match tool {
            SavedTool::Builtin => Self::Builtin,
            SavedTool::Shipped { version } => Self::Shipped(version),
            SavedTool::PlatformTools => Self::PlatformTools,
        }
    }
}

/// The conditions a preset accepts, as the file names them.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SavedConditions {
    /// The rule the single-device dialog applies.
    #[default]
    Default,
    /// The user's own list of CIDs and bootloader unlock states.
    Custom {
        cids: Vec<String>,
        states: Vec<String>,
    },
}

impl From<&FlashConditions> for SavedConditions {
    fn from(conditions: &FlashConditions) -> Self {
        match conditions {
            FlashConditions::Default => Self::Default,
            FlashConditions::Custom { cids, states } => Self::Custom {
                cids: cids.clone(),
                states: states
                    .iter()
                    .map(|state| state.raw().to_owned())
                    .collect(),
            },
        }
    }
}

impl From<SavedConditions> for FlashConditions {
    fn from(conditions: SavedConditions) -> Self {
        match conditions {
            SavedConditions::Default => Self::Default,
            // A state this release does not know cannot be honoured, so an
            // entry naming one is treated as a condition that matches nothing
            // rather than as one that matches too much.
            SavedConditions::Custom { cids, states } => Self::Custom {
                cids,
                states: states
                    .iter()
                    .filter_map(|state| SecureState::parse(state))
                    .collect(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flashfile::PackageIdentity;

    fn preset(path: &str) -> MultiFlashPreset {
        MultiFlashPreset {
            path: PathBuf::from(path),
            identity: PackageIdentity::default(),
            enabled: vec![true, false],
            reboot: true,
            verify_checksums: false,
            tool: PresetTool::Shipped("34.0.4".to_owned()),
            extract_to: Some(PathBuf::from(r"D:\firmware\arcfox")),
            conditions: FlashConditions::Custom {
                cids: vec!["0x0032".to_owned()],
                states: vec![SecureState::Unlocked, SecureState::Engineering],
            },
            no_engineering: false,
        }
    }

    /// A small three-step package: `flash boot`, `erase userdata`, `flash
    /// super`. It is a real `flashfile.xml`, so `load` identifies it for real.
    fn package_in(directory: &Path) -> PathBuf {
        let package = directory.join("flashfile.xml");
        std::fs::write(
            &package,
            r#"<flashing><header><phone_model model="arcfox"/></header><steps>
               <step operation="flash" partition="boot" filename="boot.img"/>
               <step operation="erase" partition="userdata"/>
               <step operation="flash" partition="super" filename="super.img"/>
               </steps></flashing>"#,
        )
        .unwrap();

        package
    }

    /// A file written by this release reads back the same way: the JSON is the
    /// whole round trip, and nothing of the options is lost on the way.
    #[test]
    fn a_saved_list_reads_back() {
        let directory = std::env::temp_dir().join("lmnflash-preset-file-test");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("presets.json");
        let package = package_in(&directory);

        let mut saved = preset(&package.display().to_string());
        saved.enabled = vec![true, false, true];
        saved.tool = PresetTool::PlatformTools;
        // A directory that is still there is kept as it was saved.
        saved.extract_to = Some(directory.clone());

        save(std::slice::from_ref(&saved), &path).unwrap();

        // The checklist is the mask of `[true, false, true]`: `0b101`.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"enabled\": \"0x5\""), "{text}");

        let loaded = load(&path).unwrap();

        assert!(loaded.skipped.is_empty());
        assert_eq!(loaded.presets.len(), 1);

        let entry = &loaded.presets[0];
        assert_eq!(entry.path, package);
        assert_eq!(entry.enabled, vec![true, false, true]);
        assert!(entry.reboot);
        assert!(!entry.verify_checksums);
        assert_eq!(entry.tool, PresetTool::PlatformTools);
        assert_eq!(entry.extract_to, saved.extract_to);
        assert_eq!(entry.conditions, saved.conditions);        // What the package says about itself comes from the package, not the
        // file: the identity is read again.
        assert_eq!(entry.identity.model.as_deref(), Some("arcfox"));
        assert_eq!(entry.identity.steps.len(), 3);

        std::fs::remove_file(&path).ok();
        std::fs::remove_file(&package).ok();
    }

    /// The checklist is a hex mask whose binary value is the list read
    /// backwards: the first step is the lowest bit, and four steps are one
    /// digit.
    #[test]
    fn the_checklist_is_a_backwards_binary_mask() {
        assert_eq!(encode_mask(&[true]), "0x1");
        assert_eq!(encode_mask(&[true, true, false]), "0x3");
        assert_eq!(encode_mask(&[true, false, true]), "0x5");
        assert_eq!(encode_mask(&[true, true, true, true]), "0xF");
        assert_eq!(encode_mask(&[true, true, true, true, true]), "0x1F");
        assert_eq!(encode_mask(&[false, false, false]), "0x0");
        // The digits are built from the front, so a checked step in the second
        // group is the low bit of the higher digit.
        assert_eq!(encode_mask(&[false, false, false, false, true, true]), "0x30");
        assert_eq!(encode_mask(&[]), "0x");
    }

    /// A mask reads back as the list it was made from, and one that is not a
    /// number at all is refused instead of being read as "run everything".
    #[test]
    fn the_mask_reads_back() {
        for enabled in [
            vec![],
            vec![true],
            vec![false, false, true, false, true],
            vec![true; 13],
        ] {
            let mask = encode_mask(&enabled);
            let mut flags = decode_mask(&mask).unwrap();
            flags.truncate(enabled.len());

            assert_eq!(flags, enabled, "{mask}");
        }

        assert!(decode_mask("0xZZ").is_none());
        // The prefix and the case are optional, and so is surrounding space.
        assert_eq!(
            decode_mask(" 0xf ").unwrap()[..4],
            [true, true, true, true]
        );
        assert_eq!(decode_mask("3").unwrap()[..4], [true, true, false, false]);
        assert_eq!(decode_mask("0x").unwrap(), Vec::<bool>::new());
    }

    /// An entry whose mask is not a number is left out (and its path is
    /// reported) instead of being loaded as one that runs everything.
    #[test]
    fn an_entry_with_a_broken_mask_is_left_out() {
        let directory = std::env::temp_dir().join("lmnflash-preset-file-broken-mask");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("presets.json");
        let package = package_in(&directory);

        let text = format!(
            r#"{{"version": 1, "presets": [{{"path": {}, "enabled": "0xZZ"}}]}}"#,
            serde_json::to_string(&package.display().to_string()).unwrap()
        );
        std::fs::write(&path, text).unwrap();

        let loaded = load(&path).unwrap();

        assert!(loaded.presets.is_empty());
        assert_eq!(loaded.skipped, vec![package.clone()]);

        std::fs::remove_file(&path).ok();
        std::fs::remove_file(&package).ok();
    }

    /// An entry whose package is not there any more is left out instead of
    /// being loaded as one a run could not flash.
    #[test]
    fn a_missing_package_is_left_out() {
        let directory = std::env::temp_dir().join("lmnflash-preset-file-missing");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("presets.json");

        let saved = preset(r"D:\nowhere\gone.zip");
        save(std::slice::from_ref(&saved), &path).unwrap();

        let loaded = load(&path).unwrap();

        assert!(loaded.presets.is_empty());
        assert_eq!(loaded.skipped, vec![PathBuf::from(r"D:\nowhere\gone.zip")]);

        std::fs::remove_file(&path).ok();
    }

    /// The flags fit the procedures the package has now: a step that was added
    /// since the file was written counts as checked, and the flags of steps
    /// that are gone are dropped.
    #[test]
    fn the_step_flags_fit_the_package() {
        assert_eq!(fit(vec![true, false, true], 2), vec![true, false]);
        assert_eq!(fit(vec![false], 3), vec![false, true, true]);
        assert_eq!(fit(Vec::new(), 0), Vec::<bool>::new());
    }

    /// A directory that is not there any more is dropped from the preset, so
    /// the card asks for one again instead of holding a path nothing is
    /// behind. The file itself keeps what was saved.
    #[test]
    fn a_directory_that_is_gone_is_dropped() {
        let directory = std::env::temp_dir().join("lmnflash-preset-file-gone");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("presets.json");
        let package = package_in(&directory);

        let gone = directory.join("moved-away");
        let _ = std::fs::remove_dir_all(&gone);

        let mut saved = preset(&package.display().to_string());
        saved.extract_to = Some(gone);

        save(std::slice::from_ref(&saved), &path).unwrap();

        let loaded = load(&path).unwrap();

        assert_eq!(loaded.presets.len(), 1);
        assert!(loaded.presets[0].extract_to.is_none());
        assert!(
            std::fs::read_to_string(&path).unwrap().contains("moved-away"),
            "the saved directory stays in the file"
        );

        std::fs::remove_file(&path).ok();
        std::fs::remove_file(&package).ok();
    }

    /// A file that still spells the checklist as one flag per step is read as
    /// well, so a file saved before the mask existed is not lost — and the
    /// keys it does not carry take the defaults (verify, built-in tool, the
    /// default conditions).
    #[test]
    fn a_list_of_flags_is_still_read() {
        let directory = std::env::temp_dir().join("lmnflash-preset-file-flags");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("presets.json");
        let package = package_in(&directory);

        let text = format!(
            r#"{{"version": 1, "presets": [{{"path": {}, "enabled": [true, false, true]}}]}}"#,
            serde_json::to_string(&package.display().to_string()).unwrap()
        );
        std::fs::write(&path, text).unwrap();

        let loaded = load(&path).unwrap();

        assert!(loaded.skipped.is_empty());
        assert_eq!(loaded.presets.len(), 1);

        let entry = &loaded.presets[0];
        assert_eq!(entry.enabled, vec![true, false, true]);
        assert!(entry.verify_checksums);
        assert!(!entry.reboot);
        // A key the file does not carry leaves the preset without a directory:
        // the card asks for one when it is opened.
        assert!(entry.extract_to.is_none());
        assert_eq!(entry.tool, PresetTool::Builtin);
        assert_eq!(entry.conditions, FlashConditions::Default);
        // The default rule took an engineering unit when that file was
        // written, and still does.
        assert!(!entry.no_engineering);

        std::fs::remove_file(&path).ok();
        std::fs::remove_file(&package).ok();
    }

    /// The one preset option that changes who a firmware is flashed to is
    /// written and read back as it is.
    #[test]
    fn the_engineering_choice_is_written_and_read() {
        let directory = std::env::temp_dir().join("lmnflash-preset-file-engineering");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("presets.json");
        let package = package_in(&directory);

        let entry = MultiFlashPreset {
            conditions: FlashConditions::Default,
            no_engineering: true,
            ..preset(&package.display().to_string())
        };
        save(&[entry], &path).unwrap();

        // It is a key of the entry, not of the conditions: the rule itself is
        // still the default one.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(r#""no_engineering": true"#), "{text}");

        let loaded = load(&path).unwrap();
        assert!(loaded.skipped.is_empty());
        assert!(loaded.presets[0].no_engineering);
        assert_eq!(loaded.presets[0].conditions, FlashConditions::Default);

        std::fs::remove_file(&path).ok();
        std::fs::remove_file(&package).ok();
    }
    /// Anything that is not a preset file says so, rather than being read as
    /// an empty list (which would silently drop the list the user has).
    #[test]
    fn another_json_file_is_not_a_preset_file() {
        let directory = std::env::temp_dir().join("lmnflash-preset-file-other");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("other.json");

        // JSON of another shape, and a file that is not JSON at all.
        for text in [r#"{"hello": "world"}"#, "firmware flash log"] {
            std::fs::write(&path, text).unwrap();
            assert!(
                matches!(load(&path), Err(FileError::NotAPresetFile)),
                "{text}"
            );
        }

        // A file written by a future release is refused by its version.
        std::fs::write(
            &path,
            format!(r#"{{"version": {}, "presets": []}}"#, VERSION + 1),
        )
        .unwrap();
        assert!(matches!(load(&path), Err(FileError::Newer(version)) if version == VERSION + 1));

        std::fs::remove_file(&path).ok();
    }

    /// An unlock state is stored the way a bootloader spells it, so the file
    /// stays readable — and a state this release does not know is dropped
    /// instead of being turned into one that matches devices it should not.
    #[test]
    fn unlock_states_are_spelled_the_way_a_bootloader_does() {
        let saved = SavedConditions::from(&FlashConditions::Custom {
            cids: vec!["0x0032".to_owned()],
            states: vec![SecureState::Locked, SecureState::Relocked],
        });

        let SavedConditions::Custom { states, .. } = &saved else {
            panic!("a custom condition stays custom");
        };

        assert_eq!(states, &["oem_locked", "flashing_locked"]);

        let FlashConditions::Custom { states, .. } = FlashConditions::from(SavedConditions::Custom {
            cids: vec!["0x0032".to_owned()],
            states: vec!["oem_locked".to_owned(), "nonsense".to_owned()],
        }) else {
            panic!("a custom condition stays custom");
        };

        assert_eq!(states, vec![SecureState::Locked]);
    }
}
