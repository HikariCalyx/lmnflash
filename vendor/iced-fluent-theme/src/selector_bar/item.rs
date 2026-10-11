use crate::{border_radius, theme};
use iced_core::{Border, Shadow};
use iced_widget_kit::selector_bar::item::{Catalog, Indicator, Status, Style, StyleFn};

// Tab specifications and styling from
// https://react.fluentui.dev/?path=/docs/components-tablist--docs
// and
// https://github.com/microsoft/fluentui/blob/e63be2ef9a165f4c52071b63ada90956bf45481e/packages/react-components/react-tabs/library/src/components/Tab/useTabStyles.styles.ts

impl Catalog for theme::Theme {
    type Class<'a> = StyleFn<'a, Self>;

    fn default<'a>() -> Self::Class<'a> {
        Box::new(transparent)
    }

    fn style(&self, class: &Self::Class<'_>, status: Status) -> Style {
        class(self, status)
    }
}

pub fn transparent(theme: &theme::Theme, status: Status) -> Style {
    let tokens = theme.tokens();

    let active_colour = match status {
        Status::Active => tokens.compound_brand_stroke,
        Status::Hovered => tokens.compound_brand_stroke_hover,
        Status::Pressed => tokens.compound_brand_stroke_pressed,
    };

    let pending_colour = match status {
        Status::Active => tokens.transparent_stroke,
        Status::Hovered => tokens.neutral_stroke1_hover,
        Status::Pressed => tokens.neutral_stroke1_pressed,
    };

    let border = Border {
        radius: border_radius::CIRCULAR,
        ..Border::default()
    };

    Style {
        background: None,
        border: Border::default(),
        shadow: Shadow::default(),
        snap: true,
        active_indicator: Indicator {
            background: Some(active_colour.into()),
            border,
            shadow: Shadow::default(),
            snap: true,
        },
        pending_indicator: Indicator {
            background: Some(pending_colour.into()),
            border,
            shadow: Shadow::default(),
            snap: true,
        },
    }
}
