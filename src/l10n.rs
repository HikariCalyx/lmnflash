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

    fn locale(self) -> &'static str {
        match self {
            Self::EnUs => "en-US",
            Self::ZhHans => "zh-Hans",
        }
    }
}

/// Builds the application bundle for the given language, embedded at
/// compile time.
pub fn bundle_for(language: Language) -> Bundle {
    match language {
        Language::EnUs => Bundle::new("en-US", &[include_str!("../l10n/en-US/app.ftl")]),
        Language::ZhHans => {
            Bundle::new("zh-Hans", &[include_str!("../l10n/zh-Hans/app.ftl")])
        }
    }
}

/// The default application bundle.
pub fn default_bundle() -> Bundle {
    bundle_for(Language::default())
}
