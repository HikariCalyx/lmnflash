use crate::{Theme, border_radius, stroke_width};

use iced_core::Border;
use iced_widget::{
    overlay::menu::{Catalog, Style, StyleFn},
    scrollable,
};

// Menu specifications and styling from
// https://storybooks.fluentui.dev/react/?path=/docs/components-menu-menu--docs
// and
// https://github.com/microsoft/fluentui/blob/e63be2ef9a165f4c52071b63ada90956bf45481e/packages/react-components/react-menu/library/src/components/MenuPopover/useMenuPopoverStyles.styles.ts

impl Catalog for Theme {
    type Class<'a> = StyleFn<'a, Theme>;

    fn default<'a>() -> <Self as Catalog>::Class<'a> {
        Box::new(default)
    }

    fn default_scrollable<'a>() -> <Self as scrollable::Catalog>::Class<'a> {
        Box::new(crate::scrollable::default)
    }

    fn style(&self, class: &<Self as Catalog>::Class<'_>) -> Style {
        class(self)
    }
}

pub fn default(theme: &Theme) -> Style {
    let tokens = theme.tokens();

    Style {
        background: tokens.neutral_background1.into(),
        border: Border {
            color: tokens.transparent_stroke,
            width: stroke_width::THIN,
            radius: border_radius::MEDIUM,
        },
        text_color: tokens.neutral_foreground1,
        selected_text_color: tokens.neutral_foreground2_pressed,
        selected_background: tokens.neutral_background1_pressed.into(),
        shadow: tokens.shadow16,
    }
}
