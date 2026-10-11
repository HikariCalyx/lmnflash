//! The "Multi Flashing" page (Mode 2).
//!
//! The Firmware Flash dialog offers to leave the modal and hand the user to
//! this full page, where several devices are flashed in one run.
//!
//! The page is still being built: its top row ([Preset Manage] [Start]
//! [Exit]) already drives the run's state machine, while the workspace the
//! layout will show underneath is a placeholder and the "Preset Manage"
//! dialog only collects the firmware images of the run.
//!
//! Like the other feature modules this one only *presents* state and sends
//! messages; the page's state lives in `crate`
//! (`State::flash.multi_flash`).

use iced::widget::text::Wrapping;
use iced::widget::{
    button, column, container, mouse_area, row, scrollable,
};
use iced::{Alignment, Fill};

use crate::bootloader::darker_card;
use crate::compat_iced as iced;
use crate::compat_iced::*;
use crate::text;
use crate::{Message, State};

/// Whether the "Preset Manage" dialog is open.
pub(crate) fn preset_open(state: &State) -> bool {
    state.flash.multi_flash.preset_dialog
}

/// Whether the prompt that blocks closing the window mid-run is up.
pub(crate) fn close_prompt_open(state: &State) -> bool {
    state.flash.multi_flash.close_prompt
}

/// The Multi Flashing workspace, shown in place of the Mode 2 feature grid
/// while `state.flash.multi_flash.open` is set.
pub(crate) fn view(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let multi = &state.flash.multi_flash;
    let running = multi.running;

    // [Preset Manage]. Locked while a run goes on: what is being flashed must
    // not change under it.
    let preset = button(text(l10n.tr("multi-flash-preset-manage"))).width(Fill);
    let preset = if running {
        preset
    } else {
        preset.on_press(Message::MultiFlashPresetManage)
    };

    // [Start] / [Stop]. Starting needs at least one loaded firmware image
    // (iced greys a button without `on_press` out and ignores clicks on it);
    // stopping is the one control a run must keep live.
    let start = button(text(
        l10n.tr(if running {
            "multi-flash-stop"
        } else {
            "multi-flash-start"
        }),
    ))
    .width(Fill);
    let start = if running || !multi.images.is_empty() {
        start.on_press(if running {
            Message::MultiFlashStop
        } else {
            Message::MultiFlashStart
        })
    } else {
        start
    };

    // [Exit]. Locked while a run goes on, so the run cannot be left half
    // done; it has to be stopped first.
    let exit = button(text(l10n.tr("multi-flash-exit"))).width(Fill);
    let exit = if running {
        exit
    } else {
        exit.on_press(Message::MultiFlashExit)
    };

    let content = column![
        row![preset, start, exit].spacing(8).width(Fill),
        text(l10n.tr("multi-flash-placeholder"))
            .size(14.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph),
    ]
    .spacing(16)
    .width(Fill)
    .align_x(Alignment::Start);

    container(content)
        .width(Fill)
        .height(Fill)
        .padding(16)
        .into()
}

/// Renders the "Preset Manage" dialog over the app, or `None` when it is
/// closed. Clicks on empty space are swallowed, so the modal is left only via
/// its own buttons.
pub(crate) fn preset_overlay(state: &State) -> Option<Element<'_, Message>> {
    if !preset_open(state) {
        return None;
    }

    Some(modal(
        preset_card(state),
        Message::MultiFlashPresetBackdropPressed,
    ))
}

/// Renders the "stop the operation before closing" prompt, or `None` when no
/// close was refused.
pub(crate) fn close_prompt_overlay(state: &State) -> Option<Element<'_, Message>> {
    if !close_prompt_open(state) {
        return None;
    }

    Some(modal(
        close_prompt_card(state),
        Message::MultiFlashClosePromptBackdropPressed,
    ))
}

/// Draws a dialog card the way the app's other modals are: a layer that
/// swallows clicks on the empty space, with the card centred over it.
fn modal<'a>(card: Element<'a, Message>, backdrop: Message) -> Element<'a, Message> {
    let children: Vec<Element<'a, Message>> = vec![
        mouse_area(Space::new(Fill, Fill)).on_press(backdrop).into(),
        container(card)
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill)
            .into(),
    ];

    iced::widget::Stack::with_children(children).into()
}

/// The "Preset Manage" dialog: the firmware images the run will flash.
///
/// Which images a run needs and how they are grouped is not settled yet, so
/// the dialog simply collects the files chosen so far.
fn preset_card(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let images = &state.flash.multi_flash.images;

    let mut list: Vec<Element<'_, Message>> = Vec::new();

    if images.is_empty() {
        list.push(text(l10n.tr("multi-flash-preset-empty")).size(14.0).into());
    } else {
        for image in images {
            list.push(
                text(image.display().to_string())
                    .size(12.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph)
                    .into(),
            );
        }
    }

    let content = column![
        text(l10n.tr("multi-flash-preset-manage")).size(18.0),
        horizontal_rule(1),
        column(list).spacing(4).width(Fill),
        horizontal_rule(1),
        row![
            button(text(l10n.tr("multi-flash-preset-add")))
                .width(Fill)
                .on_press(Message::MultiFlashPresetAdd),
            button(text(l10n.tr("login-cancel")))
                .width(Fill)
                .on_press(Message::MultiFlashPresetClose),
        ]
        .spacing(8),
    ]
    .spacing(10)
    .width(Fill);

    // The padding sits inside the scrollable, so the card's border is the
    // viewport and the scrollbar rides on it (the modal convention).
    container(
        scrollable(container(content).padding(16).width(Fill))
            .width(Fill)
            .height(Fill),
    )
    .width(520)
    .height(380)
    .style(darker_card)
    .into()
}

/// The prompt that a window close was refused because a run is going on.
fn close_prompt_card(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;

    let content = column![
        text(l10n.tr("multi-flash-close-prompt"))
            .size(14.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph),
        button(text(l10n.tr("multi-flash-close-prompt-ok")))
            .width(Fill)
            .on_press(Message::MultiFlashClosePromptDismissed),
    ]
    .spacing(16)
    .width(Fill);

    container(container(content).padding(16).width(Fill))
        .width(420)
        .style(darker_card)
        .into()
}
