//! The "Merge sparsechunk" dialog (Mode 3, firmware-file tools).
//!
//! Pressing the feature tile's button opens this modal, which reads the
//! headers of the picked `*_sparsechunk.N` files, shows what merging them
//! produces, and writes the merged image into a folder the user picks. Like
//! the other dialogs, this module only *presents* the state and sends
//! messages — the flow state and its update handlers live in `crate`
//! (`State::sparse_merge`).

use iced::widget::text::Wrapping;
use iced::widget::{button, column, container, mouse_area, pick_list, progress_bar, row, scrollable};
use iced::{Alignment, Fill};

use crate::bootloader::{bright_success, darker_card, warning_orange};
use crate::compat_iced as iced;
use crate::compat_iced::*;
use crate::firmware::human_size;
use crate::guided::spinner;
use crate::l10n;
use crate::sparse::{ChunkFile, Format, MergeError, Plan};
use crate::text;
use crate::{Labeled, Message, State};

/// Height of the progress bar; a bar is a status line, not a button.
const BAR_HEIGHT: f32 = 6.0;

/// Height of the chunk list, so a package's thirty chunks do not push the
/// buttons away.
const LIST_HEIGHT: f32 = 150.0;

/// Room kept on the inside of the chunk list for its floating scrollbar, which
/// is drawn over the scrollable's own bounds and would otherwise cover the
/// right-aligned sizes.
const LIST_BAR_GUTTER: f32 = 14.0;

/// Id of the chunk list, so its scroll position survives the card being
/// rebuilt on every message.
const LIST_ID: &str = "sparse-merge-list";

/// How many missing chunk numbers the warning spells out before it stops.
const MISSING_SHOWN: usize = 8;

/// Whether the Merge sparsechunk dialog is open.
pub(crate) fn is_open(state: &State) -> bool {
    state.sparse_merge.open
}

/// Renders the "Merge sparsechunk" overlay over the app, or `None` when the
/// dialog is closed. Clicks on empty space are swallowed (they neither dismiss
/// the dialog nor reach the UI underneath); the modal is left only via its own
/// buttons.
pub(crate) fn overlay(state: &State) -> Option<Element<'_, Message>> {
    if !state.sparse_merge.open {
        return None;
    }

    let children: Vec<Element<'_, Message>> = vec![
        mouse_area(Space::new(Fill, Fill))
            .on_press(Message::SparseMergeBackdropPressed)
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

/// The modal card: pick the chunks, the output folder and the shape of the
/// merged image, look at what it will be, then write it.
fn card(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let merge = &state.sparse_merge;
    let busy = merge.listing || merge.working;

    // Nothing that talks to the file system runs while the chunks are read or
    // the merged image is written. After a finished run the selection can
    // still be changed — picking other chunks starts over and clears the
    // result.
    let selectable = !busy;

    let chosen = match merge.files.len() {
        0 => l10n.tr("sparse-none-selected"),
        count => l10n.tr_with_args("sparse-files-chosen", &[("count", count.to_string())]),
    };

    let mut content = column![
        text(l10n.tr("sparse-title")).size(18.0),
        text(l10n.tr("sparse-description"))
            .size(13.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph),
        horizontal_rule(1),
        picker_row(
            l10n,
            "sparse-select-files",
            chosen,
            Message::SparseMergePickFilesRequested,
            selectable,
        ),
        picker_row(
            l10n,
            "sparse-select-folder",
            match merge.output.as_deref() {
                Some(path) => path.display().to_string(),
                None => l10n.tr("sparse-none-selected"),
            },
            Message::SparseMergePickFolderRequested,
            selectable,
        ),
        format_row(state, selectable),
    ]
    .spacing(10)
    .width(Fill);

    if merge.listing {
        content = content.push(busy_row(state, l10n.tr("sparse-listing")));
    } else if let Some(error) = &merge.plan_error {
        content = content.push(error_text(l10n, error));
    } else if let Some(plan) = &merge.plan {
        content = content.push(chunks_section(l10n, plan));
        content = content.push(target_line(state, plan));

        if !plan.missing.is_empty() {
            content = content.push(warning(missing_text(l10n, &plan.missing)));
        }

        // The merged image is written over a file of that name if one is
        // already in the folder, which is easy to do twice in a row. The run
        // that just finished left one there itself, so this is only worth
        // saying about a run that has not happened yet.
        if merge.overwrites && merge.result.is_none() {
            content = content.push(warning(l10n.tr("sparse-overwrites")));
        }
    }

    if merge.working {
        content = content.push(busy_row(state, l10n.tr("sparse-working")));

        if merge.total_bytes > 0 {
            content = content.push(bar(
                (merge.done_bytes as f32 / merge.total_bytes as f32).clamp(0.0, 1.0),
            ));
        }

        if let Some(current) = &merge.current {
            content = content.push(
                text(current.clone())
                    .size(12.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph),
            );
        }
    } else if let Some(result) = &merge.result {
        match result {
            Ok(summary) => {
                let name = summary
                    .output
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| summary.output.display().to_string());

                let message = l10n.tr_with_args(
                    "sparse-done",
                    &[
                        ("count", summary.files.to_string()),
                        ("name", name),
                        ("size", human_size(summary.bytes)),
                    ],
                );

                let done = text(message)
                    .size(13.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph)
                    .style(bright_success);

                // The image went into a folder of the user's choosing: the
                // button hands it to the system's file manager so it can be
                // found without copying the path out of the dialog.
                let locate = button(text(l10n.tr("sparse-locate")).size(13.0))
                    .padding([4.0, 10.0])
                    .on_press(Message::SparseMergeLocate);

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

    // Footer: the job starts here and Cancel closes the dialog. Nothing runs
    // until chunks whose headers were read have been picked together with an
    // output folder, and the dialog is never closed while it is working — a
    // button without `on_press` is greyed out and ignores clicks.
    content = content.push(horizontal_rule(1));

    let ready = !busy
        && merge.result.is_none()
        && merge.plan.is_some()
        && merge.output.is_some();

    let start = button(text(l10n.tr("sparse-start"))).width(Fill);
    let start: Element<'_, Message> = if ready {
        start.on_press(Message::SparseMergeStart).into()
    } else {
        start.into()
    };

    let cancel = button(text(l10n.tr("login-cancel"))).width(Fill);
    let cancel: Element<'_, Message> = if busy {
        cancel.into()
    } else {
        cancel.on_press(Message::SparseMergeCancel).into()
    };

    content = content.push(row![start, cancel].spacing(8).width(Fill));

    // The padding sits inside the scrollable, so the card's border is the
    // viewport: the scrollbar rides on the right border instead of floating
    // in the middle of the padding.
    container(
        scrollable(container(content).padding(16).width(Fill))
            .width(Fill)
            .height(Fill),
    )
    .width(560)
    .height(480)
    .style(darker_card)
    .into()
}

/// One "pick a file / folder" row: the button and what is selected.
fn picker_row<'a>(
    l10n: &l10n::Bundle,
    label_id: &'static str,
    value: String,
    message: Message,
    enabled: bool,
) -> Element<'a, Message> {
    let picker = button(text(l10n.tr(label_id))).width(180);
    let picker = if enabled {
        picker.on_press(message)
    } else {
        picker
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

/// Which image the chunks are merged into: one sparse image again, the shape a
/// bootloader takes, or the raw image a partition tool wants.
fn format_row(state: &State, enabled: bool) -> Element<'_, Message> {
    let l10n = &state.l10n;

    let options: Vec<Labeled<Format>> = [
        (Format::Sparse, "sparse-format-sparse"),
        (Format::Raw, "sparse-format-raw"),
    ]
    .into_iter()
    .map(|(value, label)| Labeled {
        value,
        label: l10n.tr(label),
    })
    .collect();

    let selected = options
        .iter()
        .find(|option| option.value == state.sparse_merge.format)
        .cloned();

    let picker: iced::widget::PickList<
        '_,
        Labeled<Format>,
        Vec<Labeled<Format>>,
        Labeled<Format>,
        Message,
        crate::fluent::Theme,
    > = pick_list(options, selected, |option| {
        Message::SparseMergeFormatSelected(option.value)
    })
    .width(Fill);

    let picker: Element<'_, Message> = if enabled {
        picker.into()
    } else {
        // A pick list cannot be disabled, so while a job runs the choice is
        // shown as the plain text it settled on.
        container(
            text(l10n.tr(match state.sparse_merge.format {
                Format::Sparse => "sparse-format-sparse",
                Format::Raw => "sparse-format-raw",
            }))
            .size(13.0),
        )
        .width(Fill)
        .padding(8)
        .style(container::rounded_box)
        .into()
    };

    row![
        container(text(l10n.tr("sparse-format")).size(13.0)).width(180),
        picker,
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .width(Fill)
    .into()
}

/// The chunks that will be merged, before anything is written.
fn chunks_section<'a>(l10n: &l10n::Bundle, plan: &Plan) -> Element<'a, Message> {
    let heading = l10n.tr_with_args(
        "sparse-chunks",
        &[
            ("count", plan.files.len().to_string()),
            ("size", human_size(plan.input_bytes())),
        ],
    );

    let rows = iced::widget::Column::with_children(plan.files.iter().map(
        |file: &ChunkFile| -> Element<'a, Message> {
            row![
                text(file.name.clone()).size(12.0).width(Fill),
                text(human_size(file.size)).size(12.0),
            ]
            .spacing(8)
            .width(Fill)
            .into()
        },
    ))
    .spacing(4)
    .width(Fill);

    column![
        text(heading).size(13.0),
        container(
            scrollable(
                // The padding is inside the scrollable, so the scrollbar
                // floats over this gutter instead of over the sizes.
                container(rows).padding([0.0, LIST_BAR_GUTTER]).width(Fill),
            )
            .id(iced::widget::Id::new(LIST_ID))
            .height(LIST_HEIGHT)
            .width(Fill),
        )
        .padding(6)
        .style(container::rounded_box),
    ]
    .spacing(6)
    .width(Fill)
    .into()
}

/// What the run will write: the name the merged image gets and how long it
/// will be.
fn target_line<'a>(state: &'a State, plan: &Plan) -> Element<'a, Message> {
    let format = state.sparse_merge.format;

    let message = state.l10n.tr_with_args(
        "sparse-output",
        &[
            ("name", plan.output_name(format)),
            ("size", human_size(plan.output_bytes(format))),
        ],
    );

    text(message)
        .size(13.0)
        .width(Fill)
        .wrapping(Wrapping::WordOrGlyph)
        .into()
}

/// The warning about chunk numbers that are missing from the selection: their
/// blocks stay holes, so the merged image would be incomplete.
fn missing_text(l10n: &l10n::Bundle, missing: &[u64]) -> String {
    let shown: Vec<String> = missing
        .iter()
        .take(MISSING_SHOWN)
        .map(|number| number.to_string())
        .collect();

    let mut numbers = shown.join(", ");

    if missing.len() > MISSING_SHOWN {
        numbers.push('…');
    }

    l10n.tr_with_args("sparse-missing", &[("numbers", numbers)])
}

/// A line the user should read before pressing Merge, in the warning colour.
fn warning<'a>(message: String) -> Element<'a, Message> {
    text(message)
        .size(13.0)
        .width(Fill)
        .wrapping(Wrapping::WordOrGlyph)
        .style(warning_orange)
        .into()
}

/// The localized message for a failed read or run. The errors a user actually
/// runs into — a file that is not a chunk, and chunks of two different images
/// — get a line of their own.
fn error_text<'a>(l10n: &l10n::Bundle, error: &MergeError) -> Element<'a, Message> {
    let message = match error {
        MergeError::NotSparse(file) => {
            l10n.tr_with_args("sparse-not-sparse", &[("file", file.clone())])
        }
        MergeError::Overlap { file, other, .. } => l10n.tr_with_args(
            "sparse-overlap",
            &[("file", file.clone()), ("other", other.clone())],
        ),
        other => l10n.tr_with_args("sparse-failed", &[("error", other.to_string())]),
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
