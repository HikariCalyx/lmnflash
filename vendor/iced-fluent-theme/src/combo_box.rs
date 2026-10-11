use crate::{
    Theme,
    font::{
        ICONS,
        size::{self, BASE300, BASE400, BASE500},
    },
    spacing,
};

use iced_core::{Padding, text};
use iced_widget::text_input::{Icon, Side};
use iced_widget::{ComboBox, combo_box::State};
use iced_widget::{combo_box, text_input};

impl iced_widget::combo_box::Catalog for crate::Theme {
    fn default_input<'a>() -> <Self as text_input::Catalog>::Class<'a> {
        Box::new(default_input)
    }
}

pub fn small<'a, T, Message, Renderer>(
    state: &'a State<T>,
    placeholder: &str,
    selection: Option<&T>,
    on_selected: impl Fn(T) -> Message + 'static,
) -> ComboBox<'a, T, Message, Theme, Renderer>
where
    T: std::fmt::Display + Clone,
    Renderer: text::Renderer<Font = iced_core::Font>,
{
    combo_box(state, placeholder, selection, on_selected)
        .size(size::BASE200)
        .padding([spacing::XS.0, spacing::S.0])
        .icon(Icon {
            font: ICONS,
            code_point: '⌄',
            size: Some(BASE300),
            spacing: spacing::M.0,
            side: Side::Right,
        })
}

pub fn medium<'a, T, Message, Renderer>(
    state: &'a State<T>,
    placeholder: &str,
    selection: Option<&T>,
    on_selected: impl Fn(T) -> Message + 'static,
) -> ComboBox<'a, T, Message, Theme, Renderer>
where
    T: std::fmt::Display + Clone,
    Renderer: text::Renderer<Font = iced_core::Font>,
{
    combo_box(state, placeholder, selection, on_selected)
        .size(size::BASE300)
        .padding(Padding {
            top: spacing::S.0,
            right: spacing::M.0,
            bottom: spacing::SNUDGE.0,
            left: spacing::M.0,
        })
        .icon(Icon {
            font: ICONS,
            code_point: '⌄',
            size: Some(BASE400),
            spacing: spacing::M.0,
            side: Side::Right,
        })
}

pub fn large<'a, T, Message, Renderer>(
    state: &'a State<T>,
    placeholder: &str,
    selection: Option<&T>,
    on_selected: impl Fn(T) -> Message + 'static,
) -> ComboBox<'a, T, Message, Theme, Renderer>
where
    T: std::fmt::Display + Clone,
    Renderer: text::Renderer<Font = iced_core::Font>,
{
    combo_box(state, placeholder, selection, on_selected)
        .size(size::BASE400)
        .padding(Padding {
            top: spacing::MNUDGE.0,
            right: spacing::L.0,
            bottom: 9.0, // Nudge height to 40
            left: spacing::L.0,
        })
        .icon(Icon {
            font: ICONS,
            code_point: '⌄',
            size: Some(BASE500),
            spacing: spacing::M.0,
            side: Side::Right,
        })
}

pub fn default_input(theme: &Theme, status: text_input::Status) -> text_input::Style {
    text_input::Style {
        icon: theme.tokens().neutral_foreground4,
        ..crate::text_input::default(theme, status)
    }
}
