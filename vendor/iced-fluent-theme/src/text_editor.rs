use crate::{Theme, border_radius, stroke_width};
use iced_core::Border;

use iced_widget::text_editor::{Catalog, Status, Style, StyleFn};

// The upstream crate ships no `text_editor` styling, but iced 0.14's
// `text_editor` helper requires `Theme: text_editor::Catalog`, so the theme
// cannot be used as an application theme without it. This module adds the
// missing implementation, matching the look of `text_input`.

impl Catalog for Theme {
    type Class<'a> = StyleFn<'a, Theme>;

    fn default<'a>() -> Self::Class<'a> {
        Box::new(default)
    }

    fn style(&self, class: &Self::Class<'_>, status: Status) -> Style {
        class(self, status)
    }
}

pub fn default(theme: &Theme, status: Status) -> Style {
    let tokens = theme.tokens();

    let base = Style {
        background: tokens.neutral_background1.into(),
        border: Border {
            color: tokens.neutral_stroke1,
            width: stroke_width::THIN,
            radius: border_radius::MEDIUM,
        },
        placeholder: tokens.neutral_foreground4,
        value: tokens.neutral_foreground1,
        selection: tokens.brand_background2,
    };

    match status {
        Status::Active => base,
        Status::Hovered => Style {
            border: Border {
                color: tokens.neutral_stroke1_hover,
                ..base.border
            },
            ..base
        },
        Status::Focused { .. } => Style {
            border: Border {
                color: tokens.brand_stroke1,
                ..base.border
            },
            ..base
        },
        Status::Disabled => Style {
            background: tokens.transparent_background.into(),
            border: Border {
                color: tokens.neutral_stroke_disabled,
                ..base.border
            },
            ..base
        },
    }
}
