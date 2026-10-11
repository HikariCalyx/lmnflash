//! The Fluent 2 theme used by the whole application.
//!
//! This is a thin adapter over the [`iced_fluent_theme`] crate. In iced 0.14 a
//! theme is a *type* — it implements [`iced::theme::Base`] and provides the
//! widget `Catalog` implementations — so the crate's [`Theme`] becomes the
//! application's theme type (see the `Element` alias in `main.rs`).
//!
//! The crate exposes two variants, `Theme::Light(Tokens)` and
//! `Theme::Dark(Tokens)`; this module only decides which one to use, following
//! the OS light/dark preference like iced does by default.
//!
//! [`iced_fluent_theme`]: https://crates.io/crates/iced-fluent-theme

use std::sync::OnceLock;

/// The Fluent 2 theme type, re-exported so the rest of the crate can name it
/// as `crate::fluent::Theme`.
pub(crate) use iced_fluent_theme::Theme;

/// The Fluent 2 design tokens of a theme.
///
/// Widget styles read these directly (`theme.tokens().neutral_background4`)
/// because the crate's [`iced::theme::Base::palette`] implementation returns
/// `None` — Fluent 2 has far more colours than a five-colour iced palette can
/// hold. `theme.tokens()` needs no path import, so this re-export is here only
/// to keep the adapter's surface complete.
#[allow(unused_imports)]
pub(crate) use iced_fluent_theme::tokens::Tokens;

static LIGHT: OnceLock<Theme> = OnceLock::new();
static DARK: OnceLock<Theme> = OnceLock::new();

/// The light Fluent 2 theme, built from the crate's default brand ramp.
pub(crate) fn light() -> Theme {
    LIGHT.get_or_init(|| Theme::light(None)).clone()
}

/// The dark Fluent 2 theme, built from the crate's default brand ramp.
pub(crate) fn dark() -> Theme {
    DARK.get_or_init(|| Theme::dark(None)).clone()
}

/// The Fluent 2 theme matching the OS light/dark preference.
///
/// The preference is resolved by `main` — iced 0.14 reports it through
/// [`iced::system::theme`], which is a `Task` — and handed in as [`State`]'s
/// `dark` flag, so that the window style and the theme are chosen from the
/// same value (see `app_theme`).
///
/// [`State`]: crate::State
pub(crate) fn theme_for(dark_mode: bool) -> Theme {
    if dark_mode { dark() } else { light() }
}

/// The Fluent page (window) background colour of a theme.
pub(crate) fn window_background(theme: &Theme) -> iced::Color {
    theme.tokens().neutral_background4
}

/// The style of a Fluent card.
///
/// The crate has no `rounded_box`, so this mirrors it from the tokens: a card
/// surface with Fluent's 4 px corner radius.
pub(crate) fn card(theme: &Theme) -> iced::widget::container::Style {
    let tokens = theme.tokens();

    iced::widget::container::Style {
        text_color: Some(tokens.neutral_foreground1),
        background: Some(tokens.neutral_card_background.into()),
        border: iced::border::rounded(4.0),
        ..Default::default()
    }
}

/// Fluent's error text style (iced's `text::danger`).
pub(crate) fn danger(theme: &Theme) -> iced::widget::text::Style {
    iced_fluent_theme::text::error(theme)
}

/// Fluent's success text style (iced's `text::success`).
pub(crate) fn success(theme: &Theme) -> iced::widget::text::Style {
    iced_fluent_theme::text::success(theme)
}

/// Fluent's primary button style (iced's `button::primary`).
pub(crate) fn primary_button(
    theme: &Theme,
    status: iced::widget::button::Status,
) -> iced::widget::button::Style {
    iced_fluent_theme::button::rounded::primary(theme, status)
}

/// Fluent's secondary button style (iced's `button::secondary`), which is what
/// a plain `button(..)` wears: [`iced_fluent_theme::button::Catalog`]'s default
/// class is `rounded::secondary`.
pub(crate) fn secondary_button(
    theme: &Theme,
    status: iced::widget::button::Status,
) -> iced::widget::button::Style {
    iced_fluent_theme::button::rounded::secondary(theme, status)
}

/// Fluent's pick-list style (iced's `pick_list::default`).
pub(crate) fn pick_list_default(
    theme: &Theme,
    status: iced::widget::pick_list::Status,
) -> iced::widget::pick_list::Style {
    iced_fluent_theme::pick_list::default(theme, status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_light_and_dark_variants_differ() {
        assert_ne!(window_background(&light()), window_background(&dark()));
    }

    #[test]
    fn the_theme_follows_the_os_preference() {
        assert_eq!(
            window_background(&theme_for(true)),
            window_background(&dark())
        );
        assert_eq!(
            window_background(&theme_for(false)),
            window_background(&light())
        );
    }
}
