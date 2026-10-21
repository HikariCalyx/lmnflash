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

use iced::widget::text::Shaping;
use iced::widget::{
    button, canvas, column, container, mouse_area, pick_list, progress_bar, row, scrollable,
    text_input, tooltip,
};
use iced::{Alignment, Fill};

use crate::bootloader::{bright_success, darker_card};
use crate::compat_iced as iced;
use crate::compat_iced::*;
use crate::firmware::human_size;
use crate::firmware_flash::menu_style;
use crate::text;
use crate::{Labeled, Message, QualcommStatus, RebootMode, State, StorageType};

/// Id of the partition table's scrollable, so its position survives the page
/// being rebuilt for every message.
const TABLE_SCROLL_ID: &str = "qualcomm-partitions";

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

/// The danger zone's entries: what the section will offer once it exists.
const DANGER_ENTRIES: [&str; 4] = [
    "qc-danger-repartition",
    "qc-danger-flash-storage",
    "qc-danger-flash-backup",
    "qc-danger-erase-storage",
];

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
    // a job runs. The reboot modes act on the session that read the table, so
    // the menu waits for one — iced cannot grey a `pick_list` out, so a plain
    // button takes its place until then.
    let reboot_options: Vec<Labeled<RebootMode>> = RebootMode::ALL
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

    let reboot: Element<'_, Message> = if table_loaded(qualcomm) {
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
        vec![button(text(l10n.tr("multi-flash-exit")))
            .on_press(Message::QualcommExit)
            .into()],
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

    let actions = split_row(
        vec![
            dump.into(),
            write.into(),
            erase.into(),
            refresh_button(qualcomm.programmer_uploaded && !qualcomm.busy),
        ],
        vec![pending(l10n.tr("qc-flash-xml"))],
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

    // [Dump Entire Storage] [Danger Zone ▾]: the menu lists what the section
    // will offer once it exists. None of it is implemented, so a pick is
    // ignored and the control keeps its placeholder — the entries behave like
    // the page's greyed-out buttons. Like Reboot, it only opens once a table
    // was read.
    let danger_labels: Vec<String> = DANGER_ENTRIES.iter().map(|id| l10n.tr(id)).collect();
    let danger_width = menu_width(&danger_labels);

    let danger: Element<'_, Message> = if table_loaded(qualcomm) {
        let menu: iced::widget::PickList<
            '_,
            String,
            Vec<String>,
            String,
            Message,
            crate::fluent::Theme,
        > = pick_list(danger_labels, None, |_| Message::QualcommDangerZoneSelected)
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

    let storage_row = row![pending(l10n.tr("qc-dump-entire-storage")), danger]
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

    let tool_row = split_row(vec![view_log.into()], tools);

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
        QualcommStatus::Rebooted => Some(
            text(l10n.tr("qc-reboot-sent"))
                .size(12.0)
                .style(bright_success)
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

/// A control that belongs to the page's layout but has no job yet.
///
/// iced draws a button without `on_press` greyed out and ignores clicks on it,
/// which is exactly how the page shows that the Firehose side is still to
/// come.
fn pending<'a>(label: String) -> Element<'a, Message> {
    button(text(label)).into()
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

/// Whether a partition table was read: the actions that work on it — reboot and
/// the danger zone — all wait for one.
fn table_loaded(qualcomm: &crate::QualcommState) -> bool {
    !qualcomm.partitions.is_empty() && !qualcomm.busy
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

