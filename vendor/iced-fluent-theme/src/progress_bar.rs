use crate::{Theme, stroke_width};
use iced_core::{Border, Color, border::Radius};

use iced_widget::{
    ProgressBar,
    progress_bar::{Catalog, Style, StyleFn},
};

use std::ops::RangeInclusive;

// Progress bar specifications and styling from
// https://storybooks.fluentui.dev/react/?path=/docs/components-progressbar--docs
// and
// https://github.com/microsoft/fluentui/blob/e63be2ef9a165f4c52071b63ada90956bf45481e/packages/react-components/react-progress/library/src/components/ProgressBar/useProgressBarStyles.styles.ts

impl Catalog for Theme {
    type Class<'a> = StyleFn<'a, Theme>;

    fn default<'a>() -> Self::Class<'a> {
        Box::new(rounded::brand)
    }

    fn style(&self, class: &Self::Class<'_>) -> Style {
        class(self)
    }
}

fn base(theme: &Theme, color: Color, radius: impl Into<Radius>) -> Style {
    let tokens = theme.tokens();

    Style {
        background: tokens.neutral_background6.into(),
        bar: color.into(),
        border: Border {
            color: tokens.transparent_stroke,
            width: 0.0,
            radius: radius.into(),
        },
    }
}

pub mod rounded {
    use super::*;
    use crate::border_radius;

    pub fn brand(theme: &Theme) -> Style {
        base(
            theme,
            theme.tokens().compound_brand_background,
            border_radius::MEDIUM,
        )
    }

    pub fn error(theme: &Theme) -> Style {
        base(
            theme,
            theme.tokens().status_danger_background3,
            border_radius::MEDIUM,
        )
    }

    pub fn warning(theme: &Theme) -> Style {
        base(
            theme,
            theme.tokens().status_warning_background3,
            border_radius::MEDIUM,
        )
    }

    pub fn success(theme: &Theme) -> Style {
        base(
            theme,
            theme.tokens().status_success_background3,
            border_radius::MEDIUM,
        )
    }
}

pub mod square {
    use super::*;
    use crate::border_radius;

    pub fn brand(theme: &Theme) -> Style {
        base(
            theme,
            theme.tokens().compound_brand_background,
            border_radius::NONE,
        )
    }

    pub fn error(theme: &Theme) -> Style {
        base(
            theme,
            theme.tokens().status_danger_background3,
            border_radius::NONE,
        )
    }

    pub fn warning(theme: &Theme) -> Style {
        base(
            theme,
            theme.tokens().status_warning_background3,
            border_radius::NONE,
        )
    }

    pub fn success(theme: &Theme) -> Style {
        base(
            theme,
            theme.tokens().status_success_background3,
            border_radius::NONE,
        )
    }
}

pub fn medium<'a>(range: RangeInclusive<f32>, value: f32) -> ProgressBar<'a, Theme> {
    iced_widget::progress_bar(range, value)
        .girth(stroke_width::THICK)
        .style(rounded::brand)
}

pub fn large<'a>(range: RangeInclusive<f32>, value: f32) -> ProgressBar<'a, Theme> {
    iced_widget::progress_bar(range, value)
        .girth(stroke_width::THICKEST)
        .style(rounded::brand)
}
