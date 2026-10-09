//! The "Unpack Image" dialog (Mode 2, Smart Device Flashing).
//!
//! Pressing the feature tile's button opens this modal, which reads the
//! selected image's directory table, shows what is inside, and writes the
//! files into a folder the user picks. Like the other dialogs, this module
//! only *presents* the state and sends messages — the flow state and its
//! update handlers live in `crate` (`State::flash.unpack`).

use iced::widget::text::Wrapping;
use iced::widget::{
    button, column, container, horizontal_rule, mouse_area, progress_bar, row, scrollable, Space,
};
use iced::{Alignment, Element, Fill};

use crate::bootloader::{bright_success, darker_card};
use crate::firmware::human_size;
use crate::guided::spinner;
use crate::image_unpack::{Entry, UnpackError};
use crate::l10n;
use crate::text;
use crate::{Message, State};

/// Height of the progress bar; a bar is a status line, not a button.
const BAR_HEIGHT: f32 = 6.0;

/// Height of the file list, so a long table does not push the buttons away.
const LIST_HEIGHT: f32 = 160.0;

/// Room kept on the inside of the file list for its floating scrollbar. The
/// bar is drawn over the scrollable's own bounds, so without this it would
/// cover the last characters of the right-aligned sizes.
const LIST_BAR_GUTTER: f32 = 14.0;

/// Id of the file list, so its scroll position survives the card being
/// rebuilt on every message.
const LIST_ID: &str = "unpack-image-list";

/// Whether the Unpack Image dialog is open.
pub(crate) fn is_open(state: &State) -> bool {
    state.flash.unpack.open
}

/// Renders the "Unpack Image" overlay over the app, or `None` when the dialog
/// is closed. Clicks on empty space are swallowed (they neither dismiss the
/// dialog nor reach the UI underneath); the modal is left only via its own
/// buttons.
pub(crate) fn overlay(state: &State) -> Option<Element<'_, Message>> {
    if !state.flash.unpack.open {
        return None;
    }

    let children: Vec<Element<'_, Message>> = vec![
        mouse_area(Space::new(Fill, Fill))
            .on_press(Message::UnpackBackdropPressed)
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

/// The modal card: pick the image and the output folder, look at the table,
/// then unpack it.
fn card(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let unpack = &state.flash.unpack;
    let busy = unpack.listing || unpack.working;

    // Nothing that talks to the file system runs while the image is read or
    // written out. After a finished run the image and the folder can still be
    // changed — picking another image starts over and clears the result.
    let selectable = !busy;

    let mut content = column![
        text(l10n.tr("flash-unpack-title")).size(18.0),
        text(l10n.tr("unpack-description"))
            .size(13.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph),
        horizontal_rule(1),
        picker_row(
            l10n,
            "unpack-select-image",
            unpack.image.as_deref(),
            Message::UnpackPickImageRequested,
            selectable,
        ),
        picker_row(
            l10n,
            "unpack-select-folder",
            unpack.output.as_deref(),
            Message::UnpackPickFolderRequested,
            selectable,
        ),
    ]
    .spacing(10)
    .width(Fill);

    if unpack.listing {
        content = content.push(busy_row(state, l10n.tr("unpack-listing")));
    } else if let Some(error) = &unpack.image_error {
        content = content.push(error_text(l10n, error));
    } else if let Some(entries) = &unpack.entries {
        content = content.push(contents_section(l10n, entries));
    }

    if unpack.working {
        content = content.push(busy_row(state, l10n.tr("unpack-working")));

        if unpack.total_bytes > 0 {
            content = content.push(bar(
                (unpack.done_bytes as f32 / unpack.total_bytes as f32).clamp(0.0, 1.0),
            ));
        }

        if let Some(current) = &unpack.current {
            content = content
                .push(text(current.clone()).size(12.0).width(Fill).wrapping(Wrapping::WordOrGlyph));
        }
    } else if let Some(result) = &unpack.result {
        match result {
            Ok(summary) => {
                let message = l10n.tr_with_args(
                    "unpack-done",
                    &[
                        ("count", summary.files.to_string()),
                        ("size", human_size(summary.bytes)),
                    ],
                );

                let done = text(message)
                    .size(13.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph)
                    .style(bright_success);

                // The files went into a folder of the user's choosing: the
                // button hands it to the system's file manager so they can be
                // found without copying the path out of the dialog.
                let locate = button(text(l10n.tr("unpack-locate")).size(13.0))
                    .padding([4.0, 10.0])
                    .on_press(Message::UnpackLocate);

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
    // until an image whose table was read has been picked together with an
    // output folder, and the dialog is never closed while it is working — a
    // button without `on_press` is greyed out and ignores clicks.
    content = content.push(horizontal_rule(1));

    let ready = !busy
        && unpack.result.is_none()
        && unpack.entries.is_some()
        && unpack.image.is_some()
        && unpack.output.is_some();

    let start = button(text(l10n.tr("unpack-start"))).width(Fill);
    let start: Element<'_, Message> = if ready {
        start.on_press(Message::UnpackStart).into()
    } else {
        start.into()
    };

    let cancel = button(text(l10n.tr("login-cancel"))).width(Fill);
    let cancel: Element<'_, Message> = if busy {
        cancel.into()
    } else {
        cancel.on_press(Message::UnpackCancel).into()
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
    .width(520)
    .height(440)
    .style(darker_card)
    .into()
}

/// One "pick a file / folder" row: the button and what is selected.
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
        container(
            text(value)
                .size(13.0)
                .wrapping(Wrapping::WordOrGlyph),
        )
        .width(Fill)
        .padding(8)
        .style(container::rounded_box),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .width(Fill)
    .into()
}

/// What the image holds, before anything is written.
fn contents_section<'a>(l10n: &l10n::Bundle, entries: &[Entry]) -> Element<'a, Message> {
    let total: u64 = entries.iter().map(|entry| entry.size).sum();

    let heading = l10n.tr_with_args(
        "unpack-contents",
        &[
            ("count", entries.len().to_string()),
            ("size", human_size(total)),
        ],
    );

    let rows = iced::widget::Column::with_children(
        entries
            .iter()
            .map(|entry| -> Element<'a, Message> {
                row![
                    text(entry.name.clone()).size(12.0).width(Fill),
                    text(human_size(entry.size)).size(12.0),
                ]
                .spacing(8)
                .width(Fill)
                .into()
            }),
    )
    .spacing(4)
    .width(Fill);

    column![
        text(heading).size(13.0),
        container(
            scrollable(
                // The padding is inside the scrollable, so the scrollbar
                // floats over this gutter instead of over the sizes.
                container(rows)
                    .padding([0.0, LIST_BAR_GUTTER])
                    .width(Fill),
            )
            .id(iced::widget::scrollable::Id::new(LIST_ID))
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

/// The localized message for a failed read or run. The one error a user
/// actually runs into — a file that is not this format — gets its own line.
fn error_text<'a>(l10n: &l10n::Bundle, error: &UnpackError) -> Element<'a, Message> {
    let message = match error {
        UnpackError::NotAnImage => l10n.tr("unpack-not-an-image"),
        other => l10n.tr_with_args("unpack-failed", &[("error", other.to_string())]),
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
        .height(BAR_HEIGHT)
        .width(Fill)
        .into()
}

/// A spinner plus its status text.
fn busy_row<'a>(state: &'a State, label: String) -> Element<'a, Message> {
    row![spinner(state.anim_tick), text(label).size(13.0)]
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
}
