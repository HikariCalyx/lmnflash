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

        for source in sources {
            let resource = FluentResource::try_new((*source).to_owned())
                .expect("failed to parse FTL resource");

            bundle
                .add_resource(resource)
                .expect("failed to add FTL resource");
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
}

impl Language {
    pub const ALL: [Self; 2] = [Self::EnUs, Self::ZhHans];

    pub fn message_id(self) -> &'static str {
        match self {
            Self::EnUs => "lang-en",
            Self::ZhHans => "lang-zh",
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn message_ids(bundle: &Bundle) -> HashSet<String> {
        bundle
            .bundle
            .messages
            .iter()
            .map(|(id, _)| id.to_string())
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
        let en = bundle_for(Language::EnUs);
        let zh = Bundle::new("zh-Hans", &[include_str!("../l10n/zh-Hans/app.ftl")]);

        assert_eq!(message_ids(&en), message_ids(&zh));
    }
}

