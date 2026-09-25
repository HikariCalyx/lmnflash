//! The "Install Driver" dialog (Mode 2, Smartphone Flash).
//!
//! Pressing the feature tile's button opens this modal. On Windows the
//! download of Motorola's Mobile Drivers starts right away and the dialog
//! shows how far it is; on Linux the dialog asks for the password that
//! installs the udev rules, since writing to `/etc` needs root. Like the other
//! dialogs, this module only *presents* the state and sends messages — the
//! flow state and its update handlers live in `crate`
//! (`State::flash.driver`).

use iced::widget::text::Wrapping;
use iced::widget::{
    button, column, container, horizontal_rule, mouse_area, progress_bar, row,
    scrollable, text_input, Space,
};
use iced::{Alignment, Element, Fill};

use crate::bootloader::{bright_success, darker_card};
use crate::driver_install::{self, DriverError};
use crate::guided::spinner;
use crate::l10n;
use crate::text;
use crate::{DriverDialog, DriverStage, Message, State};

/// Height of the download progress bar. A bar is a status line, not a button:
/// at the widget's 30 px default it drew more attention than the text it
/// belongs to.
const BAR_HEIGHT: f32 = 6.0;

/// Renders the "Install Driver" overlay over the app, or `None` when the
/// dialog is closed. Clicks on empty space are swallowed (they neither dismiss
/// the dialog nor reach the UI underneath); the modal is left only via its own
/// buttons.
pub(crate) fn overlay(state: &State) -> Option<Element<'_, Message>> {
    if state.flash.driver.dialog != DriverDialog::Open {
        return None;
    }

    let children: Vec<Element<'_, Message>> = vec![
        mouse_area(Space::new(Fill, Fill))
            .on_press(Message::DriverBackdropPressed)
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

/// The modal card: what will be installed, the progress (or the password
/// prompt), and the status of the running installation.
fn card(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let driver = &state.flash.driver;
    let strings = strings();
    let linux = driver_install::uses_password();

    let mut content = column![
        text(l10n.tr("flash-driver-title")).size(18.0),
        text(l10n.tr(strings.desc))
            .size(14.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph),
        horizontal_rule(1),
    ]
    .spacing(10)
    .width(Fill);

    if let Some(error) = &driver.error {
        content = content.push(
            text(error_text(l10n, error))
                .size(13.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph)
                .style(iced::widget::text::danger),
        );
    } else if driver.stage == DriverStage::Done {
        content = content
            .push(text(l10n.tr(strings.done)).size(13.0).style(bright_success));
    } else if driver.busy {
        let label = if driver.stage == DriverStage::Installing {
            strings.installing
        } else {
            strings.downloading
        };

        content = content.push(busy_row(state, l10n.tr(label)));

        // The size is unknown only while a server sends no length at all.
        if driver.stage == DriverStage::Downloading && driver.total > 0 {
            content = content.push(bar(driver.downloaded as f32 / driver.total as f32));
        }
    }

    // On Linux the installation starts from here, so the password has to be
    // entered first; Windows needs nothing and starts at once. The field stays
    // on screen after a failure — a rejected password is what has to be tried
    // again.
    if linux && !driver.busy && driver.stage != DriverStage::Done {
        content = content
            .push(
                row![
                    text(l10n.tr("driver-password")).size(13.0),
                    // The password is never shown, and never leaves this
                    // process except through `sudo`.
                    text_input("", &driver.password)
                        .secure(true)
                        .padding(6)
                        .width(Fill)
                        .on_input(Message::DriverPasswordChanged)
                        .on_submit(Message::DriverPasswordSubmitted),
                ]
                .spacing(8)
                .align_y(Alignment::Center)
                .width(Fill),
            )
            .push(install_button(state, !driver.password.is_empty()));
    }

    // Footer: Cancel closes the dialog, but never while the installation is
    // running.
    content = content.push(horizontal_rule(1));

    let cancel = button(text(l10n.tr("login-cancel"))).width(Fill);

    content = content.push(if driver.busy {
        cancel
    } else {
        cancel.on_press(Message::DriverCancel)
    });

    // The padding sits inside the scrollable, so the card's border is the
    // viewport: the scrollbar rides on the right border instead of floating
    // in the middle of the padding.
    container(
        scrollable(container(content).padding(16).width(Fill))
            .width(Fill)
            .height(Fill),
    )
    .width(520)
    .height(360)
    .style(darker_card)
    .into()
}

/// The button that starts the installation (Linux only; the Windows flow has
/// nothing to confirm).
fn install_button(state: &State, ready: bool) -> Element<'_, Message> {
    let install = button(text(state.l10n.tr("driver-install-button"))).width(Fill);

    if ready {
        install.on_press(Message::DriverPasswordSubmitted).into()
    } else {
        // Nothing to install with: iced greys a button without `on_press` out
        // and ignores clicks on it.
        install.into()
    }
}

/// The localized message for a failed installation.
fn error_text(l10n: &l10n::Bundle, error: &DriverError) -> String {
    match error {
        DriverError::WrongPassword => l10n.tr("driver-error-password"),
        DriverError::NoSudo => l10n.tr("driver-error-no-sudo"),
        DriverError::Failed(message) => {
            l10n.tr_with_args("driver-failed", &[("error", message.clone())])
        }
    }
}

/// The strings that differ between the two supported systems.
struct Strings {
    /// What the dialog explains it will do.
    desc: &'static str,
    /// Shown while the download runs (Windows).
    downloading: &'static str,
    /// Shown while the installer (or the rules) runs.
    installing: &'static str,
    /// Shown once the installation is done.
    done: &'static str,
}

fn strings() -> Strings {
    if driver_install::uses_password() {
        Strings {
            desc: "driver-linux-desc",
            downloading: "driver-downloading",
            installing: "driver-installing-linux",
            done: "driver-done-linux",
        }
    } else {
        Strings {
            desc: "driver-desc",
            downloading: "driver-downloading",
            installing: "driver-installing",
            done: "driver-done",
        }
    }
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
