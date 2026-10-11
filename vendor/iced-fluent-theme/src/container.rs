use crate::Theme;
use iced_widget::container::{Catalog, Style, StyleFn};

impl Catalog for Theme {
    type Class<'a> = StyleFn<'a, Theme>;

    fn default<'a>() -> Self::Class<'a> {
        Box::new(|_| Style::default())
    }

    fn style(&self, class: &Self::Class<'_>) -> Style {
        class(self)
    }
}

pub fn code_block(theme: &Theme) -> Style {
    let tokens = theme.tokens();

    Style {
        text_color: Some(tokens.neutral_foreground1),
        background: Some(tokens.neutral_background4.into()),
        ..Style::default()
    }
}
