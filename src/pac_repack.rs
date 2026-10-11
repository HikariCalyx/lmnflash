//! The "Repack Package" dialog (Mode 3, firmware-file tools).
//!
//! Unisoc ships a firmware either as one PAC package or as a scatter XML with
//! the files beside it, and the tools describe the latter with a `flash.cfg`.
//! Pressing the feature tile's "Repack" button opens this modal, which reads
//! either of those two, matches every record to the file it needs — the ones it
//! cannot match are pointed at by hand, or left out of the package — and writes
//! a package the flashing tools accept. Like the other dialogs, this module
//! only *presents* the state and sends messages: the flow state and its update
//! handlers live in `crate` (`State::pac_repack`).

use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::text::{IntoFragment, Shaping, Wrapping};
use iced::widget::{
    button, column, container, mouse_area, progress_bar, row, scrollable, text_input,
};
use iced::{Alignment, Fill, Length, Vector};

use crate::anim;
use crate::compat_iced as iced;
use crate::compat_iced::*;
use crate::bootloader::{bright_success, darker_card, warning_orange};
use crate::firmware::human_size;
use crate::guided::spinner;
use crate::l10n;
use crate::pac;
use crate::pac_build::{self, BuildError, Plan};
use crate::text;
use crate::{output_font, Message, State};

/// Height of the progress bar; a bar is a status line, not a button.
const BAR_HEIGHT: f32 = 6.0;

/// The table's columns, in the order they are drawn. They are wide enough for a
/// product's long file names, which makes the table wider than a window that is
/// not maximised: it scrolls sideways, its header included.
const COL_CHECK: f32 = 36.0;
const COL_PART: f32 = 190.0;
const COL_PARTITION: f32 = 200.0;
const COL_SIZE: f32 = 90.0;
const COL_FILE: f32 = 420.0;

/// How wide one row of the table is.
const TABLE_WIDTH: f32 = COL_CHECK + COL_PART + COL_PARTITION + COL_SIZE + COL_FILE;

/// The room a cell keeps around its content, so the columns stay apart.
const CELL_PADDING: [f32; 2] = [3.0, 6.0];

/// The room kept after the last column and under the last row for the table's
/// scrollbars, which iced floats over the content (it reserves room for them
/// in one direction only): without it the bars sit on top of the file names
/// and the buttons.
const BAR_GUTTER: f32 = 14.0;

/// The size the table's entries are written in.
const ENTRY_SIZE: f32 = 13.0;

/// Space a value takes when the record has none.
const EMPTY: &str = "—";

/// Id of the table's scrolling part, so its position survives the card being
/// rebuilt on every message (and so the update handler can send the table back
/// to its first column when another source is loaded).
pub(crate) const TABLE_ID: &str = "pac-repack-table";

/// Whether the Repack Package dialog is open.
pub(crate) fn is_open(state: &State) -> bool {
    state.pac_repack.open
}

/// Renders the "Repack Package" overlay over the app, or `None` when the dialog
/// is closed. Clicks on empty space are swallowed (they neither dismiss the
/// dialog nor reach the UI underneath); the modal is left only via its own
/// buttons.
pub(crate) fn overlay(state: &State) -> Option<Element<'_, Message>> {
    if !state.pac_repack.open {
        return None;
    }

    let children: Vec<Element<'_, Message>> = vec![
        mouse_area(Space::new(Fill, Fill))
            .on_press(Message::PacRepackBackdropPressed)
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

/// The modal card: the whole window, since the table it shows is the point of
/// the dialog. Pick the source, fill in what it does not carry, check the
/// records, then write the package.
fn card(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let repack = &state.pac_repack;
    let busy = repack.busy();

    // Nothing that talks to the file system runs while the source is read or
    // the package is written. After a finished run the source can still be
    // changed — picking another one starts over and clears the result.
    let selectable = !busy;

    let mut content = column![
        text(l10n.tr("pac-repack-title")).size(18.0),
        text(l10n.tr("pac-repack-description"))
            .size(13.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph),
        horizontal_rule(1),
        picker_row(
            l10n,
            "pac-repack-select-source",
            repack.source.as_deref(),
            Message::PacRepackPickSourceRequested,
            selectable,
        ),
    ]
    .spacing(10)
    .width(Fill)
    .height(Fill);

    if repack.reading {
        content = content.push(busy_row(state, l10n.tr("pac-repack-reading")));
    } else if let Some(error) = &repack.source_error {
        content = content.push(error_text(l10n, error));
    } else if let Some(plan) = &repack.plan {
        content = content.push(product_line(plan));

        // A scatter XML names the product but not its version or alias: those
        // two the package has to carry, so they are asked for.
        if plan.needs_metadata() {
            content = content.push(
                text(l10n.tr("pac-repack-metadata"))
                    .size(12.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph)
                    .style(warning_orange),
            );

            content = content.push(
                row![
                    metadata_input(
                        l10n,
                        "pac-repack-alias",
                        &plan.header.product_alias,
                        Message::PacRepackAliasChanged,
                        selectable,
                    ),
                    metadata_input(
                        l10n,
                        "pac-repack-version",
                        &plan.header.product_version,
                        Message::PacRepackVersionChanged,
                        selectable,
                    ),
                ]
                .spacing(8)
                .width(Fill),
            );
        }

        content = content.push(files_heading(l10n, plan));

        // The search box only filters what the table shows, and it is the same
        // one the package dialog uses: finding one record among forty is the
        // same job.
        let search_placeholder = l10n.tr("pac-search");
        let search = text_input(&search_placeholder, &repack.filter)
            .padding(6)
            .width(Fill);

        // A text input without `on_input` is greyed out and cannot be typed
        // into, which is what the box needs while a job runs.
        content = content.push(if selectable {
            search.on_input(Message::PacRepackFilterChanged)
        } else {
            search
        });

        content = content.push(table_section(
            l10n,
            plan,
            &repack.filter,
            repack.offset,
            selectable,
        ));

        if selectable {
            content = content.push(bulk_row(l10n, plan));
        }

        // Every record left out means an empty package, which is not what
        // anyone wants: say so rather than leaving the footer greyed out.
        if plan.chosen() == 0 {
            content = content.push(
                text(l10n.tr("pac-repack-nothing"))
                    .size(12.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph)
                    .style(iced::widget::text::danger),
            );
        }
    }

    if repack.working {
        content = content.push(busy_row(state, l10n.tr("pac-repack-working")));

        if repack.total_bytes > 0 {
            content = content.push(bar(
                (repack.done_bytes as f32 / repack.total_bytes as f32).clamp(0.0, 1.0),
            ));
        }

        if let Some(current) = &repack.current {
            content = content.push(
                text(current.clone())
                    .size(12.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph),
            );
        }
    } else if let Some(result) = &repack.result {
        match result {
            Ok(summary) => {
                let message = l10n.tr_with_args(
                    "pac-repack-done",
                    &[
                        ("count", summary.files.to_string()),
                        ("size", human_size(summary.bytes)),
                        (
                            "path",
                            repack
                                .output
                                .as_deref()
                                .map(|path| path.display().to_string())
                                .unwrap_or_default(),
                        ),
                    ],
                );

                let done = text(message)
                    .size(13.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph)
                    .style(bright_success);

                // The package went to a file of the user's choosing: the button
                // hands its folder to the system's file manager so it can be
                // found without copying the path out of the dialog.
                let locate = button(text(l10n.tr("unpack-locate")).size(13.0))
                    .padding([4.0, 10.0])
                    .on_press(Message::PacRepackLocate);

                content = content.push(
                    row![done, locate]
                        .spacing(8)
                        .align_y(Alignment::Center)
                        .width(Fill),
                );
            }
            Err(error) => content = content.push(error_text(l10n, error)),
        }
    }

    // Footer: the package is written here and Cancel closes the dialog. Nothing
    // runs until a source whose records all have a file has been picked, its
    // metadata is complete and at least one record is left in — a button
    // without `on_press` is greyed out and ignores clicks.
    content = content.push(horizontal_rule(1));

    let ready = !busy
        && repack.result.is_none()
        && repack
            .plan
            .as_ref()
            .map(|plan| plan.missing() == 0 && !plan.needs_metadata() && plan.chosen() > 0)
            .unwrap_or(false);

    let start = button(text(l10n.tr("pac-repack-button"))).width(Fill);
    let start: Element<'_, Message> = if ready {
        start.on_press(Message::PacRepackStart).into()
    } else {
        start.into()
    };

    let cancel = button(text(l10n.tr("login-cancel"))).width(Fill);
    let cancel: Element<'_, Message> = if busy {
        cancel.into()
    } else {
        cancel.on_press(Message::PacRepackCancel).into()
    };

    content = content.push(row![start, cancel].spacing(8).width(Fill));

    container(content.padding(16))
        .width(Fill)
        .height(Fill)
        .style(darker_card)
        .into()
}

/// The product the source describes, and its version when it knows one.
fn product_line<'a>(plan: &'a Plan) -> Element<'a, Message> {
    let product = plan.header.product_name.trim();
    let version = plan.header.product_version.trim();
    let title = format!("{product} {version}").trim().to_string();

    text(title).size(14.0).into()
}

/// How many records have a file, out of the ones that need one. The
/// configuration XML is left out: it has no row in the table to count.
fn files_heading<'a>(l10n: &l10n::Bundle, plan: &'a Plan) -> Element<'a, Message> {
    let missing = plan.missing();
    let total = plan.chosen_with_file();

    text(l10n.tr_with_args(
        "pac-repack-files",
        &[
            ("resolved", total.saturating_sub(missing).to_string()),
            ("total", total.to_string()),
        ],
    ))
    .size(13.0)
    .into()
}

/// The buttons that work on the whole list: the selection, and the one-click
/// way past a scatter XML's unmatched records.
fn bulk_row<'a>(l10n: &l10n::Bundle, plan: &'a Plan) -> Element<'a, Message> {
    let missing = plan.missing();

    let mut bulk = row![
        button(text(l10n.tr("pac-select-all")).size(13.0))
            .padding([4.0, 10.0])
            .on_press(Message::PacRepackSelectAll(true)),
        button(text(l10n.tr("pac-select-none")).size(13.0))
            .padding([4.0, 10.0])
            .on_press(Message::PacRepackSelectAll(false)),
    ]
    .spacing(8);

    // The records the source could not match are the ones worth dropping in one
    // click: leaving them out is what makes the package writable.
    if missing > 0 {
        bulk = bulk.push(
            button(text(l10n.tr_with_args(
                "pac-repack-leave-out",
                &[("count", missing.to_string())],
            ))
            .size(13.0))
            .padding([4.0, 10.0])
            .on_press(Message::PacRepackLeaveOut),
        );
    }

    bulk.into()
}

/// One "pick a file" row: the button and what is selected.
fn picker_row<'a>(
    l10n: &l10n::Bundle,
    label_id: &'static str,
    path: Option<&std::path::Path>,
    message: Message,
    enabled: bool,
) -> Element<'a, Message> {
    let picker = button(text(l10n.tr(label_id))).width(180);
    let picker = if enabled {
        picker.on_press(message)
    } else {
        picker
    };

    let value = match path {
        Some(path) => path.display().to_string(),
        None => l10n.tr("unpack-none-selected"),
    };

    row![
        picker,
        container(text(value).size(13.0).wrapping(Wrapping::WordOrGlyph))
            .width(Fill)
            .padding(8)
            .style(container::rounded_box),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .width(Fill)
    .into()
}

/// The record table: a titled header that follows the rows sideways, and the
/// rows themselves, which scroll both ways. It takes the room the card has
/// left, so a firmware with dozens of records fills the window.
fn table_section<'a>(
    l10n: &l10n::Bundle,
    plan: &'a Plan,
    filter: &'a str,
    offset: f32,
    enabled: bool,
) -> Element<'a, Message> {
    let visible = visible(plan, filter);

    // The header is drawn shifted by the table's own horizontal offset and
    // clipped to the room the table has, so its titles stay above their columns
    // while the rows are scrolled sideways.
    let header = container(anim::animated(
        header_row(l10n),
        anim::Pose {
            slide: Vector::new(-offset, 0.0),
            ..anim::dialog_rest()
        },
    ))
    .width(Fill)
    .clip(true);

    let mut section = column![header, horizontal_rule(1)]
        .spacing(4)
        .width(Fill)
        .height(Fill);

    if visible.is_empty() {
        section = section.push(
            text(l10n.tr_with_args(
                "pac-no-match",
                &[("query", filter.trim().to_owned())],
            ))
            .size(12.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph),
        );

        return section.into();
    }

    let rows = iced::widget::Column::with_children(
        visible
            .iter()
            .map(|&index| record_row(l10n, plan, index, enabled)),
    )
    // The rows end a bar's width before the table does, so the bars cover that
    // gutter instead of the last column.
    .width(Length::Fixed(TABLE_WIDTH + BAR_GUTTER))
    .push(Space::new(Fill, BAR_GUTTER));

    // Both scrollbars ride on the viewport's own edges, and the header follows
    // the horizontal one (`Message::PacRepackScrolled`).
    section = section.push(
        scrollable(rows)
            .id(iced::widget::Id::new(TABLE_ID))
            .direction(Direction::Both {
                vertical: Scrollbar::default(),
                horizontal: Scrollbar::default(),
            })
            .on_scroll(|viewport| Message::PacRepackScrolled(viewport.absolute_offset().x))
            .width(Fill)
            .height(Fill),
    );

    section.into()
}

/// The table's titles, in the widths of the columns below them.
fn header_row<'a>(l10n: &l10n::Bundle) -> Element<'a, Message> {
    row![
        cell(Space::new(0.0, 0.0), COL_CHECK),
        cell(text(l10n.tr("pac-repack-col-part")).size(13.0), COL_PART),
        cell(
            text(l10n.tr("pac-repack-col-partition")).size(13.0),
            COL_PARTITION
        ),
        cell(text(l10n.tr("pac-repack-col-size")).size(13.0), COL_SIZE),
        cell(text(l10n.tr("pac-repack-col-file")).size(13.0), COL_FILE),
    ]
    .width(Length::Fixed(TABLE_WIDTH))
    .into()
}

/// One record of the table: whether it goes into the package, the part it is
/// known by, the partition it is written to, the size of its file, and that
/// file — with the button that points it at another one.
fn record_row<'a>(
    l10n: &l10n::Bundle,
    plan: &'a Plan,
    index: usize,
    enabled: bool,
) -> Element<'a, Message> {
    let entry = &plan.entries[index];

    let record = checkbox("", entry.include).text_shaping(Shaping::Advanced);

    // A checkbox without `on_toggle` is greyed out and ignores clicks, which is
    // what the table needs while a job runs.
    let record = if enabled {
        record.on_toggle(move |checked| Message::PacRepackToggle(index, checked))
    } else {
        record
    };

    row![
        cell(record, COL_CHECK),
        cell(value_text(entry.id.as_str()), COL_PART),
        cell(
            value_text(entry.partition.as_deref().unwrap_or(EMPTY)),
            COL_PARTITION
        ),
        cell(size_text(entry.size), COL_SIZE),
        cell(file_cell(l10n, entry, index, enabled), COL_FILE),
    ]
    .width(Length::Fixed(TABLE_WIDTH))
    .into()
}

/// A table entry: the data lines up in columns, so it is written in the
/// platform's monospace font.
fn value_text<'a>(value: impl IntoFragment<'a>) -> Text<'a> {
    text(value).size(ENTRY_SIZE).font(output_font())
}

/// The size of the file a record takes.
fn size_text<'a>(size: Option<u64>) -> Element<'a, Message> {
    let value = match size {
        Some(bytes) => human_size(bytes),
        None => EMPTY.to_string(),
    };

    value_text(value).into()
}

/// The last column: the file the record takes, and the button that points it at
/// another one — the name a product uses is not always one this tool knows.
fn file_cell<'a>(
    l10n: &l10n::Bundle,
    entry: &'a pac_build::Entry,
    index: usize,
    enabled: bool,
) -> Element<'a, Message> {
    let name = match file_name(entry) {
        Some(name) => name,
        None if entry.needs_file() => l10n.tr("pac-repack-needs-file"),
        None => EMPTY.to_string(),
    };

    let mut line: iced::widget::Row<'a, Message> =
        iced::widget::Row::new().spacing(8).align_y(Alignment::Center);

    // The button comes first, so it keeps its place in the column however long
    // the file name is.
    if entry.needs_file() {
        let pick = button(text(l10n.tr("pac-repack-select-file")).size(12.0)).padding([2.0, 8.0]);

        let pick = if enabled {
            pick.on_press(Message::PacRepackSelectFile(index))
        } else {
            pick
        };

        line = line.push(pick);
    }

    line = line.push(container(
        value_text(name).width(Fill).wrapping(Wrapping::WordOrGlyph),
    ));

    line.into()
}

/// One table cell: as wide as its column, so the header and the rows line up.
fn cell<'a>(content: impl Into<Element<'a, Message>>, width: f32) -> Element<'a, Message> {
    container(content)
        .width(Length::Fixed(width))
        .padding(CELL_PADDING)
        .into()
}

/// One "the source does not carry this" text field.
fn metadata_input<'a>(
    l10n: &l10n::Bundle,
    label_id: &'static str,
    value: &'a str,
    message: fn(String) -> Message,
    enabled: bool,
) -> Element<'a, Message> {
    let placeholder = l10n.tr(label_id);
    let input = text_input(&placeholder, value).padding(6).width(Fill);

    let input = if enabled {
        input.on_input(message)
    } else {
        input
    };

    input.into()
}

/// The name of the file a record takes, once it has one.
fn file_name(entry: &pac_build::Entry) -> Option<String> {
    entry
        .choice
        .path()
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
}

/// Whether a record is shown for the current search: its id, the id its product
/// displays, or the name of the file it takes.
pub(crate) fn shows(entry: &pac_build::Entry, query: &str) -> bool {
    pac::matches(&entry.id, query)
        || pac::matches(&entry.alias, query)
        || file_name(entry).is_some_and(|name| pac::matches(&name, query))
}

/// The rows the table lists, in the order the source gives them: every record
/// the user chooses from that the search box lets through. The configuration
/// XML has no row of its own — the package always carries it — so neither the
/// table nor the buttons that work on what it shows can touch it.
pub(crate) fn visible(plan: &Plan, query: &str) -> Vec<usize> {
    plan.entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| !entry.is_configuration() && shows(entry, query))
        .map(|(index, _)| index)
        .collect()
}

/// The localized message for a source that could not be read or a run that
/// failed. The one mistake a user actually makes — picking something that is
/// neither a flash configuration nor a scatter XML — gets its own line.
fn error_text<'a>(l10n: &l10n::Bundle, error: &BuildError) -> Element<'a, Message> {
    let message = match error {
        BuildError::NotASource => l10n.tr("pac-repack-invalid"),
        other => l10n.tr_with_args("pac-repack-failed", &[("error", other.to_string())]),
    };

    text(message)
        .size(13.0)
        .width(Fill)
        .wrapping(Wrapping::WordOrGlyph)
        .style(iced::widget::text::danger)
        .into()
}

/// A progress bar at the height used by the dialogs.
fn bar<'a>(fraction: f32) -> Element<'a, Message> {
    progress_bar(0.0..=1.0, fraction)
        .girth(BAR_HEIGHT)
        .length(Fill)
        .into()
}

/// A spinner plus its status text.
fn busy_row<'a>(state: &'a State, label: String) -> Element<'a, Message> {
    row![spinner(state.anim_tick), text(label).size(13.0)]
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
}
