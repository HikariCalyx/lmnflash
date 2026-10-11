pub mod item;
use crate::Theme;
use iced_core::{Border, Shadow};
use iced_widget_kit::selector_bar::{Catalog, Style, StyleFn};

// Tab specifications and styling from
// https://react.fluentui.dev/?path=/docs/components-tablist--docs
// and
// https://github.com/microsoft/fluentui/blob/e63be2ef9a165f4c52071b63ada90956bf45481e/packages/react-components/react-tabs/library/src/components/TabList/useTabListStyles.styles.ts
// https://github.com/microsoft/fluentui/blob/e63be2ef9a165f4c52071b63ada90956bf45481e/packages/react-components/react-tabs/library/src/components/Tab/useTabAnimatedIndicator.styles.ts

impl Catalog for Theme {
    type Class<'a> = StyleFn<'a, Self>;

    fn default<'a>() -> Self::Class<'a> {
        Box::new(transparent)
    }

    fn style(&self, class: &Self::Class<'_>) -> Style {
        class(self)
    }
}

pub fn transparent(_theme: &Theme) -> Style {
    Style {
        background: None,
        border: Border::default(),
        shadow: Shadow::default(),
        snap: true,
    }
}
