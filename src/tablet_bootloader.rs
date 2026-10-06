//! The tablet "Bootloader Unlock" dialog (Mode 2, Smartphone Flash).
//!
//! Lenovo tablets unlock differently from Motorola phones. The manual flow
//! reads the serial number and the `Bootloader_SN`, which the user submits on
//! ZUI's website; the token the site generates is then downloaded and flashed
//! from here. The guided flow does the whole unlock on its own (see
//! `crate::tablet_unlock::guided_unlock`). This module renders both; the state
//! and its update handlers live in `crate` (`State::flash.tablet_unlock`).

use iced::widget::text::Wrapping;
use iced::widget::{
    button, column, container, horizontal_rule, mouse_area, row, scrollable, text_input, Space,
};
use iced::{Alignment, Element, Fill};

use crate::bootloader::{bright_success, darker_card, warning_orange};
use crate::firmware::Platform;
use crate::guided::spinner;
use crate::l10n;
use crate::tablet_unlock;
use crate::text;
use crate::{Message, State, TabletGuidedStep, TabletUnlockDialog};

/// Whether the tablet Bootloader Unlock dialog is open.
pub(crate) fn is_open(state: &State) -> bool {
    state.flash.tablet_unlock.dialog != TabletUnlockDialog::Closed
}

/// Renders the tablet Bootloader Unlock overlay, or `None` when it is closed.
/// Clicks on empty space are swallowed (they neither dismiss the dialog nor
/// reach the UI underneath); navigation happens only via the dialog's buttons.
pub(crate) fn overlay(state: &State) -> Option<Element<'_, Message>> {
    let tablet = &state.flash.tablet_unlock;

    if tablet.dialog == TabletUnlockDialog::Closed {
        return None;
    }

    let card = match tablet.dialog {
        TabletUnlockDialog::Choosing => chooser_card(state),
        TabletUnlockDialog::Guided => guided_card(state),
        _ => manual_card(state),
    };

    let mut children: Vec<Element<'_, Message>> = vec![
        mouse_area(Space::new(Fill, Fill))
            .on_press(Message::TabletUnlockBackdropPressed)
            .into(),
        container(card)
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill)
            .into(),
    ];

    // Several fastboot devices connected: pick the one to read from.
    if tablet.picker.is_some() {
        children.push(
            mouse_area(Space::new(Fill, Fill))
                .on_press(Message::TabletUnlockBackdropPressed)
                .into(),
        );
        children.push(
            container(picker_card(state))
                .width(Fill)
                .height(Fill)
                .center_x(Fill)
                .center_y(Fill)
                .into(),
        );
    }

    Some(iced::widget::Stack::with_children(children).into())
}

/// The Guided-vs-Manual chooser.
fn chooser_card(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;

    let guided = button(text(l10n.tr("flash-bootloader-guided")))
        .width(Fill)
        .on_press(Message::TabletGuidedSelected);

    let manual = button(text(l10n.tr("flash-bootloader-manual")))
        .width(Fill)
        .on_press(Message::TabletUnlockManualSelected);

    let cancel = button(text(l10n.tr("login-cancel")))
        .width(Fill)
        .on_press(Message::TabletUnlockCancel);

    container(
        column![
            text(l10n.tr("flash-bootloader-title")).size(18.0),
            text(l10n.tr("flash-bootloader-choose")).size(14.0),
            guided,
            manual,
            cancel,
        ]
        .spacing(10)
        .width(Fill),
    )
    .width(360)
    .padding(20)
    .style(darker_card)
    .into()
}

/// The manual (ZUI) tablet unlock flow: read the serial number and
/// `Bootloader_SN`, submit them on ZUI's website, then flash the unlock token
/// the site generates.
fn manual_card(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let tablet = &state.flash.tablet_unlock;
    let busy = tablet.reading || tablet.unlocking;

    let return_button = button(text(format!(
        "< {}",
        l10n.tr("flash-bootloader-return")
    )))
    .on_press(Message::TabletUnlockReturnToChooser);

    let description = text(l10n.tr("flash-bootloader-tablet-desc"))
        .size(14.0)
        .width(Fill)
        .wrapping(Wrapping::WordOrGlyph);

    let site_description = text(l10n.tr("flash-bootloader-tablet-site-desc"))
        .size(14.0)
        .width(Fill)
        .wrapping(Wrapping::WordOrGlyph);

    let site_button = button(text(l10n.tr("flash-bootloader-tablet-site-button")))
        .on_press(Message::TabletUnlockOpenSite);

    // Read-only Serial Number box (no `on_input` ⇒ disabled; copying is done
    // with the Copy button).
    let serial_copy = if busy || tablet.serial.is_empty() {
        button(text(l10n.tr("flash-bootloader-copy")))
    } else {
        button(text(l10n.tr("flash-bootloader-copy"))).on_press(Message::TabletUnlockCopySerial)
    };

    let serial_row = row![
        text(l10n.tr("flash-bootloader-tablet-serial"))
            .size(13.0)
            .width(130),
        text_input("", &tablet.serial).width(Fill),
        serial_copy,
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .width(Fill);

    // Read-only Bootloader_SN box. When the bootloader does not report it the
    // box stays empty and carries the "Not applicable" watermark.
    let sn_placeholder = if tablet.read_done && tablet.bootloader_sn.trim().is_empty() {
        l10n.tr("flash-bootloader-tablet-sn-na")
    } else {
        String::new()
    };

    let sn_copy = if busy || tablet.bootloader_sn.trim().is_empty() {
        button(text(l10n.tr("flash-bootloader-copy")))
    } else {
        button(text(l10n.tr("flash-bootloader-copy"))).on_press(Message::TabletUnlockCopySn)
    };

    let sn_row = row![
        text(l10n.tr("flash-bootloader-tablet-sn")).size(13.0).width(130),
        text_input(&sn_placeholder, &tablet.bootloader_sn).width(Fill),
        sn_copy,
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .width(Fill);

    let read_button = if busy {
        button(text(l10n.tr("flash-bootloader-read")))
    } else {
        button(text(l10n.tr("flash-bootloader-read")))
            .on_press(Message::TabletUnlockReadRequested)
    };

    let mut content = column![
        return_button,
        description,
        site_description,
        site_button,
        serial_row,
        sn_row,
        read_button,
    ]
    .spacing(10)
    .width(Fill);

    // Read status line.
    if tablet.reading {
        content = content.push(text(l10n.tr("retcn-fill-fastboot-fetching")).size(13.0));
    } else if let Some(error) = &tablet.read_error {
        content = content.push(
            text(error.clone())
                .size(13.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph)
                .style(iced::widget::text::danger),
        );
    } else if let Some(notice) = &tablet.read_notice {
        content = content.push(
            text(notice.clone())
                .size(13.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph)
                .style(warning_orange),
        );
    }

    content = content
        .push(horizontal_rule(1))
        .push(
            text(l10n.tr("flash-bootloader-tablet-unlock-desc"))
                .size(14.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph),
        );

    // Unlocking builds the token URL from the values read from a device, so a
    // device has to have been read first.
    let ready = !busy && tablet.device.is_some() && !tablet.serial.trim().is_empty();
    let unlock_button = if tablet.unlocking {
        button(text(l10n.tr("flash-bootloader-unlocking")))
    } else if ready {
        button(text(l10n.tr("flash-bootloader-unlock"))).on_press(Message::TabletUnlockRequested)
    } else {
        // Nothing to unlock with yet: iced greys a button without `on_press`
        // out and ignores clicks on it.
        button(text(l10n.tr("flash-bootloader-unlock")))
    };

    content = content.push(unlock_button);

    // Unlock status line. The unlock flashes the token and then sends
    // `oem unlock-go`, which makes the tablet show its confirmation, so how to
    // accept it is shown with the status.
    if tablet.unlocking {
        content = content.push(text(l10n.tr("flash-bootloader-unlocking")).size(13.0));

        for instruction in confirm_instructions(l10n, tablet.platform) {
            content = content.push(
                text(instruction)
                    .size(13.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph),
            );
        }

        // The user can change their mind mid-unlock: the job stops at its next
        // checkpoint and the tablet is rebooted back to the system.
        let cancel_restart = if tablet.cancelling {
            button(text(l10n.tr("flash-bootloader-tablet-cancelling")))
        } else {
            button(text(l10n.tr("flash-bootloader-tablet-cancel-restart")))
                .on_press(Message::TabletUnlockCancelRestart)
        };
        content = content.push(cancel_restart);
    } else if let Some(error) = &tablet.unlock_error {
        content = content.push(
            text(error.clone())
                .size(13.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph)
                .style(iced::widget::text::danger),
        );
    } else if let Some(info) = &tablet.unlock_info {
        content = content.push(
            text(info.clone())
                .size(13.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph)
                .style(bright_success),
        );
    }

    // The padding sits inside the scrollable, so the card's border is the
    // viewport: the scrollbar rides on the right border instead of floating
    // in the middle of the padding.
    container(
        scrollable(container(content).padding(16).width(Fill))
            .width(Fill)
            .height(Fill),
    )
    .width(640)
    .height(560)
    .style(darker_card)
    .into()
}

/// The guided (automatic) tablet unlock. First the connected fastboot devices
/// are offered for selection, then every unlock method is tried automatically.
fn guided_card(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let guided = &state.flash.tablet_unlock.guided;

    let return_button = || {
        button(text(format!(
            "< {}",
            l10n.tr("flash-bootloader-return")
        )))
        .width(Fill)
        .on_press(Message::TabletUnlockReturnToChooser)
    };

    let mut content = column![text(l10n.tr("flash-bootloader-title")).size(18.0)]
        .spacing(10)
        .width(Fill);

    match guided.step {
        TabletGuidedStep::Devices => {
            content = content.push(
                text(l10n.tr("flash-bootloader-tablet-guided-connect"))
                    .size(14.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph),
            );

            if guided.listing {
                content = content.push(text(l10n.tr("factory-reset-reading-devices")).size(13.0));
            } else if let Some(error) = &guided.list_error {
                content = content.push(
                    text(error.clone())
                        .size(13.0)
                        .width(Fill)
                        .wrapping(Wrapping::WordOrGlyph)
                        .style(iced::widget::text::danger),
                );
            } else if guided.devices.is_empty() {
                content = content.push(
                    text(l10n.tr("retcn-fill-fastboot-no-device"))
                        .size(13.0)
                        .width(Fill)
                        .wrapping(Wrapping::WordOrGlyph)
                        .style(warning_orange),
                );
            } else {
                content = content.push(text(l10n.tr("retcn-pick-device-title")).size(13.0));

                for device in &guided.devices {
                    content = content.push(
                        button(text(device.label()))
                            .width(Fill)
                            .on_press(Message::TabletGuidedDeviceSelected(device.serial.clone())),
                    );
                }
            }

            let refresh = if guided.listing {
                button(text(l10n.tr("factory-reset-refresh"))).width(Fill)
            } else {
                button(text(l10n.tr("factory-reset-refresh")))
                    .width(Fill)
                    .on_press(Message::TabletGuidedRefresh)
            };

            content = content.push(refresh).push(return_button()).push(
                button(text(l10n.tr("login-cancel")))
                    .width(Fill)
                    .on_press(Message::TabletUnlockCancel),
            );
        }
        TabletGuidedStep::Working => {
            // While the attempts run there is nothing to report yet; once they
            // are over, the result decides what is shown. While an unlock
            // command waits for the tablet, how to accept the confirmation it
            // shows IS the status: the generic "attempting…" line would leave
            // the user guessing what to press.
            if guided.unlocking {
                let mut lines = if guided.confirming {
                    confirm_instructions(l10n, guided.platform)
                } else {
                    vec![l10n.tr("flash-bootloader-tablet-guided-working")]
                };

                let first = lines.remove(0);
                content = content.push(
                    row![
                        spinner(state.anim_tick),
                        text(first)
                            .size(14.0)
                            .width(Fill)
                            .wrapping(Wrapping::WordOrGlyph),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                );

                for line in lines {
                    content = content.push(
                        text(line)
                            .size(14.0)
                            .width(Fill)
                            .wrapping(Wrapping::WordOrGlyph),
                    );
                }

                let cancel = if guided.cancelling {
                    button(text(l10n.tr("flash-bootloader-tablet-cancelling")))
                } else {
                    button(text(l10n.tr("flash-bootloader-tablet-cancel-restart")))
                        .on_press(Message::TabletGuidedCancelRestart)
                };
                content = content.push(cancel);
            } else {
                content = content.push(
                    text(l10n.tr("flash-bootloader-tablet-guided-working"))
                        .size(14.0)
                        .width(Fill)
                        .wrapping(Wrapping::WordOrGlyph),
                );

                match &guided.result {
                    Some(tablet_unlock::GuidedOutcome::Unlocked) => {
                        content = content.push(
                            text(l10n.tr("flash-bootloader-unlocked"))
                                .size(13.0)
                                .width(Fill)
                                .wrapping(Wrapping::WordOrGlyph)
                                .style(bright_success),
                        );
                    }
                    Some(tablet_unlock::GuidedOutcome::OemUnlockingDisabled) => {
                        content = content.push(
                            text(l10n.tr("flash-bootloader-oem-unlocking-required"))
                                .size(13.0)
                                .width(Fill)
                                .wrapping(Wrapping::WordOrGlyph)
                                .style(iced::widget::text::danger),
                        );
                    }
                    Some(tablet_unlock::GuidedOutcome::Failed(message)) => {
                        content = content.push(
                            text(l10n.tr_with_args(
                                "flash-bootloader-tablet-guided-failed",
                                &[("error", message.clone())],
                            ))
                            .size(13.0)
                            .width(Fill)
                            .wrapping(Wrapping::WordOrGlyph)
                            .style(iced::widget::text::danger),
                        );
                    }
                    // `ManualRequired` switches the dialog itself, and `None`
                    // cannot happen once `unlocking` is false.
                    _ => {}
                }

                content = content.push(return_button()).push(
                    button(text(l10n.tr("login-cancel")))
                        .width(Fill)
                        .on_press(Message::TabletUnlockCancel),
                );
            }
        }
    }

    // The padding sits inside the scrollable, so the card's border is the
    // viewport: the scrollbar rides on the right border instead of floating
    // in the middle of the padding.
    container(
        scrollable(container(content).padding(16).width(Fill))
            .width(Fill)
            .height(Fill),
    )
    .width(640)
    .height(560)
    .style(darker_card)
    .into()
}

/// How the tablet's on-device unlock confirmation has to be accepted.
///
/// The chipset is told apart by the running job; when it could not be
/// recognised every instruction is returned, so the user always has the one
/// that matches the screen in front of them.
fn confirm_instructions(l10n: &l10n::Bundle, platform: Option<Platform>) -> Vec<String> {
    match platform {
        Some(Platform::Qualcomm) => vec![l10n.tr("flash-bootloader-tablet-confirm-qualcomm")],
        Some(Platform::MediaTek) => vec![l10n.tr("flash-bootloader-tablet-confirm-mediatek")],
        None => vec![
            l10n.tr("flash-bootloader-tablet-confirm-mediatek"),
            l10n.tr("flash-bootloader-tablet-confirm-qualcomm"),
        ],
    }
}

/// The fastboot device picker (shown when several devices are connected).
fn picker_card(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let picker = state
        .flash
        .tablet_unlock
        .picker
        .as_ref()
        .expect("picker card is only rendered while a picker is open");

    let device_buttons = iced::widget::Column::with_children(
        picker.devices.iter().map(|serial| {
            button(text(serial.clone()))
                .width(Fill)
                .on_press(Message::TabletUnlockDeviceSelected(serial.clone()))
                .into()
        }),
    )
    .spacing(8);

    container(
        column![
            text(l10n.tr("retcn-pick-device-title")).size(18.0),
            device_buttons,
            button(text(l10n.tr("login-cancel")))
                .on_press(Message::TabletUnlockPickerCancelled),
        ]
        .spacing(12)
        .align_x(Alignment::Center),
    )
    .padding(16)
    .width(340)
    .style(darker_card)
    .into()
}
