use crate::{Theme, border_radius, font, stroke_width};

use iced_core::{
    Border, Color, Font,
    text::{self, IntoFragment, LineHeight, Shaping},
};

use iced_widget::{
    Checkbox,
    checkbox::{Catalog, Icon, Status, Style, StyleFn},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ThreeState {
    Checked,
    Unchecked,
    Indeterminate,
}

impl ThreeState {
    /// Returns the [`ThreeState`] of a [`Checkbox`] group.
    pub fn group_state<'a>(states: impl Iterator<Item = &'a bool>) -> Self {
        let states = states.collect::<Vec<_>>();

        if states.iter().all(|b| **b) {
            Self::Checked
        } else if states.iter().any(|b| **b) {
            Self::Indeterminate
        } else {
            Self::Unchecked
        }
    }

    /// Returns the next [`ThreeState`] when an indeterminate [`Checkbox`] is toggled.
    pub fn toggle(&mut self) -> Self {
        match self {
            ThreeState::Checked => ThreeState::Unchecked,
            ThreeState::Unchecked => ThreeState::Checked,
            ThreeState::Indeterminate => ThreeState::Checked,
        }
    }
}

fn base<'a, Message, Renderer>(
    label: impl IntoFragment<'a>,
    is_checked: bool,
    code_point: char,
) -> Checkbox<'a, Message, Theme, Renderer>
where
    Renderer: text::Renderer<Font = Font>,
{
    let check_mark = Icon {
        font: font::ICONS,
        code_point,
        size: Some(14.0.into()),
        line_height: LineHeight::default(),
        shaping: Shaping::Basic,
    };

    Checkbox::new(is_checked)
        .label(label)
        .size(16)
        .font(font::REGULAR)
        .text_size(font::size::BASE300)
        .text_line_height(font::line_height::BASE300)
        .icon(check_mark)
}

pub fn two_state<'a, Message, Renderer>(
    label: impl IntoFragment<'a>,
    is_checked: bool,
) -> Checkbox<'a, Message, Theme, Renderer>
where
    Renderer: text::Renderer<Font = Font>,
{
    base(label, is_checked, '✔')
}

pub fn three_state<'a, Message, Renderer>(
    label: impl IntoFragment<'a>,
    check_state: ThreeState,
) -> Checkbox<'a, Message, Theme, Renderer>
where
    Renderer: text::Renderer<Font = Font>,
{
    match check_state {
        ThreeState::Checked => base(label, true, '✔'),
        ThreeState::Unchecked => base(label, false, '✔'),
        ThreeState::Indeterminate => base(label, true, '▬'),
    }
}

// Checkbox specifications and styling from
// https://storybooks.fluentui.dev/react/?path=/docs/components-checkbox--docs
// and
// https://github.com/microsoft/fluentui/blob/e63be2ef9a165f4c52071b63ada90956bf45481e/packages/react-components/react-checkbox/library/src/components/Checkbox/useCheckboxStyles.styles.ts

impl Catalog for Theme {
    type Class<'a> = StyleFn<'a, Self>;

    fn default<'a>() -> Self::Class<'a> {
        Box::new(default)
    }

    fn style(&self, class: &Self::Class<'_>, status: Status) -> Style {
        class(self, status)
    }
}

pub fn default(theme: &Theme, status: Status) -> Style {
    let tokens = theme.tokens();

    const BASE_BORDER: Border = Border {
        color: Color::TRANSPARENT,
        radius: border_radius::SMALL,
        width: stroke_width::THIN,
    };

    match status {
        Status::Active { is_checked } => Style {
            background: if is_checked {
                tokens.compound_brand_background.into()
            } else {
                tokens.transparent_background.into()
            },
            icon_color: tokens.neutral_foreground_on_brand,
            border: Border {
                color: if is_checked {
                    tokens.compound_brand_background
                } else {
                    tokens.neutral_stroke_accessible
                },
                ..BASE_BORDER
            },
            text_color: Some(tokens.neutral_foreground1),
        },
        Status::Hovered { is_checked } => Style {
            background: if is_checked {
                tokens.compound_brand_background_hover.into()
            } else {
                tokens.transparent_background_hover.into()
            },
            icon_color: tokens.neutral_foreground_on_brand,
            border: Border {
                color: if is_checked {
                    tokens.compound_brand_background_hover
                } else {
                    tokens.neutral_stroke_accessible
                },
                ..BASE_BORDER
            },
            text_color: Some(tokens.neutral_foreground1),
        },
        Status::Disabled { .. } => Style {
            background: tokens.transparent_background.into(),
            icon_color: tokens.neutral_foreground_disabled,
            border: Border {
                color: tokens.neutral_stroke_disabled,
                ..BASE_BORDER
            },
            text_color: Some(tokens.neutral_foreground_disabled),
        },
    }
}
