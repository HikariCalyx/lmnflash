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
}

