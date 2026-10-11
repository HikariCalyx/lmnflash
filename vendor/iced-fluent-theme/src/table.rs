use crate::Theme;
use iced_widget::table::{Catalog, Style, StyleFn};

// Table specifications and styling from
// https://storybooks.fluentui.dev/react/?path=/docs/components-table--docs
// and
// https://github.com/microsoft/fluentui/blob/e63be2ef9a165f4c52071b63ada90956bf45481e/packages/react-components/react-table/library/src/components/TableRow/useTableRowStyles.styles.ts

impl Catalog for Theme {
    type Class<'a> = StyleFn<'a, Self>;

    fn default<'a>() -> Self::Class<'a> {
        Box::new(default)
    }

    fn style(&self, class: &Self::Class<'_>) -> Style {
        class(self)
    }
}

pub fn default(theme: &Theme) -> Style {
    let tokens = theme.tokens();

    Style {
        separator_x: tokens.neutral_foreground1.into(),
        separator_y: tokens.subtle_background.into(),
    }
}
