//! The "Remove System Bloatware" dialog (Mode 2, Smart Device Flashing).
//!
//! Pressing the feature tile's button opens this modal: it asks for a device
//! connected with USB debugging enabled, checks which of the known
//! preinstalled apps are on it, and removes the ones the user checks.
//!
//! Like the other dialogs, this module only *presents* state and sends
//! messages; the flow state and its update handlers live in `crate`
//! (`State::flash.bloatware`), and the ADB work in [`crate::debloat`].

use iced::widget::text::{Shaping, Wrapping};
use iced::widget::{
    button, checkbox, column, container, horizontal_rule, mouse_area, row,
    scrollable, text_input, Column, Space,
};
use iced::{Alignment, Element, Fill};

use crate::bootloader::{bright_success, darker_card, warning_orange};
use crate::debloat::{self, AdbDeviceState, RemovalOutcome};
use crate::guided::spinner;
use crate::text;
use crate::{BloatwareDialog, Message, State};

/// Height of the app list. Tall enough for a handful of rows, short enough
/// that the search box above it and the buttons under it stay in the card.
const LIST_HEIGHT: f32 = 240.0;

/// Id of that list, so its scroll position survives a redraw (the card is
/// rebuilt on every message, and a scrollable without an id would start at the
/// top again).
const LIST_SCROLL_ID: &str = "bloatware-list";

/// Whether the "Remove System Bloatware" dialog is open.
pub(crate) fn is_open(state: &State) -> bool {
    state.flash.bloatware.dialog == BloatwareDialog::Open
}

/// Renders the dialog over the app, or `None` when it is closed. Clicks on
/// empty space are swallowed (they neither dismiss the dialog nor reach the
/// UI underneath); the modal is left only via its own buttons.
pub(crate) fn overlay(state: &State) -> Option<Element<'_, Message>> {
    if !is_open(state) {
        return None;
    }

    let children: Vec<Element<'_, Message>> = vec![
        mouse_area(Space::new(Fill, Fill))
            .on_press(Message::BloatwareBackdropPressed)
            .into(),
        container(card(state))
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill)
            .into(),
    ];

    Some(iced::widget::Stack::with_children(children).into())
}

/// The modal card: one step of the flow at a time, with the footer buttons.
fn card(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let bloatware = &state.flash.bloatware;
    let busy = bloatware.busy();

    let mut content = column![
        text(l10n.tr("flash-bloatware-title")).size(18.0),
        text(l10n.tr("bloatware-desc"))
            .size(14.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph),
        horizontal_rule(1),
        body(state),
    ]
    .spacing(10)
    .width(Fill);

    // Footer: the way back this step offers (if any) and Cancel side by side,
    // Cancel to the right of it. Both take the same share of the row (see
    // `return_element`), so neither of them stretches into a button far wider
    // than its label. A running job keeps the dialog open, so Cancel is
    // greyed out then.
    content = content.push(horizontal_rule(1));

    let cancel = button(text(l10n.tr("login-cancel"))).width(Fill);
    let cancel = if busy {
        cancel
    } else {
        cancel.on_press(Message::BloatwareCancel)
    };

    let footer: Element<'_, Message> = match return_element(state) {
        Some(back) => {
            row![back, cancel]
                .spacing(8)
                .width(Fill)
                .align_y(Alignment::Center)
                .into()
        }
        None => cancel.into(),
    };

    content = content.push(footer);

    // The padding sits inside the scrollable, so the card's border is the
    // viewport: the scrollbar rides on the right border instead of floating
    // in the middle of the padding.
    container(
        scrollable(container(content).padding(16).width(Fill))
            .width(Fill)
            .height(Fill),
    )
    .width(560)
    .height(520)
    .style(darker_card)
    .into()
}

/// The step the dialog is showing: the removal, its result, the checklist,
/// the package check, or the device selection.
fn body(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let bloatware = &state.flash.bloatware;

    if bloatware.removing {
        return busy_row(state, l10n.tr("bloatware-removing"));
    }

    if let Some(results) = &bloatware.results {
        return results_view(state, results);
    }

    if bloatware.found.is_some() {
        return checklist(state);
    }

    if bloatware.checking {
        return busy_row(state, l10n.tr("bloatware-checking"));
    }

    if let Some(error) = &bloatware.check_error {
        // The way back to the device list sits in the footer.
        return danger(error);
    }

    device_section(state)
}

/// The device selection: what `adb devices` found, with a Refresh button.
fn device_section(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let bloatware = &state.flash.bloatware;

    let refresh = button(text(l10n.tr("bloatware-refresh")));

    let mut content = column![
        row![
            text(l10n.tr("bloatware-select-device")).size(13.0),
            if bloatware.preparing {
                refresh
            } else {
                refresh.on_press(Message::BloatwareRescan)
            },
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    ]
    .spacing(10)
    .width(Fill);

    if bloatware.preparing {
        content = content.push(busy_row(state, l10n.tr("bloatware-preparing")));
    } else if let Some(error) = &bloatware.prepare_error {
        content = content.push(danger(error));
    } else {
        let ready: Vec<_> = bloatware
            .devices
            .iter()
            .filter(|device| device.state == AdbDeviceState::Ready)
            .collect();

        if !ready.is_empty() {
            content = content.push(
                Column::with_children(ready.iter().map(|device| {
                    button(text(device.label()))
                        .width(Fill)
                        .on_press(Message::BloatwareDeviceSelected(device.serial.clone()))
                        .into()
                }))
                .spacing(8),
            );
        } else if bloatware
            .devices
            .iter()
            .any(|device| device.state == AdbDeviceState::Unauthorized)
        {
            // The device answered, but has not accepted this computer's
            // debugging key yet.
            content = content.push(notice(l10n.tr("bloatware-unauthorized")));
        } else {
            content = content.push(notice(l10n.tr("bloatware-no-device")));
        }
    }

    content.into()
}

/// The checklist of installed bloatware, with the search box, the list and
/// the Remove button.
fn checklist(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let bloatware = &state.flash.bloatware;
    let found = bloatware.found.as_deref().unwrap_or_default();

    let mut content = column![].spacing(10).width(Fill);

    if let Some(label) = bloatware.selected_label() {
        content = content.push(text(label).size(12.0));
    }

    if found.is_empty() {
        content = content.push(notice(l10n.tr("bloatware-none-found")));
        return content.into();
    }

    content = content.push(text(l10n.tr("bloatware-found-heading")).size(13.0));

    // The search box only filters what the list shows; the selection keeps the
    // indexes into `found`, so it survives changing the query.
    let search_placeholder = l10n.tr("bloatware-search");

    content = content.push(
        text_input(&search_placeholder, &bloatware.filter)
            .padding(6)
            .width(Fill)
            .on_input(Message::BloatwareFilterChanged),
    );

    let visible: Vec<(usize, String)> = found
        .iter()
        .enumerate()
        .filter_map(|(index, app)| {
            let name = app.label(l10n);

            debloat::matches(&name, app.package, &bloatware.filter)
                .then(|| (index, row_label(&name, app.package)))
        })
        .collect();

    if visible.is_empty() {
        content = content.push(notice(l10n.tr_with_args(
            "bloatware-no-match",
            &[("query", bloatware.filter.trim().to_owned())],
        )));
    } else {
        let rows: Vec<Element<'_, Message>> = visible
            .into_iter()
            .map(|(index, label)| {
                let checked = bloatware.checked.get(index).copied().unwrap_or(false);

                checkbox(label, checked)
                    // A checkbox label is not drawn through the local `text`
                    // helper, so it needs advanced shaping itself (CJK
                    // locales).
                    .text_shaping(Shaping::Advanced)
                    .on_toggle(move |checked| Message::BloatwareAppToggled(index, checked))
                    .into()
            })
            .collect();

        // The list scrolls on its own, so a long one does not push the buttons
        // out of the card (the card itself scrolls as well).
        content = content.push(
            scrollable(Column::with_children(rows).spacing(6))
                .id(iced::widget::scrollable::Id::new(LIST_SCROLL_ID))
                .height(LIST_HEIGHT)
                .width(Fill),
        );
    }

    content = content.push(
        row![
            button(text(l10n.tr("bloatware-select-all")))
                .on_press(Message::BloatwareSelectAll(true)),
            button(text(l10n.tr("bloatware-select-none")))
                .on_press(Message::BloatwareSelectAll(false)),
        ]
        .spacing(8),
    );

    let chosen = bloatware.has_chosen_apps();
    let remove = button(text(l10n.tr("bloatware-remove"))).width(Fill);

    if chosen {
        content = content.push(remove.on_press(Message::BloatwareRemove));
    } else {
        // Nothing selected: iced greys a button without `on_press` out and
        // ignores clicks on it.
        content = content.push(remove);
        content = content.push(notice(l10n.tr("bloatware-remove-none")));
    }

    content.into()
}

/// The label of one checklist row: the app's name, and the package it belongs
/// to when the name is not the package id itself.
fn row_label(name: &str, package: &str) -> String {
    if name == package {
        name.to_owned()
    } else {
        format!("{name} — {package}")
    }
}

/// What the removal did, one line per app.
fn results_view<'a>(
    state: &'a State,
    results: &'a [RemovalOutcome],
) -> Element<'a, Message> {
    let l10n = &state.l10n;

    let mut content = column![text(l10n.tr("bloatware-done"))
        .size(14.0)
        .style(bright_success)]
    .spacing(8)
    .width(Fill);

    for outcome in results {
        let name = outcome.app.label(l10n);

        content = content.push(match &outcome.error {
            None => text(l10n.tr_with_args(
                "bloatware-result-removed",
                &[("name", name)],
            ))
            .size(13.0)
            .style(bright_success),
            Some(error) => text(l10n.tr_with_args(
                "bloatware-result-failed",
                &[("name", name), ("error", error.clone())],
            ))
            .size(13.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph)
            .style(iced::widget::text::danger),
        });
    }

    content = content.push(
        text(l10n.tr("bloatware-undo-note"))
            .size(12.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph),
    );

    content.into()
}

/// The way back the current step offers, or `None` when it has none.
///
/// It is drawn in the footer next to Cancel, so the card does not have to know
/// which step the body is showing. Both buttons fill their half of the footer
/// (the layout is a `row!` of two `Fill`s), so a short label like "Return"
/// does not end up beside a Cancel button several times its width.
fn return_element(state: &State) -> Option<Element<'_, Message>> {
    let l10n = &state.l10n;
    let bloatware = &state.flash.bloatware;

    // Nothing to go back to while the apps are being removed.
    if bloatware.removing {
        return None;
    }

    // The result goes back to the (now shorter) checklist, not to the device
    // list.
    if bloatware.results.is_some() {
        return Some(
            button(text(l10n.tr("flash-bootloader-return")))
                .width(Fill)
                .on_press(Message::BloatwareDone)
                .into(),
        );
    }

    if bloatware.found.is_some() || bloatware.check_error.is_some() {
        return Some(
            button(text(format!("< {}", l10n.tr("flash-bootloader-return"))))
                .width(Fill)
                .on_press(Message::BloatwareBackToDevices)
                .into(),
        );
    }

    None
}

/// A spinner plus its status text.
fn busy_row<'a>(state: &'a State, label: String) -> Element<'a, Message> {
    row![spinner(state.anim_tick), text(label).size(13.0)]
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
}

/// A red failure line.
fn danger(error: &str) -> Element<'static, Message> {
    text(error.to_owned())
        .size(13.0)
        .width(Fill)
        .wrapping(Wrapping::WordOrGlyph)
        .style(iced::widget::text::danger)
        .into()
}

/// An orange hint (no device yet, nothing selected, …).
fn notice(message: String) -> Element<'static, Message> {
    text(message)
        .size(13.0)
        .width(Fill)
        .wrapping(Wrapping::WordOrGlyph)
        .style(warning_orange)
        .into()
}
