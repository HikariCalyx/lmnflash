//! The About dialog and the round "i" button that opens it (Firmware Lookup).
//!
//! The button replaces the old Return button of the card: clicking the dimmed
//! backdrop is the only way to close the dialog. The body scrolls inside a
//! fixed-size card, so a long credits list cannot grow the window.

use iced::widget::{
    button, container, horizontal_rule, mouse_area, scrollable, stack, Column, Space,
};
use iced::{Alignment, Element, Fill, Length};

use crate::text;
use crate::{Message, State};

/// The round "i" button that opens the About dialog (Firmware Lookup page).
pub(crate) fn about_button<'a>() -> Element<'a, Message> {
    let icon = container(text("i").size(15.0)).center(Fill);

    button(icon)
        .width(Length::Fixed(30.0))
        .height(Length::Fixed(30.0))
        .padding(0)
        .style(about_button_style)
        .on_press(Message::AboutPressed)
        .into()
}

/// Draws the About button as a circle using the primary button's colours, so
/// it reads on both the light and the dark theme.
fn about_button_style(theme: &iced::Theme, status: button::Status) -> button::Style {
    let mut style = button::primary(theme, status);

    style.border = iced::Border {
        radius: 15.0.into(),
        ..style.border
    };

    style
}

/// A URL in the About dialog: a button drawn as a link (no chrome) so the text
/// can be clicked, which opens the address in the system browser.
fn link_button<'a>(url: String) -> Element<'a, Message> {
    button(text(url.clone()))
        .padding(0)
        .style(link_button_style)
        .on_press(Message::AboutOpenUrl(url))
        .into()
}

/// The hyperlink look: no background, border or shadow, with the text in a
/// colour that reads as clickable on both themes (iced's palette has no link
/// colour, so the two are picked by hand like the other status colours).
fn link_button_style(theme: &iced::Theme, status: button::Status) -> button::Style {
    let color = if theme.extended_palette().is_dark {
        iced::Color::from_rgb8(0x6C, 0xB4, 0xFF)
    } else {
        iced::Color::from_rgb8(0x0B, 0x5F, 0xCC)
    };

    // Hovering fades the link a little, the same cue iced's own text button
    // style uses, so it is obviously interactive.
    let text_color = match status {
        button::Status::Hovered | button::Status::Pressed => color.scale_alpha(0.75),
        _ => color,
    };

    button::Style {
        text_color,
        // A link has no button chrome: borrow the empty background, border and
        // shadow of the built-in text-only button style.
        ..button::text(theme, button::Status::Active)
    }
}

/// The About dialog drawn over the whole window while it is open.
///
/// Its body scrolls if it outgrows the card; clicking the dimmed backdrop
/// closes it.
pub(crate) fn overlay(state: &State) -> Option<Element<'_, Message>> {
    if !state.about_open {
        return None;
    }

    let l10n = &state.l10n;

    let sections: Vec<Element<'_, Message>> = vec![
        text(l10n.tr("about-title")).size(20.0).into(),
        // Only the word before the colon is translated; the colon itself
        // and the crate version are added here.
        text(format!(
            "{}: {}",
            l10n.tr("about-version"),
            env!("CARGO_PKG_VERSION")
        ))
        .size(14.0)
        .into(),
        text(l10n.tr("about-intro")).into(),
        horizontal_rule(1).into(),
        text(l10n.tr("about-thanks-heading")).into(),
        text(l10n.tr("about-thanks-list")).into(),
        link_button(l10n.tr("about-link-lenovobl")),
        horizontal_rule(1).into(),
        text(l10n.tr("about-android-heading")).into(),
        link_button(l10n.tr("about-link-android")),
        horizontal_rule(1).into(),
        text(l10n.tr("about-source-code")).into(),
        link_button(l10n.tr("about-link-source")),
    ];

    // The card keeps a fixed size so the scrollable has a bounded viewport:
    // a `Shrink`-height scrollable grows with its content and never
    // scrolls, so a dialog whose body may grow has to be capped here.
    let card = container(
        scrollable(
            container(
                Column::with_children(sections)
                    .spacing(8)
                    .align_x(Alignment::Start),
            )
            .padding(16)
            .width(Fill),
        )
        .width(Fill)
        .height(Fill),
    )
    .width(520)
    .height(400)
    .style(container::rounded_box);

    let backdrop = mouse_area(Space::new(Fill, Fill)).on_press(Message::AboutClosed);

    Some(
        stack![
            backdrop,
            container(card)
                .width(Fill)
                .height(Fill)
                .center_x(Fill)
                .center_y(Fill),
        ]
        .into(),
    )
}
