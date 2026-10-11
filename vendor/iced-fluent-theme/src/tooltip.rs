use crate::{Theme, border_radius, stroke_width};
use iced_core::Border;
use iced_widget::container::Style;

// Tooltip specifications and styling from
// https://storybooks.fluentui.dev/react/?path=/docs/components-tooltip--docs
// and
// https://github.com/microsoft/fluentui/blob/e63be2ef9a165f4c52071b63ada90956bf45481e/packages/react-components/react-tooltip/library/src/components/Tooltip/useTooltipStyles.styles.ts

// TODO - request Tooltip padding use the Padding type and not f32

pub fn default(theme: &Theme) -> Style {
    let tokens = theme.tokens();

    Style {
        text_color: Some(tokens.neutral_foreground1),
        background: Some(tokens.neutral_background1.into()),
        border: Border {
            color: tokens.transparent_stroke,
            width: stroke_width::THIN,
            radius: border_radius::MEDIUM,
        },
        shadow: tokens.shadow8,
        ..Style::default()
    }
}

pub fn inverted(theme: &Theme) -> Style {
    let tokens = theme.tokens();

    Style {
        text_color: Some(tokens.neutral_foreground_static_inverted),
        background: Some(tokens.neutral_background_static.into()),
        border: Border {
            color: tokens.transparent_stroke,
            width: stroke_width::THIN,
            radius: border_radius::MEDIUM,
        },
        shadow: tokens.shadow8,
        ..Style::default()
    }
}
