use crate::{Theme, border_radius};
use iced_widget::rule::{Catalog, FillMode, Style, StyleFn};

// Rule specifications and styling from
// https://storybooks.fluentui.dev/react/?path=/docs/components-divider--docs
// and
// https://github.com/microsoft/fluentui/blob/e63be2ef9a165f4c52071b63ada90956bf45481e/packages/react-components/react-divider/library/src/components/Divider/useDividerStyles.styles.ts

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
        color: tokens.neutral_stroke2,
        radius: border_radius::CIRCULAR,
        fill_mode: FillMode::Full,
        snap: true,
    }
}

pub fn brand(theme: &Theme) -> Style {
    let tokens = theme.tokens();

    Style {
        color: tokens.brand_stroke1,
        radius: border_radius::CIRCULAR,
        fill_mode: FillMode::Full,
        snap: true,
    }
}

pub fn subtle(theme: &Theme) -> Style {
    let tokens = theme.tokens();

    Style {
        color: tokens.neutral_stroke3,
        radius: border_radius::CIRCULAR,
        fill_mode: FillMode::Full,
        snap: true,
    }
}

pub fn strong(theme: &Theme) -> Style {
    let tokens = theme.tokens();

    Style {
        color: tokens.neutral_stroke1,
        radius: border_radius::CIRCULAR,
        fill_mode: FillMode::Full,
        snap: true,
    }
}
