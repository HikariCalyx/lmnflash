use crate::{
    Theme,
    font::{self, line_height, size},
};

pub use iced_core::{
    Alignment, Font,
    text::IntoFragment,
    widget::text::{self, Shaping, Style},
};

use iced_widget::Text;
use iced_widget::text::Catalog;

impl Catalog for Theme {
    type Class<'a> = Box<dyn Fn(&Theme) -> Style + 'a>;

    fn default<'a>() -> Self::Class<'a> {
        Box::new(|_theme| Style::default())
    }

    fn style(&self, class: &Self::Class<'_>) -> Style {
        class(self)
    }
}

// Guidelines and naming from:
// https://storybooks.fluentui.dev/react/?path=/docs/theme-typography--docs

pub fn caption_2<'a, Theme, Renderer>(fragment: impl IntoFragment<'a>) -> Text<'a, Theme, Renderer>
where
    Theme: text::Catalog,
    Renderer: iced_core::text::Renderer,
    <Renderer as iced_core::text::Renderer>::Font: From<Font>,
{
    Text::new(fragment)
        .font(font::REGULAR)
        .size(size::BASE100)
        .line_height(line_height::BASE100)
}

pub fn caption_2_strong<'a, Theme, Renderer>(
    fragment: impl IntoFragment<'a>,
) -> Text<'a, Theme, Renderer>
where
    Theme: text::Catalog,
    Renderer: iced_core::text::Renderer,
    <Renderer as iced_core::text::Renderer>::Font: From<Font>,
{
    Text::new(fragment)
        .font(font::SEMIBOLD)
        .size(size::BASE100)
        .line_height(line_height::BASE100)
}

pub fn caption_1<'a, Theme, Renderer>(fragment: impl IntoFragment<'a>) -> Text<'a, Theme, Renderer>
where
    Theme: text::Catalog,
    Renderer: iced_core::text::Renderer,
    <Renderer as iced_core::text::Renderer>::Font: From<Font>,
{
    Text::new(fragment)
        .font(font::REGULAR)
        .size(size::BASE200)
        .line_height(line_height::BASE200)
}

pub fn caption_1_strong<'a, Theme, Renderer>(
    fragment: impl IntoFragment<'a>,
) -> Text<'a, Theme, Renderer>
where
    Theme: text::Catalog,
    Renderer: iced_core::text::Renderer,
    <Renderer as iced_core::text::Renderer>::Font: From<Font>,
{
    Text::new(fragment)
        .font(font::SEMIBOLD)
        .size(size::BASE200)
        .line_height(line_height::BASE200)
}

pub fn caption_1_stronger<'a, Theme, Renderer>(
    fragment: impl IntoFragment<'a>,
) -> Text<'a, Theme, Renderer>
where
    Theme: text::Catalog,
    Renderer: iced_core::text::Renderer,
    <Renderer as iced_core::text::Renderer>::Font: From<Font>,
{
    Text::new(fragment)
        .font(font::BOLD)
        .size(size::BASE200)
        .line_height(line_height::BASE200)
}

pub fn body_1<'a, Theme, Renderer>(fragment: impl IntoFragment<'a>) -> Text<'a, Theme, Renderer>
where
    Theme: text::Catalog,
    Renderer: iced_core::text::Renderer,
    <Renderer as iced_core::text::Renderer>::Font: From<Font>,
{
    Text::new(fragment)
        .font(font::REGULAR)
        .size(size::BASE300)
        .line_height(line_height::BASE300)
}

pub fn body_1_strong<'a, Theme, Renderer>(
    fragment: impl IntoFragment<'a>,
) -> Text<'a, Theme, Renderer>
where
    Theme: text::Catalog,
    Renderer: iced_core::text::Renderer,
    <Renderer as iced_core::text::Renderer>::Font: From<Font>,
{
    Text::new(fragment)
        .font(font::SEMIBOLD)
        .size(size::BASE300)
        .line_height(line_height::BASE300)
}

pub fn body_1_stronger<'a, Theme, Renderer>(
    fragment: impl IntoFragment<'a>,
) -> Text<'a, Theme, Renderer>
where
    Theme: text::Catalog,
    Renderer: iced_core::text::Renderer,
    <Renderer as iced_core::text::Renderer>::Font: From<Font>,
{
    Text::new(fragment)
        .font(font::BOLD)
        .size(size::BASE300)
        .line_height(line_height::BASE300)
}

pub fn body_2<'a, Theme, Renderer>(fragment: impl IntoFragment<'a>) -> Text<'a, Theme, Renderer>
where
    Theme: text::Catalog,
    Renderer: iced_core::text::Renderer,
    <Renderer as iced_core::text::Renderer>::Font: From<Font>,
{
    Text::new(fragment)
        .font(font::REGULAR)
        .size(size::BASE400)
        .line_height(line_height::BASE400)
}

pub fn subtitle_2<'a, Theme, Renderer>(fragment: impl IntoFragment<'a>) -> Text<'a, Theme, Renderer>
where
    Theme: text::Catalog,
    Renderer: iced_core::text::Renderer,
    <Renderer as iced_core::text::Renderer>::Font: From<Font>,
{
    Text::new(fragment)
        .font(font::SEMIBOLD)
        .size(size::BASE400)
        .line_height(line_height::BASE400)
}

pub fn subtitle_2_stronger<'a, Theme, Renderer>(
    fragment: impl IntoFragment<'a>,
) -> Text<'a, Theme, Renderer>
where
    Theme: text::Catalog,
    Renderer: iced_core::text::Renderer,
    <Renderer as iced_core::text::Renderer>::Font: From<Font>,
{
    Text::new(fragment)
        .font(font::BOLD)
        .size(size::BASE400)
        .line_height(line_height::BASE400)
}

pub fn subtitle_1<'a, Theme, Renderer>(fragment: impl IntoFragment<'a>) -> Text<'a, Theme, Renderer>
where
    Theme: text::Catalog,
    Renderer: iced_core::text::Renderer,
    <Renderer as iced_core::text::Renderer>::Font: From<Font>,
{
    Text::new(fragment)
        .font(font::SEMIBOLD)
        .size(size::BASE500)
        .line_height(line_height::BASE500)
}

pub fn title_3<'a, Theme, Renderer>(fragment: impl IntoFragment<'a>) -> Text<'a, Theme, Renderer>
where
    Theme: text::Catalog,
    Renderer: iced_core::text::Renderer,
    <Renderer as iced_core::text::Renderer>::Font: From<Font>,
{
    Text::new(fragment)
        .font(font::SEMIBOLD)
        .size(size::BASE600)
        .line_height(line_height::BASE600)
}

pub fn title_2<'a, Theme, Renderer>(fragment: impl IntoFragment<'a>) -> Text<'a, Theme, Renderer>
where
    Theme: text::Catalog,
    Renderer: iced_core::text::Renderer,
    <Renderer as iced_core::text::Renderer>::Font: From<Font>,
{
    Text::new(fragment)
        .font(font::SEMIBOLD)
        .size(size::HERO700)
        .line_height(line_height::HERO700)
}

pub fn title_1<'a, Theme, Renderer>(fragment: impl IntoFragment<'a>) -> Text<'a, Theme, Renderer>
where
    Theme: text::Catalog,
    Renderer: iced_core::text::Renderer,
    <Renderer as iced_core::text::Renderer>::Font: From<Font>,
{
    Text::new(fragment)
        .font(font::SEMIBOLD)
        .size(size::HERO800)
        .line_height(line_height::HERO800)
}

pub fn large_title<'a, Theme, Renderer>(
    fragment: impl IntoFragment<'a>,
) -> Text<'a, Theme, Renderer>
where
    Theme: text::Catalog,
    Renderer: iced_core::text::Renderer,
    <Renderer as iced_core::text::Renderer>::Font: From<Font>,
{
    Text::new(fragment)
        .font(font::SEMIBOLD)
        .size(size::HERO900)
        .line_height(line_height::HERO900)
}

pub fn display<'a, Theme, Renderer>(fragment: impl IntoFragment<'a>) -> Text<'a, Theme, Renderer>
where
    Theme: text::Catalog,
    Renderer: iced_core::text::Renderer,
    <Renderer as iced_core::text::Renderer>::Font: From<Font>,
{
    Text::new(fragment)
        .font(font::SEMIBOLD)
        .size(size::HERO1000)
        .line_height(line_height::HERO1000)
}

pub fn error(theme: &Theme) -> Style {
    Style {
        color: Some(theme.tokens().status_danger_foreground1),
    }
}

pub fn warning(theme: &Theme) -> Style {
    Style {
        color: Some(theme.tokens().status_warning_foreground1),
    }
}

pub fn success(theme: &Theme) -> Style {
    Style {
        color: Some(theme.tokens().status_success_foreground1),
    }
}
