//! The "Fix Captive Portal Server" dialog (Mode 2, Smart Device Flashing).
//!
//! Pressing the feature tile's button opens this modal: it asks for a device
//! connected with USB debugging enabled, tells which phone it will work on,
//! and then runs the fix on it.
//!
//! Like the other dialogs, this module only *presents* state and sends
//! messages; the flow state and its update handlers live in `crate`
//! (`State::flash.captive_portal`), and the device commands in
//! [`crate::captive_portal`].

use iced::widget::text::Wrapping;
use iced::widget::{button, column, container, mouse_area, row, scrollable};
use iced::{Alignment, Fill};

use crate::bootloader::{bright_success, darker_card, warning_orange};
use crate::captive_portal::{FixError, FixReport};
use crate::compat_iced as iced;
use crate::compat_iced::*;
use crate::debloat::AdbDeviceState;
use crate::guided::spinner;
use crate::text;
use crate::{Message, State};

/// Whether the "Fix Captive Portal Server" dialog is open.
pub(crate) fn is_open(state: &State) -> bool {
    state.flash.captive_portal.open
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
            .on_press(Message::CaptivePortalBackdropPressed)
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

    let mut content = column![
        text(l10n.tr("flash-captive-portal-title")).size(18.0),
        text(l10n.tr("captive-portal-desc"))
            .size(14.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph),
        horizontal_rule(1),
        body(state),
    ]
    .spacing(10)
    .width(Fill);

    content = content.push(horizontal_rule(1));
    content = content.push(footer(state));

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

/// The step the dialog is showing: the running fix and its result, the chosen
/// device with the note that goes with it, or the device selection.
fn body(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let portal = &state.flash.captive_portal;

    if portal.fixing {
        return busy_row(state, l10n.tr("captive-portal-working"));
    }

    if let Some(result) = &portal.result {
        return result_view(state, result);
    }

    if portal.selected.is_some() {
        return confirm_section(state);
    }

    device_section(state)
}

/// The device the fix will run on, with what it is about to do.
fn confirm_section(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let portal = &state.flash.captive_portal;

    let label = portal.selected_label().unwrap_or_default();

    column![
        text(label).size(13.0),
        text(l10n.tr("captive-portal-note"))
            .size(12.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph)
            .style(warning_orange),
    ]
    .spacing(10)
    .width(Fill)
    .into()
}

/// The device selection: what `adb devices` found, with a Refresh button.
fn device_section(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let portal = &state.flash.captive_portal;

    let refresh = button(text(l10n.tr("bloatware-refresh")));

    let mut content = column![
        row![
            text(l10n.tr("bloatware-select-device")).size(13.0),
            if portal.listing {
                refresh
            } else {
                refresh.on_press(Message::CaptivePortalRescan)
            },
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    ]
    .spacing(10)
    .width(Fill);

    if portal.listing {
        content = content.push(busy_row(state, l10n.tr("bloatware-preparing")));
    } else if let Some(error) = &portal.list_error {
        content = content.push(danger(error));
    } else {
        let ready: Vec<_> = portal
            .devices
            .iter()
            .filter(|device| device.state == AdbDeviceState::Ready)
            .collect();
        let unauthorized: Vec<_> = portal
            .devices
            .iter()
            .filter(|device| device.state == AdbDeviceState::Unauthorized)
            .collect();

        if !ready.is_empty() {
            content = content.push(
                Column::with_children(ready.iter().map(|device| {
                    button(text(device.label()))
                        .width(Fill)
                        .on_press(Message::CaptivePortalDeviceSelected(device.serial.clone()))
                        .into()
                }))
                .spacing(8),
            );
        }

        if !unauthorized.is_empty() {
            // The device answered, but has not accepted this computer's
            // debugging key yet. The prompt is only shown while the device
            // (re)connects, so one that was dismissed by mistake is brought
            // back by reconnecting to that device rather than by scanning the
            // bus again.
            content = content.push(notice(l10n.tr("bloatware-unauthorized")));
            content = content.push(
                Column::with_children(unauthorized.iter().map(|device| {
                    button(text(format!(
                        "{} — {}",
                        device.label(),
                        l10n.tr("captive-portal-retry")
                    )))
                    .width(Fill)
                    .on_press(Message::CaptivePortalRetry(device.serial.clone()))
                    .into()
                }))
                .spacing(8),
            );
        }

        if ready.is_empty() && unauthorized.is_empty() {
            content = content.push(notice(l10n.tr("bloatware-no-device")));
        }
    }

    content.into()
}

/// What the fix did: the servers were updated, and whether the connection the
/// command came in on was dropped because the device is on the network.
fn result_view<'a>(
    state: &'a State,
    result: &'a Result<FixReport, FixError>,
) -> Element<'a, Message> {
    let l10n = &state.l10n;

    let mut content = column![].spacing(8).width(Fill);

    match result {
        Ok(report) => {
            content = content.push(
                text(l10n.tr("captive-portal-done"))
                    .size(14.0)
                    .style(bright_success),
            );

            if let Some(device) = &report.disconnected {
                content = content.push(
                    text(l10n.tr_with_args(
                        "captive-portal-disconnected",
                        &[("device", device.clone())],
                    ))
                    .size(13.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph)
                    .style(warning_orange),
                );
            }

            // Android 10 and newer bring the network back up themselves; on
            // everything older the new settings are only read then, so the
            // user has to do it.
            if report.needs_manual_airplane_mode() {
                content = content.push(notice(l10n.tr("captive-portal-manual-airplane")));
            }
        }
        // A device that is too old is told apart from a failed command: there
        // is nothing the user could retry, and the wording is our own.
        Err(FixError::Unsupported) => {
            content = content.push(notice(l10n.tr("captive-portal-unsupported")));
        }
        Err(FixError::Failed(error)) => {
            content = content.push(
                text(l10n.tr_with_args(
                    "captive-portal-failed",
                    &[("error", error.clone())],
                ))
                .size(13.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph)
                .style(iced::widget::text::danger),
            );
        }
    }

    content.into()
}

/// The footer: the Fix button and Cancel.
///
/// Both fill their half of the row, so neither stretches into a button far
/// wider than its label. Fix is greyed out (a button without `on_press`, which
/// iced also ignores clicks on) until a device is chosen, and stays greyed out
/// while the fix runs and once it is done — the same device does not need a
/// second run.
fn footer(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let portal = &state.flash.captive_portal;

    let fix = button(text(l10n.tr("captive-portal-fix"))).width(Fill);
    let fix = if portal.ready_to_fix() {
        fix.on_press(Message::CaptivePortalFix)
    } else {
        fix
    };

    let cancel = button(text(l10n.tr("login-cancel"))).width(Fill);
    let cancel = if portal.busy() {
        cancel
    } else {
        cancel.on_press(Message::CaptivePortalCancel)
    };

    row![fix, cancel]
        .spacing(8)
        .width(Fill)
        .align_y(Alignment::Center)
        .into()
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

/// An orange hint (no device yet, …).
fn notice(message: String) -> Element<'static, Message> {
    text(message)
        .size(13.0)
        .width(Fill)
        .wrapping(Wrapping::WordOrGlyph)
        .style(warning_orange)
        .into()
}
