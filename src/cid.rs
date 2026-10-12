//! The CIDs a preset's conditions may name, as the community documents them.
//!
//! `reference/tiny-fastboot-script/aboutcid.txt` (which is gitignored, so its
//! list is kept here) records what each Motorola CID code means, and which of
//! them are in use. Typing a code by hand is not hard, but picking one of the
//! usual ones is what the flash-options card offers.
//!
//! A carrier's or a brand's name reads the same in every language, so it is
//! data rather than a translation — the same reasoning as the carrier brands in
//! [`crate::carrier`] and the app names in [`crate::debloat`]. What a code is
//! *for* (`China retail`, `Global`, `Unknown`) is a description and keeps an FTL
//! id, named after the code itself (`cid-name-retcn`).

use std::fmt::{self, Display, Formatter};

/// The line between two groups of the dropdown.
///
/// iced's dropdown is a flat list without separators, so the rule is an entry of
/// its own: it is there to be read, and picking it does nothing.
const RULE: &str = "────────";

/// What the CID dropdown of the flash-options card offers, in the groups the
/// reference sorts them in.
pub(crate) const MENU: &[CidMenuItem] = &[
    // The codes a phone of a current model reports. The reference spells the
    // Chinese one `RETCN`; its entry is the translated retail description.
    CidMenuItem::cid(&RETCN, "0x000B"),
    CidMenuItem::cid(&GLOBAL, "0x0032"),
    CidMenuItem::cid(&USA_LOCKDOWN, "0x0033"),
    CidMenuItem::name("FCNT", "0x0034"),
    CidMenuItem::RULE,
    // The three codes no retail phone reports.
    CidMenuItem::cid(&SUPER_CID, "0x0000"),
    CidMenuItem::cid(&FACTORY_CID, "0x00FF"),
    CidMenuItem::cid(&INVALID, "0xDEAD"),
    CidMenuItem::RULE,
    // What the reference lists as the codes of older models (the ones marked
    // deprecated there are kept: a firmware of one of those models still
    // carries it). The codes of the first two groups are not repeated.
    CidMenuItem::name("AT&T", "0x0001"),
    CidMenuItem::name("Verizon", "0x0002"),
    CidMenuItem::cid(&UNKNOWN, "0x0003"),
    CidMenuItem::name("AU-KDDI", "0x0004"),
    CidMenuItem::cid(&KOREA_SK_KT, "0x0005"),
    CidMenuItem::name("Republic Wireless", "0x0006"),
    CidMenuItem::cid(&EUROPE_APAC_RETAIL, "0x0007"),
    CidMenuItem::name("Bell", "0x0008"),
    CidMenuItem::cid(&US_RETAIL, "0x0009"),
    CidMenuItem::name("AIO", "0x000A"),
    CidMenuItem::cid(&LATAM_RETAIL, "0x000C"),
    CidMenuItem::cid(&UNKNOWN_DEPRECATED, "0x000D"),
    CidMenuItem::cid(&CANADA_RETAIL, "0x000E"),
    CidMenuItem::name("TracFone", "0x000F"),
    CidMenuItem::name("Cricket", "0x0010"),
    CidMenuItem::name("Amazon", "0x0012"),
    CidMenuItem::name("T-Mobile", "0x0015"),
    CidMenuItem::cid(&GOOGLE_DEVELOPER, "0x00DE"),
];

/// One line of that dropdown: the rule between two groups, or a CID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CidMenuItem {
    /// The line between two groups of the list.
    Rule,
    /// A code the reference names, with what it is called.
    Cid {
        /// What the code is called, in the language of the app.
        label: Label,
        /// The code itself, written the way a flashfile does (`0x000B`).
        code: &'static str,
    },
}

impl CidMenuItem {
    /// The rule between two groups of the list, spelled for [`MENU`].
    pub(crate) const RULE: Self = Self::Rule;

    /// An entry whose name is a description, translated through an FTL id.
    const fn cid(label: &'static Label, code: &'static str) -> Self {
        Self::Cid { label: *label, code }
    }

    /// An entry whose name is a carrier's: shown as it is, like the carrier
    /// brands of [`crate::carrier`].
    const fn name(name: &'static str, code: &'static str) -> Self {
        Self::Cid {
            label: Label::Text(name),
            code,
        }
    }
}

/// The name of a CID: an FTL id for what a code is for, or the name itself for
/// a carrier's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Label {
    /// An FTL message id, rendered in the UI language.
    Message(&'static str),
    /// A name shown as it is.
    Text(&'static str),
}

impl Label {
    /// The name to show in the language of `l10n`.
    fn text(&self, l10n: &crate::l10n::Bundle) -> String {
        match self {
            Self::Message(id) => l10n.tr(id),
            Self::Text(name) => (*name).to_owned(),
        }
    }
}

// The translated names of the list, as constants so the table above stays one
// row per code (a row built by a helper call keeps `rustfmt` from breaking it
// over four lines).
const RETCN: Label = Label::Message("cid-name-retcn");
const GLOBAL: Label = Label::Message("cid-name-global");
const USA_LOCKDOWN: Label = Label::Message("cid-name-usa-lockdown");
const SUPER_CID: Label = Label::Message("cid-name-super");
const FACTORY_CID: Label = Label::Message("cid-name-factory");
const INVALID: Label = Label::Message("cid-name-invalid");
const UNKNOWN: Label = Label::Message("cid-name-unknown");
const KOREA_SK_KT: Label = Label::Message("cid-name-korea-sk-kt");
const EUROPE_APAC_RETAIL: Label = Label::Message("cid-name-europe-apac-retail");
const US_RETAIL: Label = Label::Message("cid-name-us-retail");
const LATAM_RETAIL: Label = Label::Message("cid-name-latam-retail");
const UNKNOWN_DEPRECATED: Label = Label::Message("cid-name-unknown-deprecated");
const CANADA_RETAIL: Label = Label::Message("cid-name-canada-retail");
const GOOGLE_DEVELOPER: Label = Label::Message("cid-name-google-developer");

/// An entry of the dropdown as the card shows it: the name in the language the
/// app is showing, with the code it fills the field with (`None` for a rule).
///
/// iced's `pick_list` renders its options by their `ToString`, so the label is
/// built here rather than looked up while drawing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CidOption {
    label: String,
    code: Option<&'static str>,
}

impl CidOption {
    /// The CID this entry stands for, or `None` for the rule between two
    /// groups of the list.
    pub(crate) fn code(&self) -> Option<&'static str> {
        self.code
    }
}

impl Display for CidOption {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.label)
    }
}

/// The dropdown the flash-options card offers, with every name in the language
/// of `l10n`.
pub(crate) fn options(l10n: &crate::l10n::Bundle) -> Vec<CidOption> {
    MENU.iter()
        .map(|item| match item {
            CidMenuItem::Rule => CidOption {
                label: RULE.to_owned(),
                code: None,
            },
            CidMenuItem::Cid { label, code } => CidOption {
                label: format!("{} ({code})", label.text(l10n)),
                code: Some(code),
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::flashfile;
    use crate::l10n::{self, Language};

    /// The list as the card would show it, in one language.
    fn listed(language: Language) -> Vec<String> {
        options(&l10n::bundle_for(language))
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// A code is written the way the app compares and shows CIDs, so picking
    /// one fills the field with something `flashfile::normalize_cid` leaves
    /// alone.
    #[test]
    fn every_code_is_a_normalized_cid() {
        for item in MENU {
            let CidMenuItem::Cid { code, .. } = item else {
                continue;
            };

            assert_eq!(flashfile::normalize_cid(code), *code, "{code}");
            assert!(code.starts_with("0x"), "{code}");
            assert_eq!(code.len(), 6, "{code}");
        }
    }

    /// Two groups are told apart by a rule, and no two entries name one code —
    /// a dropdown that lists a code twice only invites the same pick twice.
    #[test]
    fn the_list_holds_each_code_once_and_rules_between_the_groups() {
        let mut codes: Vec<&str> = Vec::new();
        let mut rules = 0;

        for item in MENU {
            match item {
                CidMenuItem::Rule => rules += 1,
                CidMenuItem::Cid { code, .. } => {
                    assert!(!codes.contains(code), "{code} is listed twice");
                    codes.push(code);
                }
            }
        }

        assert_eq!(rules, 2, "the three groups are separated by two rules");
        // The rules are inside the list, never at its ends.
        assert!(matches!(MENU.first(), Some(CidMenuItem::Cid { .. })));
        assert!(matches!(MENU.last(), Some(CidMenuItem::Cid { .. })));
    }

    /// The seven codes the flash-options card is expected to offer first, in
    /// the order the user asked for them.
    #[test]
    fn the_usual_codes_come_first() {
        let listed: Vec<String> = listed(Language::EnUs).into_iter().take(7).collect();

        assert_eq!(
            listed,
            [
                "China retail (0x000B)",
                "Global (0x0032)",
                "USA Lockdown (0x0033)",
                "FCNT (0x0034)",
                "────────",
                "SuperCID (0x0000)",
                "FactoryCID (0x00FF)",
            ]
        );
    }

    /// A rule is not a CID: the card fills the field from the entry that was
    /// picked, and a rule has nothing to fill it with.
    #[test]
    fn a_rule_stands_for_no_code() {
        let options = options(&l10n::bundle_for(Language::EnUs));

        let rule = options
            .iter()
            .find(|option| option.code().is_none())
            .expect("the list separates its groups");
        assert_eq!(rule.to_string(), RULE);

        let cid = options
            .iter()
            .find(|option| option.code() == Some("0x0032"))
            .expect("0x0032 is in the list");
        assert_eq!(cid.to_string(), "Global (0x0032)");
    }

    /// The names that are descriptions follow the UI language: this is what a
    /// wrong FTL id breaks (`Bundle::tr` panics on an id that does not exist,
    /// which in a release build is a silent death).
    #[test]
    fn every_cid_name_is_there_in_every_language() {
        for language in Language::ALL {
            let bundle = l10n::bundle_for(language);

            for option in options(&bundle) {
                let label = option.to_string();

                assert!(!label.trim().is_empty(), "{language:?}");
                assert!(!label.contains('\u{2068}'), "{language:?}: {label}");

                match option.code() {
                    Some(code) => {
                        assert!(
                            label.ends_with(&format!("({code})")),
                            "{language:?}: {label}"
                        );
                        // The name is more than the code in brackets.
                        assert_ne!(label, format!("({code})"), "{language:?}: {label}");
                    }
                    None => assert_eq!(label, RULE, "{language:?}"),
                }
            }
        }
    }

    /// A description really is translated — a list that reads the same in every
    /// language would mean the FTL ids were not used at all.
    #[test]
    fn the_descriptions_are_translated() {
        let english = listed(Language::EnUs);
        let german = listed(Language::De);
        let chinese = listed(Language::ZhHans);

        assert_ne!(english, german);
        assert_ne!(english, chinese);
        // A carrier's name is not: it is the same word everywhere.
        assert!(english.iter().any(|line| line == "Verizon (0x0002)"));
        assert!(german.iter().any(|line| line == "Verizon (0x0002)"));
        assert!(chinese.iter().any(|line| line == "Verizon (0x0002)"));
    }
}
