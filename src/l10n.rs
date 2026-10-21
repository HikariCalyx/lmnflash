//! Localization support backed by Fluent (FTL) resources.

use fluent::{FluentArgs, FluentBundle, FluentResource, FluentValue};
use unic_langid::LanguageIdentifier;

/// A wrapper around a [`FluentBundle`] that formats messages by id.
pub struct Bundle {
    bundle: FluentBundle<FluentResource>,
}

impl Bundle {
    /// Creates a bundle for the given locale from raw FTL sources.
    pub fn new(locale: &str, sources: &[&str]) -> Self {
        let langid: LanguageIdentifier = locale.parse().expect("invalid locale");

        let mut bundle = FluentBundle::new(vec![langid]);

        // Fluent wraps interpolated values in U+2068/U+2069 bidi isolation
        // marks by default; most UI fonts have no glyphs for them, so the
        // marks render as boxes (□) around the value. All of our messages
        // are LTR (English/Chinese), so isolation can be disabled.
        bundle.set_use_isolating(false);

        for source in sources {
            let resource = FluentResource::try_new((*source).to_owned())
                .expect("failed to parse FTL resource");

            // Duplicate ids across sources (the English fallback resource)
            // come back as `FluentError::Overriding`; the existing entry is
            // kept and the error is informational, so it is safe to ignore.
            let _ = bundle.add_resource(resource);
        }

        Self { bundle }
    }

    /// Formats the message `id` with no arguments.
    ///
    /// Panics if the message is missing or has no value; the set of available
    /// messages is fixed at compile time, so this indicates a bug.
    pub fn tr(&self, id: &str) -> String {
        let message = self
            .bundle
            .get_message(id)
            .unwrap_or_else(|| panic!("missing FTL message `{id}`"));

        let pattern = message
            .value()
            .unwrap_or_else(|| panic!("FTL message `{id}` has no value"));

        let mut errors = Vec::new();

        self.bundle
            .format_pattern(pattern, None, &mut errors)
            .into_owned()
    }

    /// Formats the message `id`, interpolating the given arguments.
    ///
    /// Arguments are exposed to the FTL message as `$key` variables.
    pub fn tr_with_args(&self, id: &str, args: &[(&str, String)]) -> String {
        let message = self
            .bundle
            .get_message(id)
            .unwrap_or_else(|| panic!("missing FTL message `{id}`"));

        let pattern = message
            .value()
            .unwrap_or_else(|| panic!("FTL message `{id}` has no value"));

        let mut fluent_args = FluentArgs::new();
        for (key, value) in args {
            fluent_args.set((*key).to_owned(), FluentValue::from(value.clone()));
        }

        let mut errors = Vec::new();

        self.bundle
            .format_pattern(pattern, Some(&fluent_args), &mut errors)
            .into_owned()
    }
}

/// Supported UI languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Language {
    #[default]
    EnUs,
    ZhHans,
    ZhHant,
    Ja,
    Ko,
    Ru,
    Da,
    De,
    Fr,
    It,
    Nb,
    Nl,
    PtBr,
    Fi,
    Es,
    Sv,
    Uk,
}

impl Language {
    pub const ALL: [Self; 17] = [
        Self::EnUs,
        Self::ZhHans,
        Self::ZhHant,
        Self::Ja,
        Self::Ko,
        Self::Ru,
        Self::Da,
        Self::De,
        Self::Fr,
        Self::It,
        Self::Nb,
        Self::Nl,
        Self::PtBr,
        Self::Fi,
        Self::Es,
        Self::Sv,
        Self::Uk,
    ];

    pub fn message_id(self) -> &'static str {
        match self {
            Self::EnUs => "lang-en",
            Self::ZhHans => "lang-zh",
            Self::ZhHant => "lang-zhhant",
            Self::Ja => "lang-ja",
            Self::Ko => "lang-ko",
            Self::Ru => "lang-ru",
            Self::Da => "lang-da",
            Self::De => "lang-de",
            Self::Fr => "lang-fr",
            Self::It => "lang-it",
            Self::Nb => "lang-nb",
            Self::Nl => "lang-nl",
            Self::PtBr => "lang-ptbr",
            Self::Fi => "lang-fi",
            Self::Es => "lang-es",
            Self::Sv => "lang-sv",
            Self::Uk => "lang-uk",
        }
    }

    /// The locale code persisted in `config.conf`.
    pub fn code(self) -> &'static str {
        match self {
            Self::EnUs => "en-US",
            Self::ZhHans => "zh-Hans",
            Self::ZhHant => "zh-Hant",
            Self::Ja => "ja",
            Self::Ko => "ko",
            Self::Ru => "ru",
            Self::Da => "da",
            Self::De => "de",
            Self::Fr => "fr",
            Self::It => "it",
            Self::Nb => "nb",
            Self::Nl => "nl",
            Self::PtBr => "pt-BR",
            Self::Fi => "fi",
            Self::Es => "es",
            Self::Sv => "sv",
            Self::Uk => "uk",
        }
    }

    /// Parses a locale code (also tolerates plain language codes).
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "en-US" | "en" => Some(Self::EnUs),
            "zh-Hans" | "zh-CN" | "zh" => Some(Self::ZhHans),
            "zh-Hant" | "zh-TW" | "zh-HK" | "zh-MO" => Some(Self::ZhHant),
            "ja" | "ja-JP" => Some(Self::Ja),
            "ko" | "ko-KR" => Some(Self::Ko),
            "ru" | "ru-RU" => Some(Self::Ru),
            "da" | "da-DK" => Some(Self::Da),
            "de" | "de-DE" => Some(Self::De),
            "fr" | "fr-FR" => Some(Self::Fr),
            "it" | "it-IT" => Some(Self::It),
            "nb" | "nb-NO" | "no" | "no-NO" => Some(Self::Nb),
            "nl" | "nl-NL" => Some(Self::Nl),
            "pt" | "pt-BR" => Some(Self::PtBr),
            "fi" | "fi-FI" => Some(Self::Fi),
            "es" | "es-ES" => Some(Self::Es),
            "sv" | "sv-SE" => Some(Self::Sv),
            "uk" | "uk-UA" => Some(Self::Uk),
            _ => None,
        }
    }
}

/// Builds the application bundle for the given language, embedded at
/// compile time.
///
/// The English resource is always added last so that any message missing
/// from a translation falls back to English instead of panicking (a panic
/// in `tr` would silently kill the app in release builds).
pub fn bundle_for(language: Language) -> Bundle {
    let en = include_str!("../l10n/en-US/app.ftl");

    match language {
        Language::EnUs => Bundle::new("en-US", &[en]),
        Language::ZhHans => {
            Bundle::new("zh-Hans", &[include_str!("../l10n/zh-Hans/app.ftl"), en])
        }
        Language::ZhHant => {
            Bundle::new("zh-Hant", &[include_str!("../l10n/zh-Hant/app.ftl"), en])
        }
        Language::Ja => Bundle::new("ja", &[include_str!("../l10n/ja/app.ftl"), en]),
        Language::Ko => Bundle::new("ko", &[include_str!("../l10n/ko/app.ftl"), en]),
        Language::Ru => Bundle::new("ru", &[include_str!("../l10n/ru/app.ftl"), en]),
        Language::Da => Bundle::new("da", &[include_str!("../l10n/da/app.ftl"), en]),
        Language::De => Bundle::new("de", &[include_str!("../l10n/de/app.ftl"), en]),
        Language::Fr => Bundle::new("fr", &[include_str!("../l10n/fr/app.ftl"), en]),
        Language::It => Bundle::new("it", &[include_str!("../l10n/it/app.ftl"), en]),
        Language::Nb => Bundle::new("nb", &[include_str!("../l10n/nb/app.ftl"), en]),
        Language::Nl => Bundle::new("nl", &[include_str!("../l10n/nl/app.ftl"), en]),
        Language::PtBr => {
            Bundle::new("pt-BR", &[include_str!("../l10n/pt-BR/app.ftl"), en])
        }
        Language::Fi => Bundle::new("fi", &[include_str!("../l10n/fi/app.ftl"), en]),
        Language::Es => Bundle::new("es", &[include_str!("../l10n/es/app.ftl"), en]),
        Language::Sv => Bundle::new("sv", &[include_str!("../l10n/sv/app.ftl"), en]),
        Language::Uk => Bundle::new("uk", &[include_str!("../l10n/uk/app.ftl"), en]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fluent::FluentResource;
    use fluent_syntax::ast;
    use std::collections::HashSet;

    /// Collects the ids of all `Message` entries in an FTL source.
    fn message_ids(source: &str) -> HashSet<String> {
        let resource = FluentResource::try_new(source.to_owned())
            .expect("invalid FTL source");

        resource
            .entries()
            .filter_map(|entry| match entry {
                ast::Entry::Message(message) => Some(message.id.name.to_string()),
                _ => None,
            })
            .collect()
    }

    /// Every message id used by the app must exist in ALL bundles.
    ///
    /// The English resource doubles as the runtime fallback, so a missing
    /// translation would only show up as untranslated English text; this
    /// test keeps the translations complete anyway. It also catches e.g. a
    /// lost newline that merges two FTL entries into one.
    #[test]
    fn all_bundles_define_the_same_messages() {
        let en = message_ids(include_str!("../l10n/en-US/app.ftl"));

        for source in [
            include_str!("../l10n/zh-Hans/app.ftl"),
            include_str!("../l10n/zh-Hant/app.ftl"),
            include_str!("../l10n/ja/app.ftl"),
            include_str!("../l10n/ko/app.ftl"),
            include_str!("../l10n/ru/app.ftl"),
            include_str!("../l10n/da/app.ftl"),
            include_str!("../l10n/de/app.ftl"),
            include_str!("../l10n/fr/app.ftl"),
            include_str!("../l10n/it/app.ftl"),
            include_str!("../l10n/nb/app.ftl"),
            include_str!("../l10n/nl/app.ftl"),
            include_str!("../l10n/pt-BR/app.ftl"),
            include_str!("../l10n/fi/app.ftl"),
            include_str!("../l10n/es/app.ftl"),
            include_str!("../l10n/sv/app.ftl"),
            include_str!("../l10n/uk/app.ftl"),
        ] {
            assert_eq!(message_ids(source), en);
        }
    }

    /// The About dialog is built from several pieces: the heading, the intro,
    /// the thanks heading and list, the Android heading and the two link
    /// targets. The thanks list is one multi-line value, so a locale that lost
    /// its line breaks would run the three credits together.
    #[test]
    fn the_about_dialog_renders_in_every_locale() {
        for language in Language::ALL {
            let bundle = bundle_for(language);

            for id in [
                "about-title",
                "about-version",
                "about-intro",
                "about-thanks-heading",
                "about-android-heading",
                "about-source-code",
                "about-link-lenovobl",
                "about-link-android",
                "about-link-source",
            ] {
                let text = bundle.tr(id);

                assert!(!text.trim().is_empty(), "{language:?}: {id}");
                assert!(!text.contains('\u{2068}'), "{language:?}: {id}");
                assert!(!text.contains('\u{2069}'), "{language:?}: {id}");
            }

            let list = bundle.tr("about-thanks-list");

            assert!(list.lines().count() >= 3, "{language:?}: {list:?}");
            assert!(!list.contains('\u{2068}'), "{language:?}: {list:?}");

            // Both link targets have to be openable as they are.
            for id in ["about-link-lenovobl", "about-link-android", "about-link-source"] {
                assert!(
                    bundle.tr(id).starts_with("https://"),
                    "{language:?}: {id}"
                );
            }
        }
    }

    /// Interpolated values must not be wrapped in bidi isolation marks —
    /// they have no glyphs in common UI fonts and render as boxes (□).
    #[test]
    fn interpolated_values_are_not_wrapped_in_bidi_isolates() {
        let bundle = bundle_for(Language::EnUs);

        let text = bundle.tr_with_args(
            "retcn-fill-fastboot-filled",
            &[("serial", "ZY22LHBR98".to_string())],
        );

        assert_eq!(text, "Filled from device ZY22LHBR98");
        assert!(!text.contains('\u{2068}'));
        assert!(!text.contains('\u{2069}'));
    }

    #[test]
    fn language_codes_round_trip() {
        for language in Language::ALL {
            assert_eq!(Language::from_code(language.code()), Some(language));
        }

        assert_eq!(Language::from_code("zh"), Some(Language::ZhHans));
        assert_eq!(Language::from_code("en"), Some(Language::EnUs));
        assert_eq!(Language::from_code("zh-CN"), Some(Language::ZhHans));
        assert_eq!(Language::from_code("zh-TW"), Some(Language::ZhHant));
        assert_eq!(Language::from_code("zh-HK"), Some(Language::ZhHant));
        assert_eq!(Language::from_code("ja-JP"), Some(Language::Ja));
        assert_eq!(Language::from_code("ko-KR"), Some(Language::Ko));
        assert_eq!(Language::from_code("ru-RU"), Some(Language::Ru));
        assert_eq!(Language::from_code("fr-FR"), Some(Language::Fr));
        assert_eq!(Language::from_code("pl"), None);
    }

    /// The brick warning and its countdown must interpolate in every locale.
    ///
    /// A translation that names an argument differently (or not at all) would
    /// otherwise only show up as a warning with a hole in it.
    #[test]
    fn the_project_mismatch_warning_renders_in_every_locale() {
        for language in Language::ALL {
            let bundle = bundle_for(language);

            let warning = bundle.tr_with_args(
                "firmware-flash-project-mismatch",
                &[
                    ("package", "arcfox".to_owned()),
                    ("device", "eqs".to_owned()),
                ],
            );
            assert!(warning.contains("arcfox"), "{language:?}: {warning}");
            assert!(warning.contains("eqs"), "{language:?}: {warning}");

            let countdown = bundle.tr_with_args(
                "firmware-flash-brick-countdown",
                &[("seconds", "10".to_owned())],
            );
            assert!(countdown.contains("10"), "{language:?}: {countdown}");
        }
    }

    /// Every "Install Driver" string the dialog can show must exist and
    /// interpolate in every locale: the failure message carries the raw error,
    /// and a translation that names `$error` differently would leave a hole in
    /// it (or, worse, reach `tr` with a missing id and kill a release build).
    #[test]
    fn the_driver_dialog_renders_in_every_locale() {
        const PLAIN: &[&str] = &[
            "flash-driver-title",
            "driver-smartphone-button",
            "driver-tablet-button",
            "driver-install-button",
            "driver-desc",
            "driver-linux-desc",
            "driver-downloading",
            "driver-installing",
            "driver-installing-linux",
            "driver-done",
            "driver-done-linux",
            "driver-password",
            "driver-error-password",
            "driver-error-no-sudo",
        ];

        for language in Language::ALL {
            let bundle = bundle_for(language);

            for id in PLAIN {
                assert!(!bundle.tr(id).trim().is_empty(), "{language:?}: {id}");
            }

            let failure = bundle.tr_with_args(
                "driver-failed",
                &[("error", "connection reset".to_owned())],
            );
            assert!(
                failure.contains("connection reset"),
                "{language:?}: {failure}"
            );
            assert!(!failure.contains('\u{2068}'), "{language:?}: {failure}");
        }
    }

    /// Every "Open Terminal" string must exist and interpolate in every locale:
    /// the two arguments carry a directory and a raw error, and a translation
    /// that names one differently would leave a hole in the line.
    #[test]
    fn the_terminal_messages_render_in_every_locale() {
        const PLAIN: &[&str] = &[
            "firmware-flash-terminal",
            "firmware-flash-terminal-preparing",
            "firmware-flash-terminal-minimal-adb",
        ];

        for language in Language::ALL {
            let bundle = bundle_for(language);

            for id in PLAIN {
                assert!(!bundle.tr(id).trim().is_empty(), "{language:?}: {id}");
            }

            let opened = bundle.tr_with_args(
                "firmware-flash-terminal-opened",
                &[("path", r"C:\tools\platform-tools".to_owned())],
            );
            assert!(
                opened.contains(r"C:\tools\platform-tools"),
                "{language:?}: {opened}"
            );

            let failed = bundle.tr_with_args(
                "firmware-flash-terminal-failed",
                &[("error", "no terminal emulator found".to_owned())],
            );
            assert!(
                failed.contains("no terminal emulator found"),
                "{language:?}: {failed}"
            );
            assert!(!failed.contains('\u{2068}'), "{language:?}: {failed}");
        }
    }

    /// Every `softwarefix://` handler string must exist and interpolate in
    /// every locale: the status and failure lines carry an argument, and a
    /// translation that names one differently would leave a hole in the line.
    #[test]
    fn the_protocol_messages_render_in_every_locale() {
        const PLAIN: &[&str] = &[
            "login-protocol-title",
            "login-protocol-none",
            "login-protocol-ours",
            "login-protocol-switch",
            "login-protocol-restore",
            "login-button-hint-browser",
            "login-browser-waiting",
            "login-browser-cookie-hint",
            "login-retry-webview",
            "login-browser-cookie-chrome",
            "login-browser-cookie-edge",
            "login-browser-cookie-firefox",
            "login-browser-cookie-other",
        ];

        for language in Language::ALL {
            let bundle = bundle_for(language);

            for id in PLAIN {
                assert!(!bundle.tr(id).trim().is_empty(), "{language:?}: {id}");
            }

            let current = bundle.tr_with_args(
                "login-protocol-current",
                &[("program", r"C:\Tools\SoftwareFix.exe".to_owned())],
            );
            assert!(
                current.contains(r"C:\Tools\SoftwareFix.exe"),
                "{language:?}: {current}"
            );
            assert!(!current.contains('\u{2068}'), "{language:?}: {current}");

            let failed = bundle.tr_with_args(
                "login-protocol-failed",
                &[("error", "access denied".to_owned())],
            );
            assert!(failed.contains("access denied"), "{language:?}: {failed}");
            assert!(!failed.contains('\u{2068}'), "{language:?}: {failed}");
        }
    }

    /// Every tablet Bootloader Unlock string must exist and interpolate in
    /// every locale: the failure line carries the bootloader's raw message, and
    /// a translation that names `$error` differently would leave a hole in it.
    #[test]
    fn the_tablet_unlock_messages_render_in_every_locale() {
        const PLAIN: &[&str] = &[
            "flash-bootloader-tablet-desc",
            "flash-bootloader-tablet-site-desc",
            "flash-bootloader-tablet-site-button",
            "flash-bootloader-tablet-serial",
            "flash-bootloader-tablet-sn",
            "flash-bootloader-tablet-sn-na",
            "flash-bootloader-tablet-legion",
            "flash-bootloader-tablet-unlock-desc",
            "flash-bootloader-tablet-not-generated",
            "flash-bootloader-tablet-confirm-unlock",
            "flash-bootloader-tablet-guided-connect",
            "flash-bootloader-tablet-guided-working",
            "flash-bootloader-tablet-guided-manual",
            "flash-bootloader-tablet-confirm-mediatek",
            "flash-bootloader-tablet-confirm-qualcomm",
            "flash-bootloader-tablet-cancel-restart",
            "flash-bootloader-tablet-cancelling",
        ];

        for language in Language::ALL {
            let bundle = bundle_for(language);

            for id in PLAIN {
                assert!(!bundle.tr(id).trim().is_empty(), "{language:?}: {id}");
            }

            let failure = bundle.tr_with_args(
                "flash-bootloader-tablet-unknown-error",
                &[("error", "Partition not found".to_owned())],
            );
            assert!(
                failure.contains("Partition not found"),
                "{language:?}: {failure}"
            );
            assert!(!failure.contains('\u{2068}'), "{language:?}: {failure}");

            let guided = bundle.tr_with_args(
                "flash-bootloader-tablet-guided-failed",
                &[("error", "device offline".to_owned())],
            );
            assert!(guided.contains("device offline"), "{language:?}: {guided}");
            assert!(!guided.contains('\u{2068}'), "{language:?}: {guided}");
        }
    }

    /// Every "Remove System Bloatware" string must exist and interpolate in
    /// every locale: the result lines carry the app name and the raw device
    /// error, and a translation that names an argument differently would leave
    /// a hole in the line (or reach `tr` with a missing id, which is fatal in
    /// a release build).
    #[test]
    fn the_bloatware_dialog_renders_in_every_locale() {
        const PLAIN: &[&str] = &[
            "flash-bloatware-title",
            "flash-bloatware-button",
            "bloatware-desc",
            "bloatware-preparing",
            "bloatware-refresh",
            "bloatware-select-device",
            "bloatware-no-device",
            "bloatware-unauthorized",
            "bloatware-retry",
            "bloatware-checking",
            "bloatware-found-heading",
            "bloatware-search",
            "bloatware-none-found",
            "bloatware-select-all",
            "bloatware-select-none",
            "bloatware-remove",
            "bloatware-remove-none",
            "bloatware-removing",
            "bloatware-done",
            "bloatware-undo-note",
        ];

        for language in Language::ALL {
            let bundle = bundle_for(language);

            for id in PLAIN {
                let text = bundle.tr(id);

                assert!(!text.trim().is_empty(), "{language:?}: {id}");
                assert!(!text.contains('\u{2068}'), "{language:?}: {id}");
            }

            // Every app whose name is a translated message must be named in
            // the locale; a product name carries its own text.
            for app in crate::debloat::BLOATWARE {
                if let crate::debloat::Label::Message(id) = app.label {
                    let name = bundle.tr(id);

                    assert!(!name.trim().is_empty(), "{language:?}: {id}");
                    assert!(!name.contains('\u{2068}'), "{language:?}: {id}");
                }
            }

            let removed = bundle.tr_with_args(
                "bloatware-result-removed",
                &[("name", "SmartFeed".to_owned())],
            );
            assert!(removed.contains("SmartFeed"), "{language:?}: {removed}");
            assert!(!removed.contains('\u{2068}'), "{language:?}: {removed}");

            let failed = bundle.tr_with_args(
                "bloatware-result-failed",
                &[
                    ("name", "SmartFeed".to_owned()),
                    ("error", "Failure [DELETE_FAILED_INTERNAL_ERROR]".to_owned()),
                ],
            );
            assert!(failed.contains("SmartFeed"), "{language:?}: {failed}");
            assert!(
                failed.contains("Failure [DELETE_FAILED_INTERNAL_ERROR]"),
                "{language:?}: {failed}"
            );
            assert!(!failed.contains('\u{2068}'), "{language:?}: {failed}");

            let no_match =
                bundle.tr_with_args("bloatware-no-match", &[("query", "verizon".to_owned())]);
            assert!(no_match.contains("verizon"), "{language:?}: {no_match}");
            assert!(!no_match.contains('\u{2068}'), "{language:?}: {no_match}");
        }
    }

    /// The "Connect ADB Wireless Debugging" dialog: the tile, the two pairing
    /// methods with their instructions, and the status lines — the last three
    /// carry the connected device, or the raw ADB error.
    #[test]
    fn the_wireless_dialog_renders_in_every_locale() {
        const PLAIN: &[&str] = &[
            "flash-wireless-title",
            "flash-wireless-button",
            "wireless-desc",
            "wireless-method-qr",
            "wireless-method-code",
            "wireless-qr-desc",
            "wireless-qr-regenerate",
            "wireless-qr-waiting",
            "wireless-code-desc",
            "wireless-code-device",
            "wireless-code-placeholder",
            "wireless-no-service",
            "wireless-pair",
            "wireless-code-required",
            "wireless-working",
            "wireless-paired",
            "wireless-disconnect",
            "wireless-disconnecting",
            "wireless-disconnect-all",
            "wireless-disconnected-all",
        ];

        for language in Language::ALL {
            let bundle = bundle_for(language);

            for id in PLAIN {
                let text = bundle.tr(id);

                assert!(!text.trim().is_empty(), "{language:?}: {id}");
                assert!(!text.contains('\u{2068}'), "{language:?}: {id}");
            }

            let connected = bundle.tr_with_args(
                "wireless-connected",
                &[("device", "192.168.1.5:5555".to_owned())],
            );
            assert!(
                connected.contains("192.168.1.5:5555"),
                "{language:?}: {connected}"
            );
            assert!(!connected.contains('\u{2068}'), "{language:?}: {connected}");

            let failed = bundle.tr_with_args(
                "wireless-failed",
                &[("error", "failed to authenticate".to_owned())],
            );
            assert!(
                failed.contains("failed to authenticate"),
                "{language:?}: {failed}"
            );
            assert!(!failed.contains('\u{2068}'), "{language:?}: {failed}");

            let mdns = bundle.tr_with_args(
                "wireless-mdns-failed",
                &[("error", "no such device".to_owned())],
            );
            assert!(mdns.contains("no such device"), "{language:?}: {mdns}");
            assert!(!mdns.contains('\u{2068}'), "{language:?}: {mdns}");

            let disconnected = bundle.tr_with_args(
                "wireless-disconnected",
                &[("device", "192.168.1.5:5555".to_owned())],
            );
            assert!(
                disconnected.contains("192.168.1.5:5555"),
                "{language:?}: {disconnected}"
            );
            assert!(
                !disconnected.contains('\u{2068}'),
                "{language:?}: {disconnected}"
            );

            let disconnect_failed = bundle.tr_with_args(
                "wireless-disconnect-failed",
                &[("error", "no such device".to_owned())],
            );
            assert!(
                disconnect_failed.contains("no such device"),
                "{language:?}: {disconnect_failed}"
            );
            assert!(
                !disconnect_failed.contains('\u{2068}'),
                "{language:?}: {disconnect_failed}"
            );
        }
    }

    /// The "Fix Captive Portal Server" dialog: the tile, the explanation and
    /// the status lines, the last two of which carry the dropped connection
    /// and the raw device error.
    #[test]
    fn the_captive_portal_dialog_renders_in_every_locale() {
        const PLAIN: &[&str] = &[
            "flash-captive-portal-title",
            "flash-captive-portal-button",
            "captive-portal-desc",
            "captive-portal-note",
            "captive-portal-fix",
            "captive-portal-working",
            "captive-portal-done",
            "captive-portal-unsupported",
            "captive-portal-manual-airplane",
            "captive-portal-retry",
        ];

        for language in Language::ALL {
            let bundle = bundle_for(language);

            for id in PLAIN {
                let text = bundle.tr(id);

                assert!(!text.trim().is_empty(), "{language:?}: {id}");
                assert!(!text.contains('\u{2068}'), "{language:?}: {id}");
            }

            let disconnected = bundle.tr_with_args(
                "captive-portal-disconnected",
                &[("device", "192.168.1.5:5555".to_owned())],
            );
            assert!(
                disconnected.contains("192.168.1.5:5555"),
                "{language:?}: {disconnected}"
            );
            assert!(
                !disconnected.contains('\u{2068}'),
                "{language:?}: {disconnected}"
            );

            let failed = bundle.tr_with_args(
                "captive-portal-failed",
                &[("error", "device offline".to_owned())],
            );
            assert!(failed.contains("device offline"), "{language:?}: {failed}");
            assert!(!failed.contains('\u{2068}'), "{language:?}: {failed}");
        }
    }

    /// The "Single Partition Flash" dialog: the tile, the form the user fills
    /// in, the two AVB boxes and the status lines of the job.
    #[test]
    fn the_partition_flash_dialog_renders_in_every_locale() {
        const PLAIN: &[&str] = &[
            "flash-partition-title",
            "flash-partition-button",
            "partition-flash-desc",
            "partition-flash-partition",
            "partition-flash-partition-required",
            "partition-flash-image",
            "partition-flash-select",
            "partition-flash-no-image",
            "partition-flash-drop",
            "partition-flash-switch",
            "partition-flash-switching",
            "partition-flash-switched",
            "partition-flash-slot",
            "partition-flash-mode",
            "partition-flash-mode-bootloader",
            "partition-flash-mode-fastbootd",
            "partition-flash-mode-unknown",
            "partition-flash-disable-verity",
            "partition-flash-disable-verification",
            "partition-flash-no-avb",
            "partition-flash-flash",
            "partition-flash-flashing",
            "partition-flash-done",
        ];

        for language in Language::ALL {
            let bundle = bundle_for(language);

            for id in PLAIN {
                let text = bundle.tr(id);

                assert!(!text.trim().is_empty(), "{language:?}: {id}");
                assert!(!text.contains('\u{2068}'), "{language:?}: {id}");
            }
        }
    }

    /// The Tablet Firmware Decrypt dialog and the tile that opens it: the
    /// heading, the directory/password controls and the two counter lines,
    /// which carry the succeeded/failed numbers.
    #[test]
    fn the_decrypt_dialog_renders_in_every_locale() {
        const PLAIN: &[&str] = &[
            "decrypt-title",
            "decrypt-description",
            "decrypt-select-dir",
            "decrypt-no-dir",
            "decrypt-custom-password",
            "decrypt-password-label",
            "decrypt-button",
            "decrypt-working",
            "decrypt-no-files",
            "decrypt-password-required",
            "decrypt-failed-files",
        ];

        for language in Language::ALL {
            let bundle = bundle_for(language);

            for id in PLAIN {
                let text = bundle.tr(id);

                assert!(!text.trim().is_empty(), "{language:?}: {id}");
                assert!(!text.contains('\u{2068}'), "{language:?}: {id}");
            }

            // The done line carries the two numbers the summary reports.
            let done = bundle.tr_with_args(
                "decrypt-done",
                &[("ok", "7".to_owned()), ("fail", "1".to_owned())],
            );
            assert!(done.contains('7'), "{language:?}: {done}");
            assert!(done.contains('1'), "{language:?}: {done}");
            assert!(!done.contains('\u{2068}'), "{language:?}: {done}");
        }
    }

    /// Every "Unpack Image" string must exist and interpolate in every locale:
    /// the contents heading carries a count and a size, the done line the same
    /// pair, and the failure line the raw error. A translation that names an
    /// argument differently would leave a hole in the line (or reach `tr` with
    /// a missing id, which is fatal in a release build).
    #[test]
    fn the_unpack_dialog_renders_in_every_locale() {
        const PLAIN: &[&str] = &[
            "unpack-title",
            "unpack-button",
            "unpack-description",
            "unpack-select-image",
            "unpack-select-folder",
            "unpack-none-selected",
            "unpack-locate",
            "unpack-start",
            "unpack-listing",
            "unpack-not-an-image",
            "unpack-working",
        ];

        for language in Language::ALL {
            let bundle = bundle_for(language);

            for id in PLAIN {
                let text = bundle.tr(id);

                assert!(!text.trim().is_empty(), "{language:?}: {id}");
                assert!(!text.contains('\u{2068}'), "{language:?}: {id}");
            }

            let contents = bundle.tr_with_args(
                "unpack-contents",
                &[("count", "3".to_owned()), ("size", "1.5 MB".to_owned())],
            );
            assert!(contents.contains('3'), "{language:?}: {contents}");
            assert!(contents.contains("1.5 MB"), "{language:?}: {contents}");
            assert!(!contents.contains('\u{2068}'), "{language:?}: {contents}");

            let done = bundle.tr_with_args(
                "unpack-done",
                &[("count", "3".to_owned()), ("size", "1.5 MB".to_owned())],
            );
            assert!(done.contains('3'), "{language:?}: {done}");
            assert!(done.contains("1.5 MB"), "{language:?}: {done}");
            assert!(!done.contains('\u{2068}'), "{language:?}: {done}");

            let failed = bundle.tr_with_args(
                "unpack-failed",
                &[("error", "the image directory is truncated".to_owned())],
            );
            assert!(
                failed.contains("the image directory is truncated"),
                "{language:?}: {failed}"
            );
            assert!(!failed.contains('\u{2068}'), "{language:?}: {failed}");
        }
    }

    /// The "Unisoc PAC" package tool: the tile that opens it, its dialog, and
    /// the failure line, which carries the raw error.
    #[test]
    fn the_pac_dialog_renders_in_every_locale() {
        const PLAIN: &[&str] = &[
            "pac-title",
            "pac-unpack-button",
            "pac-repack-button",
            "pac-description",
            "pac-select-file",
            "pac-listing",
            "pac-invalid",
            "pac-encrypted",
            "pac-select-all",
            "pac-select-none",
            "pac-none-chosen",
            "pac-search",
            "pac-incomplete",
            "pac-spd-script",
            "pac-spd-no-config",
            "pac-spd-need-fdl",
        ];

        for language in Language::ALL {
            let bundle = bundle_for(language);

            for id in PLAIN {
                let text = bundle.tr(id);

                assert!(!text.trim().is_empty(), "{language:?}: {id}");
                assert!(!text.contains('\u{2068}'), "{language:?}: {id}");
            }

            let failed = bundle.tr_with_args(
                "pac-failed",
                &[("error", "the file table is truncated".to_owned())],
            );
            assert!(
                failed.contains("the file table is truncated"),
                "{language:?}: {failed}"
            );
            assert!(!failed.contains('\u{2068}'), "{language:?}: {failed}");

            let no_match =
                bundle.tr_with_args("pac-no-match", &[("query", "fdl2".to_owned())]);
            assert!(no_match.contains("fdl2"), "{language:?}: {no_match}");
            assert!(!no_match.contains('\u{2068}'), "{language:?}: {no_match}");

            let partial = bundle.tr_with_args(
                "pac-partial",
                &[
                    ("found", "3.4 GB".to_owned()),
                    ("expected", "7.3 GB".to_owned()),
                ],
            );
            assert!(partial.contains("3.4 GB"), "{language:?}: {partial}");
            assert!(partial.contains("7.3 GB"), "{language:?}: {partial}");
            assert!(!partial.contains('\u{2068}'), "{language:?}: {partial}");

            let written = bundle.tr_with_args(
                "pac-spd-written",
                &[("files", "HNR552T_spd_dump.cmd".to_owned())],
            );
            assert!(
                written.contains("HNR552T_spd_dump.cmd"),
                "{language:?}: {written}"
            );
            assert!(!written.contains('\u{2068}'), "{language:?}: {written}");

            let scripts_failed = bundle.tr_with_args(
                "pac-spd-failed",
                &[("error", "no configuration XML".to_owned())],
            );
            assert!(
                scripts_failed.contains("no configuration XML"),
                "{language:?}: {scripts_failed}"
            );
            assert!(
                !scripts_failed.contains('\u{2068}'),
                "{language:?}: {scripts_failed}"
            );
        }
    }

    /// The "Repack Package" dialog: the source picker, the metadata it may have
    /// to ask for, the record list, and the two lines that carry arguments —
    /// how many records have a file, and what the run wrote.
    #[test]
    fn the_repack_dialog_renders_in_every_locale() {
        const PLAIN: &[&str] = &[
            "pac-repack-title",
            "pac-repack-description",
            "pac-repack-select-source",
            "pac-repack-reading",
            "pac-repack-invalid",
            "pac-repack-needs-file",
            "pac-repack-select-file",
            "pac-repack-alias",
            "pac-repack-version",
            "pac-repack-metadata",
            "pac-repack-nothing",
            "pac-repack-save",
            "pac-repack-working",
            "pac-repack-col-part",
            "pac-repack-col-partition",
            "pac-repack-col-size",
            "pac-repack-col-file",
        ];

        for language in Language::ALL {
            let bundle = bundle_for(language);

            for id in PLAIN {
                let text = bundle.tr(id);

                assert!(!text.trim().is_empty(), "{language:?}: {id}");
                assert!(!text.contains('\u{2068}'), "{language:?}: {id}");
            }

            let files = bundle.tr_with_args(
                "pac-repack-files",
                &[("resolved", "35".to_owned()), ("total", "40".to_owned())],
            );
            assert!(files.contains("35"), "{language:?}: {files}");
            assert!(files.contains("40"), "{language:?}: {files}");
            assert!(!files.contains('\u{2068}'), "{language:?}: {files}");

            let leave_out =
                bundle.tr_with_args("pac-repack-leave-out", &[("count", "5".to_owned())]);
            assert!(leave_out.contains('5'), "{language:?}: {leave_out}");
            assert!(!leave_out.contains('\u{2068}'), "{language:?}: {leave_out}");

            let done = bundle.tr_with_args(
                "pac-repack-done",
                &[
                    ("count", "40".to_owned()),
                    ("size", "3.4 GB".to_owned()),
                    ("path", "D:\\firmware\\ums9620.pac".to_owned()),
                ],
            );
            assert!(done.contains("40"), "{language:?}: {done}");
            assert!(done.contains("3.4 GB"), "{language:?}: {done}");
            assert!(
                done.contains("D:\\firmware\\ums9620.pac"),
                "{language:?}: {done}"
            );
            assert!(!done.contains('\u{2068}'), "{language:?}: {done}");

            let failed = bundle.tr_with_args(
                "pac-repack-failed",
                &[("error", "the record `BOOT` has no file".to_owned())],
            );
            assert!(
                failed.contains("the record `BOOT` has no file"),
                "{language:?}: {failed}"
            );
            assert!(!failed.contains('\u{2068}'), "{language:?}: {failed}");
        }
    }

    /// The Multi Flashing workspace has a tile of its own in the Mode 2 grid:
    /// its table of devices, its dialogs and its way out must be present in
    /// every locale.
    #[test]
    fn the_multi_flash_page_renders_in_every_locale() {
        for language in Language::ALL {
            let bundle = bundle_for(language);

            for id in [
                "flash-multi-flashing-title",
                "flash-multi-flashing-button",
                "multi-flash-idle",
                "multi-flash-exit",
                "multi-flash-preset-manage",
                "firmware-flash-locate-log",
                "multi-flash-preset-add",
                "multi-flash-preset-finish",
                "multi-flash-preset-empty",
                "multi-flash-start",
                "multi-flash-stop",
                "multi-flash-close-prompt",
                "multi-flash-close-prompt-ok",
                "multi-flash-col-progress",
                "multi-flash-col-current",
                "multi-flash-no-preset",
                "multi-flash-device-finished",
                "multi-flash-options-title",
                "multi-flash-options-reboot",
                "multi-flash-options-conditions",
                "multi-flash-options-default",
                "multi-flash-options-default-desc",
                "multi-flash-options-no-engineering",
                "multi-flash-options-custom",
                "multi-flash-options-cid",
                "multi-flash-options-cid-add",
                "multi-flash-options-cid-preset",
                "multi-flash-options-states",
                "multi-flash-options-incomplete",
                "multi-flash-options-confirm",
                "multi-flash-options-extract",
                "multi-flash-options-extract-choose",
                "multi-flash-options-extract-required",
                "multi-flash-options-unpacking",
                "multi-flash-preset-reboot",
                "multi-flash-preset-edit",
                "multi-flash-preset-remove",
                "multi-flash-preset-codename",
                "multi-flash-preset-save",
                "multi-flash-preset-load",
                "multi-flash-preset-invalid",
                "multi-flash-preset-no-verify",
            ] {
                let text = bundle.tr(id);

                assert!(!text.trim().is_empty(), "{language:?}: {id}");
                assert!(!text.contains('\u{2068}'), "{language:?}: {id}");
            }

            // The messages that carry a value are formatted as well: a
            // translation that renames `$path`/`$count`/`$version`/`$error`
            // would leave a hole the id check above cannot see.
            for (id, args, expected) in [
                (
                    "multi-flash-preset-saved",
                    vec![("path", "C:\\firmware\\presets.json".to_owned())],
                    "presets.json",
                ),
                (
                    "multi-flash-preset-loaded",
                    vec![
                        ("count", "3".to_owned()),
                        ("path", "C:\\firmware\\presets.json".to_owned()),
                    ],
                    "presets.json",
                ),
                (
                    "multi-flash-preset-load-skipped",
                    vec![("count", "2".to_owned())],
                    "2",
                ),
                (
                    "multi-flash-preset-newer",
                    vec![("version", "7".to_owned())],
                    "7",
                ),
                (
                    "multi-flash-preset-failed",
                    vec![("error", "no space left".to_owned())],
                    "no space left",
                ),
                (
                    "multi-flash-preset-conflict",
                    vec![
                        ("count", "2".to_owned()),
                        ("presets", "#1, #3".to_owned()),
                    ],
                    "#1, #3",
                ),
                (
                    "multi-flash-device-failed",
                    vec![("error", "boot: not enough space".to_owned())],
                    "boot: not enough space",
                ),
                (
                    "multi-flash-run-failed",
                    vec![("error", "the log directory was refused".to_owned())],
                    "the log directory was refused",
                ),
                (
                    "multi-flash-still-flashing",
                    vec![("count", "2".to_owned())],
                    "2",
                ),
                (
                    "multi-flash-log-directory",
                    vec![(
                        "path",
                        "C:\\Users\\me\\Documents\\lmnflash\\logs".to_owned(),
                    )],
                    "lmnflash",
                ),
            ] {
                let text = bundle.tr_with_args(id, &args);

                assert!(text.contains(expected), "{language:?}: {id}: {text}");
                assert!(!text.contains('\u{2068}'), "{language:?}: {id}");
            }
        }
    }

    /// The Partition Manager has a tile of its own in the Mode 2 grid: its
    /// vendor buttons, the table, the actions above it and the tools below it
    /// must be present in every locale.
    #[test]
    fn the_partition_manager_page_renders_in_every_locale() {
        for language in Language::ALL {
            let bundle = bundle_for(language);

            for id in [
                "flash-partition-manager-title",
                "flash-partition-manager-qualcomm-button",
                "flash-partition-manager-mediatek-button",
                "flash-partition-manager-unisoc-button",
                "qc-inject-firehose",
                "qc-dump",
                "qc-write",
                "qc-erase",
                "qc-flash-xml",
                "qc-col-lun",
                "qc-col-begin",
                "qc-col-end",
                "qc-idle",
                "qc-search",
                "qc-dump-entire-storage",
                "qc-danger-zone",
                "qc-danger-repartition",
                "qc-danger-flash-storage",
                "qc-danger-flash-backup",
                "qc-danger-erase-storage",
                "qc-view-log",
                "qc-storage-type",
                "qc-storage-autodetect",
                "qc-storage-emmc",
                "qc-storage-ufs",
                "qc-storage-spi-nor",
                "qc-storage-nand",
                "qc-storage-nvme",
                "qc-port",
                "qc-device-manager-tooltip",
                "qc-reboot-normal",
                "qc-reboot-oem-unlock",
                "qc-reboot-recovery",
                "qc-reboot-fastbootd",
                "qc-firehose-programmer",
                "qc-firehose-programmer-filter",
                "qc-select-port",
                "qc-firehose-injecting",
                "qc-reading-partitions",
                "qc-partitions-none",
                "qc-rebooting",
                "qc-reboot-sent",
                "qc-select-partition",
                "qc-select-one-partition",
                "qc-dump-folder-title",
                "qc-dump-file-title",
                "qc-dump-storage-file-title",
                "qc-dump-storage-folder-title",
                "qc-lun-hint",
                "qc-lun-reading",
                "qc-lun-none",
                "qc-close-title",
                "qc-close-body",
                "qc-close-confirm",
                "qc-flash-storage-hint",
                "qc-flash-storage-pick-one",
                "qc-flash-storage-pick-many",
                "qc-flash-storage-none",
                "qc-flash-storage-nothing",
                "qc-flash-storage-go",
                "qc-flash-storage-title",
                "qc-erase-confirm-title",
                "qc-write-file-title",
                "qc-image-filter",
                // The table head and the controls reused from other pages are
                // as much a part of this page as its own strings.
                "partition-flash-partition",
                "pac-repack-col-size",
                "multi-flash-exit",
                "firmware-flash-reboot",
                "flash-factory-reset-title",
            ] {
                let text = bundle.tr(id);

                assert!(!text.trim().is_empty(), "{language:?}: {id}");
                assert!(!text.contains('\u{2068}'), "{language:?}: {id}");
            }

            // The parameterized lines: a locale that names `$count`, `$error`
            // or `$query` differently leaves a hole the parity test cannot
            // see, so the value has to land in the sentence.
            for (id, args) in [
                ("qc-partitions-found", vec![("count", "7"), ("storage", "UFS")]),
                ("qc-firehose-failed", vec![("error", "NAK")]),
                ("qc-reboot-failed", vec![("error", "NAK")]),
                ("qc-no-match", vec![("query", "boot")]),
                ("qc-dumping", vec![("name", "boot_a"), ("percent", "42")]),
                ("qc-writing", vec![("name", "boot_a"), ("percent", "42")]),
                (
                    "qc-dump-done",
                    vec![("count", "2"), ("path", "C:\\dumps")],
                ),
                ("qc-dump-failed", vec![("error", "NAK")]),
                (
                    "qc-lun-entry",
                    vec![
                        ("lun", "4"),
                        ("bytes", "118,111,436,800"),
                        ("size", "110.00 GB"),
                    ],
                ),
                ("qc-lun-failed", vec![("error", "NAK")]),
                (
                    "qc-storage-dump-done",
                    vec![("count", "2"), ("path", "C:\\dumps")],
                ),
                ("qc-storage-dump-failed", vec![("error", "NAK")]),
                (
                    "qc-flash-storage-smaller",
                    vec![("size", "0.50 MB")],
                ),
                (
                    "qc-flash-storage-larger",
                    vec![("size", "0.50 MB")],
                ),
                ("qc-storage-flash-done", vec![("count", "2")]),
                ("qc-storage-flash-failed", vec![("error", "NAK")]),
                ("qc-write-done", vec![("name", "boot_a")]),
                ("qc-write-failed", vec![("error", "NAK")]),
                ("qc-erasing", vec![("name", "boot_a"), ("percent", "42")]),
                ("qc-erase-done", vec![("count", "2")]),
                ("qc-erase-failed", vec![("error", "NAK")]),
                ("qc-erase-confirm", vec![("count", "2")]),
                ("qc-refresh-failed", vec![("error", "NAK")]),
            ] {
                let text = bundle.tr_with_args(
                    id,
                    &args
                        .iter()
                        .map(|(key, value)| (*key, (*value).to_owned()))
                        .collect::<Vec<_>>(),
                );

                for (_, value) in args {
                    assert!(text.contains(value), "{language:?}: {id}: {text}");
                }
                assert!(!text.contains('\u{2068}'), "{language:?}: {id}");
            }
        }
    }

    /// The Firmware Flash description exists in two variants: the plain one,
    /// and one without the "erases all data" clause shown while no user-data
    /// erase is selected. Both must be present in every locale.
    #[test]
    fn the_firmware_flash_description_renders_in_every_locale() {
        for language in Language::ALL {
            let bundle = bundle_for(language);

            for id in ["firmware-flash-desc", "firmware-flash-desc-no-erase"] {
                let text = bundle.tr(id);

                assert!(!text.trim().is_empty(), "{language:?}: {id}");
                assert!(!text.contains('\u{2068}'), "{language:?}: {id}");
            }
        }
    }

    /// The question asked before an export, and the line naming which tool the
    /// copy is: a locale that spells the argument differently would leave a
    /// hole in the sentence, which the id parity test cannot see.
    #[test]
    fn the_export_copy_question_renders_in_every_locale() {
        for language in Language::ALL {
            let bundle = bundle_for(language);

            let question = bundle.tr("firmware-flash-export-copy-question");
            assert!(!question.trim().is_empty(), "{language:?}");
            assert!(!question.contains('\u{2068}'), "{language:?}");

            let note = bundle.tr_with_args(
                "firmware-flash-export-copy-note",
                &[("tool", "mfastboot 34.0.4".to_owned())],
            );

            assert!(
                note.contains("mfastboot 34.0.4"),
                "{language:?}: {note}"
            );
            assert!(!note.contains('\u{2068}'), "{language:?}: {note}");
        }
    }
}

