//! The "Bootloader Unlock" dialog (Mode 2, Smartphone Flash).
//!
//! This module renders the overlay that appears when the Bootloader Unlock
//! feature tile is pressed: first an "Assistive vs Manual" chooser, then the
//! manual unlock flow (read the Device ID from fastboot, open Motorola's page,
//! and unlock with the returned key). The flow *state* and its update handlers
//! live in `crate` (`State::flash.bootloader`); this module only describes how
//! that state is presented and which messages its widgets send.

use iced::widget::{
    button, column, container, horizontal_rule, mouse_area, row, scrollable,
    text_input, Space,
};
use iced::{Alignment, Element, Fill};

use crate::text;
use crate::{BootloaderDialog, Message, State};

/// A brighter success green than the theme default, used for positive status
/// lines (e.g. "Device ID copied to the clipboard.") so they stand out on the
/// dark dialog background.
fn bright_success(_theme: &iced::Theme) -> iced::widget::text::Style {
    iced::widget::text::Style {
        color: Some(iced::Color::from_rgb8(0x46, 0xE8, 0x8E)),
    }
}

/// An orange used for warnings (e.g. "this phone may be unable to unlock the
/// bootloader") so it reads as a caution rather than an outright error.
fn warning_orange(_theme: &iced::Theme) -> iced::widget::text::Style {
    iced::widget::text::Style {
        color: Some(iced::Color::from_rgb8(0xFF, 0xA5, 0x00)),
    }
}

/// The rounded-box card style used by the dialogs, but with a much darker
/// background so the popup clearly stands out from the page behind it.
///
/// On light themes the standard card colour is kept (darkening a white card
/// would make the dark text illegible).
fn darker_card(theme: &iced::Theme) -> iced::widget::container::Style {
    let mut style = iced::widget::container::rounded_box(theme);
    let background = theme.palette().background;

    if background.r + background.g + background.b < 1.5 {
        let factor = 0.35;
        style.background = Some(iced::Background::Color(iced::Color {
            r: background.r * factor,
            g: background.g * factor,
            b: background.b * factor,
            a: background.a,
        }));
    }

    style
}

/// Renders the Bootloader Unlock overlay over the app, or `None` when no
/// dialog is open. The overlay covers the whole window, including the tab
/// bar. Clicks on empty space are swallowed (they neither dismiss the dialog
/// nor reach the UI underneath); navigation happens only via the dialog's own
/// buttons.
pub(crate) fn overlay(state: &State) -> Option<Element<'_, Message>> {
    match state.flash.bootloader.dialog {
        BootloaderDialog::Closed => None,
        BootloaderDialog::Choosing => {
            let mut children: Vec<Element<'_, Message>> = vec![
                mouse_area(Space::new(Fill, Fill))
                    .on_press(Message::BootloaderBackdropPressed)
                    .into(),
            ];
            children.push(
                container(chooser_card(state))
                    .width(Fill)
                    .height(Fill)
                    .center_x(Fill)
                    .center_y(Fill)
                    .into(),
            );
            Some(iced::widget::Stack::with_children(children).into())
        }
        BootloaderDialog::Manual => {
            let mut children: Vec<Element<'_, Message>> = vec![
                mouse_area(Space::new(Fill, Fill))
                    .on_press(Message::BootloaderBackdropPressed)
                    .into(),
            ];
            children.push(
                container(manual_card(state))
                    .width(Fill)
                    .height(Fill)
                    .center_x(Fill)
                    .center_y(Fill)
                    .into(),
            );
            if state.flash.bootloader.picker.is_some() {
                children.push(
                    mouse_area(Space::new(Fill, Fill))
                        .on_press(Message::BootloaderBackdropPressed)
                        .into(),
                );
                children.push(
                    container(device_picker_card(state))
                        .width(Fill)
                        .height(Fill)
                        .center_x(Fill)
                        .center_y(Fill)
                        .into(),
                );
            }
            Some(iced::widget::Stack::with_children(children).into())
        }
    }
}

/// The first "Bootloader Unlock" dialog: pick between an assistive (not yet
/// available) and a manual unlock flow.
fn chooser_card(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;

    // The assistive flow is not implemented yet, so its button has no
    // action and iced renders it greyed out and unclickable.
    let assistive = button(text(l10n.tr("flash-bootloader-assistive"))).width(Fill);

    let manual = button(text(l10n.tr("flash-bootloader-manual")))
        .width(Fill)
        .on_press(Message::BootloaderManualSelected);

    let cancel = button(text(l10n.tr("login-cancel")))
        .width(Fill)
        .on_press(Message::BootloaderCancel);

    container(
        column![
            text(l10n.tr("flash-bootloader-title")).size(18.0),
            text(l10n.tr("flash-bootloader-choose")).size(14.0),
            assistive,
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

/// The manual bootloader-unlock flow: read the Device ID, open Motorola's
/// page to get the unlock key, then unlock the device with that key.
fn manual_card(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let bootloader = &state.flash.bootloader;
    let busy = bootloader.reading || bootloader.unlocking;

    let return_button = button(text(format!(
        "< {}",
        l10n.tr("flash-bootloader-return")
    )))
    .on_press(Message::BootloaderReturnToChooser);

    let description = text(l10n.tr("flash-bootloader-manual-desc"))
        .size(14.0)
        .width(Fill)
        .wrapping(iced::widget::text::Wrapping::WordOrGlyph);

    let site_button = button(text(l10n.tr("flash-bootloader-open-site")))
        .on_press(Message::BootloaderOpenSite);

    // Read-only Device ID box (no `on_input` ⇒ disabled; the Copy button
    // writes it to the clipboard).
    let device_box = text_input("", &bootloader.device_id);

    let copy_button = if busy || bootloader.device_id.is_empty() {
        button(text(l10n.tr("flash-bootloader-copy")))
    } else {
        button(text(l10n.tr("flash-bootloader-copy")))
            .on_press(Message::BootloaderCopyDeviceId)
    };

    let read_button = if busy {
        button(text(l10n.tr("flash-bootloader-read")))
    } else {
        button(text(l10n.tr("flash-bootloader-read")))
            .on_press(Message::BootloaderReadRequested)
    };

    let device_row =
        row![device_box.width(Fill), copy_button, read_button]
            .spacing(8)
            .align_y(Alignment::Center)
            .width(Fill);

    let mut content = column![
        return_button,
        description,
        site_button,
        text(l10n.tr("flash-bootloader-device-id")).size(13.0),
        device_row,
    ]
    .spacing(10)
    .width(Fill);

    // Status line under the Device ID row.
    if bootloader.reading {
        content = content
            .push(text(l10n.tr("retcn-fill-fastboot-fetching")).size(13.0));
    } else if let Some(error) = &bootloader.read_error {
        content = content
            .push(text(error.clone()).size(13.0).style(iced::widget::text::danger));
    } else if let Some(info) = &bootloader.read_info {
        content = content
            .push(text(info.clone()).size(13.0).style(bright_success));
    }

    // Eligibility check status (queried automatically after the Device ID is
    // read from Motorola's verifyPhone endpoint). Failures (e.g. no Internet)
    // are silent — only a definitive qualifies / not-qualified answer is shown.
    if bootloader.checking {
        content = content
            .push(text(l10n.tr("flash-bootloader-checking")).size(13.0));
    } else if let Some(eligible) = bootloader.eligible {
        if eligible {
            content = content.push(
                text(l10n.tr("flash-bootloader-eligible"))
                    .size(13.0)
                    .style(bright_success),
            );
        } else {
            content = content.push(
                text(l10n.tr("flash-bootloader-not-eligible"))
                    .size(13.0)
                    .style(warning_orange),
            );
        }
    }

    // The unlock key is a separate code returned by Motorola's website — the
    // Device ID itself is never the key. A pasted Device ID is recognised by
    // its first 16-character segment plus the `#` separator that follows it,
    // so the field is rejected when it starts with that prefix (more forgiving
    // than an exact match, e.g. if extra whitespace/newlines were pasted).
    let device_id_prefix: String = bootloader
        .device_id
        .trim()
        .chars()
        .take(17)
        .collect();
    let key_is_device_id =
        !device_id_prefix.is_empty() && bootloader.key_input.trim().starts_with(&device_id_prefix);

    let unlock_button = if bootloader.unlocking {
        button(text(l10n.tr("flash-bootloader-unlocking")))
    } else if busy || key_is_device_id || bootloader.key_input.trim().is_empty() {
        // No real unlock key yet (empty / spaces / pasted the Device ID):
        // greyed out.
        button(text(l10n.tr("flash-bootloader-unlock")))
    } else {
        button(text(l10n.tr("flash-bootloader-unlock")))
            .on_press(Message::BootloaderUnlockRequested)
    };

    content = content
        .push(horizontal_rule(1))
        .push(text(l10n.tr("flash-bootloader-obtain-key")).size(13.0))
        .push(text(l10n.tr("flash-bootloader-warranty-note")).size(13.0))
        .push(text(l10n.tr("flash-bootloader-key-desc")).size(14.0))
        .push(
            row![
                // Right-click pastes the clipboard into the field (for users
                // who don't use Ctrl+V).
                mouse_area(
                    text_input("", &bootloader.key_input)
                        .on_input(Message::BootloaderKeyChanged)
                        .on_submit(Message::BootloaderUnlockRequested)
                        .padding(6)
                        .width(Fill),
                )
                .on_right_press(Message::BootloaderPasteRequested),
                unlock_button,
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .width(Fill),
        );

    // Status line under the Unlock row.
    if bootloader.unlocking {
        content = content
            .push(text(l10n.tr("flash-bootloader-unlocking")).size(13.0));
    } else if let Some(error) = &bootloader.unlock_error {
        content = content
            .push(text(error.clone()).size(13.0).style(iced::widget::text::danger));
    } else if key_is_device_id {
        content = content.push(
            text(l10n.tr("flash-bootloader-key-is-device-id"))
                .size(13.0)
                .style(warning_orange),
        );
    } else if let Some(notice) = &bootloader.unlock_notice {
        content = content
            .push(text(notice.clone()).size(13.0).style(warning_orange));
    } else if let Some(info) = &bootloader.unlock_info {
        content = content
            .push(text(info.clone()).size(13.0).style(bright_success));
    }

    container(scrollable(content).width(Fill).height(Fill))
        .width(640)
        .height(560)
        .padding(16)
        .style(darker_card)
        .into()
}

/// The fastboot device picker (shown when several devices are connected).
fn device_picker_card(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let picker = state
        .flash
        .bootloader
        .picker
        .as_ref()
        .expect("picker card is only rendered while a picker is open");

    let device_buttons = iced::widget::Column::with_children(
        picker.serials.iter().map(|serial| {
            button(text(serial.clone()))
                .width(Fill)
                .on_press(Message::BootloaderDeviceSelected(serial.clone()))
                .into()
        }),
    )
    .spacing(8);

    container(
        column![
            text(l10n.tr("retcn-pick-device-title")).size(18.0),
            device_buttons,
            button(text(l10n.tr("login-cancel")))
                .on_press(Message::BootloaderPickerCancelled),
        ]
        .spacing(12)
        .align_x(Alignment::Center),
    )
    .padding(16)
    .width(340)
    .style(darker_card)
    .into()
}
