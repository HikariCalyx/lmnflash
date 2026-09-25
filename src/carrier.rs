//! The software channels (`ro.carrier`) of Motorola phones and the names
//! behind them.
//!
//! A bootloader answers `getvar ro.carrier` with the *software channel* of the
//! unit — the carrier or the retail region its firmware is sold for — so the
//! value is a code (`retcn`, `tmo`, `amxbr`) rather than a name. This module
//! turns those codes into something readable.
//!
//! `reference/tiny-fastboot-script/aboutcid.txt` documents the channels. The
//! words (`Retail`, `Open`, `Unknown`, the region names) live in the FTL files,
//! so a region is named in the language of the menu showing it. A carrier is
//! named by its brand, which is the same everywhere — with one exception: a
//! carrier of a CJK region is written in the script of the CJK language the menu
//! is in, whatever that carrier's own region is (`cmcc` is 中国移动 in zh-Hans,
//! 中國移動 in zh-Hant, 中国移動 in Japanese and 중국이동 in Korean). Those
//! carriers therefore carry an FTL message, and every other locale spells that
//! message in English.

use crate::l10n::Bundle;
use Channel::{Operator, Retail};
use Name::{Brand, Message};

/// How a channel is named.
#[derive(Clone, Copy)]
enum Name {
    /// A brand that is written the same way in every language.
    Brand(&'static str),
    /// The id of the FTL message holding the name, for the carriers of a CJK
    /// region, which are written in the script of the CJK menu rather than in
    /// English.
    Message(&'static str),
}

/// What a channel code stands for.
#[derive(Clone, Copy)]
enum Channel {
    /// A carrier, named by its brand.
    Operator(Name),
    /// A retail channel, named by the region it is sold in.
    Retail {
        /// The FTL message with the region's name.
        region: &'static str,
        /// Whether it is one of the channels Motorola sells without a network
        /// lock (the `OPEN…` ones).
        open: bool,
    },
}

/// The name of the software channel `code`, or the localized `Unknown` for a
/// value that is not in the table.
pub fn name(code: &str, l10n: &Bundle) -> String {
    match channel(code) {
        Some(Channel::Operator(Name::Brand(brand))) => brand.to_owned(),
        Some(Channel::Operator(Name::Message(id))) => l10n.tr(id),
        Some(Channel::Retail { region, open }) => {
            let region = l10n.tr(region);
            let message = if open {
                "carrier-retail-open"
            } else {
                "carrier-retail"
            };

            l10n.tr_with_args(message, &[("region", region)])
        }
        None => l10n.tr("carrier-unknown"),
    }
}

/// The channel code with its name behind it, e.g. `retcn (Retail China)`.
///
/// The code itself is kept: it is what the bootloader said, and what the
/// flashing guides and the firmware archives talk about.
pub fn labeled(code: &str, l10n: &Bundle) -> String {
    let code = code.trim();

    if code.is_empty() {
        return l10n.tr("carrier-unknown");
    }

    format!("{code} ({})", name(code, l10n))
}

/// A retail channel of `region`, sold without a network lock when `open`.
///
/// The table is a `const`, so this is a `const fn` — and it keeps the table one
/// channel per row, where a plain `Channel::Retail { .. }` literal would be
/// broken over five lines to keep the formatter happy.
const fn retail(region: &'static str, open: bool) -> Channel {
    Retail { region, open }
}

/// The indexed channels and what they stand for.
///
/// The codes are spelled the way `reference/tiny-fastboot-script/aboutcid.txt`
/// lists them; several are not all upper case there (`Spectrum`, `TracFone`,
/// `Softbank`), which is why the lookup ignores case.
const CHANNELS: &[(&str, Channel)] = &[
    // China.
    ("retcn", retail("carrier-region-cn", false)),
    ("cmcc", Operator(Message("carrier-op-cmcc"))),
    ("ctcn", Operator(Message("carrier-op-ctcn"))),
    // Europe and Asia-Pacific.
    ("eegb", Operator(Brand("EE UK"))),
    ("o2gb", Operator(Brand("O2 UK"))),
    ("retapac", retail("carrier-region-apac", false)),
    ("reteu", retail("carrier-region-eu", false)),
    ("openeu", retail("carrier-region-eu", true)),
    ("retgb", retail("carrier-region-gb", false)),
    ("retin", retail("carrier-region-in", false)),
    ("retkr", retail("carrier-region-kr", false)),
    ("retmea", retail("carrier-region-mea", false)),
    ("retru", retail("carrier-region-ru", false)),
    ("playpl", Operator(Brand("Play Poland"))),
    ("pluspl", Operator(Brand("Plus Poland"))),
    ("docomo", Operator(Message("carrier-op-docomo"))),
    ("softbank", Operator(Message("carrier-op-softbank"))),
    ("vfeu", Operator(Brand("Vodafone Europe"))),
    ("tescogb", Operator(Brand("Tesco Mobile UK"))),
    ("telstra", Operator(Brand("Telstra Australia"))),
    ("teleu", Operator(Brand("Telenor Serbia"))),
    ("timit", Operator(Brand("TIM Italy"))),
    ("trueth", Operator(Brand("True Thailand"))),
    ("reteu_de", retail("carrier-region-de", false)),
    ("reteu_pl", retail("carrier-region-pl", false)),
    ("reteu_rs", retail("carrier-region-rs", false)),
    ("reteu_is", retail("carrier-region-is", false)),
    // North America.
    ("acg", Operator(Brand("Associated Carrier Group"))),
    ("amz", Operator(Brand("Amazon"))),
    ("att", Operator(Brand("AT&T"))),
    ("boost", Operator(Brand("Boost Mobile"))),
    ("cc", Operator(Brand("Consumer Cellular"))),
    ("comcast", Operator(Brand("Xfinity Mobile"))),
    ("cricket", Operator(Brand("Cricket Wireless"))),
    ("fi", Operator(Brand("Google Fi"))),
    ("metropcs", Operator(Brand("Metro by T-Mobile"))),
    ("retca", retail("carrier-region-ca", false)),
    ("retus", retail("carrier-region-us", false)),
    ("spectrum", Operator(Brand("Spectrum"))),
    ("tmo", Operator(Brand("T-Mobile USA"))),
    ("tracfone", Operator(Brand("TracFone Wireless"))),
    ("usc", Operator(Brand("US Cellular"))),
    ("vzw", Operator(Brand("Verizon Wireless"))),
    ("retus_vs", Operator(Brand("Visible Wireless"))),
    // Latin America.
    ("amxmx", Operator(Brand("Telcel Mexico"))),
    ("attmx", Operator(Brand("AT&T Mexico"))),
    ("altmx", Operator(Brand("Altan Redes Mexico"))),
    ("amxbr", Operator(Brand("Claro Brazil"))),
    ("amxcl", Operator(Brand("Claro Chile"))),
    ("amxco", Operator(Brand("Claro Colombia"))),
    ("amxpe", Operator(Brand("Claro Peru"))),
    ("amxla", Operator(Brand("Claro Central America"))),
    ("opencl", retail("carrier-region-cl", true)),
    ("openla", retail("carrier-region-la", true)),
    ("openmx", retail("carrier-region-mx", true)),
    ("openpe", retail("carrier-region-pe", true)),
    ("retar", retail("carrier-region-ar", false)),
    ("retbr", retail("carrier-region-br", false)),
    ("retla", retail("carrier-region-la", false)),
    ("tefbr", Operator(Brand("Vivo Brazil"))),
    ("tefmx", Operator(Brand("Movistar Mexico"))),
    ("timbr", Operator(Brand("TIM Brazil"))),
    ("tigca", Operator(Brand("Tigo Guatemala"))),
];

/// The channel a bootloader value names.
///
/// The comparison ignores case: the same channel comes back as `VZW`, `vzw` or
/// `Spectrum` depending on the bootloader.
fn channel(code: &str) -> Option<Channel> {
    let code = code.trim();

    CHANNELS
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(code))
        .map(|(_, channel)| *channel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::l10n::{bundle_for, Language};

    #[test]
    fn names_a_retail_channel_in_the_menus_language() {
        let en = bundle_for(Language::EnUs);
        let de = bundle_for(Language::De);
        let zh = bundle_for(Language::ZhHans);
        let zh_hant = bundle_for(Language::ZhHant);

        // The code the bootloader answered stays in front of the name.
        assert_eq!(labeled("retcn", &en), "retcn (Retail China)");
        assert_eq!(labeled("retcn", &zh), "retcn (中国大陆零售版)");
        assert_eq!(
            labeled("retcn", &zh_hant),
            "retcn (中國大陸零售版)"
        );

        // The region is named in the language of the menu, so the same channel
        // reads differently per language.
        assert_eq!(name("reteu_de", &en), "Retail Germany");
        assert_eq!(name("reteu_de", &de), "Retail Deutschland");
        assert_eq!(name("reteu_de", &zh), "德国零售版");

        // The `OPEN…` channels are the ones sold without a network lock.
        assert_eq!(name("opencl", &en), "Open Chile");
        assert_eq!(name("openla", &en), "Open Latin America");
    }

    #[test]
    fn names_the_carriers_by_their_brand() {
        let en = bundle_for(Language::EnUs);
        let de = bundle_for(Language::De);
        let ja = bundle_for(Language::Ja);

        assert_eq!(name("tmo", &en), "T-Mobile USA");
        assert_eq!(name("vzw", &en), "Verizon Wireless");
        assert_eq!(name("retus_vs", &en), "Visible Wireless");

        // A brand is written the same way in every language — it is not from
        // the country the menu is for.
        assert_eq!(name("tmo", &de), "T-Mobile USA");
        assert_eq!(name("tmo", &ja), "T-Mobile USA");
        assert_eq!(name("tefbr", &de), "Vivo Brazil");
        // A code the table does not have is unknown in that language too.
        assert_eq!(name("vivo", &en), "Unknown");
        assert_eq!(name("vivo", &de), "Unbekannt");
    }

    /// A carrier of a CJK region is written in the CJK language the menu is
    /// in — not in the carrier's own language — and every other menu writes it
    /// in English.
    #[test]
    fn cjk_carriers_are_written_in_the_menus_cjk_language() {
        assert_eq!(name("cmcc", &bundle_for(Language::EnUs)), "China Mobile");
        assert_eq!(name("cmcc", &bundle_for(Language::De)), "China Mobile");
        assert_eq!(name("cmcc", &bundle_for(Language::ZhHans)), "中国移动");
        assert_eq!(name("cmcc", &bundle_for(Language::ZhHant)), "中國移動");
        assert_eq!(name("cmcc", &bundle_for(Language::Ja)), "中国移動");
        assert_eq!(name("cmcc", &bundle_for(Language::Ko)), "중국이동");

        assert_eq!(name("ctcn", &bundle_for(Language::EnUs)), "China Telecom");
        assert_eq!(name("ctcn", &bundle_for(Language::ZhHant)), "中國電信");
        assert_eq!(name("ctcn", &bundle_for(Language::Ja)), "中国電信");
        assert_eq!(name("ctcn", &bundle_for(Language::Ko)), "중국전신");

        assert_eq!(name("docomo", &bundle_for(Language::EnUs)), "NTT Docomo");
        assert_eq!(name("docomo", &bundle_for(Language::ZhHans)), "NTT都科摩");
        assert_eq!(name("docomo", &bundle_for(Language::ZhHant)), "NTT都科摩");
        assert_eq!(name("docomo", &bundle_for(Language::Ja)), "NTTドコモ");
        assert_eq!(name("docomo", &bundle_for(Language::Ko)), "NTT도코모");

        // A Japanese carrier goes the same way: the Chinese menu names it with
        // its Chinese name rather than with ソフトバンク.
        assert_eq!(name("softbank", &bundle_for(Language::EnUs)), "SoftBank");
        assert_eq!(name("softbank", &bundle_for(Language::ZhHans)), "软银");
        assert_eq!(name("softbank", &bundle_for(Language::ZhHant)), "軟銀");
        assert_eq!(name("softbank", &bundle_for(Language::Ja)), "ソフトバンク");
        assert_eq!(name("softbank", &bundle_for(Language::Ko)), "소프트뱅크");
    }

    /// The codes are compared without case: the bootloaders and the reference
    /// spell them inconsistently (`VZW`, `TracFone`, `Softbank`).
    #[test]
    fn matches_the_codes_without_case() {
        let en = bundle_for(Language::EnUs);

        for code in ["retcn", "RETCN", "Retcn", " retcn "] {
            assert_eq!(name(code, &en), "Retail China", "{code}");
        }

        assert_eq!(name("VZW", &en), name("vzw", &en));
        assert_eq!(name("TracFone", &en), "TracFone Wireless");
        assert_eq!(name("Spectrum", &en), "Spectrum");
        assert_eq!(name("Softbank", &en), "SoftBank");
        assert_eq!(name("RETEU_DE", &en), "Retail Germany");
        assert_eq!(name("DOCOMO", &en), "NTT Docomo");
    }

    /// A value that is not indexed is reported as unknown, and the value itself
    /// is not lost — it is what the phone answered.
    #[test]
    fn falls_back_to_unknown() {
        let en = bundle_for(Language::EnUs);
        let zh = bundle_for(Language::ZhHans);

        assert_eq!(labeled("xyzzy", &en), "xyzzy (Unknown)");
        assert_eq!(labeled("xyzzy", &zh), "xyzzy (未知)");

        // Nothing to name, but still no panic and no stray brackets.
        assert_eq!(labeled("", &en), "Unknown");
        assert_eq!(labeled("   ", &en), "Unknown");
        assert_eq!(name("", &en), "Unknown");
    }

    /// Every indexed channel must be nameable in every language.
    ///
    /// The table names FTL messages for the regions and for the CJK operators;
    /// a typo in one of those ids would panic inside `tr`, which under
    /// `panic = "abort"` kills a release build without a word. The locale
    /// parity test cannot see it — it only compares the locales with each
    /// other.
    #[test]
    fn every_indexed_channel_is_named_in_every_language() {
        for language in Language::ALL {
            let bundle = bundle_for(language);

            for (code, _) in CHANNELS {
                let label = labeled(code, &bundle);

                assert!(!label.contains('{'), "{language:?}: {code} → {label}");
                assert!(label.ends_with(')'), "{language:?}: {label}");
            }
        }
    }
}
