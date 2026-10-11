//! A [Fluent 2](https://fluent2.microsoft.design) inspired theme for the [Iced](https://github.com/hecrj/iced) GUI library.

pub mod border_radius;
pub mod button;
pub mod checkbox;
pub mod color;
pub mod combo_box;
pub mod container;
pub mod font;
pub mod menu;
pub mod pane_grid;
pub mod pick_list;
pub mod progress_bar;
pub mod radio;
pub mod rule;
pub mod scrollable;
pub mod shadow;
pub mod slider;
pub mod spacing;
pub mod stroke_width;
pub mod table;
pub mod text;
pub mod text_editor;
pub mod text_input;
mod theme;
pub mod tokens;
pub mod tooltip;

#[cfg(feature = "markdown")]
pub mod markdown;

#[cfg(feature = "menu_bar")]
pub mod menu_bar;

#[cfg(feature = "ribbon")]
pub mod ribbon;

#[cfg(feature = "selector_bar")]
pub mod selector_bar;

#[cfg(feature = "svg")]
pub mod svg;

pub use crate::theme::*;
