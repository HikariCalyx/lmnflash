use crate::{Theme, border_radius, font::size, spacing, stroke_width};
use iced_core::{Border, Padding, text};

use iced_widget::{
    TextInput,
    text_input::{Catalog, Status, Style, StyleFn},
};

use iced_widget::text_input;

// TextInput specifications and styling from
// https://storybooks.fluentui.dev/react/?path=/docs/components-input--docs
// and
// https://github.com/microsoft/fluentui/blob/e63be2ef9a165f4c52071b63ada90956bf45481e/packages/react-components/react-input/library/src/components/Input/useInputStyles.styles.ts

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
        icon: tokens.neutral_foreground1,
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

pub fn small<'a, Message, Renderer>(
    placeholder: &str,
    value: &str,
) -> TextInput<'a, Message, Theme, Renderer>
where
    Message: Clone,
    Renderer: text::Renderer,
{
    text_input(placeholder, value)
        .size(size::BASE200)
        .padding([spacing::XS.0, spacing::S.0])
}

pub fn medium<'a, Message, Renderer>(
    placeholder: &str,
    value: &str,
) -> TextInput<'a, Message, Theme, Renderer>
where
    Message: Clone,
    Renderer: text::Renderer,
{
    text_input(placeholder, value)
        .size(size::BASE300)
        .padding(Padding {
            top: spacing::S.0,
            right: spacing::M.0,
            bottom: spacing::SNUDGE.0,
            left: spacing::M.0,
        })
}

pub fn large<'a, Message, Renderer>(
    placeholder: &str,
    value: &str,
) -> TextInput<'a, Message, Theme, Renderer>
where
    Message: Clone,
    Renderer: text::Renderer,
{
    text_input(placeholder, value)
        .size(size::BASE400)
        .padding(Padding {
            top: spacing::MNUDGE.0,
            right: spacing::L.0,
            bottom: 9.0, // Nudge height to 40
            left: spacing::L.0,
        })
}
