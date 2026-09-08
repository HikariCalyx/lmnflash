//! The "Guided" bootloader-unlock wizard (Mode 2, Smartphone Flash).
//!
//! This is a step-by-step alternative to the Manual flow. It walks the user
//! through: 1) signing in to Motorola in an embedded webview child window,
//! 2) reading the Device ID from fastboot, 3) requesting the unlock key from
//! Motorola (sent by e-mail), and 4) entering the key to unlock the phone.
//!
//! Like `bootloader`, this module only *presents* state and sends messages;
//! the state and update handlers live in `crate` (`GuidedState` inside
//! `State::flash.bootloader`). Steps 2 and 4 reuse the shared read / unlock
//! machinery (and its status fields), so the same fastboot commands and error
//! classification apply in both flows.

use iced::widget::{
    button, canvas, column, container, horizontal_rule, mouse_area, row,
    scrollable, text_input, Space,
};
use iced::widget::text::Wrapping;
use iced::{Alignment, Element, Fill};

use crate::bootloader::{bright_success, darker_card, device_picker_card, warning_orange};
use crate::text;
use crate::{GuidedStep, Message, State};

/// Renders the Guided wizard overlay over the whole window (including the tab
/// bar), or `None` if it should not be shown. A device picker and the
/// "request unlock key?" confirm prompt are stacked on top of the wizard card
/// when relevant.
pub(crate) fn overlay(state: &State) -> Option<Element<'_, Message>> {
    let bootloader = &state.flash.bootloader;

    let mut children: Vec<Element<'_, Message>> = vec![
        mouse_area(Space::new(Fill, Fill))
            .on_press(Message::BootloaderBackdropPressed)
            .into(),
    ];
    children.push(
        container(guided_card(state))
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill)
            .into(),
    );

    // Several fastboot devices connected: pick the target first.
    if bootloader.picker.is_some() {
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

    // The "Requesting an unlock key voids the warranty" confirm prompt.
    if bootloader.guided.confirming_request {
        children.push(
            mouse_area(Space::new(Fill, Fill))
                .on_press(Message::BootloaderBackdropPressed)
                .into(),
        );
        children.push(
            container(request_confirm_card(state))
                .width(Fill)
                .height(Fill)
                .center_x(Fill)
                .center_y(Fill)
                .into(),
        );
    }

    Some(iced::widget::Stack::with_children(children).into())
}

/// The wizard card, sized like the manual flow's card. Its contents change
/// with the current step.
fn guided_card(state: &State) -> Element<'_, Message> {
    let step = state.flash.bootloader.guided.step;
    let content: Element<'_, Message> = match step {
        GuidedStep::Login => login_card(state),
        GuidedStep::Read => read_card(state),
        GuidedStep::Request => request_card(state),
        GuidedStep::Key => key_card(state),
    };

    container(scrollable(content).width(Fill).height(Fill))
        .width(640)
        .height(560)
        .padding(16)
        .style(darker_card)
        .into()
}

fn return_button(l10n: &crate::l10n::Bundle) -> Element<'_, Message> {
    button(text(format!(
        "< {}",
        l10n.tr("flash-bootloader-return")
    )))
    .on_press(Message::BootloaderReturnToChooser)
    .into()
}

/// Step 1 — sign in to Motorola (embedded webview child window) and show the
/// logged-in account before continuing.
fn login_card(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let guided = &state.flash.bootloader.guided;

    let mut content = column![
        return_button(l10n),
        text(l10n.tr("guided-title")).size(18.0),
        text(l10n.tr("guided-intro"))
            .size(14.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph),
        text(l10n.tr("guided-click-login"))
            .size(14.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph),
    ]
    .spacing(10)
    .width(Fill);

    if guided.logged_in {
        if guided.fetching_account {
            // The account profile is still being read after the login.
            content = content.push(
                row![
                    spinner(state.anim_tick),
                    text(l10n.tr("guided-getting-account"))
                        .size(14.0)
                        .width(Fill)
                        .wrapping(Wrapping::WordOrGlyph),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
        } else if let (Some(name), Some(email)) =
            (&guided.account_name, &guided.account_email)
        {
            content = content.push(
                text(l10n.tr_with_args(
                    "guided-logged-in-as",
                    &[("name", name.clone()), ("email", email.clone())],
                ))
                .size(14.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph)
                .style(bright_success),
            );
        }
    }

    if guided.logging_out {
        content = content.push(
            row![
                spinner(state.anim_tick),
                text(l10n.tr("guided-signing-out")).size(13.0),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );
    } else if guided.logging_in {
        content = content.push(
            row![
                spinner(state.anim_tick),
                text(l10n.tr("guided-opening")).size(13.0),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );
    } else if let Some(error) = &guided.login_error {
        content = content.push(text(error.clone()).size(13.0).style(iced::widget::text::danger));
    }

    let label = if guided.logged_in {
        "guided-change-account"
    } else {
        "guided-login"
    };
    let login_button = if guided.logging_in || guided.logging_out {
        button(text(l10n.tr(label)))
    } else {
        button(text(l10n.tr(label))).on_press(Message::GuidedLogin)
    };
    content = content.push(login_button);

    if guided.logged_in {
        content = content
            .push(Space::with_height(12.0))
            .push(
                button(text(l10n.tr("guided-next")))
                    .width(Fill)
                    .on_press(Message::GuidedNext),
            );
    }

    content.into()
}

/// Step 2 — read the Device ID from fastboot and request the unlock key.
fn read_card(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let bootloader = &state.flash.bootloader;
    let busy = bootloader.reading || bootloader.unlocking;

    let mut content = column![
        return_button(l10n),
        text(l10n.tr("guided-read-desc"))
            .size(14.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph),
        text(l10n.tr("flash-bootloader-device-id")).size(13.0),
        row![
            text_input("", &bootloader.device_id).width(Fill),
            if busy {
                button(text(l10n.tr("flash-bootloader-read")))
            } else {
                button(text(l10n.tr("flash-bootloader-read")))
                    .on_press(Message::BootloaderReadRequested)
            },
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Fill),
    ]
    .spacing(10)
    .width(Fill);

    // Status of the Device ID read.
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

    // Motorola unlock-eligibility (checked automatically after the read).
    if bootloader.checking {
        content = content.push(text(l10n.tr("flash-bootloader-checking")).size(13.0));
    } else if let Some(eligible) = bootloader.eligible {
        let (id, style): (&str, fn(&iced::Theme) -> iced::widget::text::Style) =
            if eligible {
                ("flash-bootloader-eligible", bright_success)
            } else {
                ("flash-bootloader-not-eligible", warning_orange)
            };
        content = content
            .push(text(l10n.tr(id)).size(13.0).style(style));
    }

    // Request the unlock key by e-mail once the Device ID is known.
    let request_button = if busy || bootloader.device_id.trim().is_empty() {
        button(text(l10n.tr("guided-request-unlock-key"))).width(Fill)
    } else {
        button(text(l10n.tr("guided-request-unlock-key")))
            .width(Fill)
            .on_press(Message::GuidedRequestKey)
    };
    content = content
        .push(horizontal_rule(1))
        .push(text(l10n.tr("guided-request-hint"))
            .size(13.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph))
        .push(request_button);

    content.into()
}

/// Step 3 — the "Requesting an Unlock Key voids the warranty" prompt, plus
/// the progress / error state while the request is submitted.
fn request_card(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let guided = &state.flash.bootloader.guided;

    let mut content = column![
        return_button(l10n),
        text(l10n.tr("guided-request-heading")).size(18.0),
    ]
    .spacing(10)
    .width(Fill);

    if guided.requesting {
        content = content.push(
            row![
                spinner(state.anim_tick),
                text(l10n.tr("guided-requesting")).size(13.0),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );
    } else if let Some(error) = &guided.request_error {
        content = content
            .push(text(error.clone()).size(13.0).style(iced::widget::text::danger))
            .push(
                button(text(l10n.tr("guided-try-again")))
                    .on_press(Message::GuidedRequestKey),
            );
    } else {
        // Only visible briefly while the confirm prompt sits on top.
        content = content.push(text(l10n.tr("guided-request-pending")).size(13.0));
    }

    content.into()
}

/// The modal prompt confirming that requesting an Unlock Key voids the
/// phone's warranty.
fn request_confirm_card(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;

    container(
        column![
            text(l10n.tr("guided-request-confirm"))
                .size(14.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph),
            row![
                button(text(l10n.tr("guided-yes")))
                    .on_press(Message::GuidedRequestConfirmYes),
                button(text(l10n.tr("guided-no")))
                    .on_press(Message::GuidedRequestConfirmNo),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        ]
        .spacing(14)
        .width(Fill),
    )
    .width(420)
    .padding(18)
    .style(darker_card)
    .into()
}

/// Step 4 — check the inbox for the key e-mail and unlock the phone.
fn key_card(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let bootloader = &state.flash.bootloader;
    let guided = &bootloader.guided;
    let busy = bootloader.reading || bootloader.unlocking;

    let email = guided.account_email.clone().unwrap_or_default();
    let mut content = column![
        return_button(l10n),
        text(l10n.tr_with_args("guided-email-check", &[("email", email)]))
            .size(14.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph),
        horizontal_rule(1),
        text(l10n.tr("guided-key-desc")).size(14.0),
    ]
    .spacing(10)
    .width(Fill);

    // The unlock key is a separate code returned by Motorola's website — the
    // Device ID itself is never the key (mirrors the Manual flow's guard).
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
        button(text(l10n.tr("flash-bootloader-unlock")))
    } else {
        button(text(l10n.tr("flash-bootloader-unlock")))
            .on_press(Message::BootloaderUnlockRequested)
    };

    content = content.push(
        row![
            // Right-click pastes the clipboard into the field (for users who
            // don't use Ctrl+V).
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

    content.into()
}

/// A small circular loading spinner (8 dots orbiting). Pure drawing — the
/// rotation is driven from `state.anim_tick`, which a time subscription in the
/// parent advances only while a spinner is visible, so this widget never has
/// to tick itself.
struct Spinner {
    /// Progress around one full revolution, in `0..1`.
    phase: f32,
}

impl<Message> canvas::Program<Message> for Spinner {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        theme: &iced::Theme,
        bounds: iced::Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        use std::f32::consts::TAU;

        let mut frame = canvas::Frame::new(renderer, bounds.size());

        let base = theme.palette().text;
        let center = frame.center();
        let size = bounds.width.min(bounds.height);
        let radius = size * 0.35;
        let dot_radius = (size * 0.08).max(0.8);

        const DOTS: usize = 8;
        let head = (self.phase * DOTS as f32) as usize % DOTS;

        for index in 0..DOTS {
            // The dot at the head is brightest; the ones trailing behind it
            // fade out.
            let back = (head + DOTS - index) % DOTS;
            let alpha = if back == 0 {
                1.0
            } else {
                1.0 - back as f32 / DOTS as f32
            };

            let angle = index as f32 / DOTS as f32 * TAU;
            let position = iced::Point::new(
                center.x + angle.cos() * radius,
                center.y + angle.sin() * radius,
            );
            let path = canvas::Path::circle(position, dot_radius);
            frame.fill(&path, iced::Color { a: alpha, ..base });
        }

        vec![frame.into_geometry()]
    }
}

/// A 16×16 spinner driven by the app's animation tick.
fn spinner<'a>(anim_tick: u64) -> Element<'a, Message> {
    let phase = (anim_tick % 32) as f32 / 32.0;
    canvas::Canvas::new(Spinner { phase })
        .width(16)
        .height(16)
        .into()
}
