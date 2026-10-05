//! The tablet "Bootloader Unlock" dialog (Mode 2, Smartphone Flash).
//!
//! Lenovo tablets unlock differently from Motorola phones: the serial number
//! and the `Bootloader_SN` are submitted on ZUI's website, and the unlock
//! token it generates is downloaded and flashed from here. This module renders
//! the chooser and the (manual) unlock card; the state and its update handlers
//! live in `crate` (`State::flash.tablet_unlock`).
//!
//! Only the manual flow is implemented for tablets — the guided wizard is
//! Motorola-specific, so its button is left greyed out.

use iced::widget::text::Wrapping;
use iced::widget::{
    button, column, container, horizontal_rule, mouse_area, row, scrollable, text_input, Space,
};
use iced::{Alignment, Element, Fill};

use crate::bootloader::{bright_success, darker_card, warning_orange};
use crate::text;
use crate::{Message, State, TabletUnlockDialog};

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

/// The Guided-vs-Manual chooser. The guided wizard is not implemented for
/// tablets, so its button is disabled.
fn chooser_card(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;

    let guided = button(text(l10n.tr("flash-bootloader-guided"))).width(Fill);

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

    // Unlock status line.
    if tablet.unlocking {
        content = content.push(text(l10n.tr("flash-bootloader-unlocking")).size(13.0));
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
