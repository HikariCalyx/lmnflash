use crate::{Theme, border_radius, stroke_width};
use iced_core::{Border, Shadow};

use iced_widget::{
    container,
    scrollable::{AutoScroll, Catalog, Rail, Scroller, Status, Style, StyleFn},
};

impl Catalog for Theme {
    type Class<'a> = StyleFn<'a, Self>;

    fn default<'a>() -> Self::Class<'a> {
        Box::new(default)
    }

    fn style(&self, class: &Self::Class<'_>, status: Status) -> Style {
        class(self, status)
    }
}

// Scrollable styling referenced from
// https://github.com/microsoft/microsoft-ui-xaml/blob/main/src/controls/dev/CommonStyles/ScrollBar.xaml
// https://github.com/microsoft/microsoft-ui-xaml/blob/main/src/controls/dev/CommonStyles/ScrollBar_themeresources.xaml
// with colours mapped to Fluent UI React 9 colours
// https://storybooks.fluentui.dev/react/?path=/docs/theme-colors--docs

pub fn default(theme: &Theme, status: Status) -> Style {
    let tokens = theme.tokens();

    let scrollbar = Rail {
        background: Some(tokens.neutral_background3.into()),
        border: Border::default(),
        scroller: Scroller {
            background: tokens.neutral_foreground3.into(),
            border: Border {
                radius: border_radius::MEDIUM,
                ..Border::default()
            },
        },
    };

    let auto_scroll = AutoScroll {
        background: tokens.neutral_foreground3.into(),
        border: Border {
            color: tokens.neutral_background1,
            width: stroke_width::THIN,
            radius: border_radius::CIRCULAR,
        },
        shadow: Shadow {
            color: tokens.neutral_shadow_key,
            offset: [0.0, 2.0].into(),
            blur_radius: 12.0,
        },
        icon: tokens.neutral_foreground2,
    };

    match status {
        Status::Active { .. } => Style {
            container: container::Style::default(),
            vertical_rail: scrollbar,
            horizontal_rail: scrollbar,
            gap: None,
            auto_scroll,
        },
        Status::Hovered {
            is_horizontal_scrollbar_hovered,
            is_vertical_scrollbar_hovered,
            ..
        } => {
            let hovered_scrollbar = Rail {
                scroller: Scroller {
                    background: tokens.neutral_foreground2.into(),
                    ..scrollbar.scroller
                },
                ..scrollbar
            };

            Style {
                container: container::Style::default(),
                vertical_rail: if is_vertical_scrollbar_hovered {
                    hovered_scrollbar
                } else {
                    scrollbar
                },
                horizontal_rail: if is_horizontal_scrollbar_hovered {
                    hovered_scrollbar
                } else {
                    scrollbar
                },
                gap: None,
                auto_scroll,
            }
        }
        Status::Dragged {
            is_horizontal_scrollbar_dragged,
            is_vertical_scrollbar_dragged,
            ..
        } => {
            let dragged_scrollbar = Rail {
                scroller: Scroller {
                    background: tokens.neutral_foreground1.into(),
                    ..scrollbar.scroller
                },
                ..scrollbar
            };

            Style {
                container: container::Style::default(),
                vertical_rail: if is_vertical_scrollbar_dragged {
                    dragged_scrollbar
                } else {
                    scrollbar
                },
                horizontal_rail: if is_horizontal_scrollbar_dragged {
                    dragged_scrollbar
                } else {
                    scrollbar
                },
                gap: None,
                auto_scroll,
            }
        }
    }
}
