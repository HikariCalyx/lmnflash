//! The "Firmware Flash" dialog (Mode 2, Smartphone Flash).
//!
//! Pressing the Firmware Flash tile opens this modal: the user picks a
//! firmware package (the factory ZIP, or a `flashfile.xml` with its image
//! files next to it), chooses which tool flashes it (`mfastboot` when one is
//! shipped next to the application, otherwise the built-in fastboot), picks
//! the connected phone, and confirms before every step of the package is
//! written.
//!
//! Like the other dialogs, this module only *presents* state and sends
//! messages; the flow state and its update handlers live in `crate`
//! (`State::flash.firmware`).

use iced::widget::text::Wrapping;
use iced::widget::{
    button, checkbox, column, container, horizontal_rule, mouse_area, pick_list,
    progress_bar, row, scrollable, Space,
};
use iced::widget::text::Shaping;
use iced::{Alignment, Element, Fill};

use crate::bootloader::{bright_success, darker_card, warning_orange};
use crate::fastboot_info::SecureState;
use crate::flash_engine::Engine;
use crate::flashfile::{EraseGroup, FlashPart};
use crate::guided::spinner;
use crate::text;
use crate::{enabled_procedures, FirmwareFlashDialog, Labeled, Message, State};

/// Id of the flashing log's scrollable, so [`crate`] can scroll it to the
/// newest line as lines arrive.
pub(crate) const LOG_SCROLL_ID: &str = "firmware-flash-log";

/// Renders the Firmware Flash overlay over the app, or `None` when the dialog
/// is closed. Clicks on empty space are swallowed (they neither dismiss the
/// dialog nor reach the UI underneath); the modal is left only via Cancel.
pub(crate) fn overlay(state: &State) -> Option<Element<'_, Message>> {
    if state.flash.firmware.dialog != FirmwareFlashDialog::Open {
        return None;
    }

    let children: Vec<Element<'_, Message>> = vec![
        mouse_area(Space::new(Fill, Fill))
            .on_press(Message::FirmwareFlashBackdropPressed)
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

/// The modal card: the setup form, the confirmation, or the running flash
/// with its progress and log.
fn card(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let flash = &state.flash.firmware;
    let busy = flash.loading || flash.running;

    let mut content = column![
        text(l10n.tr("flash-firmware-title")).size(18.0),
        text(l10n.tr("firmware-flash-desc"))
            .size(14.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph),
        horizontal_rule(1),
    ]
    .spacing(10)
    .width(Fill);

    if flash.running || flash.result.is_some() {
        content = content.push(running_section(state));
    } else if flash.editing {
        content = content.push(procedures_section(state));
    } else if flash.confirming {
        content = content.push(confirmation_section(state));
    } else {
        content = content.push(setup_section(state));
    }

    // Footer: Cancel closes the dialog, but never while a job is running so
    // the phone is not left half-flashed.
    content = content.push(horizontal_rule(1));

    let cancel = button(text(l10n.tr("login-cancel"))).width(Fill);
    content = content.push(if busy {
        cancel
    } else {
        cancel.on_press(Message::FirmwareFlashCancel)
    });

    container(scrollable(content).width(Fill).height(Fill))
        .width(560)
        .height(520)
        .padding(16)
        .style(darker_card)
        .into()
}

/// Package selection, tool selection, and device selection.
fn setup_section(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let flash = &state.flash.firmware;
    let busy = flash.loading;

    let pick = button(text(l10n.tr("firmware-flash-select")));
    let pick = if busy {
        pick
    } else {
        pick.on_press(Message::FirmwareFlashPickRequested)
    };

    let path_text = match &flash.package {
        Some(path) => path.display().to_string(),
        None => l10n.tr("firmware-flash-no-package"),
    };

    let mut content = column![row![
        pick,
        container(
            text(path_text)
                .size(12.0)
                .width(380)
                .wrapping(Wrapping::WordOrGlyph),
        )
        .padding(8)
        .style(container::rounded_box),
    ]
    .spacing(8)
    .align_y(Alignment::Center)]
    .spacing(10)
    .width(Fill);

    if flash.loading {
        content = content.push(busy_row(state, l10n.tr("firmware-flash-loading")));

        if let Some((done, total)) = flash.extracted {
            content = content.push(progress_bar(0.0..=1.0, fraction(done, total)));
        }
    } else if let Some(error) = &flash.error {
        content = content.push(
            text(error.clone())
                .size(13.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph)
                .style(iced::widget::text::danger),
        );
    }

    // What the package contains, once it has been read.
    if let Some(package) = &flash.plan {
        let mut fields = column![
            info_row(l10n.tr("firmware-flash-model"), package.model.clone()),
            info_row(
                l10n.tr("firmware-flash-version"),
                package.software_version.clone(),
            ),
            // The carrier/region the package is for (from the flashfile, or
            // read out of its `vbmeta` image).
            info_row(l10n.tr("firmware-flash-cid"), package.cid.clone()),
            // The procedures that will run, with the button that opens the
            // checklist used to pick them.
            row![
                text(l10n.tr("firmware-flash-steps")).size(12.0).width(140),
                text(format!(
                    "{}/{}",
                    enabled_procedures(flash),
                    package.steps.len()
                ))
                .size(12.0)
                .width(Fill),
                button(text(l10n.tr("firmware-flash-edit")).size(12.0))
                    .on_press(Message::FirmwareFlashEditProcedures),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        ]
        .spacing(2)
        .align_x(Alignment::Start);

        // Partitions the package must not write were dropped when it was
        // loaded, so name them rather than letting their absence surprise.
        if !package.ignored_partitions.is_empty() {
            fields = fields.push(
                text(l10n.tr_with_args(
                    "firmware-flash-ignored",
                    &[("partitions", package.ignored_partitions.join(", "))],
                ))
                .size(12.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph)
                .style(warning_orange),
            );
        }

        content = content.push(
            container(fields)
                .padding(8)
                .width(Fill)
                .style(container::rounded_box),
        );
    }

    // Which tool runs the steps.
    let options = engine_options(state);
    let selected = options
        .iter()
        .find(|option| option.value == flash.engine)
        .cloned()
        .unwrap_or_else(|| options[0].clone());
    let engine_picker: iced::widget::PickList<'_, Labeled<Engine>, Vec<Labeled<Engine>>, Labeled<Engine>, Message> =
        pick_list(options, Some(selected), |option| {
            Message::FirmwareFlashEngineSelected(option.value)
        })
        .text_shaping(Shaping::Advanced);

    content = content.push(
        row![
            text(l10n.tr("firmware-flash-tool")).size(13.0),
            engine_picker,
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    );

    // The shipped `mfastboot` is an Intel binary: an ARM machine only runs it
    // through an emulator (Rosetta 2 on macOS, box64 on Linux), which may
    // still have to be installed. Say how to get it instead of letting the
    // first step fail, and keep Start disabled until it (or the built-in
    // engine) is chosen.
    let emulator_hint = emulator_hint(&flash.engine);
    if let Some((needed, install)) = emulator_hint {
        for key in [needed, install] {
            content = content.push(
                text(l10n.tr(key))
                    .size(12.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph)
                    .style(warning_orange),
            );
        }
    }

    content = content
        .push(
            checkbox(
                l10n.tr("firmware-flash-verify"),
                flash.verify_checksums,
            )
            .text_shaping(Shaping::Advanced)
            .on_toggle(Message::FirmwareFlashVerifyToggled),
        )
        .push(device_header(state, busy));

    if flash.listing {
        content = content.push(busy_row(state, l10n.tr("factory-reset-reading-devices")));
    } else if let Some(error) = &flash.list_error {
        content = content.push(
            text(error.clone())
                .size(13.0)
                .style(iced::widget::text::danger),
        );
    } else if flash.devices.is_empty() {
        let error_id = if flash.total == 0 {
            "retcn-fill-fastboot-no-device"
        } else {
            "retcn-fill-fastboot-unsupported-device"
        };

        content = content.push(
            text(l10n.tr(error_id))
                .size(13.0)
                .style(iced::widget::text::danger),
        );
    } else {
        content = content.push(device_picker(state));
    }

    // What the selected phone reports about itself.
    if let Some(info) = device_info_section(state) {
        content = content.push(info);
    }

    // Flashing needs a parsed package, a selected device, at least one
    // checked procedure, and a tool that can actually start (an Intel build
    // whose emulator is missing cannot).
    let ready = flash.plan.is_some()
        && flash.selected.is_some()
        && enabled_procedures(flash) > 0
        && emulator_hint.is_none();
    let start = button(text(l10n.tr("firmware-flash-start"))).width(Fill);
    content = content.push(if ready && !busy {
        start.on_press(Message::FirmwareFlashStart)
    } else {
        start
    });

    content.into()
}

/// The checklist that picks which procedures of the package are executed.
fn procedures_section(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let flash = &state.flash.firmware;

    let Some(package) = &flash.plan else {
        return column![].into();
    };

    let enabled = enabled_procedures(flash);
    let total = package.steps.len();

    let procedures = iced::widget::Column::with_children(
        package
            .steps
            .iter()
            .enumerate()
            .map(|(index, step)| {
                checkbox(
                    format!("{}. {}", index + 1, step.describe()),
                    flash.enabled.get(index).copied().unwrap_or(true),
                )
                .text_shaping(Shaping::Advanced)
                .on_toggle(move |enabled| {
                    Message::FirmwareFlashProcedureToggled(index, enabled)
                })
                .into()
            }),
    )
    .spacing(2)
    .align_x(Alignment::Start);

    let mut content = column![
        row![
            text(l10n.tr("firmware-flash-steps")).size(16.0),
            text(format!("{enabled}/{total}")).size(13.0),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        container(scrollable(procedures).width(Fill).height(250))
            .padding(6)
            .width(Fill)
            .style(container::rounded_box),
        // One button per firmware part, mirroring the menus of the flashing
        // scripts Motorola ships with the firmware.
        row![
            part_button(state, FlashPart::Ap),
            part_button(state, FlashPart::Bp),
            part_button(state, FlashPart::Bl),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        // Standalone erases the package does not run by itself. They are
        // independent of the checklist above, so they toggle.
        row![
            erase_group_button(state, EraseGroup::Userdata),
            erase_group_button(state, EraseGroup::NvCache),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        row![
            button(text(l10n.tr("firmware-flash-select-all")))
                .on_press(Message::FirmwareFlashSelectAllProcedures(true)),
            button(text(l10n.tr("firmware-flash-select-none")))
                .on_press(Message::FirmwareFlashSelectAllProcedures(false)),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    ]
    .spacing(10)
    .width(Fill);

    // Say exactly which partitions the added erases will clear; partition
    // names are technical, so they are not localized.
    let erases = erased_partitions(state);
    if !erases.is_empty() {
        content = content.push(
            text(l10n.tr_with_args(
                "firmware-flash-erase-list",
                &[("partitions", erases.join(", "))],
            ))
            .size(12.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph)
            .style(warning_orange),
        );
    }

    // Nothing checked means there is nothing to flash, so say so.
    if enabled == 0 {
        content = content.push(
            text(l10n.tr("firmware-flash-procedures-none"))
                .size(13.0)
                .style(iced::widget::text::danger),
        );
    }

    content = content.push(
        button(text(format!(
            "< {}",
            l10n.tr("flash-bootloader-return")
        )))
        .on_press(Message::FirmwareFlashEditDone),
    );

    content.into()
}

/// The warning shown after "Start Flashing", before anything is written.
fn confirmation_section(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;

    column![
        text(l10n.tr("firmware-flash-confirm"))
            .size(13.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph)
            .style(warning_orange),
        row![
            button(text(l10n.tr("guided-yes"))).on_press(Message::FirmwareFlashConfirmed),
            button(text(l10n.tr("guided-no"))).on_press(Message::FirmwareFlashBack),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    ]
    .spacing(10)
    .width(Fill)
    .into()
}

/// The running flash: current step, byte progress, and the tool's log.
fn running_section(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let flash = &state.flash.firmware;

    let mut content = column![].spacing(8).width(Fill);

    if flash.running {
        content = content.push(busy_row(state, l10n.tr("firmware-flash-flashing")));

        if flash.step_total > 0 {
            let step = l10n.tr_with_args(
                "firmware-flash-step",
                &[
                    ("index", (flash.step_index + 1).to_string()),
                    ("total", flash.step_total.to_string()),
                    ("step", flash.step_label.clone()),
                ],
            );
            content = content.push(text(step).size(13.0).width(Fill));

            // How far the procedure list is, as a slim bar with the count
            // beside it (the numbers need no translation).
            content = content.push(
                row![
                    progress_bar(
                        0.0..=1.0,
                        (flash.step_index + 1) as f32 / flash.step_total as f32,
                    )
                    .height(6.0)
                    .width(Fill),
                    text(format!("{}/{}", flash.step_index + 1, flash.step_total)).size(12.0),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
        }

        // Bytes of the procedure that is currently running.
        if let Some((done, total)) = flash.progress {
            content = content.push(progress_bar(0.0..=1.0, fraction(done, total)));
        }
    } else if let Some(result) = &flash.result {
        match result {
            Ok(()) => {
                content = content.push(
                    text(l10n.tr("firmware-flash-done"))
                        .size(13.0)
                        .style(bright_success),
                );
            }
            Err(error) => {
                content = content
                    .push(
                        text(l10n.tr("firmware-flash-failed"))
                            .size(13.0)
                            .style(iced::widget::text::danger),
                    )
                    .push(
                        text(error.clone())
                            .size(13.0)
                            .width(Fill)
                            .wrapping(Wrapping::WordOrGlyph)
                            .style(iced::widget::text::danger),
                    );
            }
        }

        // The phone is still in fastboot once the flash is done. Leaving it is
        // a separate step, because the boot mode flag has to be cleared first:
        // without that the phone would boot back into fastboot.
        if flash.rebooting {
            content = content.push(busy_row(state, l10n.tr("firmware-flash-rebooting")));
        } else if let Some(outcome) = &flash.reboot_result {
            match outcome {
                Ok(()) => {
                    content = content.push(
                        text(l10n.tr("firmware-flash-reboot-sent"))
                            .size(13.0)
                            .style(bright_success),
                    );
                }
                Err(error) => {
                    content = content.push(
                        text(format!(
                            "{}: {error}",
                            l10n.tr("firmware-flash-reboot-failed")
                        ))
                        .size(13.0)
                        .width(Fill)
                        .wrapping(Wrapping::WordOrGlyph)
                        .style(iced::widget::text::danger),
                    );
                }
            }
        }

        let mut reboot = button(text(l10n.tr("firmware-flash-reboot")));
        if !flash.rebooting && flash.selected.is_some() {
            reboot = reboot.on_press(Message::FirmwareFlashReboot);
        }

        // Back to the setup form: the package stays loaded, so a retry does
        // not have to unpack the firmware again.
        content = content.push(
            row![
                button(text(format!(
                    "< {}",
                    l10n.tr("flash-bootloader-return")
                )))
                .on_press(Message::FirmwareFlashBack),
                reboot,
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );
    }

    if !flash.log.is_empty() {
        // The log is tool output, so it is shown in a monospace font. iced's
        // text cannot be selected, so the whole log is offered as a copy
        // button instead.
        let lines = iced::widget::Column::with_children(
            flash.log.iter().map(|line| {
                text(line.clone())
                    .size(11.0)
                    .font(iced::Font::MONOSPACE)
                    .into()
            }),
        )
        .spacing(2)
        .align_x(Alignment::Start);

        content = content.push(
            column![
                row![
                    text(l10n.tr("firmware-flash-log")).size(13.0),
                    button(text(l10n.tr("firmware-flash-copy-log")).size(12.0))
                        .on_press(Message::FirmwareFlashCopyLog),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                container(
                    scrollable(lines)
                        .id(iced::widget::scrollable::Id::new(LOG_SCROLL_ID))
                        .width(Fill)
                        .height(180),
                )
                .padding(6)
                .width(Fill)
                .style(container::rounded_box),
            ]
            .spacing(4),
        );
    }

    content.into()
}

/// The engines offered by the picker: the built-in fastboot, then every
/// `mfastboot` build found next to the application (newest first).
fn engine_options(state: &State) -> Vec<Labeled<Engine>> {
    let mut options = vec![Labeled {
        value: Engine::Builtin,
        label: state.l10n.tr("firmware-flash-tool-builtin"),
    }];

    for tool in &state.flash.firmware.mfastboot {
        options.push(Labeled {
            value: Engine::Mfastboot(tool.clone()),
            label: tool.label(),
        });
    }

    options
}

/// The localized lines telling the user to install the emulator the selected
/// engine needs (Rosetta 2, box64), or `None` when it can start right now.
fn emulator_hint(engine: &Engine) -> Option<(&'static str, &'static str)> {
    match engine {
        Engine::Mfastboot(tool) => tool.emulator.hint_ids(),
        Engine::Builtin => None,
    }
}

/// An "Erase Userdata" / "Erase NV cache" button. These add standalone
/// `erase` commands to the job, so the button toggles and is highlighted
/// while its group is part of the flash.
fn erase_group_button(state: &State, group: EraseGroup) -> Element<'_, Message> {
    if state.flash.firmware.running || state.flash.firmware.loading {
        return button(text(state.l10n.tr(group.label_id()))).into();
    }

    let select = button(text(state.l10n.tr(group.label_id())))
        .on_press(Message::FirmwareFlashEraseGroupToggled(group));

    if state.flash.firmware.erase_groups.contains(&group) {
        select.style(button::primary).into()
    } else {
        select.into()
    }
}

/// The partitions the added erase groups clear on top of the package (the
/// ones the package has no `erase` step of its own for).
fn erased_partitions(state: &State) -> Vec<String> {
    crate::extra_erase_partitions(&state.flash.firmware)
}

/// A "Select AP" / "Select BP" / "Select BL" button: checks every procedure
/// of that firmware part without unchecking anything else, so several parts
/// can be combined (and `erase`/`oem` rows, which belong to no part, keep
/// whatever the user chose). Disabled when the package contains none of them
/// (the count in the label then says `0`).
fn part_button(state: &State, part: FlashPart) -> Element<'_, Message> {
    let count = state
        .flash
        .firmware
        .plan
        .as_ref()
        .map(|package| {
            package
                .steps
                .iter()
                .filter(|step| step.part() == Some(part))
                .count()
        })
        .unwrap_or(0);

    let label = state.l10n.tr_with_args(
        "firmware-flash-select-part",
        &[
            ("part", part.name().to_owned()),
            ("count", count.to_string()),
        ],
    );

    let select = button(text(label).size(13.0));

    if count == 0 {
        select.into()
    } else {
        select
            .on_press(Message::FirmwareFlashSelectPart(part))
            .into()
    }
}

/// The connected supported devices as a dropdown — the selected one is what
/// the flash would run on.
fn device_picker(state: &State) -> Element<'_, Message> {
    let options: Vec<Labeled<String>> = state
        .flash
        .firmware
        .devices
        .iter()
        .map(|device| Labeled {
            value: device.serial.clone(),
            label: device.label(),
        })
        .collect();

    let selected = state
        .flash
        .firmware
        .selected
        .as_ref()
        .and_then(|serial| {
            options
                .iter()
                .find(|option| &option.value == serial)
                .cloned()
        });

    let picker: iced::widget::PickList<'_, Labeled<String>, Vec<Labeled<String>>, Labeled<String>, Message> =
        pick_list(options, selected, |option| {
            Message::FirmwareFlashDeviceSelected(option.value)
        })
        .text_shaping(Shaping::Advanced)
        .placeholder(state.l10n.tr("factory-reset-select-device"))
        .width(Fill);

    picker.into()
}

/// What the selected device reports about itself (`securestate`, `cid` and
/// `product`), plus a warning when the package is for another
/// carrier/region.
///
/// `None` when no device is selected, so nothing is shown for it.
fn device_info_section(state: &State) -> Option<Element<'_, Message>> {
    let l10n = &state.l10n;
    let flash = &state.flash.firmware;

    if flash.selected.is_none() {
        return None;
    }

    if flash.reading_device {
        return Some(busy_row(state, l10n.tr("retcn-fill-fastboot-fetching")));
    }

    if let Some(error) = &flash.device_vars_error {
        return Some(
            text(error.clone())
                .size(12.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph)
                .style(iced::widget::text::danger)
                .into(),
        );
    }

    let variables = flash.device_vars.as_ref()?;

    let parsed_state = variables
        .securestate
        .as_deref()
        .and_then(SecureState::parse);

    // An engineering (prototype) bootloader does not act on the CID, so it is
    // marked as ignored and no mismatch is reported for it.
    let cid_ignored = parsed_state.is_some_and(SecureState::ignores_cid);

    // The unlock state is shown by the name it is known by, not by the
    // variable the bootloader answers with; an unexpected value is shown as
    // it was reported.
    let securestate = variables.securestate.as_deref().map(|state| match parsed_state {
        Some(parsed) => l10n.tr(parsed.message_id()),
        None => state.to_owned(),
    });

    let cid = variables.cid.as_ref().map(|cid| {
        if cid_ignored {
            l10n.tr_with_args("firmware-flash-cid-ignored", &[("cid", cid.clone())])
        } else {
            cid.clone()
        }
    });

    let rows = column![
        info_row(
            l10n.tr("firmware-flash-device-securestate"),
            securestate,
        ),
        info_row(l10n.tr("firmware-flash-device-cid"), cid),
        info_row(
            l10n.tr("firmware-flash-device-product"),
            variables.product.clone(),
        ),
    ]
    .spacing(2)
    .align_x(Alignment::Start);

    let mut content = column![
        container(rows)
            .padding(8)
            .width(Fill)
            .style(container::rounded_box)
    ]
    .spacing(6)
    .width(Fill);

    if let (Some(package), Some(device)) = (
        flash
            .plan
            .as_ref()
            .and_then(|package| package.cid.as_ref()),
        variables.cid.as_ref(),
    ) {
        if !cid_ignored && crate::flashfile::cid_mismatch(package, device) {
            content = content.push(
                text(l10n.tr_with_args(
                    "firmware-flash-cid-mismatch",
                    &[
                        ("package", package.clone()),
                        ("device", device.clone()),
                    ],
                ))
                .size(12.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph)
                .style(warning_orange),
            );
        }
    }

    Some(content.into())
}

/// The "Select a device" label with the Refresh button that re-scans the USB
/// bus for connected phones.
fn device_header(state: &State, busy: bool) -> Element<'_, Message> {
    let refresh = button(text(state.l10n.tr("factory-reset-refresh")));

    row![
        text(state.l10n.tr("factory-reset-select-device")).size(13.0),
        if busy {
            refresh
        } else {
            refresh.on_press(Message::FirmwareFlashRescan)
        },
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

/// One `label: value` line of the package summary.
fn info_row(label: String, value: Option<String>) -> Element<'static, Message> {
    let value = value.unwrap_or_else(|| "—".to_owned());

    row![
        text(label).size(12.0).width(140),
        text(value).size(12.0).width(Fill),
    ]
    .spacing(8)
    .into()
}

/// A spinner plus its status text.
fn busy_row<'a>(state: &'a State, label: String) -> Element<'a, Message> {
    row![spinner(state.anim_tick), text(label).size(13.0)]
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
}

/// `done / total` as a `0.0..=1.0` fraction, for the progress bars.
fn fraction(done: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        (done as f32 / total as f32).clamp(0.0, 1.0)
    }
}
