//! A re-export of the `iced` crate with Fluent-aware widget styles.
//!
//! iced 0.14 makes the theme part of every widget's type: the application's
//! theme is [`iced_fluent_theme::Theme`], so iced's built-in style functions
//! (which are defined for iced's own `Theme`) no longer apply. Rather than
//! respell every call site, this module re-exports all of `iced` and replaces
//! the handful of styles the app passes explicitly with their Fluent
//! equivalents.
//!
//! A module only needs:
//!
//! ```ignore
//! use crate::compat_iced as iced;
//! use crate::compat_iced::*;
//! ```
//!
//! and its existing `iced::…` paths *and* bare widget names keep working —
//! `iced::widget::text::danger` now resolves through this module.
//!
//! Nothing here changes behaviour; it is naming and styling only.

pub(crate) use iced::*;

/// `iced::Result` (`Result<(), iced::Error>`) would otherwise shadow the
/// standard `Result` for every module that imports this prelude.
pub(crate) use std::result::Result;

/// The Fluent-themed element and text aliases (see `crate::Element`).
pub(crate) use crate::{Element, Text};

/// Fluent-themed `Column` / `Row`, so a signature (or a qualified
/// `iced::widget::Column<'_, Message>`) still means the Fluent element.
pub(crate) use widget::{Column, Row};

/// The two shims that live in `widget` but are used unqualified.
pub(crate) use widget::{checkbox, horizontal_rule};

/// iced 0.13's two-argument `Space::new` and `Space::with_height`, which
/// iced 0.14 removed.
#[allow(non_snake_case)]
pub(crate) mod Space {
    use iced::Length;

    /// Empty space of the given width and height.
    pub(crate) fn new(
        width: impl Into<Length>,
        height: impl Into<Length>,
    ) -> iced::widget::Space {
        iced::widget::Space::new().width(width).height(height)
    }

    /// Empty space that shrinks horizontally and fills the given height.
    pub(crate) fn with_height(height: impl Into<Length>) -> iced::widget::Space {
        iced::widget::Space::new().height(height)
    }
}

pub(crate) mod widget {
    pub(crate) use iced::widget::*;

    /// Fluent-themed `Column` / `Row` aliases, so a signature (or a qualified
    /// `iced::widget::Column<'_, Message>`) still means the Fluent element.
    pub(crate) type Column<'a, M> = iced::widget::Column<'a, M, crate::fluent::Theme>;
    pub(crate) type Row<'a, M> = iced::widget::Row<'a, M, crate::fluent::Theme>;

    /// iced 0.14 renamed `horizontal_rule` to [`iced::widget::rule::horizontal`].
    pub(crate) use iced::widget::rule::horizontal as horizontal_rule;

    /// A themed [`iced::widget::Checkbox`] with iced 0.13's `(label, checked)`
    /// argument order.
    pub(crate) fn checkbox<'a>(
        label: impl Into<String>,
        is_checked: bool,
    ) -> iced::widget::Checkbox<'a, crate::Message, crate::fluent::Theme> {
        iced::widget::checkbox(is_checked).label(label.into())
    }

    /// Fluent replacements for the iced text styles.
    pub(crate) mod text {
        pub(crate) use iced::widget::text::*;

        /// Fluent's error colour (iced's `text::danger`).
        pub(crate) fn danger(
            theme: &crate::fluent::Theme,
        ) -> iced::widget::text::Style {
            crate::fluent::danger(theme)
        }

        /// Fluent's success colour (iced's `text::success`).
        pub(crate) fn success(
            theme: &crate::fluent::Theme,
        ) -> iced::widget::text::Style {
            crate::fluent::success(theme)
        }
    }

    /// Fluent replacements for the iced container styles.
    pub(crate) mod container {
        pub(crate) use iced::widget::container::*;

        /// A Fluent card (iced's `container::rounded_box`).
        pub(crate) fn rounded_box(
            theme: &crate::fluent::Theme,
        ) -> iced::widget::container::Style {
            crate::fluent::card(theme)
        }
    }

    /// Fluent replacements for the iced button styles.
    pub(crate) mod button {
        pub(crate) use iced::widget::button::*;

        /// Fluent's primary button (iced's `button::primary`).
        pub(crate) fn primary(
            theme: &crate::fluent::Theme,
            status: iced::widget::button::Status,
        ) -> iced::widget::button::Style {
            crate::fluent::primary_button(theme, status)
        }

        /// Fluent's secondary button (iced's `button::secondary`) — the class
        /// a plain `button(..)` uses.
        pub(crate) fn secondary(
            theme: &crate::fluent::Theme,
            status: iced::widget::button::Status,
        ) -> iced::widget::button::Style {
            crate::fluent::secondary_button(theme, status)
        }
    }

    /// Fluent replacements for the iced pick-list styles.
    pub(crate) mod pick_list {
        pub(crate) use iced::widget::pick_list::*;

        /// Fluent's pick-list look (iced's `pick_list::default`).
        pub(crate) fn default(
            theme: &crate::fluent::Theme,
            status: iced::widget::pick_list::Status,
        ) -> iced::widget::pick_list::Style {
            crate::fluent::pick_list_default(theme, status)
        }
    }
}
