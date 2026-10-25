//! The "Partition Manager" page (Mode 2), in its Qualcomm form.
//!
//! A full-window workspace like Multi Flashing: it shows the partition table
//! of a Qualcomm phone read through Firehose, together with the actions that
//! dump, write, erase or re-read the ranges in it.
//!
//! Injecting the programmer and reading the table (and every action that talks
//! to the phone afterwards: dumping, writing, erasing and resetting) is
//! implemented in [`crate::firehose`]. The rows are selected the way a file
//! list is — Command/Control toggles one, Shift takes a range, Control+A takes
//! every row on screen — and the Dump / Write / Erase buttons act on that
//! selection.

use iced::widget::text::{Shaping, Wrapping};
use iced::widget::{
    button, canvas, column, container, mouse_area, pick_list, progress_bar, row, scrollable,
    text_input, tooltip,
};
use iced::{Alignment, Fill};

use crate::bootloader::{bright_success, darker_card};
use crate::compat_iced as iced;
use crate::compat_iced::*;
use crate::firmware::{grouped_bytes, human_size, human_size_exact};
use crate::firmware_flash::menu_style;
use crate::text;
use crate::{
    AfterFlash, DangerEntry, FlashInfoDialog, ImageFit, Labeled, LunDialog, Message, QualcommStatus,
    RebootMode, State, StorageFlashDialog, StorageType, XmlFlashDialog,
};

/// Id of the partition table's scrollable, so its position survives the page
/// being rebuilt for every message.
const TABLE_SCROLL_ID: &str = "qualcomm-partitions";

/// Id of the LUN picker's list, for the same reason.
const LUN_SCROLL_ID: &str = "qualcomm-luns";

/// Id of the storage-flash card's list.
const STORAGE_FLASH_SCROLL_ID: &str = "qualcomm-storage-flash";

/// How tall the two write cards' lists are: the card keeps its buttons in view
/// below them, and the list scrolls on its own.
const STORAGE_FLASH_LIST_HEIGHT: f32 = 180.0;

/// Id of the partition-backup card's list.
const BACKUP_SCROLL_ID: &str = "qualcomm-backup";

/// Id of the "Flash XML" card's checklist.
const XML_FLASH_SCROLL_ID: &str = "qualcomm-xml-flash";

/// How tall the checklist is: the card keeps its buttons in view below it.
const XML_FLASH_LIST_HEIGHT: f32 = 200.0;

/// The width of the card's "After Flashing" menu: wide enough for its longest
/// entry ("Reboot to recovery").
const AFTER_FLASH_MENU_WIDTH: f32 = 190.0;

/// The width of one partition dropdown in the backup card.
const BACKUP_MENU_WIDTH: f32 = 190.0;

/// The width of the labels of the identity card's rows, so the four values
/// line up.
const INJECT_ID_LABEL_WIDTH: f32 = 110.0;

/// How many hex characters of the key hash go on one line: the hash is
/// sixty-four of them, which is twice what the card is wide.
const INJECT_HASH_CHUNK: usize = 32;

/// The width of a fact's label in the "Flash Info" card: wide enough for the
/// longest one the phone sends without squeezing its value.
const STORAGE_INFO_LABEL_WIDTH: f32 = 230.0;

/// How tall the "Flash Info" card's list of facts is: the card keeps its
/// buttons in view below it, and the list scrolls on its own.
const STORAGE_INFO_HEIGHT: f32 = 280.0;

/// Id of the "Flash Info" card's list.
const FLASH_INFO_SCROLL_ID: &str = "qualcomm-flash-info";

/// The width of one image dropdown in the storage-flash card: wide enough for
/// a dumped file name (`lun4 (2).img`).
const STORAGE_FLASH_MENU_WIDTH: f32 = 190.0;

/// The width of the table's "Partition Name" column, in logical pixels.
const COLUMN_PARTITION: f32 = 220.0;
/// The width of its "LUN" column.
const COLUMN_LUN: f32 = 60.0;
/// The width of its "Begin Offset" column.
const COLUMN_BEGIN: f32 = 130.0;
/// The width of its "End Offset" column.
const COLUMN_END: f32 = 130.0;
/// The width of its "Size" column.
const COLUMN_SIZE: f32 = 110.0;

/// The room a table cell keeps around its content, so the columns stay apart.
const CELL_PADDING: [f32; 2] = [3.0, 6.0];

/// The refresh control shows the conventional icon instead of its word.
const REFRESH_ICON: &str = "↻";

/// The height of the job's progress bar under the table.
const BAR_HEIGHT: f32 = 6.0;

/// The narrowest the port dropdown gets: wide enough for the longest name a
/// COM port can have (`COM255`), so picking one never makes it smaller.
const PORT_MENU_WIDTH: f32 = 130.0;

/// The width of the storage dropdown: wide enough for its longest entry
/// ("SPI NOR Flash").
const STORAGE_MENU_WIDTH: f32 = 150.0;

/// Whether the Qualcomm Partition Manager takes the place of the Mode 2
/// feature grid.
pub(crate) fn is_open(state: &State) -> bool {
    state.flash.qualcomm.open
}

/// The Qualcomm Partition Manager workspace, shown in place of the Mode 2
/// feature grid while `state.flash.qualcomm.open` is set.
pub(crate) fn view(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let qualcomm = &state.flash.qualcomm;

    // [Inject Firehose] [Reboot ▾] ... [Exit]: the two actions stay together
    // at the left, the way out sits at the far right. Injection needs a port
    // to talk to, so its button is greyed out until one is picked, and while
    // a job runs. Exit is greyed out once the programmer is running: the
    // phone is in Firehose mode then, and only a reset (or closing the window,
    // which asks first) gets it out again.
    let exit = if qualcomm.busy || qualcomm.programmer_uploaded {
        button(text(l10n.tr("multi-flash-exit")))
    } else {
        button(text(l10n.tr("multi-flash-exit"))).on_press(Message::QualcommExit)
    };

    // The reboot modes act on the session that read the table, so the menu
    // waits for one — iced cannot grey a `pick_list` out, so a plain button
    // takes its place until then. After a whole-storage write the rows are gone
    // but the session is not, and a normal reset needs no table: only that one
    // is offered until the table is read again.
    let reboot_modes: Vec<RebootMode> = if qualcomm.partitions.is_empty() {
        // Without a table the modes that name partitions in it have nothing to
        // work with; a plain reset and a power-off still do.
        vec![RebootMode::Normal, RebootMode::PowerOff]
    } else {
        RebootMode::ALL.to_vec()
    };
    let reboot_options: Vec<Labeled<RebootMode>> = reboot_modes
        .iter()
        .map(|&mode| Labeled {
            value: mode,
            label: l10n.tr(mode.message_id()),
        })
        .collect();
    let reboot_labels: Vec<String> = reboot_options
        .iter()
        .map(|option| option.label.clone())
        .collect();
    let reboot_width = menu_width(&reboot_labels);

    let reboot: Element<'_, Message> = if reboot_ready(qualcomm) {
        let menu: iced::widget::PickList<
            '_,
            Labeled<RebootMode>,
            Vec<Labeled<RebootMode>>,
            Labeled<RebootMode>,
            Message,
            crate::fluent::Theme,
        > = pick_list(reboot_options, None, |option| {
            Message::QualcommRebootModeSelected(option.value)
        })
        .placeholder(l10n.tr("firmware-flash-reboot"))
        .width(reboot_width)        // The control always shows its placeholder, so it has to read like the
        // button it replaces — iced's own placeholder colour is the dim grey of
        // an empty field.
        .style(menu_style)
        .text_shaping(Shaping::Advanced);

        menu.into()
    } else {
        button(text(l10n.tr("firmware-flash-reboot")))
            .width(reboot_width)
            .into()
    };

    let inject = if qualcomm.port.is_some()
        && !qualcomm.busy
        && !qualcomm.programmer_uploaded
    {
        button(text(l10n.tr("qc-inject-firehose"))).on_press(Message::QualcommInjectFirehose)
    } else {
        button(text(l10n.tr("qc-inject-firehose")))
    };

    let top = split_row(
        vec![inject.into(), reboot],
        vec![exit.into()],
    );

    // [Dump] [Write] [Erase] [↻] ... [Flash XML]: the actions of the table
    // stay together at the left. Dump works on every selected row, Write on
    // exactly one (it needs an image for one partition), and both need the
    // selection the table's rows give.
    let chosen = qualcomm.selected.iter().filter(|selected| **selected).count();
    let dump = if chosen > 0 && !qualcomm.busy {
        button(text(l10n.tr("qc-dump"))).on_press(Message::QualcommDump)
    } else {
        button(text(l10n.tr("qc-dump")))
    };
    let write = if chosen == 1 && !qualcomm.busy {
        button(text(l10n.tr("qc-write"))).on_press(Message::QualcommWrite)
    } else {
        button(text(l10n.tr("qc-write")))
    };
    let erase = if chosen > 0 && !qualcomm.busy {
        button(text(l10n.tr("qc-erase"))).on_press(Message::QualcommErase)
    } else {
        button(text(l10n.tr("qc-erase")))
    };

    // The package flash runs on the programmer in the phone, so its button
    // waits for one and for no job to be under way.
    let flash_xml = if qualcomm.programmer_uploaded && !qualcomm.busy {
        button(text(l10n.tr("qc-flash-xml"))).on_press(Message::QualcommXmlFlash)
    } else {
        button(text(l10n.tr("qc-flash-xml")))
    };

    let actions = split_row(
        vec![
            dump.into(),
            write.into(),
            erase.into(),
            refresh_button(qualcomm.programmer_uploaded && !qualcomm.busy),
        ],
        vec![flash_xml.into()],
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

    // The rows the table shows: what the injection read, narrowed down by the
    // search box. The offsets are byte positions, so a range can be compared
    // with a dump without knowing the sector size. A row is a toggle: the
    // table is a list in which any number of partitions can be selected, and
    // an action that works on a range uses that selection.
    let query = qualcomm.filter.trim().to_lowercase();
    let rows: Vec<Element<'_, Message>> = qualcomm
        .partitions
        .iter()
        .enumerate()
        .filter(|(_, partition)| {
            query.is_empty() || partition.name.to_lowercase().contains(&query)
        })
        .map(|(index, partition)| {
            let selected = qualcomm.selected.get(index).copied().unwrap_or(false);
            let line: Element<'_, Message> = row![
                value(&partition.name, COLUMN_PARTITION),
                value(partition.lun.to_string(), COLUMN_LUN),
                value(format!("0x{:X}", partition.begin_offset()), COLUMN_BEGIN),
                value(format!("0x{:X}", partition.end_offset()), COLUMN_END),
                value(human_size(partition.size_bytes()), COLUMN_SIZE),
            ]
            .align_y(Alignment::Center)
            .into();

            mouse_area(
                container(line)
                    .width(Fill)
                    .padding([1.0, 2.0])
                    .style(move |theme: &crate::fluent::Theme| {
                        selected_row_style(theme, selected)
                    }),
            )
            .on_press(Message::QualcommPartitionClicked(index))
            .into()
        })
        .collect();

    // An empty body says why it is empty: nothing read yet, nothing on that
    // storage, or nothing matching the search.
    let empty: Option<Element<'_, Message>> = if rows.is_empty() {
        let message = match &qualcomm.status {
            QualcommStatus::Table { count } if *count > 0 => {
                l10n.tr_with_args("qc-no-match", &[("query", qualcomm.filter.clone())])
            }
            QualcommStatus::Table { .. } => l10n.tr("qc-partitions-none"),
            _ => l10n.tr("qc-idle"),
        };

        Some(text(message).size(12.0).width(Fill).into())
    } else {
        None
    };

    let mut body_rows = Column::with_children(rows).spacing(3).width(Fill);
    if let Some(empty) = empty {
        body_rows = body_rows.push(empty);
    }

    let body = scrollable(
        container(body_rows).padding([8.0, 6.0]),
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
    // query (case-insensitively).
    let search = text_input("", &state.flash.qualcomm.filter)
        .on_input(Message::QualcommFilterChanged)
        .width(Fill);

    let search_row = row![text(l10n.tr("qc-search")).size(12.0), search]
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Fill);

    // [Dump Entire Storage] [Danger Zone ▾]: the first reads whole physical
    // partitions, the second lists what the section will offer once it
    // exists. Only "Flash Entire Storage" is live so far; the other entries
    // are ignored and the control keeps its placeholder, which is how the
    // page's greyed-out buttons behave. Like Reboot, both only work once a
    // table was read.
    let dump_storage = if table_loaded(qualcomm) {
        button(text(l10n.tr("qc-dump-entire-storage"))).on_press(Message::QualcommDumpStorage)
    } else {
        button(text(l10n.tr("qc-dump-entire-storage")))
    };

    let danger_options: Vec<Labeled<DangerEntry>> = DangerEntry::ALL
        .iter()
        .map(|&entry| Labeled {
            value: entry,
            label: l10n.tr(entry.message_id()),
        })
        .collect();
    let danger_labels: Vec<String> = danger_options
        .iter()
        .map(|option| option.label.clone())
        .collect();
    let danger_width = menu_width(&danger_labels);

    let danger: Element<'_, Message> = if table_loaded(qualcomm) {
        let menu: iced::widget::PickList<
            '_,
            Labeled<DangerEntry>,
            Vec<Labeled<DangerEntry>>,
            Labeled<DangerEntry>,
            Message,
            crate::fluent::Theme,
        > = pick_list(danger_options, None, |option| {
            Message::QualcommDangerZoneSelected(option.value)
        })
        .placeholder(l10n.tr("qc-danger-zone"))
        .width(danger_width)
        .style(menu_style)
        .text_shaping(Shaping::Advanced);

        menu.into()
    } else {
        button(text(l10n.tr("qc-danger-zone")))
            .width(danger_width)
            .into()
    };

    let storage_row = row![dump_storage, danger]
        .spacing(8)
        .align_y(Alignment::Center);

    // [View Log] ... [Storage Type ▾] [Port ▾] [Device Manager]: the tools stay
    // at the left, while the two dropdowns and the Windows-only Device Manager
    // button sit at the far right. The port menu lists the COM ports (or
    // `/dev/ttyUSB*` / `/dev/cu.usbmodem*` devices) that enumerate as
    // 05C6:9008.
    let storage_options: Vec<Labeled<StorageType>> = StorageType::ALL
        .iter()
        .map(|&storage| Labeled {
            value: storage,
            label: l10n.tr(storage.message_id()),
        })
        .collect();
    let storage_selected = Labeled {
        value: state.flash.qualcomm.storage,
        label: l10n.tr(state.flash.qualcomm.storage.message_id()),
    };
    let storage: iced::widget::PickList<
        '_,
        Labeled<StorageType>,
        Vec<Labeled<StorageType>>,
        Labeled<StorageType>,
        Message,
        crate::fluent::Theme,
    > = pick_list(storage_options, Some(storage_selected), |option| {
        Message::QualcommStorageTypeSelected(option.value)
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
        state.flash.qualcomm.ports.clone(),
        state.flash.qualcomm.port.clone(),
        Message::QualcommPortSelected,
    )
    .placeholder(l10n.tr("qc-port"))
    // A Windows port is a short name, but macOS and Linux offer a device node
    // (`/dev/cu.usbmodem14201`), so the width follows the longest entry.
    .width(menu_width(&state.flash.qualcomm.ports).max(PORT_MENU_WIDTH))
    .text_shaping(Shaping::Advanced);

    // The Device Manager only exists on Windows, so the button is only there.
    let mut tools = vec![
        text(l10n.tr("qc-storage-type")).size(12.0).into(),
        storage.into(),
        port.into(),
    ];

    if cfg!(windows) {
        tools.push(device_manager_button(l10n));
    }

    // [View Log] is live once a session wrote its file: it shows the log in
    // the file manager, which is where an injection's failure can be read in
    // full. Until something ran there is nothing to show.
    let view_log = if qualcomm.log_path.is_some() {
        button(text(l10n.tr("qc-view-log"))).on_press(Message::QualcommLocateLog)
    } else {
        button(text(l10n.tr("qc-view-log")))
    };

    // [Flash Info] asks the phone what its storage looks like. It needs the
    // programmer that runs in the phone, so it waits for one and for no job to
    // be under way.
    let flash_info = if qualcomm.programmer_uploaded && !qualcomm.busy {
        button(text(l10n.tr("qc-flash-info"))).on_press(Message::QualcommFlashInfo)
    } else {
        button(text(l10n.tr("qc-flash-info")))
    };

    let tool_row = split_row(vec![view_log.into(), flash_info.into()], tools);

    // What the page is doing (or how the last action went) sits right under
    // the table, so an error is next to the table it is about.
    let status_row: Option<Element<'_, Message>> = match &qualcomm.status {
        QualcommStatus::Idle => None,
        QualcommStatus::Injecting => Some(
            row![
                crate::guided::spinner(state.anim_tick),
                text(l10n.tr("qc-firehose-injecting")).size(12.0),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into(),
        ),
        QualcommStatus::Reading => Some(
            row![
                crate::guided::spinner(state.anim_tick),
                text(l10n.tr("qc-reading-partitions")).size(12.0),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into(),
        ),
        QualcommStatus::Rebooting => Some(
            row![
                crate::guided::spinner(state.anim_tick),
                text(l10n.tr("qc-rebooting")).size(12.0),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into(),
        ),
        QualcommStatus::Table { count } => Some(
            text(l10n.tr_with_args(
                "qc-partitions-found",
                &[
                    ("count", count.to_string()),
                    (
                        "storage",
                        l10n.tr(
                            qualcomm
                                .detected
                                .unwrap_or(qualcomm.storage)
                                .message_id(),
                        ),
                    ),
                ],
            ))
            .size(12.0)
            .style(bright_success)
            .into(),
        ),
        QualcommStatus::InjectFailed(error) => Some(
            text(l10n.tr_with_args("qc-firehose-failed", &[("error", error.clone())]))
                .size(12.0)
                .style(iced::widget::text::danger)
                .into(),
        ),
        QualcommStatus::InjectCancelled => Some(
            text(l10n.tr("qc-inject-cancelled"))
                .size(12.0)
                .into(),
        ),
        QualcommStatus::XmlFlashDone { count } => Some(
            text(l10n.tr_with_args(
                "qc-xml-done",
                &[("count", count.to_string())],
            ))
            .size(12.0)
            .style(bright_success)
            .into(),
        ),
        QualcommStatus::XmlFlashFailed(error) => Some(
            text(l10n.tr_with_args("qc-xml-failed", &[("error", error.clone())]))
                .size(12.0)
                .style(iced::widget::text::danger)
                .into(),
        ),
        QualcommStatus::Rebooted => Some(
            text(l10n.tr("qc-reboot-sent"))
                .size(12.0)
                .style(bright_success)
                .into(),
        ),
        QualcommStatus::PowerOffSent => Some(
            text(l10n.tr("qc-power-off-sent"))
                .size(12.0)
                .style(bright_success)
                .into(),
        ),
        QualcommStatus::PowerOffFailed(error) => Some(
            text(l10n.tr_with_args(
                "qc-power-off-failed",
                &[("error", error.clone())],
            ))
            .size(12.0)
            .style(iced::widget::text::danger)
            .into(),
        ),
        QualcommStatus::RebootFailed(error) => Some(
            text(l10n.tr_with_args("qc-reboot-failed", &[("error", error.clone())]))
                .size(12.0)
                .style(iced::widget::text::danger)
                .into(),
        ),
        QualcommStatus::Dumping { name, done, total } => Some(
            row![
                crate::guided::spinner(state.anim_tick),
                text(l10n.tr_with_args(
                    "qc-dumping",
                    &[
                        ("name", name.clone()),
                        ("percent", percent(*done, *total)),
                    ],
                ))
                .size(12.0),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into(),
        ),
        QualcommStatus::Writing { name, done, total } => Some(
            row![
                crate::guided::spinner(state.anim_tick),
                text(l10n.tr_with_args(
                    "qc-writing",
                    &[
                        ("name", name.clone()),
                        ("percent", percent(*done, *total)),
                    ],
                ))
                .size(12.0),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into(),
        ),
        QualcommStatus::Erasing { name, done, total } => Some(
            row![
                crate::guided::spinner(state.anim_tick),
                text(l10n.tr_with_args(
                    "qc-erasing",
                    &[
                        ("name", name.clone()),
                        ("percent", percent(*done, *total)),
                    ],
                ))
                .size(12.0),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into(),
        ),
        QualcommStatus::EraseDone { count } => Some(
            text(l10n.tr_with_args(
                "qc-erase-done",
                &[("count", count.to_string())],
            ))
            .size(12.0)
            .style(bright_success)
            .into(),
        ),
        QualcommStatus::EraseFailed(error) => Some(
            text(l10n.tr_with_args("qc-erase-failed", &[("error", error.clone())]))
                .size(12.0)
                .style(iced::widget::text::danger)
                .into(),
        ),
        QualcommStatus::Refreshing => Some(
            row![
                crate::guided::spinner(state.anim_tick),
                text(l10n.tr("qc-reading-partitions")).size(12.0),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into(),
        ),
        QualcommStatus::RefreshFailed(error) => Some(
            text(l10n.tr_with_args(
                "qc-refresh-failed",
                &[("error", error.clone())],
            ))
            .size(12.0)
            .style(iced::widget::text::danger)
            .into(),
        ),
        QualcommStatus::DumpDone { count, directory } => Some(
            text(l10n.tr_with_args(
                "qc-dump-done",
                &[
                    ("count", count.to_string()),
                    ("path", directory.display().to_string()),
                ],
            ))
            .size(12.0)
            .style(bright_success)
            .into(),
        ),
        QualcommStatus::DumpFailed(error) => Some(
            text(l10n.tr_with_args("qc-dump-failed", &[("error", error.clone())]))
                .size(12.0)
                .style(iced::widget::text::danger)
                .into(),
        ),
        QualcommStatus::StorageDumpDone { count, path } => Some(
            text(l10n.tr_with_args(
                "qc-storage-dump-done",
                &[
                    ("count", count.to_string()),
                    ("path", path.display().to_string()),
                ],
            ))
            .size(12.0)
            .style(bright_success)
            .into(),
        ),
        QualcommStatus::StorageDumpFailed(error) => Some(
            text(l10n.tr_with_args(
                "qc-storage-dump-failed",
                &[("error", error.clone())],
            ))
            .size(12.0)
            .style(iced::widget::text::danger)
            .into(),
        ),
        QualcommStatus::StorageFlashDone { count } => Some(
            text(l10n.tr_with_args(
                "qc-storage-flash-done",
                &[("count", count.to_string())],
            ))
            .size(12.0)
            .style(bright_success)
            .into(),
        ),
        QualcommStatus::StorageFlashFailed(error) => Some(
            text(l10n.tr_with_args(
                "qc-storage-flash-failed",
                &[("error", error.clone())],
            ))
            .size(12.0)
            .style(iced::widget::text::danger)
            .into(),
        ),
        QualcommStatus::BackupDone { count } => Some(
            text(l10n.tr_with_args(
                "qc-backup-done",
                &[("count", count.to_string())],
            ))
            .size(12.0)
            .style(bright_success)
            .into(),
        ),
        QualcommStatus::BackupFailed(error) => Some(
            text(l10n.tr_with_args("qc-backup-failed", &[("error", error.clone())]))
                .size(12.0)
                .style(iced::widget::text::danger)
                .into(),
        ),
        QualcommStatus::WriteDone(name) => Some(
            text(l10n.tr_with_args("qc-write-done", &[("name", name.clone())]))
                .size(12.0)
                .style(bright_success)
                .into(),
        ),
        QualcommStatus::WriteFailed(error) => Some(
            text(l10n.tr_with_args("qc-write-failed", &[("error", error.clone())]))
                .size(12.0)
                .style(iced::widget::text::danger)
                .into(),
        ),
    };

    let mut layout: Vec<Element<'_, Message>> = vec![
        top,
        horizontal_rule(1).into(),
        actions,
        horizontal_rule(1).into(),
        table.into(),
    ];

    // The job's bar sits right under the table, above the line that names the
    // partition it is on: it shows the whole job, which is what a dump of
    // several partitions is.
    if let Some(progress) = &qualcomm.progress {
        layout.push(
            progress_bar(0.0..=1.0, progress.fraction())
                .girth(BAR_HEIGHT)
                .length(Fill)
                .into(),
        );
    }

    if let Some(status_row) = status_row {
        layout.push(status_row);
    }

    layout.push(horizontal_rule(1).into());
    layout.push(search_row.into());
    layout.push(storage_row.into());
    layout.push(tool_row);

    let content = Column::with_children(layout)
        .spacing(8)
        .width(Fill)
        .height(Fill)
        .align_x(Alignment::Start);

    let page: Element<'_, Message> = container(content)
        .width(Fill)
        .height(Fill)
        .padding(16)
        .into();

    page
}

/// Whether the "close LMN Flash?" prompt is up.
///
/// It stands in for a window close request that arrived while a Firehose
/// session was live — a running programmer inside the phone is not something a
/// stray click on the title bar should end without a word.
pub(crate) fn close_prompt_open(state: &State) -> bool {
    state.flash.qualcomm.close_prompt.is_some()
}

/// The prompt a window close request opens while a Firehose session is live, or
/// `None` while there is no request to answer.
pub(crate) fn close_prompt_overlay(state: &State) -> Option<Element<'_, Message>> {
    if !close_prompt_open(state) {
        return None;
    }

    let l10n = &state.l10n;

    let content = column![
        text(l10n.tr("qc-close-title")).size(18.0),
        text(l10n.tr("qc-close-body"))
            .size(13.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph),
        horizontal_rule(1),
        row![
            button(text(l10n.tr("qc-close-confirm")))
                .width(Fill)
                .on_press(Message::QualcommCloseConfirmed),
            button(text(l10n.tr("login-cancel")))
                .width(Fill)
                .on_press(Message::QualcommClosePromptCancelled),
        ]
        .spacing(8)
        .width(Fill),
    ]
    .spacing(10)
    .width(Fill);

    let card: Element<'_, Message> = container(container(content).padding(16).width(Fill))
        .width(460)
        .style(darker_card)
        .into();

    let children: Vec<Element<'_, Message>> = vec![
        // The click is swallowed rather than taken for an answer: the prompt
        // is left through its own two buttons.
        mouse_area(Space::new(Fill, Fill))
            .on_press(Message::QualcommClosePromptBackdropPressed)
            .into(),
        container(card)
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill)
            .into(),
    ];

    Some(iced::widget::Stack::with_children(children).into())
}

/// Which card the storage-flash dialog shows, as the low nibble of its overlay
/// key, or `None` while it is closed.
pub(crate) fn storage_flash_card_step(state: &State) -> Option<u8> {
    Some(match state.flash.qualcomm.storage_flash.as_ref()? {
        StorageFlashDialog::Reading => 0,
        StorageFlashDialog::Ready { .. } => 1,
        StorageFlashDialog::Failed(_) => 2,
    })
}

/// The "Flash Entire Storage" card, or `None` while it is closed.
pub(crate) fn storage_flash_overlay(state: &State) -> Option<Element<'_, Message>> {
    let dialog = state.flash.qualcomm.storage_flash.as_ref()?;

    let children: Vec<Element<'_, Message>> = vec![
        mouse_area(Space::new(Fill, Fill))
            .on_press(Message::QualcommStorageFlashBackdropPressed)
            .into(),
        container(storage_flash_card(state, dialog))
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill)
            .into(),
    ];

    Some(iced::widget::Stack::with_children(children).into())
}

/// The card itself: one row per physical partition, each with the image that
/// is written to it.
fn storage_flash_card<'a>(
    state: &'a State,
    dialog: &'a StorageFlashDialog,
) -> Element<'a, Message> {
    let l10n = &state.l10n;

    let mut content = column![
        text(l10n.tr("qc-danger-flash-storage")).size(18.0),
        text(l10n.tr("qc-flash-storage-hint"))
            .size(13.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph),
        horizontal_rule(1),
    ]
    .spacing(10)
    .width(Fill);

    match dialog {
        StorageFlashDialog::Reading => {
            content = content.push(
                row![
                    crate::guided::spinner(state.anim_tick),
                    text(l10n.tr("qc-lun-reading")).size(13.0),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
        }
        StorageFlashDialog::Failed(error) => {
            content = content.push(
                text(l10n.tr_with_args("qc-lun-failed", &[("error", error.clone())]))
                    .size(13.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph)
                    .style(iced::widget::text::danger),
            );
            content = content.push(horizontal_rule(1));
            content = content.push(
                button(text(l10n.tr("login-cancel")))
                    .width(Fill)
                    .on_press(Message::QualcommStorageFlashDialogClosed),
            );
        }
        StorageFlashDialog::Ready {
            luns,
            files,
            assigned,
        } => {
            // One image per LUN for a storage with a single physical partition;
            // a UFS phone takes one per LUN it brings images for.
            let multiple = luns.len() > 1;
            let pick = button(text(l10n.tr(if multiple {
                "qc-flash-storage-pick-many"
            } else {
                "qc-flash-storage-pick-one"
            })))
            .on_press(Message::QualcommStorageFlashPickRequested);
            content = content.push(pick);

            let none_label = l10n.tr("qc-flash-storage-none");
            let mut rows: Vec<Element<'a, Message>> = Vec::new();
            let mut fits = 0_usize;
            let mut broken = false;

            for (row, lun) in luns.iter().enumerate() {
                let chosen = assigned.get(row).copied().flatten();
                let chosen = chosen.filter(|index| *index < files.len());

                // An image that is already written to another LUN is left out
                // of this row's choices: one file, one partition.
                let mut options: Vec<Labeled<Option<usize>>> = vec![Labeled {
                    value: None,
                    label: none_label.clone(),
                }];
                for (index, file) in files.iter().enumerate() {
                    let taken = assigned
                        .iter()
                        .enumerate()
                        .any(|(other, choice)| other != row && *choice == Some(index));

                    if taken {
                        continue;
                    }

                    options.push(Labeled {
                        value: Some(index),
                        label: file_name(&file.path),
                    });
                }

                let selected = Labeled {
                    value: chosen,
                    label: chosen.map_or_else(
                        || none_label.clone(),
                        |index| file_name(&files[index].path),
                    ),
                };

                let menu: iced::widget::PickList<
                    'a,
                    Labeled<Option<usize>>,
                    Vec<Labeled<Option<usize>>>,
                    Labeled<Option<usize>>,
                    Message,
                    crate::fluent::Theme,
                > = pick_list(options, Some(selected), move |option| {
                    Message::QualcommStorageFlashAssigned(row, option.value)
                })
                .placeholder(none_label.clone())
                .width(STORAGE_FLASH_MENU_WIDTH)
                .text_shaping(Shaping::Advanced);

                let mut line = column![
                    row![
                        text(l10n.tr_with_args(
                            "qc-lun-entry",
                            &[
                                ("lun", lun.number.to_string()),
                                ("bytes", grouped_bytes(lun.bytes())),
                                ("size", human_size_exact(lun.bytes())),
                            ],
                        ))
                        .size(12.0)
                        .width(Fill),
                        menu,
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .width(Fill),
                ]
                .spacing(2)
                .width(Fill);

                if let Some(file) = chosen.and_then(|index| files.get(index)) {
                    match ImageFit::of(file.length, lun.bytes()) {
                        ImageFit::Exact => fits += 1,
                        ImageFit::Smaller(difference) => {
                            fits += 1;
                            line = line.push(
                                text(l10n.tr_with_args(
                                    "qc-flash-storage-smaller",
                                    &[("size", human_size(difference))],
                                ))
                                .size(11.0)
                                .width(Fill)
                                .wrapping(Wrapping::WordOrGlyph),
                            );
                        }
                        ImageFit::Larger(difference) => {
                            broken = true;
                            line = line.push(
                                text(l10n.tr_with_args(
                                    "qc-flash-storage-larger",
                                    &[("size", human_size(difference))],
                                ))
                                .size(11.0)
                                .width(Fill)
                                .wrapping(Wrapping::WordOrGlyph)
                                .style(iced::widget::text::danger),
                            );
                        }
                    }
                }

                rows.push(line.into());
            }

            content = content.push(
                container(
                    scrollable(
                        iced::widget::Column::with_children(rows)
                            .spacing(8)
                            .align_x(Alignment::Start),
                    )
                    .id(iced::widget::Id::new(STORAGE_FLASH_SCROLL_ID))
                    .width(Fill)
                    .height(STORAGE_FLASH_LIST_HEIGHT),
                )
                .padding(6)
                .width(Fill)
                .style(container::rounded_box),
            );

            if fits == 0 {
                content = content.push(text(l10n.tr("qc-flash-storage-nothing")).size(12.0));
            }

            content = content.push(horizontal_rule(1));

            // A row whose image does not fit shuts the button: writing part of
            // it would spill into whatever the phone keeps after the LUN.
            let flash = button(text(l10n.tr("qc-flash-storage-go"))).width(Fill);
            let flash: Element<'a, Message> = if fits == 0 || broken {
                flash.into()
            } else {
                flash.on_press(Message::QualcommStorageFlashConfirmed).into()
            };

            content = content.push(
                row![
                    flash,
                    button(text(l10n.tr("login-cancel")))
                        .width(Fill)
                        .on_press(Message::QualcommStorageFlashDialogClosed),
                ]
                .spacing(8)
                .width(Fill),
            );
        }
    }

    container(
        scrollable(container(content).padding(16).width(Fill))
            .width(Fill)
            .height(Fill),
    )
    .width(560)
    .height(440)
    .style(darker_card)
    .into()
}

/// Which card the "Flash Info" dialog shows, as the low nibble of its overlay
/// key, or `None` while it is closed.
pub(crate) fn flash_info_card_step(state: &State) -> Option<u8> {
    Some(match state.flash.qualcomm.flash_info.as_ref()? {
        FlashInfoDialog::Reading => 0,
        FlashInfoDialog::Ready { .. } => 1,
        FlashInfoDialog::Failed(_) => 2,
    })
}

/// The "Flash Info" card, or `None` while it is closed.
pub(crate) fn flash_info_overlay(state: &State) -> Option<Element<'_, Message>> {
    let dialog = state.flash.qualcomm.flash_info.as_ref()?;

    let children: Vec<Element<'_, Message>> = vec![
        mouse_area(Space::new(Fill, Fill))
            .on_press(Message::QualcommFlashInfoBackdropPressed)
            .into(),
        container(flash_info_card(state, dialog))
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill)
            .into(),
    ];

    Some(iced::widget::Stack::with_children(children).into())
}

/// The card itself: what the phone reports, read out, with its own XML a
/// folded panel below it.
///
/// The facts are the lines the phone named — one row per label — and the raw
/// XML is written in the monospace face the page's offsets use, so it lines up
/// the way the phone sent it. It stays folded: it is long, and the card's button
/// copies it whole when it is wanted somewhere else.
fn flash_info_card<'a>(state: &'a State, dialog: &'a FlashInfoDialog) -> Element<'a, Message> {
    let l10n = &state.l10n;

    let mut content = column![
        text(l10n.tr("qc-flash-info")).size(18.0),
        horizontal_rule(1),
    ]
    .spacing(10)
    .width(Fill);

    match dialog {
        FlashInfoDialog::Reading => {
            content = content.push(
                row![
                    crate::guided::spinner(state.anim_tick),
                    text(l10n.tr("qc-lun-reading")).size(13.0),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
        }
        FlashInfoDialog::Failed(error) => {
            content = content.push(
                text(l10n.tr_with_args("qc-lun-failed", &[("error", error.clone())]))
                    .size(13.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph)
                    .style(iced::widget::text::danger),
            );
            content = content.push(horizontal_rule(1));
            content = content.push(
                button(text(l10n.tr("qc-flash-info-done")))
                    .width(Fill)
                    .on_press(Message::QualcommFlashInfoClosed),
            );
        }
        FlashInfoDialog::Ready {
            info,
            raw_open,
            copied,
        } => {
            // The facts of every LUN in one list of their own, so the buttons
            // and the raw panel's lid stay in view however much the phone
            // reports.
            let mut rows: Vec<Element<'a, Message>> = Vec::new();

            for lun in &info.luns {
                rows.push(text(format!("LUN {}", lun.number)).size(14.0).into());

                for (label, value) in &lun.facts {
                    rows.push(
                        row![
                            text(label.clone())
                                .size(12.0)
                                .width(STORAGE_INFO_LABEL_WIDTH),
                            text(value.clone())
                                .size(12.0)
                                .width(Fill)
                                .wrapping(Wrapping::WordOrGlyph),
                        ]
                        .spacing(10)
                        .align_y(Alignment::Start)
                        .width(Fill)
                        .into(),
                    );
                }
            }

            if *raw_open {
                rows.push(
                    container(
                        text(info.raw())
                            .size(12.0)
                            .font(crate::output_font())
                            .width(Fill)
                            .wrapping(Wrapping::WordOrGlyph),
                    )
                    .padding(8)
                    .width(Fill)
                    .style(container::rounded_box)
                    .into(),
                );
            }

            content = content.push(
                container(
                    scrollable(
                        iced::widget::Column::with_children(rows)
                            .spacing(3)
                            .align_x(Alignment::Start),
                    )
                    .id(iced::widget::Id::new(FLASH_INFO_SCROLL_ID))
                    .width(Fill)
                    .height(STORAGE_INFO_HEIGHT),
                )
                .padding(6)
                .width(Fill)
                .style(container::rounded_box),
            );

            // The raw answer is a panel of its own, folded until it is asked
            // for: the arrow says which way it is.
            let arrow = if *raw_open { "▾" } else { "▸" };
            content = content.push(
                button(text(format!("{arrow} {}", l10n.tr("qc-flash-info-raw"))).size(12.0))
                    .padding([4.0, 8.0])
                    .on_press(Message::QualcommFlashInfoToggleRaw),
            );

            if *copied {
                content = content.push(
                    text(l10n.tr("qc-flash-info-copied"))
                        .size(12.0)
                        .style(bright_success),
                );
            }

            content = content.push(horizontal_rule(1));

            // Two ways to take the answer away: read out, or exactly as the
            // phone sent it. Done closes the card on a line of its own, so no
            // label has to be squeezed.
            content = content.push(
                row![
                    button(text(l10n.tr("qc-flash-info-copy-parsed")))
                        .width(Fill)
                        .on_press(Message::QualcommFlashInfoCopyParsed),
                    button(text(l10n.tr("qc-flash-info-copy")))
                        .width(Fill)
                        .on_press(Message::QualcommFlashInfoCopy),
                ]
                .spacing(8)
                .width(Fill),
            );
            content = content.push(
                button(text(l10n.tr("qc-flash-info-done")))
                    .width(Fill)
                    .on_press(Message::QualcommFlashInfoClosed),
            );
        }
    }

    container(
        scrollable(container(content).padding(16).width(Fill))
            .width(Fill)
            .height(Fill),
    )
    .width(640)
    .height(480)
    .style(darker_card)
    .into()
}

/// The name of an image, without the folder it is in.
fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Whether the "Flash Partition Backup" card is up.
pub(crate) fn backup_open(state: &State) -> bool {
    state.flash.qualcomm.backup.is_some()
}

/// The "Flash Partition Backup" card, or `None` while it is closed.
pub(crate) fn backup_overlay(state: &State) -> Option<Element<'_, Message>> {
    if !backup_open(state) {
        return None;
    }

    let children: Vec<Element<'_, Message>> = vec![
        mouse_area(Space::new(Fill, Fill))
            .on_press(Message::QualcommBackupBackdropPressed)
            .into(),
        container(backup_card(state))
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill)
            .into(),
    ];

    Some(iced::widget::Stack::with_children(children).into())
}

/// The card itself: one row per picked image, with the partition it goes back
/// to.
fn backup_card<'a>(state: &'a State) -> Element<'a, Message> {
    let l10n = &state.l10n;
    let qualcomm = &state.flash.qualcomm;
    let backup = qualcomm.backup.as_ref().expect("the card is open");

    let mut content = column![
        text(l10n.tr("qc-danger-flash-backup")).size(18.0),
        text(l10n.tr("qc-backup-hint"))
            .size(13.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph),
        horizontal_rule(1),
        button(text(l10n.tr("qc-flash-storage-pick-many")))
            .on_press(Message::QualcommBackupPickRequested),
    ]
    .spacing(10)
    .width(Fill);

    let none_label = l10n.tr("qc-flash-storage-none");
    let partitions = &qualcomm.partitions;
    let mut rows: Vec<Element<'a, Message>> = Vec::new();
    let mut assigned = 0_usize;
    let mut fits = 0_usize;
    let mut broken = false;

    for (row, image) in backup.files.iter().enumerate() {
        let chosen = backup.assigned.get(row).copied().flatten();
        let chosen = chosen.filter(|index| *index < partitions.len());

        // A partition an image is already assigned to is left out of this
        // row's choices: one file, one partition.
        let mut options: Vec<Labeled<Option<usize>>> = vec![Labeled {
            value: None,
            label: none_label.clone(),
        }];
        for (index, partition) in partitions.iter().enumerate() {
            let taken = backup
                .assigned
                .iter()
                .enumerate()
                .any(|(other, choice)| other != row && *choice == Some(index));

            if taken {
                continue;
            }

            options.push(Labeled {
                value: Some(index),
                label: partition.name.clone(),
            });
        }

        let selected = Labeled {
            value: chosen,
            label: chosen.map_or_else(
                || none_label.clone(),
                |index| partitions[index].name.clone(),
            ),
        };

        let menu: iced::widget::PickList<
            'a,
            Labeled<Option<usize>>,
            Vec<Labeled<Option<usize>>>,
            Labeled<Option<usize>>,
            Message,
            crate::fluent::Theme,
        > = pick_list(options, Some(selected), move |option| {
            Message::QualcommBackupAssigned(row, option.value)
        })
        .placeholder(none_label.clone())
        .width(BACKUP_MENU_WIDTH)
        .text_shaping(Shaping::Advanced);

        let mut line = column![
            row![
                text(file_name(&image.path)).size(12.0).width(Fill),
                menu,
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .width(Fill),
        ]
        .spacing(2)
        .width(Fill);

        if let Some(partition) = chosen.and_then(|index| partitions.get(index)) {
            assigned += 1;
            match ImageFit::of(image.length, partition.size_bytes()) {
                ImageFit::Exact => fits += 1,
                ImageFit::Smaller(difference) => {
                    fits += 1;
                    line = line.push(
                        text(l10n.tr_with_args(
                            "qc-backup-smaller",
                            &[("size", human_size(difference))],
                        ))
                        .size(11.0)
                        .width(Fill)
                        .wrapping(Wrapping::WordOrGlyph),
                    );
                }
                ImageFit::Larger(difference) => {
                    broken = true;
                    line = line.push(
                        text(l10n.tr_with_args(
                            "qc-backup-larger",
                            &[("size", human_size(difference))],
                        ))
                        .size(11.0)
                        .width(Fill)
                        .wrapping(Wrapping::WordOrGlyph)
                        .style(iced::widget::text::danger),
                    );
                }
            }
        }

        rows.push(line.into());
    }

    content = content.push(
        container(
            scrollable(
                iced::widget::Column::with_children(rows)
                    .spacing(8)
                    .align_x(Alignment::Start),
            )
            .id(iced::widget::Id::new(BACKUP_SCROLL_ID))
            .width(Fill)
            .height(STORAGE_FLASH_LIST_HEIGHT),
        )
        .padding(6)
        .width(Fill)
        .style(container::rounded_box),
    );

    if assigned == 0 {
        content = content.push(text(l10n.tr("qc-backup-nothing")).size(12.0));
    }

    content = content.push(horizontal_rule(1));

    // An image that does not fit its partition shuts the button: the extra
    // sectors would be written over the next partition.
    let flash = button(text(l10n.tr("qc-flash-storage-go"))).width(Fill);
    let flash: Element<'a, Message> = if fits == 0 || broken {
        flash.into()
    } else {
        flash.on_press(Message::QualcommBackupConfirmed).into()
    };

    content = content.push(
        row![
            flash,
            button(text(l10n.tr("login-cancel")))
                .width(Fill)
                .on_press(Message::QualcommBackupClosed),
        ]
        .spacing(8)
        .width(Fill),
    );

    container(
        scrollable(container(content).padding(16).width(Fill))
            .width(Fill)
            .height(Fill),
    )
    .width(620)
    .height(480)
    .style(darker_card)
    .into()
}

/// Whether the "Check the phone's identity" card is up.
pub(crate) fn inject_check_open(state: &State) -> bool {
    state
        .flash
        .qualcomm
        .injection
        .as_ref()
        .is_some_and(|wait| wait.preflight.is_some() && !wait.reselecting)
}

/// The "Check the phone's identity" card, or `None` while it is closed.
pub(crate) fn inject_check_overlay(state: &State) -> Option<Element<'_, Message>> {
    if !inject_check_open(state) {
        return None;
    }

    let children: Vec<Element<'_, Message>> = vec![
        mouse_area(Space::new(Fill, Fill))
            .on_press(Message::QualcommInjectCheckBackdropPressed)
            .into(),
        container(inject_check_card(state))
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill)
            .into(),
    ];

    Some(iced::widget::Stack::with_children(children).into())
}

/// The card an injection stops at: the identity the phone's boot ROM reported,
/// and the programmer waiting behind it.
///
/// The values are what a programmer has to be built for, so they are shown in
/// the same monospace face the page prints offsets in, one per row, with the
/// programmer's own name under them. The last of them is the key hash, and
/// below it the card says whether the programmer waiting behind it is signed
/// against that same root key — which it has to be to run at all.
fn inject_check_card<'a>(state: &'a State) -> Element<'a, Message> {
    let l10n = &state.l10n;
    let wait = state
        .flash
        .qualcomm
        .injection
        .as_ref()
        .expect("the card is open");
    let preflight = wait.preflight.as_ref().expect("the card is open");
    let identity = &preflight.identity;

    let mut rows = column![].spacing(6).width(Fill);
    for (label, value) in [
        ("HW_ID", identity.hw_id_text()),
        ("MSM_ID", identity.msm_id_text()),
        ("OEM_ID", identity.oem_id_text()),
        ("MODEL_ID", identity.model_id_text()),
        // A boot ROM that keeps the key hash back leaves a dash rather than a
        // row that is not there: the four ids above are still worth reading.
        (
            "PK_HASH",
            identity.pk_hash_text().unwrap_or_else(|| "—".to_owned()),
        ),
    ] {
        rows = rows.push(
            row![
                container(text(label).size(13.0))
                    .width(INJECT_ID_LABEL_WIDTH)
                    .align_y(Alignment::Center),
                // The key hash is sixty-four characters, so it is broken in two
                // halves; the four ids above it are short enough to stay on one
                // line.
                {
                    let characters: Vec<char> = value.chars().collect();
                    let mut lines = column![].spacing(2).width(Fill);
                    for chunk in characters.chunks(INJECT_HASH_CHUNK) {
                        lines = lines.push(
                            text(chunk.iter().collect::<String>())
                                .size(13.0)
                                .font(crate::output_font()),
                        );
                    }
                    lines
                },
            ]
            .spacing(8)
            .width(Fill),
        );
    }

    container(
        column![
            text(l10n.tr("qc-inject-check-title")).size(18.0),
            horizontal_rule(1),
            text(l10n.tr("qc-inject-check-hint"))
                .size(13.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph),
            container(rows)
                .padding(12)
                .width(Fill)
                .style(container::rounded_box),
            // Whether the programmer belongs to this phone: the one thing the
            // card exists for.
            root_comparison(l10n, preflight),
            text(l10n.tr_with_args(
                "qc-inject-check-programmer",
                &[("name", file_name(&preflight.programmer))],
            ))
            .size(12.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph),
            horizontal_rule(1),
            row![
                button(text(l10n.tr("qc-inject-check-continue")))
                    .width(Fill)
                    .on_press(Message::QualcommInjectAnswered(true)),
                button(text(l10n.tr("qc-inject-check-reselect")))
                    .width(Fill)
                    .on_press(Message::QualcommInjectReselect),
                button(text(l10n.tr("qc-inject-check-cancel")))
                    .width(Fill)
                    .on_press(Message::QualcommInjectAnswered(false)),
            ]
            .spacing(8)
            .width(Fill),
        ]
        .spacing(10)
        .width(Fill),
    )
    .padding(16)
    .width(480)
    .style(darker_card)
    .into()
}

/// The line that says whether the programmer is signed against the root key the
/// phone is fused for.
///
/// The programmer's own hash is only spelled out when the two differ or could
/// not be compared: when they match it says exactly what the `PK_HASH` row
/// above already shows. A phone that reports one of Qualcomm's test root keys
/// is unfused — secure boot is off — so nothing is checked against the
/// programmer and the two hashes may well differ.
fn root_comparison<'a>(
    l10n: &'a crate::l10n::Bundle,
    preflight: &crate::firehose::Preflight,
) -> Element<'a, Message> {
    if preflight.unfused() {
        return text(l10n.tr("qc-inject-check-unfused"))
            .size(12.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph)
            .into();
    }

    match preflight.roots_match() {
        Some(true) => text(l10n.tr("qc-inject-check-match"))
            .size(12.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph)
            .style(bright_success)
            .into(),
        Some(false) => {
            let hash = preflight
                .programmer_hash_text()
                .unwrap_or_else(|| "—".to_owned());

            text(l10n.tr_with_args(
                "qc-inject-check-mismatch",
                &[("hash", hash)],
            ))
            .size(12.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph)
            .style(iced::widget::text::danger)
            .into()
        }
        // Nothing to compare: either the boot ROM kept its hash back, or the
        // programmer's chain could not be read. The second says why.
        None => match &preflight.programmer_hash {
            Ok(_) => Element::from(Space::new(Fill, 0)),
            Err(error) => text(l10n.tr_with_args(
                "qc-inject-check-unread",
                &[("error", error.clone())],
            ))
            .size(12.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph)
            .into(),
        },
    }
}

/// Which card the "Flash XML" dialog shows, as the low nibble of its overlay
/// key, or `None` while it is closed.
pub(crate) fn xml_flash_card_step(state: &State) -> Option<u8> {
    Some(match state.flash.qualcomm.xml_flash.as_ref()? {
        // Asking what kind of package it is, and the picker that follows it,
        // are the same card: the picker is a window of its own.
        XmlFlashDialog::Choosing | XmlFlashDialog::Picking(_) => 0,
        XmlFlashDialog::Ready(_) => 1,
        XmlFlashDialog::Failed(_) => 2,
    })
}

/// The "Flash XML" card, or `None` while it is closed.
pub(crate) fn xml_flash_overlay(state: &State) -> Option<Element<'_, Message>> {
    let dialog = state.flash.qualcomm.xml_flash.as_ref()?;

    let children: Vec<Element<'_, Message>> = vec![
        mouse_area(Space::new(Fill, Fill))
            .on_press(Message::QualcommXmlBackdropPressed)
            .into(),
        container(xml_flash_card(state, dialog))
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill)
            .into(),
    ];

    Some(iced::widget::Stack::with_children(children).into())
}

/// The card itself: the choice of build, the checklist a package turns into, or
/// why the files could not be read.
fn xml_flash_card<'a>(state: &'a State, dialog: &'a XmlFlashDialog) -> Element<'a, Message> {
    let l10n = &state.l10n;

    let mut content = column![
        text(l10n.tr("qc-xml-title")).size(18.0),
        horizontal_rule(1),
    ]
    .spacing(10)
    .width(Fill);

    match dialog {
        XmlFlashDialog::Choosing | XmlFlashDialog::Picking(_) => {
            content = content.push(
                text(l10n.tr("qc-xml-choose"))
                    .size(13.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph),
            );
            content = content.push(
                button(text(l10n.tr("qc-xml-meta")))
                    .width(Fill)
                    .on_press(Message::QualcommXmlBuildChosen(crate::qfil::Build::Meta)),
            );
            content = content.push(
                button(text(l10n.tr("qc-xml-flat")))
                    .width(Fill)
                    .on_press(Message::QualcommXmlBuildChosen(crate::qfil::Build::Flat)),
            );
            content = content.push(horizontal_rule(1));
            content = content.push(
                button(text(l10n.tr("login-cancel")))
                    .width(Fill)
                    .on_press(Message::QualcommXmlClosed),
            );
        }
        XmlFlashDialog::Ready(plan) => {
            content = content.push(
                text(l10n.tr("qc-xml-hint"))
                    .size(12.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph),
            );

            // The programmer the package names, when it is a meta build: the
            // page flashes with the one it has running, so this says which.
            if let Some(programmer) = &plan.plan.programmer {
                content = content.push(
                    text(l10n.tr_with_args(
                        "qc-xml-programmer",
                        &[("name", file_name(programmer))],
                    ))
                    .size(12.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph),
                );
            }

            let mut rows: Vec<Element<'a, Message>> = Vec::new();
            for (index, entry) in plan.plan.entries.iter().enumerate() {
                let chosen = plan.chosen.get(index).copied().unwrap_or(false);

                let mut line = column![
                    row![
                        checkbox(entry.label.clone(), chosen)
                            .on_toggle(move |_| Message::QualcommXmlRowToggled(index)),
                        text(entry.file.clone())
                            .size(12.0)
                            .width(Fill)
                            .wrapping(Wrapping::WordOrGlyph),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .width(Fill),
                ]
                .spacing(2)
                .width(Fill);

                if !entry.exists {
                    line = line.push(
                        text(l10n.tr_with_args(
                            "qc-xml-no-file",
                            &[("name", entry.file.clone())],
                        ))
                        .size(11.0)
                        .width(Fill)
                        .wrapping(Wrapping::WordOrGlyph)
                        .style(iced::widget::text::danger),
                    );
                } else if entry.sparse {
                    line = line.push(
                        text(l10n.tr("qc-xml-sparse"))
                            .size(11.0)
                            .width(Fill)
                            .wrapping(Wrapping::WordOrGlyph),
                    );
                }

                rows.push(line.into());
            }

            content = content.push(
                container(
                    scrollable(
                        iced::widget::Column::with_children(rows)
                            .spacing(8)
                            .align_x(Alignment::Start),
                    )
                    .id(iced::widget::Id::new(XML_FLASH_SCROLL_ID))
                    .width(Fill)
                    .height(XML_FLASH_LIST_HEIGHT),
                )
                .padding(6)
                .width(Fill)
                .style(container::rounded_box),
            );

            let missing = plan.plan.missing();
            if !missing.is_empty() {
                content = content.push(
                    text(l10n.tr_with_args(
                        "qc-xml-missing",
                        &[
                            ("count", missing.len().to_string()),
                            ("files", missing.join(", ")),
                        ],
                    ))
                    .size(11.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph)
                    .style(iced::widget::text::danger),
                );
            }

            // [After Flashing ▾]: what the phone does once the package is
            // down. The packages end differently by themselves, and the choice
            // stays for the next package as well.
            let after_options: Vec<Labeled<AfterFlash>> = AfterFlash::ALL
                .iter()
                .map(|&after| Labeled {
                    value: after,
                    label: l10n.tr(after.message_id()),
                })
                .collect();
            let after_selected = Labeled {
                value: state.flash.qualcomm.after_flash,
                label: l10n.tr(state.flash.qualcomm.after_flash.message_id()),
            };
            let after: iced::widget::PickList<
                'a,
                Labeled<AfterFlash>,
                Vec<Labeled<AfterFlash>>,
                Labeled<AfterFlash>,
                Message,
                crate::fluent::Theme,
            > = pick_list(after_options, Some(after_selected), |option| {
                Message::QualcommXmlAfterFlashSelected(option.value)
            })
            .width(AFTER_FLASH_MENU_WIDTH)
            .text_shaping(Shaping::Advanced);

            content = content.push(
                row![
                    text(l10n.tr("qc-xml-after")).size(12.0).width(Fill),
                    after,
                ]
                .spacing(8)
                .align_y(Alignment::Center)
                .width(Fill),
            );

            content = content.push(horizontal_rule(1));

            let flash = button(text(l10n.tr("qc-xml-go"))).width(Fill);
            let flash: Element<'a, Message> = if plan.kept() == 0 {
                flash.into()
            } else {
                flash.on_press(Message::QualcommXmlConfirmed).into()
            };

            content = content.push(
                row![
                    flash,
                    button(text(l10n.tr("login-cancel")))
                        .width(Fill)
                        .on_press(Message::QualcommXmlClosed),
                ]
                .spacing(8)
                .width(Fill),
            );
        }
        XmlFlashDialog::Failed(error) => {
            content = content.push(
                text(error.clone())
                    .size(13.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph)
                    .style(iced::widget::text::danger),
            );
            content = content.push(horizontal_rule(1));
            content = content.push(
                button(text(l10n.tr("login-cancel")))
                    .width(Fill)
                    .on_press(Message::QualcommXmlClosed),
            );
        }
    }

    container(
        scrollable(container(content).padding(16).width(Fill))
            .width(Fill)
            .height(Fill),
    )
    .width(560)
    .height(440)
    .style(darker_card)
    .into()
}

/// Which card the LUN picker shows, as the low nibble of its overlay key, or
/// `None` while no picker is open.
///
/// The three cards — asking the phone, its answer, and a failed question — are
/// steps of one dialog, so the page's transitions slide from one to the next.
pub(crate) fn lun_card_step(state: &State) -> Option<u8> {
    Some(match state.flash.qualcomm.lun_dialog.as_ref()? {
        LunDialog::Reading => 0,
        LunDialog::Luns { .. } => 1,
        LunDialog::Failed(_) => 2,
    })
}

/// The LUN picker "Dump Entire Storage" opens, or `None` while it is closed.
///
/// It is the layer the app's dialogs are drawn with: a backdrop that swallows
/// clicks on the empty space, with the card centred over it. The dimming scrim
/// behind it and the way it eases in and out belong to the window's dialog
/// animation ([`crate::animated_overlay`] in the view).
///
/// The card is left through its own buttons only: a click beside it is
/// swallowed rather than taken for a dismissal, so the LUNs that were checked
/// are not lost by a stray click.
pub(crate) fn lun_overlay(state: &State) -> Option<Element<'_, Message>> {
    let dialog = state.flash.qualcomm.lun_dialog.as_ref()?;

    let children: Vec<Element<'_, Message>> = vec![
        mouse_area(Space::new(Fill, Fill))
            .on_press(Message::QualcommLunBackdropPressed)
            .into(),
        container(lun_card(state, dialog))
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill)
            .into(),
    ];

    Some(iced::widget::Stack::with_children(children).into())
}

/// The picker's card: what the storage is split into, one checkbox per LUN.
fn lun_card<'a>(state: &'a State, dialog: &'a LunDialog) -> Element<'a, Message> {
    let l10n = &state.l10n;

    let mut content = column![
        text(l10n.tr("qc-dump-entire-storage")).size(18.0),
        text(l10n.tr("qc-lun-hint"))
            .size(13.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph),
        horizontal_rule(1),
    ]
    .spacing(10)
    .width(Fill);

    match dialog {
        LunDialog::Reading => {
            content = content.push(
                row![
                    crate::guided::spinner(state.anim_tick),
                    text(l10n.tr("qc-lun-reading")).size(13.0),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
        }
        LunDialog::Failed(error) => {
            content = content.push(
                text(l10n.tr_with_args("qc-lun-failed", &[("error", error.clone())]))
                    .size(13.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph)
                    .style(iced::widget::text::danger),
            );
            content = content.push(horizontal_rule(1));
            content = content.push(
                button(text(l10n.tr("login-cancel")))
                    .width(Fill)
                    .on_press(Message::QualcommLunDialogClosed),
            );
        }
        LunDialog::Luns { luns, checked } => {
            let controls = Row::with_children(vec![
                button(text(l10n.tr("firmware-flash-select-all")).size(12.0))
                    .padding([4.0, 8.0])
                    .on_press(Message::QualcommLunsAll(true))
                    .into(),
                button(text(l10n.tr("firmware-flash-select-none")).size(12.0))
                    .padding([4.0, 8.0])
                    .on_press(Message::QualcommLunsAll(false))
                    .into(),
            ])
            .spacing(6)
            .align_y(Alignment::Center)
            .wrap();
            content = content.push(controls);

            let rows: Vec<Element<'a, Message>> = luns
                .iter()
                .enumerate()
                .map(|(index, lun)| -> Element<'a, Message> {
                    // The exact size first: two LUNs can round to the same
                    // readable one, and the byte count says which is which.
                    checkbox(
                        l10n.tr_with_args(
                            "qc-lun-entry",
                            &[
                                ("lun", lun.number.to_string()),
                                ("bytes", grouped_bytes(lun.bytes())),
                                ("size", human_size_exact(lun.bytes())),
                            ],
                        ),
                        checked.get(index).copied().unwrap_or(false),
                    )
                    .text_shaping(Shaping::Advanced)
                    .on_toggle(move |checked| Message::QualcommLunToggled(index, checked))
                    .into()
                })
                .collect();

            // Its own scrollable, so a phone with many LUNs does not push the
            // buttons out of the card; the id keeps its position across
            // redraws.
            content = content.push(
                container(
                    scrollable(
                        iced::widget::Column::with_children(rows)
                            .spacing(4)
                            .align_x(Alignment::Start),
                    )
                    .id(iced::widget::Id::new(LUN_SCROLL_ID))
                    .width(Fill)
                    .height(180),
                )
                .padding(6)
                .width(Fill)
                .style(container::rounded_box),
            );

            let chosen = checked.iter().filter(|checked| **checked).count();
            if chosen == 0 {
                // A dump of nothing would do nothing, so the button stays shut
                // while the list is empty of choices.
                content = content.push(text(l10n.tr("qc-lun-none")).size(12.0));
            }

            content = content.push(horizontal_rule(1));

            let dump = button(text(l10n.tr("qc-dump"))).width(Fill);
            let dump: Element<'a, Message> = if chosen == 0 {
                dump.into()
            } else {
                dump.on_press(Message::QualcommLunsConfirmed).into()
            };

            content = content.push(
                row![
                    dump,
                    button(text(l10n.tr("login-cancel")))
                        .width(Fill)
                        .on_press(Message::QualcommLunDialogClosed),
                ]
                .spacing(8)
                .width(Fill),
            );
        }
    }

    // The padding sits inside the scrollable, so the card's border is the
    // viewport: the scrollbar rides on the right border instead of floating in
    // the middle of the padding.
    container(
        scrollable(container(content).padding(16).width(Fill))
            .width(Fill)
            .height(Fill),
    )
    .width(520)
    .height(420)
    .style(darker_card)
    .into()
}

/// The whole percent of a progress, for the status line.
fn percent(done: u64, total: u64) -> String {
    if total == 0 {
        "0".to_owned()
    } else {
        (done.saturating_mul(100) / total).min(100).to_string()
    }
}

/// The background of a table row: the brand tint while the row is selected —
/// the same `brand_background2` Fluent uses for a text selection, which stays
/// readable in both light and dark themes — and nothing otherwise.
fn selected_row_style(theme: &crate::fluent::Theme, selected: bool) -> container::Style {
    if !selected {
        return container::Style::default();
    }

    container::Style {
        background: Some(iced::Background::Color(theme.tokens().brand_background2)),
        ..container::Style::default()
    }
}

/// A row of controls in two groups: one at the left edge of the row, one at the
/// right.
///
/// The groups are layers of a `Stack`, each a full-width container that aligns
/// its content to one side — the same way the language selector is pinned to a
/// corner. A `Space` with [`Fill`] width inside the row would not do: a row
/// gives a fluid child only the room the rest of the row leaves, which is
/// nothing once the fixed-width controls are placed.
fn split_row<'a>(
    start: Vec<Element<'a, Message>>,
    end: Vec<Element<'a, Message>>,
) -> Element<'a, Message> {
    let group = |items: Vec<Element<'a, Message>>| {
        Row::with_children(items)
            .spacing(8.0)
            .align_y(Alignment::Center)
    };

    let start = container(group(start)).align_left(Fill);
    let end = container(group(end)).align_right(Fill);

    iced::widget::Stack::with_children(vec![start.into(), end.into()])
        .width(Fill)
        .into()
}

/// The refresh control: an icon rather than the word, so the action row keeps
/// its room for the labels that say what they do.
///
/// It reads the partition table again, which only means anything while the
/// programmer runs, so it is greyed out until then.
fn refresh_button<'a>(live: bool) -> Element<'a, Message> {
    let control = button(text(REFRESH_ICON).size(16.0));

    if live {
        control.on_press(Message::QualcommRefresh).into()
    } else {
        control.into()
    }
}

/// Whether a partition table was read: the actions that work on it — the
/// danger zone, dump, write and erase — all wait for one.
fn table_loaded(qualcomm: &crate::QualcommState) -> bool {
    !qualcomm.partitions.is_empty() && !qualcomm.busy
}

/// Whether the reboot menu can open.
///
/// A normal reset only needs the programmer that is running inside the phone,
/// which is also the way out of a page whose table was cleared by a
/// whole-storage write; the modes that name partitions in the table are only
/// offered once it was read again.
pub(crate) fn reboot_ready(qualcomm: &crate::QualcommState) -> bool {
    !qualcomm.busy && (qualcomm.programmer_uploaded || !qualcomm.partitions.is_empty())
}

/// Opens Windows' Device Manager, where the phone shows up as a serial port.
///
/// The button carries only the monitor icon, so a tooltip names what it does.
fn device_manager_button<'a>(l10n: &'a crate::l10n::Bundle) -> Element<'a, Message> {
    let icon = button(
        canvas::Canvas::new(DeviceManagerIcon)
            .width(18)
            .height(18),
    )
    .on_press(Message::QualcommOpenDeviceManager);

    let hint = container(text(l10n.tr("qc-device-manager-tooltip")).size(12.0))
        .padding([3.0, 6.0])
        .style(container::rounded_box);

    tooltip(icon, hint, tooltip::Position::Top)
        .gap(4)
        .padding(0)
        .into()
}

/// The Device Manager button's icon: a monitor, drawn rather than taken from
/// a font — the bundled Inter face has no such glyph, and an emoji fallback
/// is often a colour bitmap the text renderer cannot draw.
struct DeviceManagerIcon;

impl canvas::Program<Message, crate::fluent::Theme> for DeviceManagerIcon {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        theme: &crate::fluent::Theme,
        bounds: iced::Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        // The icon takes the colour the button gives its label.
        let ink = theme.tokens().neutral_foreground1;
        let (width, height) = (bounds.width, bounds.height);
        let stroke = canvas::Stroke::default()
            .with_color(ink)
            .with_width(1.4);

        // The screen, its stand, and the foot it stands on.
        frame.stroke(
            &canvas::Path::rectangle(
                iced::Point::new(width * 0.12, height * 0.16),
                iced::Size::new(width * 0.76, height * 0.48),
            ),
            stroke,
        );
        frame.stroke(
            &canvas::Path::line(
                iced::Point::new(width * 0.5, height * 0.64),
                iced::Point::new(width * 0.5, height * 0.8),
            ),
            stroke,
        );
        frame.stroke(
            &canvas::Path::line(
                iced::Point::new(width * 0.3, height * 0.86),
                iced::Point::new(width * 0.7, height * 0.86),
            ),
            stroke,
        );

        vec![frame.into_geometry()]
    }
}

/// The width a dropdown needs so that its menu shows every label in full: iced
/// sizes the menu to the control that opens it, and the view has no renderer
/// to measure the text with.
///
/// A label that carries a CJK character is counted as twice as wide as a Latin
/// one, which is enough to keep the longest translation readable.
fn menu_width(labels: &[String]) -> f32 {
    let widest = labels
        .iter()
        .map(|label| {
            let wide = label
                .chars()
                .any(|character| character as u32 >= 0x2E80);
            let per_character = if wide { 16.0 } else { 8.5 };

            label.chars().count() as f32 * per_character
        })
        .fold(0.0_f32, f32::max);

    (widest + 40.0).clamp(120.0, 360.0)
}

/// A table entry: the data lines up in columns, so it is written in the
/// platform's monospace font.
fn value<'a>(
    value: impl iced::widget::text::IntoFragment<'a>,
    width: impl Into<iced::Length>,
) -> Element<'a, Message> {
    cell(text(value).size(12.0).font(crate::output_font()), width)
}

/// One table cell: as wide as its column, so the head and the rows line up.
fn cell<'a>(
    content: impl Into<Element<'a, Message>>,
    width: impl Into<iced::Length>,
) -> Element<'a, Message> {
    container(content).width(width).padding(CELL_PADDING).into()
}

