use crate::{
    Theme, border_radius, stroke_width,
    text::{body_1_strong, caption_1, subtitle_2},
};

use iced_core::{
    Alignment, Border, Color, Element, Font, Length, Shadow, border::Radius, text::IntoFragment,
};

use iced_widget::{
    Button,
    button::{Catalog, Status, Style, StyleFn},
    text,
};

// Button specifications and styling from
// https://react.fluentui.dev/?path=/docs/components-button-button--docs
// and
// https://github.com/microsoft/fluentui/blob/e63be2ef9a165f4c52071b63ada90956bf45481e/packages/react-components/react-button/library/src/components/Button/useButtonStyles.styles.ts

impl Catalog for Theme {
    type Class<'a> = StyleFn<'a, Self>;

    fn default<'a>() -> Self::Class<'a> {
        Box::new(rounded::secondary)
    }

    fn style(&self, class: &Self::Class<'_>, status: Status) -> Style {
        class(self, status)
    }
}

pub fn small<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
) -> Button<'a, Message, Theme, Renderer>
where
    Theme: Catalog,
    Renderer: iced_core::Renderer,
{
    Button::new(content).clip(true).height(24).padding([0, 16])
}

pub fn small_with_text<'a, Message, Theme, Renderer>(
    fragment: impl IntoFragment<'a>,
) -> Button<'a, Message, Theme, Renderer>
where
    Theme: 'a + Catalog + text::Catalog,
    Renderer: 'a + iced_core::Renderer + iced_core::text::Renderer,
    <Renderer as iced_core::text::Renderer>::Font: From<Font>,
{
    let text = caption_1(fragment)
        .height(Length::Fill)
        .align_y(Alignment::Center);

    Button::new(text).clip(true).height(24).padding([0, 16])
}

pub fn medium<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
) -> Button<'a, Message, Theme, Renderer>
where
    Theme: Catalog,
    Renderer: iced_core::Renderer,
{
    Button::new(content).clip(true).height(32).padding([0, 20])
}

pub fn medium_with_text<'a, Message, Theme, Renderer>(
    fragment: impl IntoFragment<'a>,
) -> Button<'a, Message, Theme, Renderer>
where
    Theme: 'a + Catalog + text::Catalog,
    Renderer: 'a + iced_core::Renderer + iced_core::text::Renderer,
    <Renderer as iced_core::text::Renderer>::Font: From<Font>,
{
    let text = body_1_strong(fragment)
        .height(Length::Fill)
        .align_y(Alignment::Center);

    Button::new(text).clip(true).height(32).padding([0, 20])
}

pub fn large<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
) -> Button<'a, Message, Theme, Renderer>
where
    Theme: Catalog,
    Renderer: iced_core::Renderer,
{
    Button::new(content).clip(true).height(40).padding([0, 24])
}

pub fn large_with_text<'a, Message, Theme, Renderer>(
    fragment: impl IntoFragment<'a>,
) -> Button<'a, Message, Theme, Renderer>
where
    Theme: 'a + Catalog + text::Catalog,
    Renderer: 'a + iced_core::Renderer + iced_core::text::Renderer,
    <Renderer as iced_core::text::Renderer>::Font: From<Font>,
{
    let text = subtitle_2(fragment)
        .height(Length::Fill)
        .align_y(Alignment::Center);

    Button::new(text).clip(true).height(40).padding([0, 24])
}

fn border_base(color: Color, radius: Radius) -> Border {
    Border {
        color,
        width: stroke_width::THIN,
        radius,
    }
}

fn secondary_base(theme: &Theme, status: Status, radius: Radius) -> Style {
    let tokens = theme.tokens();

    Style {
        background: Some(
            match status {
                Status::Active => tokens.neutral_background1,
                Status::Hovered => tokens.neutral_background1_hover,
                Status::Pressed => tokens.neutral_background1_pressed,
                Status::Disabled => tokens.neutral_background_disabled,
            }
            .into(),
        ),
        text_color: match status {
            Status::Active => tokens.neutral_foreground1,
            Status::Hovered => tokens.neutral_foreground1_hover,
            Status::Pressed => tokens.neutral_foreground1_pressed,
            Status::Disabled => tokens.neutral_foreground_disabled,
        },
        border: border_base(
            match status {
                Status::Active => tokens.neutral_stroke1,
                Status::Hovered => tokens.neutral_stroke1_hover,
                Status::Pressed => tokens.neutral_stroke1_pressed,
                Status::Disabled => tokens.neutral_stroke_disabled,
            },
            radius,
        ),
        shadow: Shadow::default(),
        snap: true,
    }
}

fn primary_base(theme: &Theme, status: Status, radius: Radius) -> Style {
    let tokens = theme.tokens();

    Style {
        background: Some(
            match status {
                Status::Active => tokens.brand_background,
                Status::Hovered => tokens.brand_background_hover,
                Status::Pressed => tokens.brand_background_pressed,
                Status::Disabled => tokens.neutral_background_disabled,
            }
            .into(),
        ),
        text_color: match status {
            Status::Active => tokens.neutral_foreground_on_brand,
            Status::Hovered => tokens.neutral_foreground_on_brand,
            Status::Pressed => tokens.neutral_foreground_on_brand,
            Status::Disabled => tokens.neutral_foreground_disabled,
        },
        border: border_base(
            match status {
                Status::Active => tokens.brand_background,
                Status::Hovered => tokens.brand_background_hover,
                Status::Pressed => tokens.brand_background_pressed,
                Status::Disabled => tokens.neutral_background_disabled,
            },
            radius,
        ),
        shadow: Shadow::default(),
        snap: true,
    }
}

fn outline_base(theme: &Theme, status: Status, radius: Radius) -> Style {
    let tokens = theme.tokens();

    Style {
        background: Some(
            match status {
                Status::Active => tokens.transparent_background,
                Status::Hovered => tokens.transparent_background_hover,
                Status::Pressed => tokens.transparent_background_pressed,
                Status::Disabled => tokens.neutral_background_disabled,
            }
            .into(),
        ),
        text_color: match status {
            Status::Active => tokens.neutral_foreground1,
            Status::Hovered => tokens.neutral_foreground1_hover,
            Status::Pressed => tokens.neutral_foreground1_pressed,
            Status::Disabled => tokens.neutral_foreground_disabled,
        },
        border: border_base(
            match status {
                Status::Active => tokens.neutral_stroke1,
                Status::Hovered => tokens.neutral_stroke1_hover,
                Status::Pressed => tokens.neutral_stroke1_pressed,
                Status::Disabled => tokens.neutral_stroke_disabled,
            },
            radius,
        ),
        shadow: Shadow::default(),
        snap: true,
    }
}

fn subtle_base(theme: &Theme, status: Status, radius: Radius) -> Style {
    let tokens = theme.tokens();

    Style {
        background: Some(
            match status {
                Status::Active => tokens.subtle_background,
                Status::Hovered => tokens.subtle_background_hover,
                Status::Pressed => tokens.subtle_background_pressed,
                Status::Disabled => tokens.neutral_background_disabled,
            }
            .into(),
        ),
        text_color: match status {
            Status::Active => tokens.neutral_foreground2,
            Status::Hovered => tokens.neutral_foreground2_hover,
            Status::Pressed => tokens.neutral_foreground2_pressed,
            Status::Disabled => tokens.neutral_foreground_disabled,
        },
        border: border_base(
            match status {
                Status::Active => tokens.subtle_background,
                Status::Hovered => tokens.subtle_background_hover,
                Status::Pressed => tokens.subtle_background_pressed,
                Status::Disabled => tokens.neutral_background_disabled,
            },
            radius,
        ),
        shadow: Shadow::default(),
        snap: true,
    }
}

fn transparent_base(theme: &Theme, status: Status, radius: Radius) -> Style {
    let tokens = theme.tokens();

    Style {
        background: Some(
            match status {
                Status::Active => tokens.transparent_background,
                Status::Hovered => tokens.transparent_background_hover,
                Status::Pressed => tokens.transparent_background_pressed,
                Status::Disabled => tokens.neutral_background_disabled,
            }
            .into(),
        ),
        text_color: match status {
            Status::Active => tokens.neutral_foreground2,
            Status::Hovered => tokens.neutral_foreground2_brand_hover,
            Status::Pressed => tokens.neutral_foreground2_brand_pressed,
            Status::Disabled => tokens.neutral_foreground_disabled,
        },
        border: border_base(Color::TRANSPARENT, radius),
        shadow: Shadow::default(),
        snap: true,
    }
}

pub mod rounded {
    use super::*;

    pub fn secondary(theme: &Theme, status: Status) -> Style {
        secondary_base(theme, status, border_radius::MEDIUM)
    }

    pub fn primary(theme: &Theme, status: Status) -> Style {
        primary_base(theme, status, border_radius::MEDIUM)
    }

    pub fn outline(theme: &Theme, status: Status) -> Style {
        outline_base(theme, status, border_radius::MEDIUM)
    }

    pub fn subtle(theme: &Theme, status: Status) -> Style {
        subtle_base(theme, status, border_radius::MEDIUM)
    }

    pub fn transparent(theme: &Theme, status: Status) -> Style {
        transparent_base(theme, status, border_radius::MEDIUM)
    }
}

pub mod circular {
    use super::*;

    pub fn secondary(theme: &Theme, status: Status) -> Style {
        secondary_base(theme, status, border_radius::CIRCULAR)
    }

    pub fn primary(theme: &Theme, status: Status) -> Style {
        primary_base(theme, status, border_radius::CIRCULAR)
    }

    pub fn outline(theme: &Theme, status: Status) -> Style {
        outline_base(theme, status, border_radius::CIRCULAR)
    }

    pub fn subtle(theme: &Theme, status: Status) -> Style {
        subtle_base(theme, status, border_radius::CIRCULAR)
    }

    pub fn transparent(theme: &Theme, status: Status) -> Style {
        transparent_base(theme, status, border_radius::CIRCULAR)
    }
}

pub mod square {
    use super::*;

    pub fn secondary(theme: &Theme, status: Status) -> Style {
        secondary_base(theme, status, border_radius::NONE)
    }

    pub fn primary(theme: &Theme, status: Status) -> Style {
        primary_base(theme, status, border_radius::NONE)
    }

    pub fn outline(theme: &Theme, status: Status) -> Style {
        outline_base(theme, status, border_radius::NONE)
    }

    pub fn subtle(theme: &Theme, status: Status) -> Style {
        subtle_base(theme, status, border_radius::NONE)
    }

    pub fn transparent(theme: &Theme, status: Status) -> Style {
        transparent_base(theme, status, border_radius::NONE)
    }
}
