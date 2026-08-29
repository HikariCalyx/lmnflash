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
}

impl Language {
    pub const ALL: [Self; 3] = [Self::EnUs, Self::ZhHans, Self::ZhHant];

    pub fn message_id(self) -> &'static str {
        match self {
            Self::EnUs => "lang-en",
            Self::ZhHans => "lang-zh",
            Self::ZhHant => "lang-zhhant",
        }
    }

    /// The locale code persisted in `config.conf`.
    pub fn code(self) -> &'static str {
        match self {
            Self::EnUs => "en-US",
            Self::ZhHans => "zh-Hans",
            Self::ZhHant => "zh-Hant",
        }
    }

    /// Parses a locale code (also tolerates plain `en` / `zh`).
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "en-US" | "en" => Some(Self::EnUs),
            "zh-Hans" | "zh-CN" | "zh" => Some(Self::ZhHans),
            "zh-Hant" | "zh-TW" | "zh-HK" | "zh-MO" => Some(Self::ZhHant),
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
        let zh_hans = message_ids(include_str!("../l10n/zh-Hans/app.ftl"));
        let zh_hant = message_ids(include_str!("../l10n/zh-Hant/app.ftl"));

        assert_eq!(en, zh_hans);
        assert_eq!(en, zh_hant);
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
        assert_eq!(Language::from_code("fr"), None);
    }
}

