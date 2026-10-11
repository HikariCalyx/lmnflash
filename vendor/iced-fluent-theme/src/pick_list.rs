use crate::{
    Theme, border_radius,
    font::{ICONS, line_height, size},
    spacing, stroke_width,
};

use iced_core::{
    Border, Padding,
    text::{self, Shaping},
};

use iced_widget::pick_list;
use std::{borrow::Borrow, fmt};

use iced_widget::{
    PickList,
    overlay::menu,
    pick_list::{Catalog, Handle, Icon, Status, Style, StyleFn},
};

// Button specifications and styling from
// https://storybooks.fluentui.dev/react/?path=/docs/components-select--docs
// and
// https://github.com/microsoft/fluentui/blob/e63be2ef9a165f4c52071b63ada90956bf45481e/packages/react-components/react-select/library/src/components/Select/useSelectStyles.styles.ts

impl Catalog for Theme {
    type Class<'a> = StyleFn<'a, Self>;

    fn default<'a>() -> <Self as Catalog>::Class<'a> {
        Box::new(default)
    }

    fn default_menu<'a>() -> <Self as menu::Catalog>::Class<'a> {
        Box::new(crate::menu::default)
    }

    fn style(&self, class: &<Self as Catalog>::Class<'_>, status: Status) -> Style {
        class(self, status)
    }
}

pub fn default(theme: &Theme, status: Status) -> Style {
    let tokens = theme.tokens();

    let base = Style {
        text_color: tokens.neutral_foreground1,
        placeholder_color: tokens.neutral_foreground4,
        handle_color: tokens.neutral_foreground1,
        background: tokens.neutral_background1.into(),
        border: Border {
            color: tokens.neutral_stroke1,
            width: stroke_width::THIN,
            radius: border_radius::MEDIUM,
        },
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
        Status::Opened { .. } => Style {
            border: Border {
                color: tokens.brand_stroke1,
                ..base.border
            },
            ..base
        },
    }
}

pub fn small<'a, T, L, V, Message, Renderer>(
    selected: Option<V>,
    options: L,
    on_selected: impl Fn(T) -> Message + 'a,
) -> PickList<'a, T, L, V, Message, Theme, Renderer>
where
    T: fmt::Display + PartialEq + Clone + 'a,
    L: Borrow<[T]> + 'a,
    V: Borrow<T> + 'a,
    Message: Clone,
    Renderer: text::Renderer<Font = iced_core::Font>,
{
    pick_list(options, selected, on_selected)
        .text_size(size::BASE200)
        .padding([spacing::XS.0, spacing::S.0])
        .handle(Handle::Static(Icon {
            font: ICONS,
            code_point: '⌄',
            size: Some(size::BASE200),
            line_height: line_height::BASE200,
            shaping: Shaping::Auto,
        }))
}

pub fn medium<'a, T, L, V, Message, Renderer>(
    selected: Option<V>,
    options: L,
    on_selected: impl Fn(T) -> Message + 'a,
) -> PickList<'a, T, L, V, Message, Theme, Renderer>
where
    T: fmt::Display + PartialEq + Clone + 'a,
    L: Borrow<[T]> + 'a,
    V: Borrow<T> + 'a,
    Message: Clone,
    Renderer: text::Renderer<Font = iced_core::Font>,
{
    pick_list(options, selected, on_selected)
        .text_size(size::BASE300)
        .padding(Padding {
            top: spacing::S.0,
            right: spacing::M.0,
            bottom: spacing::SNUDGE.0,
            left: spacing::M.0,
        })
        .handle(Handle::Static(Icon {
            font: ICONS,
            code_point: '⌄',
            size: Some(size::BASE300),
            line_height: line_height::BASE300,
            shaping: Shaping::Auto,
        }))
}

pub fn large<'a, T, L, V, Message, Renderer>(
    selected: Option<V>,
    options: L,
    on_selected: impl Fn(T) -> Message + 'a,
) -> PickList<'a, T, L, V, Message, Theme, Renderer>
where
    T: fmt::Display + PartialEq + Clone + 'a,
    L: Borrow<[T]> + 'a,
    V: Borrow<T> + 'a,
    Message: Clone,
    Renderer: text::Renderer<Font = iced_core::Font>,
{
    pick_list(options, selected, on_selected)
        .text_size(size::BASE400)
        .padding(Padding {
            top: spacing::MNUDGE.0,
            right: spacing::L.0,
            bottom: 9.0, // Nudge height to 40
            left: spacing::L.0,
        })
        .handle(Handle::Static(Icon {
            font: ICONS,
            code_point: '⌄',
            size: Some(size::BASE400),
            line_height: line_height::BASE400,
            shaping: Shaping::Auto,
        }))
}
