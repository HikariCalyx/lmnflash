use crate::{Theme, border_radius};
use iced_core::Border;
use iced_widget::slider::{Catalog, Handle, HandleShape, Rail, Status, Style};

// Slider specifications and styling from
// https://storybooks.fluentui.dev/react/?path=/docs/components-slider--docs
// and
// https://github.com/microsoft/fluentui/blob/e63be2ef9a165f4c52071b63ada90956bf45481e/packages/react-components/react-slider/library/src/components/Slider/useSliderStyles.styles.ts

pub type StyleFn<'a, Theme> = Box<dyn Fn(&Theme, Status) -> Style + 'a>;

impl Catalog for Theme {
    type Class<'a> = StyleFn<'a, Self>;

    fn default<'a>() -> Self::Class<'a> {
        Box::new(medium)
    }

    fn style(&self, class: &Self::Class<'_>, status: Status) -> Style {
        class(self, status)
    }
}

fn base(
    theme: &Theme,
    status: Status,
    rail_width: f32,
    handle_radius: f32,
    handle_border_width: f32,
) -> Style {
    let tokens = theme.tokens();

    Style {
        rail: Rail {
            backgrounds: (
                match status {
                    Status::Active => tokens.compound_brand_background.into(),
                    Status::Hovered => tokens.compound_brand_background_hover.into(),
                    Status::Dragged => tokens.compound_brand_background_pressed.into(),
                },
                tokens.neutral_stroke_accessible.into(),
            ),
            width: rail_width,
            border: Border {
                radius: border_radius::CIRCULAR,
                width: 0.0,
                color: tokens.transparent_background,
            },
        },
        handle: Handle {
            shape: HandleShape::Circle {
                radius: handle_radius,
            },
            background: match status {
                Status::Active => tokens.compound_brand_background.into(),
                Status::Hovered => tokens.compound_brand_background_hover.into(),
                Status::Dragged => tokens.compound_brand_background_pressed.into(),
            },
            border_color: tokens.neutral_stroke1,
            border_width: handle_border_width,
        },
    }
}

pub fn medium(theme: &Theme, status: Status) -> Style {
    let handle_width = match status {
        Status::Active => 6.0,
        Status::Hovered => 3.0,
        Status::Dragged => 7.0,
    };

    base(theme, status, 4.0, 10.0, handle_width)
}

pub fn small(theme: &Theme, status: Status) -> Style {
    let handle_width = match status {
        Status::Active => 4.0,
        Status::Hovered => 3.0,
        Status::Dragged => 5.0,
    };

    base(theme, status, 2.0, 8.0, handle_width)
}
