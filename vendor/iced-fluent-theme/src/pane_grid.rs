use crate::Theme;
use iced_core::Border;
use iced_widget::pane_grid::{self, Catalog, Highlight, Line, StyleFn};
use pane_grid::Style;

impl Catalog for Theme {
    type Class<'a> = StyleFn<'a, Self>;

    fn default<'a>() -> <Self as Catalog>::Class<'a> {
        Box::new(default)
    }

    fn style(&self, class: &StyleFn<'_, Self>) -> Style {
        class(self)
    }
}

pub fn default(theme: &Theme) -> Style {
    let tokens = theme.tokens();

    let hovered_region = Highlight {
        background: tokens.brand_background.into(),
        border: Border::default(),
    };

    let picked_split = Line {
        color: tokens.neutral_background2_pressed,
        width: 3.0,
    };

    let hovered_split = Line {
        color: tokens.neutral_background2_hover,
        width: 3.0,
    };

    Style {
        hovered_region,
        picked_split,
        hovered_split,
    }
}
