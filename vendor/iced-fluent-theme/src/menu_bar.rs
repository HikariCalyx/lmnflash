use crate::{Theme, border_radius};

use iced_aw::{
    menu::{Catalog, Style},
    style::{Status, StyleFn},
};

use iced_core::{Border, Shadow};

// Menubar specifications and styling from
// https://storybooks.fluentui.dev/react/?path=/docs/components-menu-menulist--docs
// and
// https://github.com/microsoft/fluentui/blob/e63be2ef9a165f4c52071b6g3ada90956bf45481e/packages/react-components/react-menu/library/src/components/MenuPopover/useMenuPopoverStyles.styles.ts

impl Catalog for Theme {
    type Class<'a> = StyleFn<'a, Self, Style>;

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
        bar_background: tokens.transparent_background.into(),
        bar_border: Border::default(),
        bar_shadow: Shadow::default(),
        menu_background: tokens.neutral_background1.into(),
        menu_border: Border {
            color: tokens.transparent_stroke,
            width: 0.0,
            radius: border_radius::MEDIUM,
        },
        menu_shadow: tokens.shadow16,
        path: tokens.neutral_background1_pressed.into(),
        path_border: Border::default(),
    };

    match status {
        Status::Active => base,
        Status::Hovered => Style {
            bar_background: tokens.neutral_background1_hover.into(),
            menu_background: tokens.neutral_background1_hover.into(),
            ..base
        },
        Status::Pressed => Style {
            bar_background: tokens.neutral_background1_pressed.into(),
            menu_background: tokens.neutral_background1_pressed.into(),
            ..base
        },
        Status::Disabled => Style {
            bar_background: tokens.transparent_background.into(),
            menu_background: tokens.transparent_background.into(),
            ..base
        },
        Status::Focused => Style { ..base },
        Status::Selected => Style { ..base },
    }
}
