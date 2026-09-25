//! The "Factory Reset" dialog (Mode 2, Smartphone Flash).
//!
//! Pressing the Factory Reset tile opens this modal: the user picks one of the
//! connected fastboot devices (with a Refresh button to re-scan the USB bus),
//! the reset requirements are checked over fastboot (`securestate`,
//! `fdr-allowed`), and — when both are met — the user confirms before
//! `erase userdata` / `erase metadata` are run.
//!
//! Like the other dialogs, this module only *presents* state and sends
//! messages; the flow state and its update handlers live in `crate`
//! (`State::flash.factory_reset`).

use iced::widget::text::Wrapping;
use iced::widget::{
    button, column, container, horizontal_rule, mouse_area, row, scrollable, Space,
};
use iced::{Alignment, Element, Fill};

use crate::bootloader::{bright_success, darker_card, warning_orange};
use crate::fastboot_info::FactoryResetBlocker;
use crate::guided::spinner;
use crate::text;
use crate::{FactoryResetDialog, Message, ResetCheckOutcome, State};

/// Renders the Factory Reset overlay over the app, or `None` when the dialog
/// is closed. Clicks on empty space are swallowed (they neither dismiss the
/// dialog nor reach the UI underneath); the modal is left only via its own
/// buttons.
pub(crate) fn overlay(state: &State) -> Option<Element<'_, Message>> {
    if state.flash.factory_reset.dialog != FactoryResetDialog::Open {
        return None;
    }

    let children: Vec<Element<'_, Message>> = vec![
        mouse_area(Space::new(Fill, Fill))
            .on_press(Message::FactoryResetBackdropPressed)
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
    let reset = &state.flash.factory_reset;
    let busy = reset.listing || reset.checking || reset.resetting;

    let mut content = column![
        text(l10n.tr("flash-factory-reset-title")).size(18.0),
        text(l10n.tr("factory-reset-desc"))
            .size(14.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph),
    ]
    .spacing(10)
    .width(Fill);

    // Once a device is picked, show which one the dialog is working on
    // (serial number, XT code and project codename).
    if let Some(label) = reset.selected_label() {
        content = content.push(text(label).size(13.0));
    }

    content = content.push(horizontal_rule(1));

    if reset.resetting {
        content = content.push(busy_row(state, l10n.tr("factory-reset-resetting")));
    } else if let Some(result) = &reset.result {
        match result {
            Ok(()) => {
                content = content
                    .push(text(l10n.tr("factory-reset-done")).size(13.0).style(bright_success));
            }
            Err(error) => {
                content = content
                    .push(text(l10n.tr("factory-reset-failed")).size(13.0).style(iced::widget::text::danger))
                    .push(text(error.clone()).size(13.0).style(iced::widget::text::danger));
            }
        }

        content = content.push(
            button(text(l10n.tr("login-cancel"))).on_press(Message::FactoryResetCancel),
        );
    } else if reset.checking {
        content = content.push(busy_row(state, l10n.tr("factory-reset-checking")));
    } else if let Some(outcome) = &reset.check {
        match outcome {
            // All requirements met: ask for confirmation before wiping.
            ResetCheckOutcome::Allowed { frp_protected } => {
                // The bootloader does not clear Google's Factory Reset
                // Protection, so say so before the phone is wiped.
                if *frp_protected {
                    content = content.push(
                        text(l10n.tr("factory-reset-no-frp"))
                            .size(13.0)
                            .width(Fill)
                            .wrapping(Wrapping::WordOrGlyph)
                            .style(warning_orange),
                    );
                }

                content = content
                    .push(
                        text(l10n.tr("factory-reset-confirm"))
                            .size(13.0)
                            .width(Fill)
                            .wrapping(Wrapping::WordOrGlyph)
                            .style(warning_orange),
                    )
                    .push(
                        row![
                            button(text(l10n.tr("guided-yes")))
                                .on_press(Message::FactoryResetConfirmed),
                            button(text(l10n.tr("guided-no")))
                                .on_press(Message::FactoryResetBack),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    );
            }
            // A requirement is not met: explain which one and go back to the
            // device list.
            ResetCheckOutcome::Blocked(blocker) => {
                content = content
                    .push(blocked_message(state, *blocker))
                    .push(return_button(state));
            }
            ResetCheckOutcome::Failed(error) => {
                content = content
                    .push(text(error.clone()).size(13.0).style(iced::widget::text::danger))
                    .push(return_button(state));
            }
        }
    } else {
        // Device selection. The label is always paired with the Refresh
        // button, which re-scans the USB bus: a phone that is not in
        // bootloader mode yet (e.g. still in fastbootD) is only found once it
        // has been restarted into the bootloader by hand.
        content = content.push(device_header(state, busy));

        if reset.listing {
            content = content.push(busy_row(state, l10n.tr("factory-reset-reading-devices")));
        } else if let Some(error) = &reset.list_error {
            content = content
                .push(text(error.clone()).size(13.0).style(iced::widget::text::danger));
        } else if reset.devices.is_empty() {
            // Nothing to pick: either no device at all or only unsupported
            // ones (another vendor, or not in bootloader mode).
            let error_id = if reset.total == 0 {
                "retcn-fill-fastboot-no-device"
            } else {
                "retcn-fill-fastboot-unsupported-device"
            };

            content = content
                .push(text(l10n.tr(error_id)).size(13.0).style(iced::widget::text::danger));
        } else {
            content = content.push(device_list(state));
        }
    }

    // Footer: the modal is left via Cancel (greyed out while an operation is
    // running so the device is never left half-erased).
    if !matches!(reset.result, Some(_)) {
        content = content.push(horizontal_rule(1));

        let cancel = button(text(l10n.tr("login-cancel"))).width(Fill);
        content = content.push(if busy {
            cancel
        } else {
            cancel.on_press(Message::FactoryResetCancel)
        });
    }

    // The padding sits inside the scrollable, so the card's border is the
    // viewport: the scrollbar rides on the right border instead of floating
    // in the middle of the padding.
    container(
        scrollable(container(content).padding(16).width(Fill))
            .width(Fill)
            .height(Fill),
    )
    .width(520)
    .height(480)
    .style(darker_card)
    .into()
}

/// The clickable list of connected supported devices.
fn device_list(state: &State) -> Element<'_, Message> {
    let devices = iced::widget::Column::with_children(
        state
            .flash
            .factory_reset
            .devices
            .iter()
            .map(|device| {
                button(text(device.label()))
                    .width(Fill)
                    .on_press(Message::FactoryResetDeviceSelected(device.serial.clone()))
                    .into()
            }),
    )
    .spacing(8);

    devices.into()
}

/// The "Select a device" label with the Refresh button that re-scans the USB
/// bus for connected phones.
fn device_header(state: &State, busy: bool) -> Element<'_, Message> {
    let refresh = button(text(state.l10n.tr("factory-reset-refresh")));

    row![
        text(state.l10n.tr("factory-reset-select-device")).size(13.0),
        if busy {
            refresh
        } else {
            refresh.on_press(Message::FactoryResetRescan)
        },
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

/// The localized explanation for a requirement that blocks the reset.
fn blocked_message(state: &State, blocker: FactoryResetBlocker) -> Element<'_, Message> {
    let id = match blocker {
        FactoryResetBlocker::Locked => "factory-reset-blocked-locked",
        FactoryResetBlocker::FdrNotAllowed => "factory-reset-blocked-fdr",
    };

    text(state.l10n.tr(id))
        .size(13.0)
        .width(Fill)
        .wrapping(Wrapping::WordOrGlyph)
        .style(iced::widget::text::danger)
        .into()
}

/// A spinner plus its status text.
fn busy_row<'a>(state: &'a State, label: String) -> Element<'a, Message> {
    row![
        spinner(state.anim_tick),
        text(label).size(13.0),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

/// "< Return" — back to the device list.
fn return_button(state: &State) -> Element<'_, Message> {
    button(text(format!(
        "< {}",
        state.l10n.tr("flash-bootloader-return")
    )))
    .on_press(Message::FactoryResetBack)
    .into()
}
