//! The "Connect ADB Wireless Debugging" dialog (Mode 2, Smart Device
//! Flashing).
//!
//! Pressing the feature tile's button opens this modal: it pairs the computer
//! with a phone that has wireless debugging on, either by showing a QR code
//! for the phone to scan or by taking the six-digit pairing code the phone
//! displays, and then reports the connection.
//!
//! Like the other dialogs this module only *presents* state and sends messages;
//! the flow state and its update handlers live in `crate`
//! (`State::flash.wireless`), and the pairing itself in [`crate::wireless`].

use iced::widget::text::{Shaping, Wrapping};
use iced::widget::{
    button, canvas, column, container, mouse_area, pick_list, row, scrollable,
    text_input,
};
use iced::{Alignment, Fill};

use crate::bootloader::{bright_success, darker_card, warning_orange};
use crate::compat_iced as iced;
use crate::compat_iced::*;
use crate::guided::spinner;
use crate::text;
use crate::wireless::{self, PairOutcome};
use crate::{DisconnectOutcome, Labeled, Message, State, WirelessMethod};

/// The side of the QR code drawing, in pixels (quiet zone included): big
/// enough for a phone camera at arm's length to read it comfortably.
const QR_SIZE: f32 = 240.0;

/// The quiet zone a QR code needs around its modules, in modules.
const QUIET_ZONE: f32 = 4.0;

/// Whether the "Connect ADB Wireless Debugging" dialog is open.
pub(crate) fn is_open(state: &State) -> bool {
    state.flash.wireless.open
}

/// Renders the dialog over the app, or `None` when it is closed. Clicks on
/// empty space are swallowed (they neither dismiss the dialog nor reach the UI
/// underneath); the modal is left only via its own buttons.
pub(crate) fn overlay(state: &State) -> Option<Element<'_, Message>> {
    if !is_open(state) {
        return None;
    }

    let children: Vec<Element<'_, Message>> = vec![
        mouse_area(Space::new(Fill, Fill))
            .on_press(Message::WirelessBackdropPressed)
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

/// The modal card: the two pairing methods, the one that is chosen, and the
/// status of the last attempt.
fn card(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let wireless = &state.flash.wireless;
    let busy = wireless.busy();

    // The two ways to pair, as a pair of buttons of which one is chosen.
    let method = |choice: WirelessMethod, id: &str| -> Element<'_, Message> {
        let label = container(text(l10n.tr(id)))
            .width(Fill)
            .center_x(Fill);

        let tab = button(label).width(Fill);
        let tab = if busy {
            tab
        } else {
            tab.on_press(Message::WirelessMethodSelected(choice))
        };

        if wireless.method == choice {
            tab.style(button::primary)
        } else {
            tab
        }
        .into()
    };

    let mut content = column![
        text(l10n.tr("flash-wireless-title")).size(18.0),
        text(l10n.tr("wireless-desc"))
            .size(14.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph),
        horizontal_rule(1),
        row![
            method(WirelessMethod::Qr, "wireless-method-qr"),
            method(WirelessMethod::Code, "wireless-method-code"),
        ]
        .spacing(8)
        .width(Fill),
        body(state),
    ]
    .spacing(10)
    .width(Fill);

    content = content.push(horizontal_rule(1));

    // The footer holds everything the user can do next, on one row: the chosen
    // method's action, the "Disconnect" menu, and [Cancel]. Each takes the same
    // share of the row, so none of them stretches into a button far wider than
    // its label. A running job greys the buttons out (iced renders a button
    // without `on_press` disabled and ignores clicks on it); the menu cannot be
    // greyed out that way, so the handler drops its picks instead.
    let action = match wireless.method {
        WirelessMethod::Qr => {
            let action = button(text(l10n.tr("wireless-qr-regenerate"))).width(Fill);

            if busy {
                action
            } else {
                action.on_press(Message::WirelessCodeRegenerated)
            }
        }
        WirelessMethod::Code => {
            let action = button(text(l10n.tr("wireless-pair"))).width(Fill);

            if wireless.ready_to_pair() {
                action.on_press(Message::WirelessPairRequested)
            } else {
                action
            }
        }
    };

    let disconnect = disconnect_menu(state);

    let cancel = button(text(l10n.tr("login-cancel"))).width(Fill);
    let cancel = if busy {
        cancel
    } else {
        cancel.on_press(Message::WirelessCancel)
    };

    content = content.push(
        row![action, disconnect, cancel]
            .spacing(8)
            .width(Fill)
            .align_y(Alignment::Center),
    );

    // The padding sits inside the scrollable, so the card's border is the
    // viewport: the scrollbar rides on the right border instead of floating
    // in the middle of the padding.
    container(
        scrollable(container(content).padding(16).width(Fill))
            .width(Fill)
            .height(Fill),
    )
    .width(520)
    .height(580)
    .style(darker_card)
    .into()
}

/// The chosen method's form, and the status of the last attempt under it.
fn body(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let wireless = &state.flash.wireless;

    let mut content = column![match wireless.method {
        WirelessMethod::Qr => qr_form(state),
        WirelessMethod::Code => code_form(state),
    }]
    .spacing(10)
    .width(Fill);

    if wireless.busy() {
        // Which action is running decides what the line says.
        let status = if wireless.disconnecting {
            l10n.tr("wireless-disconnecting")
        } else {
            l10n.tr("wireless-working")
        };

        content = content.push(busy_row(state, status));
    } else if let Some(result) = &wireless.disconnect_result {
        content = content.push(match result {
            DisconnectOutcome::Done(wireless::DisconnectTarget::Device(address)) => {
                success(l10n.tr_with_args(
                    "wireless-disconnected",
                    &[("device", address.clone())],
                ))
            }
            DisconnectOutcome::Done(wireless::DisconnectTarget::Everything) => {
                success(l10n.tr("wireless-disconnected-all"))
            }
            DisconnectOutcome::Failed(error) => danger(
                &l10n.tr_with_args("wireless-disconnect-failed", &[("error", error.clone())]),
            ),
        });
    } else if let Some(result) = &wireless.result {
        content = content.push(match result {
            Ok(PairOutcome::Connected(device)) => success(l10n.tr_with_args(
                "wireless-connected",
                &[("device", device.label())],
            )),
            Ok(PairOutcome::Paired) => success(l10n.tr("wireless-paired")),
            Err(error) => danger(
                &l10n.tr_with_args("wireless-failed", &[("error", error.clone())]),
            ),
        });
    }

    if let Some(error) = &wireless.discover_error {
        content = content.push(danger(
            &l10n.tr_with_args("wireless-mdns-failed", &[("error", error.clone())]),
        ));
    }

    content.into()
}

/// The QR code, the instructions for scanning it, and the button that replaces
/// it with a fresh one.
fn qr_form(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let wireless = &state.flash.wireless;

    let mut content = column![text(l10n.tr("wireless-qr-desc"))
        .size(13.0)
        .width(Fill)
        .wrapping(Wrapping::WordOrGlyph)]
    .spacing(10)
    .width(Fill)
    .align_x(Alignment::Center);

    if let Some(image) = &wireless.qr {
        content = content.push(container(qr_code(image)).center_x(Fill));
    }

    // The phone tells us it scanned the code by announcing the pairing service
    // the code named (see `wireless::matches`); until then there is nothing to
    // do but wait, so the hint goes away with the result.
    if wireless.result.is_none() && wireless.disconnect_result.is_none() && !wireless.busy() {
        content = content.push(text(l10n.tr("wireless-qr-waiting")).size(13.0));
    }

    content.into()
}

/// The list of phones waiting for a pairing code, the code itself, and the
/// button that pairs with them.
fn code_form(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let wireless = &state.flash.wireless;
    let busy = wireless.busy();

    let mut content = column![text(l10n.tr("wireless-code-desc"))
        .size(13.0)
        .width(Fill)
        .wrapping(Wrapping::WordOrGlyph)]
    .spacing(10)
    .width(Fill);

    // The phones that announced a pairing service, as a dropdown.
    if wireless.pairing_services.is_empty() {
        content = content.push(notice(l10n.tr("wireless-no-service")));
    } else {
        let options: Vec<Labeled<String>> = wireless
            .pairing_services
            .iter()
            .map(|service| Labeled {
                value: service.fullname.clone(),
                label: service.label(),
            })
            .collect();
        let selected = wireless.chosen.as_ref().and_then(|chosen| {
            options
                .iter()
                .find(|option| &option.value == chosen)
                .cloned()
        });

        // iced 0.14 needs the theme spelled out for a `pick_list` (see the
        // repo's note on type inference in widget rows).
        let picker: iced::widget::PickList<
            '_,
            Labeled<String>,
            Vec<Labeled<String>>,
            Labeled<String>,
            Message,
            crate::fluent::Theme,
        > = pick_list(options, selected, |option| {
            Message::WirelessServiceSelected(option.value)
        })
        .placeholder(l10n.tr("wireless-code-device"))
        .text_shaping(Shaping::Advanced)
        .width(Fill);

        content = content.push(picker);
    }

    // The six-digit code the phone shows.
    let placeholder = l10n.tr("wireless-code-placeholder");
    let input = text_input(placeholder.as_str(), &wireless.code_input).width(Fill);
    let input = if busy {
        input
    } else {
        input.on_input(Message::WirelessCodeChanged)
    };

    content = content.push(input);

    // The [Pair] button sits in the footer; this hint explains why it is greyed
    // out while the phone's code is still missing.
    if !busy && wireless.manual_service().is_some() && wireless.code_input.trim().is_empty() {
        content = content.push(notice(l10n.tr("wireless-code-required")));
    }

    content.into()
}

/// The "Disconnect" menu: one entry per wireless device `adb` lists, and last
/// the entry that drops all of them.
///
/// It is a `pick_list` built with no selection, so it always shows the
/// "Disconnect" placeholder and acts as a menu: iced publishes `on_select` for
/// every option click, so picking the same entry twice runs it again.
fn disconnect_menu(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;

    let options: Vec<Labeled<wireless::DisconnectTarget>> = state
        .flash
        .wireless
        .disconnect_options()
        .into_iter()
        .map(|target| Labeled {
            label: match &target {
                wireless::DisconnectTarget::Device(serial) => serial.clone(),
                wireless::DisconnectTarget::Everything => {
                    l10n.tr("wireless-disconnect-all")
                }
            },
            value: target,
        })
        .collect();

    // iced 0.14 needs the theme spelled out for a `pick_list` (see the repo's
    // note on type inference in widget rows).
    let menu: iced::widget::PickList<
        '_,
        Labeled<wireless::DisconnectTarget>,
        Vec<Labeled<wireless::DisconnectTarget>>,
        Labeled<wireless::DisconnectTarget>,
        Message,
        crate::fluent::Theme,
    > = pick_list(options, None, |option| {
        Message::WirelessDisconnectSelected(option.value)
    })
    .placeholder(l10n.tr("wireless-disconnect"))
    .text_shaping(Shaping::Advanced)
    .style(crate::firmware_flash::menu_style)
    .width(Fill);

    menu.into()
}

/// The QR code, drawn module by module.
fn qr_code(image: &wireless::QrImage) -> Element<'_, Message> {
    canvas::Canvas::new(QrCode {
        image: image.clone(),
    })
    .width(QR_SIZE)
    .height(QR_SIZE)
    .into()
}

/// Draws a [`wireless::QrImage`] as black modules on a white square.
///
/// The colours are fixed: a QR code has to stay dark on light to be readable,
/// whatever the window theme is doing.
struct QrCode {
    image: wireless::QrImage,
}

impl<Message> canvas::Program<Message, crate::fluent::Theme> for QrCode {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &crate::fluent::Theme,
        bounds: iced::Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());

        // The drawing is square and centred in the widget, and the quiet zone
        // around the modules is part of it.
        let side = bounds.width.min(bounds.height);
        let offset = iced::Point::new((bounds.width - side) / 2.0, (bounds.height - side) / 2.0);
        let modules = self.image.width as f32 + 2.0 * QUIET_ZONE;
        let cell = side / modules;

        frame.fill_rectangle(
            iced::Point::ORIGIN,
            iced::Size::new(bounds.width, bounds.height),
            iced::Color::WHITE,
        );

        for y in 0..self.image.width {
            for x in 0..self.image.width {
                if !self.image.dark(x, y) {
                    continue;
                }

                // A hair wider than a cell, so rounding cannot leave seams
                // between neighbouring modules.
                frame.fill_rectangle(
                    iced::Point::new(
                        offset.x + (QUIET_ZONE + x as f32) * cell,
                        offset.y + (QUIET_ZONE + y as f32) * cell,
                    ),
                    iced::Size::new(cell + 0.5, cell + 0.5),
                    iced::Color::BLACK,
                );
            }
        }

        vec![frame.into_geometry()]
    }
}

/// A spinner plus its status text.
fn busy_row<'a>(state: &'a State, label: String) -> Element<'a, Message> {
    row![spinner(state.anim_tick), text(label).size(13.0)]
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
}

/// A green success line.
fn success(message: String) -> Element<'static, Message> {
    text(message)
        .size(13.0)
        .width(Fill)
        .wrapping(Wrapping::WordOrGlyph)
        .style(bright_success)
        .into()
}

/// A red failure line.
fn danger(error: &str) -> Element<'static, Message> {
    text(error.to_owned())
        .size(13.0)
        .width(Fill)
        .wrapping(Wrapping::WordOrGlyph)
        .style(iced::widget::text::danger)
        .into()
}

/// An orange hint (no phone found yet, no code typed, …).
fn notice(message: String) -> Element<'static, Message> {
    text(message)
        .size(13.0)
        .width(Fill)
        .wrapping(Wrapping::WordOrGlyph)
        .style(warning_orange)
        .into()
}
