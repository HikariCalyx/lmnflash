//! The "Unisoc PAC" dialog (Mode 3, firmware-file tools).
//!
//! Pressing the feature tile's "Unpack" button opens this modal, which reads
//! the selected package's file table, lets the user pick which of its files to
//! write out, and copies them into a folder they choose. Like the other
//! dialogs, this module only *presents* the state and sends messages — the
//! flow state and its update handlers live in `crate` (`State::pac`).

use iced::widget::text::{Shaping, Wrapping};
use iced::widget::{
    button, checkbox, column, container, horizontal_rule, mouse_area, progress_bar, row, scrollable,
    text_input, Space,
};
use iced::{Alignment, Element, Fill};

use crate::bootloader::{bright_success, darker_card, warning_orange};
use crate::firmware::human_size;
use crate::guided::spinner;
use crate::l10n;
use crate::pac::{has_configuration, matches, Package, PacError};
use crate::spd_script;
use crate::text;
use crate::{Message, State};

/// Height of the progress bar; a bar is a status line, not a button.
const BAR_HEIGHT: f32 = 6.0;

/// Room kept on the inside of the file list for its floating scrollbar, so it
/// does not cover the end of the longest file names.
const LIST_BAR_GUTTER: f32 = 14.0;

/// Id of the file list, so its scroll position survives the card being rebuilt
/// on every message.
const LIST_ID: &str = "pac-unpack-list";

/// Whether the Unisoc PAC dialog is open.
pub(crate) fn is_open(state: &State) -> bool {
    state.pac.open
}

/// Renders the "Unisoc PAC" overlay over the app, or `None` when the dialog is
/// closed. Clicks on empty space are swallowed (they neither dismiss the
/// dialog nor reach the UI underneath); the modal is left only via its own
/// buttons.
pub(crate) fn overlay(state: &State) -> Option<Element<'_, Message>> {
    if !state.pac.open {
        return None;
    }

    let children: Vec<Element<'_, Message>> = vec![
        mouse_area(Space::new(Fill, Fill))
            .on_press(Message::PacBackdropPressed)
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

/// The modal card: pick the package and the output folder, look at the file
/// table, then unpack the checked files.
fn card(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let pac = &state.pac;
    let busy = pac.busy();

    // Nothing that talks to the file system runs while the package is read or
    // written out. After a finished run the package and the folder can still
    // be changed — picking another package starts over and clears the result.
    let selectable = !busy;

    let mut content = column![
        text(l10n.tr("pac-title")).size(18.0),
        text(l10n.tr("pac-description"))
            .size(13.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph),
        horizontal_rule(1),
        picker_row(
            l10n,
            "pac-select-file",
            pac.file.as_deref(),
            Message::PacPickFileRequested,
            selectable,
        ),
        picker_row(
            l10n,
            "unpack-select-folder",
            pac.output.as_deref(),
            Message::PacPickFolderRequested,
            selectable,
        ),
        script_row(
            l10n,
            pac.package.as_ref(),
            &pac.selected,
            pac.spd_script,
            selectable,
        ),
    ]
    .spacing(10)
    .width(Fill)
    .height(Fill);

    if pac.listing {
        content = content.push(busy_row(state, l10n.tr("pac-listing")));
    } else if let Some(error) = &pac.file_error {
        content = content.push(error_text(l10n, error));
    } else if let Some(package) = &pac.package {
        content = content.push(package_section(
            l10n,
            package,
            &pac.selected,
            &pac.filter,
            selectable,
        ));
    }

    if pac.working {
        content = content.push(busy_row(state, l10n.tr("unpack-working")));

        if pac.total_bytes > 0 {
            content = content.push(bar(
                (pac.done_bytes as f32 / pac.total_bytes as f32).clamp(0.0, 1.0),
            ));
        }

        if let Some(current) = &pac.current {
            content = content.push(
                text(current.clone())
                    .size(12.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph),
            );
        }
    } else if let Some(result) = &pac.result {
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
                    .on_press(Message::PacLocate);

                content = content.push(
                    row![done, locate]
                        .spacing(8)
                        .align_y(Alignment::Center)
                        .width(Fill),
                );

                // The scripts were written into the same folder: say which, so
                // they can be started without looking for them — or why they
                // could not be written, since the files are there either way.
                match pac.scripts.as_ref() {
                    Some(Ok(names)) if !names.is_empty() => {
                        content = content.push(
                            text(l10n.tr_with_args(
                                "pac-spd-written",
                                &[("files", names.join(", "))],
                            ))
                            .size(12.0)
                            .width(Fill)
                            .wrapping(Wrapping::WordOrGlyph),
                        );
                    }
                    Some(Err(error)) => {
                        content = content.push(
                            text(l10n.tr_with_args(
                                "pac-spd-failed",
                                &[("error", error.clone())],
                            ))
                            .size(12.0)
                            .width(Fill)
                            .wrapping(Wrapping::WordOrGlyph)
                            .style(iced::widget::text::danger),
                        );
                    }
                    _ => {}
                }
            }
            Err(error) => content = content.push(error_text(l10n, error)),
        }
    }

    // Footer: the job starts here and Cancel closes the dialog. Nothing runs
    // until a package whose table was read has been picked together with an
    // output folder and at least one of its files, and the dialog is never
    // closed while it is working — a button without `on_press` is greyed out
    // and ignores clicks.
    content = content.push(horizontal_rule(1));

    let ready = !busy
        && pac.result.is_none()
        && pac.package.is_some()
        && pac.file.is_some()
        && pac.output.is_some()
        && pac.chosen() > 0;

    let start = button(text(l10n.tr("pac-unpack-button"))).width(Fill);
    let start: Element<'_, Message> = if ready {
        start.on_press(Message::PacStart).into()
    } else {
        start.into()
    };

    let cancel = button(text(l10n.tr("login-cancel"))).width(Fill);
    let cancel: Element<'_, Message> = if busy {
        cancel.into()
    } else {
        cancel.on_press(Message::PacCancel).into()
    };

    content = content.push(row![start, cancel].spacing(8).width(Fill));

    // The card fills the window, the way the repack dialog does: the file list
    // is the only part that grows, so a package with dozens of files uses the
    // height instead of being squeezed into a corner of it.
    container(content.padding(16))
        .width(Fill)
        .height(Fill)
        .style(darker_card)
        .into()
}

/// The option that also writes the `spd_dump` scripts next to the files.
///
/// They are rendered from the package's own configuration XML, and their
/// commands name the two download agents — so the option can only be asked for
/// while both of those files are part of what the run writes. The box is left
/// greyed out otherwise, and says which of the two is missing.
fn script_row<'a>(
    l10n: &l10n::Bundle,
    package: Option<&'a Package>,
    selected: &'a [bool],
    asked: bool,
    enabled: bool,
) -> Element<'a, Message> {
    let known = package.is_some_and(has_configuration);
    let ready = package.is_some_and(|package| spd_script::can_generate(package, selected));

    let option = checkbox(l10n.tr("pac-spd-script"), asked).text_shaping(Shaping::Advanced);

    // A checkbox without `on_toggle` is greyed out and ignores clicks, which is
    // what this one needs while a job runs or the scripts cannot be written.
    let option = if enabled && ready {
        option.on_toggle(Message::PacScriptToggled)
    } else {
        option
    };

    let mut content = column![option].spacing(4).width(Fill);

    if package.is_some() && !known {
        content = content.push(
            text(l10n.tr("pac-spd-no-config"))
                .size(12.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph)
                .style(warning_orange),
        );
    } else if package.is_some() && !ready {
        content = content.push(
            text(l10n.tr("pac-spd-need-fdl"))
                .size(12.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph)
                .style(warning_orange),
        );
    }

    content.into()
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

/// What the package holds, and which of its files are checked.
fn package_section<'a>(
    l10n: &l10n::Bundle,
    package: &'a Package,
    selected: &'a [bool],
    filter: &'a str,
    enabled: bool,
) -> Element<'a, Message> {
    let mut content = column![].spacing(6).width(Fill).height(Fill);

    // What the package says about itself: the product and its version come
    // from the file, so they are shown as they are. An alias that just repeats
    // the product name is not worth a line of its own.
    let product = package.header.product_name.trim();
    let version = package.header.product_version.trim();
    let alias = package.header.product_alias.trim();

    let title = format!("{product} {version}").trim().to_string();

    if !title.is_empty() {
        content = content.push(text(title).size(14.0));
    }

    if !alias.is_empty() && !alias.eq_ignore_ascii_case(product) {
        content = content.push(text(format!("({alias})")).size(12.0));
    }

    // A package can be an interrupted download: the file table survives, so its
    // files are still listed — say that some of them are missing or cut short,
    // instead of leaving the user to wonder about the sizes.
    if !package.complete {
        content = content.push(
            text(l10n.tr_with_args(
                "pac-partial",
                &[
                    ("found", human_size(package.actual_size)),
                    ("expected", human_size(package.expected_size)),
                ],
            ))
            .size(12.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph)
            .style(warning_orange),
        );
    }

    let total: u64 = package.files.iter().map(|entry| entry.size).sum();

    content = content.push(
        text(l10n.tr_with_args(
            "unpack-contents",
            &[
                ("count", package.files.len().to_string()),
                ("size", human_size(total)),
            ],
        ))
        .size(13.0),
    );

    // The search box only filters what the list shows; the selection keeps the
    // indexes into `package.files`, so it survives changing the query.
    let search_placeholder = l10n.tr("pac-search");
    let search = text_input(&search_placeholder, filter)
        .padding(6)
        .width(Fill);

    // A text input without `on_input` is greyed out and cannot be typed into,
    // which is what the box needs while a job runs.
    let search = if enabled {
        search.on_input(Message::PacFilterChanged)
    } else {
        search
    };

    content = content.push(search);

    let visible: Vec<usize> = package
        .files
        .iter()
        .enumerate()
        .filter(|(_, entry)| matches(&entry.name, filter))
        .map(|(index, _)| index)
        .collect();

    if visible.is_empty() {
        content = content.push(
            text(l10n.tr_with_args(
                "pac-no-match",
                &[("query", filter.trim().to_owned())],
            ))
            .size(12.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph),
        );
    } else {
        let files = iced::widget::Column::with_children(
            visible
                .iter()
                .map(|&index| -> Element<'a, Message> {
                    let entry = &package.files[index];

                    // A file the missing tail cut short is written with the
                    // bytes that are there, and says so.
                    let label = if entry.complete {
                        format!("{}  ({})", entry.name, human_size(entry.size))
                    } else {
                        format!(
                            "{}  ({})  {}",
                            entry.name,
                            human_size(entry.size),
                            l10n.tr("pac-incomplete")
                        )
                    };

                    let file = checkbox(label, selected.get(index).copied().unwrap_or(false))
                        .text_shaping(Shaping::Advanced);

                    // A checkbox without `on_toggle` is greyed out and ignores
                    // clicks, which is what the list needs while a job runs.
                    let file = if enabled {
                        file.on_toggle(move |checked| Message::PacEntryToggled(index, checked))
                    } else {
                        file
                    };

                    file.into()
                }),
        )
        .spacing(2)
        .align_x(Alignment::Start);

        content = content.push(
            container(
                scrollable(
                    // The padding is inside the scrollable, so the scrollbar
                    // floats over this gutter instead of over the longest file
                    // names.
                    container(files)
                        .padding([0.0, LIST_BAR_GUTTER])
                        .width(Fill),
                )
                .id(iced::widget::scrollable::Id::new(LIST_ID))
                .height(Fill)
                .width(Fill),
            )
            .padding(6)
            .height(Fill)
            .style(container::rounded_box),
        );
    }

    if enabled {
        content = content.push(
            row![
                button(text(l10n.tr("pac-select-all")).size(13.0))
                    .padding([4.0, 10.0])
                    .on_press(Message::PacSelectAll(true)),
                button(text(l10n.tr("pac-select-none")).size(13.0))
                    .padding([4.0, 10.0])
                    .on_press(Message::PacSelectAll(false)),
            ]
            .spacing(8),
        );
    }

    let chosen = selected.iter().filter(|checked| **checked).count();

    if chosen == 0 {
        content = content.push(
            text(l10n.tr("pac-none-chosen"))
                .size(12.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph)
                .style(iced::widget::text::danger),
        );
    }

    content.into()
}

/// The localized message for a failed read or run. The two errors a user
/// actually runs into — a file that is not this format, and a package whose
/// file table is encrypted — get their own lines.
fn error_text<'a>(l10n: &l10n::Bundle, error: &PacError) -> Element<'a, Message> {
    let message = match error {
        PacError::NotAPac => l10n.tr("pac-invalid"),
        PacError::Encrypted => l10n.tr("pac-encrypted"),
        other => l10n.tr_with_args("pac-failed", &[("error", other.to_string())]),
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
