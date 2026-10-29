//! The "Partition Manager" page (Mode 2), in its Unisoc form.
//!
//! The page is the Qualcomm Partition Manager's workspace with the two steps a
//! Unisoc phone needs in place of the Qualcomm ones — "Inject FDL", which loads
//! the download agents, and "Flash PAC", which writes a package — while
//! everything behind them is still missing. Of the page itself only "Exit" and
//! "Inject FDL" are live: the card the latter opens sets the download agents up
//! (which SoC's addresses, which files, and optionally the payload that runs
//! before them), and everything that would talk to a phone afterwards is drawn
//! greyed out.
//!
//! The labels the two pages share (Dump, Write, Erase, the columns, the
//! storage types, the danger zone) keep their `qc-` FTL ids: they are the
//! same words, and the page that grows out of this one will say them too.

use iced::widget::text::{Shaping, Wrapping};
use iced::widget::{
    button, checkbox, column, container, mouse_area, pick_list, row, scrollable,
    text_input,
};
use iced::{Alignment, Fill};

use crate::bootloader::{bright_success, darker_card, warning_orange};
use crate::compat_iced as iced;
use crate::compat_iced::*;
use crate::guided::spinner;
use crate::pac::PacError;
use crate::qualcomm::{
    device_manager_button, menu_width, split_row, value, COLUMN_BEGIN,
    COLUMN_END, COLUMN_LUN, COLUMN_PARTITION, COLUMN_SIZE, PORT_MENU_WIDTH,
    REFRESH_ICON, STORAGE_MENU_WIDTH,
};
use crate::text;
use crate::{
    DangerEntry, FdlDialog, FdlNotice, FdlPreset, FdlSlot, Labeled, Message,
    RebootMode, State, StorageType,
};

/// Id of the partition table's scrollable, so its position survives the page
/// being rebuilt for every message.
const TABLE_SCROLL_ID: &str = "unisoc-partitions";

/// The width of the "Inject FDL" card: wide enough for the payload's field and
/// the address beside it.
const INJECT_CARD_WIDTH: f32 = 620.0;

/// The width of the card's row labels, so the fields line up.
const INJECT_LABEL_WIDTH: f32 = 100.0;

/// The width of an address field: `0x28007000` is the longest a preset uses.
const ADDRESS_FIELD_WIDTH: f32 = 110.0;

/// Whether the Unisoc Partition Manager takes the place of the Mode 2 feature
/// grid.
pub(crate) fn is_open(state: &State) -> bool {
    state.flash.unisoc.open
}

/// The Unisoc Partition Manager workspace, shown in place of the Mode 2
/// feature grid while `state.flash.unisoc.open` is set.
pub(crate) fn view(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let unisoc = &state.flash.unisoc;

    // [Inject FDL] [Reboot ▾] ... [Exit]: the row the Qualcomm page carries.
    // Injecting the download agents is the step every action on the page waits
    // for, and it is the only action that exists so far: the card it opens
    // sets them up, and the reboot menu — whose modes are sessions of their
    // own — is greyed out. Exit is the way back to the feature grid.
    let inject = button(text(l10n.tr("uc-inject-fdl")))
        .on_press(Message::UnisocFdlOpened);

    // The menu is drawn as the control it becomes, at the width its longest
    // entry needs, so the row keeps the room the real menu takes.
    let reboot_labels: Vec<String> = RebootMode::ALL
        .iter()
        .map(|&mode| l10n.tr(mode.message_id()))
        .collect();
    let reboot = button(text(l10n.tr("firmware-flash-reboot")))
        .width(menu_width(&reboot_labels));

    let exit = button(text(l10n.tr("multi-flash-exit")))
        .on_press(Message::UnisocExit);

    let top =
        split_row(vec![inject.into(), reboot.into()], vec![exit.into()]);

    // [Dump] [Write] [Erase] [↻] ... [Flash PAC]: the actions of the table
    // stay together at the left, the package flash sits at the far right. All
    // of them work on a table read through the download agent, so all of them
    // are greyed out.
    let dump = button(text(l10n.tr("qc-dump")));
    let write = button(text(l10n.tr("qc-write")));
    let erase = button(text(l10n.tr("qc-erase")));
    let refresh = button(text(REFRESH_ICON).size(16.0));
    let flash_pac = button(text(l10n.tr("uc-flash-pac")));

    let actions = split_row(
        vec![dump.into(), write.into(), erase.into(), refresh.into()],
        vec![flash_pac.into()],
    );

    // The partition table: the head stays in place while the body scrolls.
    let head = row![
        value(l10n.tr("partition-flash-partition"), COLUMN_PARTITION),
        value(l10n.tr("qc-col-lun"), COLUMN_LUN),
        value(l10n.tr("qc-col-begin"), COLUMN_BEGIN),
        value(l10n.tr("qc-col-end"), COLUMN_END),
        value(l10n.tr("pac-repack-col-size"), COLUMN_SIZE),
    ]
    .align_y(Alignment::Center);

    // Nothing was read yet, and nothing can be: the body says so.
    let body = scrollable(
        container(text(l10n.tr("qc-idle")).size(12.0).width(Fill))
            .padding([8.0, 6.0]),
    )
    .id(iced::widget::Id::new(TABLE_SCROLL_ID))
    .width(Fill)
    .height(Fill);

    let table = container(
        column![head, horizontal_rule(1), body]
            .spacing(4)
            .width(Fill)
            .height(Fill),
    )
    .width(Fill)
    .height(Fill)
    .padding(12)
    .style(darker_card);

    // Search Partition: narrows the rows down to the names that contain the
    // query. The box keeps what was typed, so the page opens the way it was
    // left — it has no rows to narrow down yet.
    let search = text_input("", &unisoc.filter)
        .on_input(Message::UnisocFilterChanged)
        .width(Fill);

    let search_row = row![text(l10n.tr("qc-search")).size(12.0), search]
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Fill);

    // [Dump Entire Storage] [Danger Zone ▾]: both work on a table read through
    // the download agent, so both are greyed out. The danger menu is drawn at
    // the width its longest entry needs, like the reboot menu above.
    let dump_storage = button(text(l10n.tr("qc-dump-entire-storage")));

    let danger_labels: Vec<String> = DangerEntry::ALL
        .iter()
        .map(|&entry| l10n.tr(entry.message_id()))
        .collect();
    let danger = button(text(l10n.tr("qc-danger-zone")))
        .width(menu_width(&danger_labels));

    let storage_row = row![dump_storage, danger]
        .spacing(8)
        .align_y(Alignment::Center);

    // [View Log] [Flash Info] ... [Storage Type ▾] [Port ▾] [Device Manager]:
    // the two actions need a session the page cannot start, so they are
    // greyed out; the two dropdowns keep the choices the flow will be set up
    // with. The port menu lists what the download mode offers — nothing,
    // until the page scans for one.
    let view_log = button(text(l10n.tr("qc-view-log")));
    let flash_info = button(text(l10n.tr("qc-flash-info")));

    let storage_options: Vec<Labeled<StorageType>> = StorageType::ALL
        .iter()
        .map(|&storage| Labeled {
            value: storage,
            label: l10n.tr(storage.message_id()),
        })
        .collect();
    let storage_selected = Labeled {
        value: unisoc.storage,
        label: l10n.tr(unisoc.storage.message_id()),
    };
    let storage: iced::widget::PickList<
        '_,
        Labeled<StorageType>,
        Vec<Labeled<StorageType>>,
        Labeled<StorageType>,
        Message,
        crate::fluent::Theme,
    > = pick_list(storage_options, Some(storage_selected), |option| {
        Message::UnisocStorageTypeSelected(option.value)
    })
    .width(STORAGE_MENU_WIDTH)
    .text_shaping(Shaping::Advanced);

    let port: iced::widget::PickList<
        '_,
        String,
        Vec<String>,
        String,
        Message,
        crate::fluent::Theme,
    > = pick_list(
        unisoc.ports.clone(),
        unisoc.port.clone(),
        Message::UnisocPortSelected,
    )
    .placeholder(l10n.tr("qc-port"))
    .width(menu_width(&unisoc.ports).max(PORT_MENU_WIDTH))
    .text_shaping(Shaping::Advanced);

    let mut tools = vec![
        text(l10n.tr("qc-storage-type")).size(12.0).into(),
        storage.into(),
        port.into(),
    ];

    if cfg!(windows) {
        tools.push(device_manager_button(l10n, None));
    }

    let tool_row = split_row(vec![view_log.into(), flash_info.into()], tools);

    let layout: Vec<Element<'_, Message>> = vec![
        top,
        horizontal_rule(1).into(),
        actions,
        horizontal_rule(1).into(),
        table.into(),
        horizontal_rule(1).into(),
        search_row.into(),
        storage_row.into(),
        tool_row,
    ];

    let content = Column::with_children(layout)
        .spacing(8)
        .width(Fill)
        .height(Fill)
        .align_x(Alignment::Start);

    container(content)
        .width(Fill)
        .height(Fill)
        .padding(16)
        .into()
}

/// Whether the "Inject FDL" card is up.
pub(crate) fn inject_open(state: &State) -> bool {
    state.flash.unisoc.fdl.open
}

/// The "Inject FDL" card, or `None` while it is closed.
///
/// The card is left through its own buttons only: a click beside it is
/// swallowed, since the addresses and the payload in it are not cheap to type
/// in again.
pub(crate) fn inject_overlay(state: &State) -> Option<Element<'_, Message>> {
    let fdl = &state.flash.unisoc.fdl;
    if !fdl.open {
        return None;
    }

    let children: Vec<Element<'_, Message>> = vec![
        mouse_area(Space::new(Fill, Fill))
            .on_press(Message::UnisocFdlBackdropPressed)
            .into(),
        container(inject_card(state, fdl))
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill)
            .into(),
    ];

    Some(iced::widget::Stack::with_children(children).into())
}

/// The card itself: which SoC's addresses the agents are loaded at, the two
/// agents, and — optionally — the payload that runs before them.
///
/// There is no port to pick, unlike the Qualcomm page: the command is started
/// first and a phone is plugged in afterwards, so the card is set up on its own
/// and the phone is caught by its boot ROM once "Inject" is pressed.
fn inject_card<'a>(
    state: &'a State,
    fdl: &'a FdlDialog,
) -> Element<'a, Message> {
    let l10n = &state.l10n;

    // Nothing is changed while a package is read: the fields it fills would
    // otherwise be overwritten under the user.
    let enabled = !fdl.reading_package;

    let mut content = column![
        text(l10n.tr("uc-inject-fdl")).size(18.0),
        preset_row(l10n, fdl),
        agent_row(l10n, "uc-fdl1-address", FdlSlot::One, fdl, enabled),
        agent_row(l10n, "uc-fdl2-address", FdlSlot::Two, fdl, enabled),
        horizontal_rule(1),
        bypass_section(l10n, fdl, enabled),
        horizontal_rule(1),
    ]
    .spacing(10)
    .width(Fill);

    // [Inject] [Load from PAC] ... [Cancel]: the two things the card can do
    // stay together at the left, the way out sits at the right.
    let inject = button(text(l10n.tr("uc-inject")))
        .on_press(Message::UnisocFdlInject);

    let load = button(text(l10n.tr("uc-load-from-pac")));
    let load = if enabled {
        load.on_press(Message::UnisocFdlPickPackage)
    } else {
        load
    };

    let cancel = button(text(l10n.tr("login-cancel")))
        .on_press(Message::UnisocFdlClosed);

    content = content.push(split_row(
        vec![inject.into(), load.into()],
        vec![cancel.into()],
    ));

    if let Some(notice) = notice_row(state, fdl) {
        content = content.push(notice);
    }

    container(content)
        .padding(16)
        .width(INJECT_CARD_WIDTH)
        .style(darker_card)
        .into()
}

/// The card's CPU row: picking a SoC fills the two addresses below it in.
fn preset_row<'a>(
    l10n: &'a crate::l10n::Bundle,
    fdl: &FdlDialog,
) -> Element<'a, Message> {
    let options: Vec<Labeled<FdlPreset>> = FdlPreset::ALL
        .iter()
        .map(|&preset| Labeled {
            value: preset,
            label: preset.label(l10n),
        })
        .collect();
    let labels: Vec<String> = options
        .iter()
        .map(|option| option.label.clone())
        .collect();
    let selected = Labeled {
        value: fdl.preset,
        label: fdl.preset.label(l10n),
    };

    let menu: iced::widget::PickList<
        '_,
        Labeled<FdlPreset>,
        Vec<Labeled<FdlPreset>>,
        Labeled<FdlPreset>,
        Message,
        crate::fluent::Theme,
    > = pick_list(options, Some(selected), |option| {
        Message::UnisocFdlPresetSelected(option.value)
    })
    .placeholder(l10n.tr("uc-cpu-preset"))
    .width(menu_width(&labels))
    .text_shaping(Shaping::Advanced);

    row![row_label(l10n.tr("uc-cpu-preset")), menu]
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
}

/// One download agent's row: the address its SoC loads it at, the button that
/// picks the file, and what was picked.
fn agent_row<'a>(
    l10n: &'a crate::l10n::Bundle,
    label_id: &'static str,
    slot: FdlSlot,
    fdl: &'a FdlDialog,
    enabled: bool,
) -> Element<'a, Message> {
    let (address, image) = match slot {
        FdlSlot::One => (&fdl.fdl1_address, fdl.fdl1.as_ref()),
        FdlSlot::Two => (&fdl.fdl2_address, fdl.fdl2.as_ref()),
    };

    // A field without `on_input` is read-only, which is what the card needs
    // while a package is being read.
    let field = text_input("0x0", address).width(ADDRESS_FIELD_WIDTH);
    let field = if enabled {
        field.on_input(move |value| {
            Message::UnisocFdlAddressChanged(slot, value)
        })
    } else {
        field
    };

    let pick = button(text(l10n.tr("uc-select-file")));
    let pick = if enabled {
        pick.on_press(Message::UnisocFdlPickFile(slot))
    } else {
        pick
    };

    let chosen = match image {
        Some(image) => image.label(),
        None => l10n.tr("unpack-none-selected"),
    };

    let mut content = column![
        row![row_label(l10n.tr(label_id)), field, pick]
            .spacing(8)
            .align_y(Alignment::Center),
    ]
    .spacing(2)
    .width(Fill);

    if !address.trim().is_empty() && !is_address(address) {
        content = content.push(hex_hint(l10n.tr("uc-hex-invalid")));
    }

    // What was picked sits under the row: a file name is longer than the room
    // the row leaves for it, and the address beside it is what is edited.
    content = content.push(
        text(chosen)
            .size(12.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph),
    );

    content.into()
}

/// The card's bypass section: the switch, and the payload it sends to unlock
/// the phone's execution verification before the agents are loaded.
fn bypass_section<'a>(
    l10n: &'a crate::l10n::Bundle,
    fdl: &FdlDialog,
    enabled: bool,
) -> Element<'a, Message> {
    let switch = checkbox(l10n.tr("uc-bypass-verification"), fdl.bypass)
        .text_shaping(Shaping::Advanced);
    let switch = if enabled {
        switch.on_toggle(Message::UnisocFdlBypassToggled)
    } else {
        switch
    };

    // The two fields only mean anything while the payload is sent at all, so
    // they stay read-only until the switch is on.
    let editable = enabled && fdl.bypass;

    let address =
        text_input("0x0", &fdl.exec_address).width(ADDRESS_FIELD_WIDTH);
    let address = if editable {
        address.on_input(Message::UnisocFdlExecAddressChanged)
    } else {
        address
    };

    let payload = text_input("", &fdl.payload).width(Fill);
    let payload = if editable {
        payload.on_input(Message::UnisocFdlPayloadChanged)
    } else {
        payload
    };

    let mut content = column![
        switch,
        row![
            row_label(l10n.tr("uc-exec-address")),
            address,
            row_label(l10n.tr("uc-payload-hex")),
            payload,
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    ]
    .spacing(6)
    .width(Fill);

    // A typo in either field is what a phone in download mode would be given,
    // so the section says when it cannot be read as hexadecimal.
    if editable
        && !fdl.exec_address.trim().is_empty()
        && !is_address(&fdl.exec_address)
    {
        content = content.push(hex_hint(l10n.tr("uc-hex-invalid")));
    }

    if editable
        && !fdl.payload.trim().is_empty()
        && !is_hex_payload(&fdl.payload)
    {
        content = content.push(hex_hint(l10n.tr("uc-payload-invalid")));
    }

    content.into()
}

/// What the card says under its buttons: what it is doing, what the last
/// action did, or why it could not.
fn notice_row<'a>(
    state: &'a State,
    fdl: &'a FdlDialog,
) -> Option<Element<'a, Message>> {
    let l10n = &state.l10n;

    if fdl.reading_package {
        return Some(
            row![
                spinner(state.anim_tick),
                text(l10n.tr("uc-reading-pac")).size(12.0),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into(),
        );
    }

    let notice = fdl.notice.as_ref()?;

    // A notice is a sentence and the colour it is drawn in: what the card did
    // is good news for some and a warning for others.
    type Paint = fn(&crate::fluent::Theme) -> iced::widget::text::Style;

    let (message, style): (String, Paint) = match notice {
        FdlNotice::NotImplemented => {
            (l10n.tr("uc-inject-not-implemented"), warning_orange)
        }
        FdlNotice::PackageLoaded(package) => (
            l10n.tr_with_args("uc-pac-loaded", &[("pac", package.clone())]),
            bright_success,
        ),
        FdlNotice::PackageFailed(PacError::NotAPac) => {
            (l10n.tr("pac-invalid"), iced::widget::text::danger)
        }
        FdlNotice::PackageFailed(PacError::Encrypted) => {
            (l10n.tr("pac-encrypted"), iced::widget::text::danger)
        }
        FdlNotice::PackageFailed(error) => (
            l10n.tr_with_args("pac-failed", &[("error", error.to_string())]),
            iced::widget::text::danger,
        ),
        FdlNotice::PackageMissing(id) => (
            l10n.tr_with_args("uc-pac-missing", &[("id", id.clone())]),
            iced::widget::text::danger,
        ),
    };

    Some(
        text(message)
            .size(12.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph)
            .style(style)
            .into(),
    )
}

/// A label at the left of one of the card's rows.
fn row_label<'a>(label: String) -> Element<'a, Message> {
    container(text(label).size(13.0))
        .width(INJECT_LABEL_WIDTH)
        .align_y(Alignment::Center)
        .into()
}

/// A field's complaint, in the colour the app uses for what will not work.
fn hex_hint<'a>(message: String) -> Element<'a, Message> {
    text(message)
        .size(12.0)
        .width(Fill)
        .wrapping(Wrapping::WordOrGlyph)
        .style(iced::widget::text::danger)
        .into()
}

/// Whether an address field holds a number: hexadecimal digits, with or
/// without the `0x` the card fills in.
fn is_address(text: &str) -> bool {
    let text = text.trim();
    let digits = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .unwrap_or(text);

    !digits.is_empty()
        && digits.chars().all(|digit| digit.is_ascii_hexdigit())
        && u64::from_str_radix(digits, 16).is_ok()
}

/// Whether the payload field holds whole bytes: hexadecimal digits, whitespace
/// allowed between them.
fn is_hex_payload(text: &str) -> bool {
    let digits: Vec<char> = text
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();

    !digits.is_empty()
        && digits.len() % 2 == 0
        && digits.iter().all(|digit| digit.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::{is_address, is_hex_payload};

    /// The `0x` is optional, since the presets write it and a user may not.
    #[test]
    fn an_address_is_hexadecimal_with_or_without_the_prefix() {
        assert!(is_address("0x5500"));
        assert!(is_address("0X9EFFFE00"));
        assert!(is_address(" 5500 "));
        assert!(is_address("0"));

        assert!(!is_address(""));
        assert!(!is_address("   "));
        assert!(!is_address("0x"));
        assert!(!is_address("0xGG"));
        assert!(!is_address("5500 1234"));
    }

    /// The payload is sent byte by byte, so a half byte at the end is not one.
    #[test]
    fn the_payload_is_whole_bytes_of_hexadecimal_digits() {
        assert!(is_hex_payload("DEADBEEF"));
        assert!(is_hex_payload("00 01 02 03"));
        assert!(is_hex_payload("de ad be ef"));

        assert!(!is_hex_payload(""));
        assert!(!is_hex_payload("   "));
        assert!(!is_hex_payload("DEADBEE"));
        assert!(!is_hex_payload("00 0G"));
    }
}
