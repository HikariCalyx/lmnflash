use crate::{Theme, spacing};

use iced_widget::{
    Radio,
    radio::{Catalog, Status, Style, StyleFn},
};

use iced_widget::radio;

// Radio specifications and styling from
// https://storybooks.fluentui.dev/react/?path=/docs/components-radiogroup--docs
// and
// https://github.com/microsoft/fluentui/blob/f395ff9d7e54605b4200ebc71151e81dec87a6bb/packages/react-components/react-radio/library/src/components/Radio/useRadioStyles.styles.ts#L1

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

    match status {
        Status::Active { is_selected } => Style {
            background: tokens.transparent_background.into(),
            dot_color: tokens.compound_brand_foreground1,
            border_width: 1.0,
            border_color: if is_selected {
                tokens.compound_brand_stroke
            } else {
                tokens.neutral_stroke_accessible
            },
            text_color: if is_selected {
                Some(tokens.neutral_foreground1)
            } else {
                Some(tokens.neutral_foreground3)
            },
        },
        Status::Hovered { is_selected } => Style {
            background: tokens.transparent_background.into(),
            dot_color: tokens.compound_brand_foreground1_hover,
            border_width: 1.0,
            border_color: if is_selected {
                tokens.compound_brand_stroke_hover
            } else {
                tokens.neutral_stroke_accessible_hover
            },
            text_color: Some(tokens.neutral_foreground2),
        },
    }
}

pub fn disabled(theme: &Theme, _status: Status) -> Style {
    let tokens = theme.tokens();

    Style {
        background: tokens.transparent_background.into(),
        dot_color: tokens.neutral_foreground_disabled,
        border_width: 1.0,
        border_color: tokens.neutral_stroke_disabled,
        text_color: Some(tokens.neutral_foreground_disabled),
    }
}

pub fn regular<'a, Message, Renderer, F, V>(
    label: impl Into<String>,
    value: V,
    selected: Option<V>,
    f: F,
) -> Radio<'a, Message, Theme, Renderer>
where
    Message: Clone,
    Renderer: iced_core::text::Renderer,
    V: Eq + Copy,
    F: FnOnce(V) -> Message,
{
    radio(label, value, selected, f).spacing(spacing::M)
}
