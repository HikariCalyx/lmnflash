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
use crate::fastboot_info::{self, SecureState};
use crate::flash_engine::{Engine, RebootMode};
use crate::flashfile::{EraseGroup, FlashPart};
use crate::guided::spinner;
use crate::text;
use crate::{enabled_procedures, FirmwareFlashDialog, Labeled, Message, State};

/// Id of the flashing log's scrollable, so [`crate`] can scroll it to the
/// newest line as lines arrive.
pub(crate) const LOG_SCROLL_ID: &str = "firmware-flash-log";

/// Height of every progress bar in the dialog. A bar is a status line, not a
/// button: at the widget's 30 px default it drew more attention than the text
/// it belongs to.
const BAR_HEIGHT: f32 = 6.0;

/// Size of the tool output the dialog shows (the flashing log and `Read Info`).
///
/// Monospace reads smaller than the proportional font at the same size, so it
/// is a step up from the labels around it.
const OUTPUT_SIZE: f32 = 13.0;

/// The font the tool output is written in.
///
/// iced's [`iced::Font::MONOSPACE`] is a *generic* family and iced bundles no
/// font of its own, so the system picks it — and on a machine whose monospace
/// font is a CJK one that is a trap: those fonts map ASCII `0x5C` to their own
/// character, so a Korean font shows the won sign (`₩`) where the output had a
/// backslash. Naming the platform's classic monospace font keeps the output
/// ASCII-shaped.
fn output_font() -> iced::Font {
    let family = {
        #[cfg(target_os = "windows")]
        {
            "Consolas"
        }
        #[cfg(target_os = "macos")]
        {
            "Menlo"
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            "DejaVu Sans Mono"
        }
    };

    iced::Font::with_name(family)
}

/// A progress bar at the height used everywhere in this dialog.
fn bar<'a>(fraction: f32) -> Element<'a, Message> {
    progress_bar(0.0..=1.0, fraction)
        .height(BAR_HEIGHT)
        .width(Fill)
        .into()
}

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
    } else if flash.info_view {
        content = content.push(info_section(state));
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

    // The padding sits inside the scrollable, so the card's border is the
    // viewport: the scrollbar rides on the right border instead of floating
    // in the middle of the padding.
    container(
        scrollable(container(content).padding(16).width(Fill))
            .width(Fill)
            .height(Fill),
    )
    .width(560)
    .height(520)
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
            content = content.push(bar(fraction(done, total)));
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
            // The codename and CID the package's `vbmeta` carries
            // (`arcfox_50`); a package without one shows nothing here.
            info_row(
                l10n.tr("firmware-flash-project-code"),
                package.project_code.clone(),
            ),
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
        .push(device_row(state, busy));

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
    let start = if ready && !busy {
        start.on_press(Message::FirmwareFlashStart)
    } else {
        start
    };

    // Exporting needs no phone: the script is written from the package and the
    // checked procedures, so it can be prepared before the device is plugged
    // in (or for someone else to run).
    let export = button(text(l10n.tr("firmware-flash-export")));
    let export = if !busy && flash.plan.is_some() && enabled_procedures(flash) > 0 {
        export.on_press(Message::FirmwareFlashExport)
    } else {
        export
    };

    // Rebooting needs a phone but no firmware package: it is the way back out
    // of fastboot (or into another fastboot mode) on its own. The modes are a
    // dropdown rather than a page of their own, and picking one runs it — the
    // control keeps its `Reboot` placeholder, so it reads as a menu and every
    // pick (even the same one twice) sends a fresh command.
    let mut modes = vec![
        reboot_option(l10n, "firmware-flash-reboot-system", RebootMode::System),
        reboot_option(
            l10n,
            "firmware-flash-reboot-bootloader",
            RebootMode::Bootloader,
        ),
        reboot_option(
            l10n,
            "firmware-flash-reboot-recovery",
            RebootMode::Recovery,
        ),
    ];

    // `reboot fastboot` only means something while the phone is in the
    // bootloader, so that choice is offered only when it said so itself.
    if in_bootloader(state) {
        modes.push(reboot_option(
            l10n,
            "firmware-flash-reboot-fastbootd",
            RebootMode::Fastbootd,
        ));
    }

    modes.push(reboot_option(
        l10n,
        "firmware-flash-reboot-sideload",
        RebootMode::Sideload,
    ));

    let reboot: iced::widget::PickList<
        '_,
        Labeled<RebootMode>,
        Vec<Labeled<RebootMode>>,
        Labeled<RebootMode>,
        Message,
    > = pick_list(modes, None, |mode| {
        Message::FirmwareFlashRebootSelected(mode.value)
    })
    .placeholder(l10n.tr("firmware-flash-reboot"))
    .text_shaping(Shaping::Advanced)
    .style(reboot_menu_style)
    .width(Fill);

    // The three controls share the row, so they share its width too.
    let export = export.width(Fill);

    content = content.push(
        row![start, reboot, export]
            .spacing(8)
            .align_y(Alignment::Center),
    );

    // Whether the phone answered a reboot, or why it did not.
    if let Some(status) = reboot_status(state) {
        content = content.push(status);
    }

    // Where the last export went, or why it could not be written.
    match &flash.export_result {
        Some(Ok(path)) => {
            content = content.push(
                text(l10n.tr_with_args(
                    "firmware-flash-export-done",
                    &[("path", path.display().to_string())],
                ))
                .size(12.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph)
                .style(bright_success),
            );
        }
        Some(Err(error)) => {
            content = content.push(
                text(l10n.tr_with_args(
                    "firmware-flash-export-failed",
                    &[("error", error.clone())],
                ))
                .size(12.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph)
                .style(iced::widget::text::danger),
            );
        }
        None => {}
    }

    content.into()
}

/// One choice of the Reboot dropdown.
fn reboot_option(
    l10n: &crate::l10n::Bundle,
    label_id: &str,
    value: RebootMode,
) -> Labeled<RebootMode> {
    Labeled {
        label: l10n.tr(label_id),
        value,
    }
}

/// Whether the selected phone is in the bootloader rather than in userspace
/// fastboot (`fastbootd`).
///
/// The phone has to say `no` itself: an unanswered or empty `is-userspace` is
/// not a `no`, and only a bootloader can be sent to fastbootd.
fn in_bootloader(state: &State) -> bool {
    state
        .flash
        .firmware
        .device_vars
        .as_ref()
        .and_then(|variables| variables.is_userspace.as_deref())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("no"))
}

/// The Reboot dropdown only ever shows its placeholder (`Reboot`), so it has
/// to read as an active control rather than an empty field: its text takes the
/// colour the buttons next to it label themselves with.
fn reboot_menu_style(theme: &iced::Theme, status: pick_list::Status) -> pick_list::Style {
    let mut style = pick_list::default(theme, status);
    style.placeholder_color = theme.extended_palette().primary.strong.text;

    style
}

/// Whether a reboot is running, and how the last one went.
fn reboot_status(state: &State) -> Option<Element<'_, Message>> {
    let l10n = &state.l10n;
    let flash = &state.flash.firmware;

    if flash.rebooting {
        return Some(busy_row(state, l10n.tr("firmware-flash-rebooting")));
    }

    match &flash.reboot_result {
        None => None,
        Some(Ok(())) => Some(
            text(l10n.tr("firmware-flash-reboot-sent"))
                .size(13.0)
                .style(bright_success)
                .into(),
        ),
        Some(Err(error)) => Some(
            text(format!(
                "{}: {error}",
                l10n.tr("firmware-flash-reboot-failed")
            ))
            .size(13.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph)
            .style(iced::widget::text::danger)
            .into(),
        ),
    }
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
///
/// When the package is made for another phone than the selected one, that is
/// spelled out and "Yes" stays disabled until the countdown ran out: flashing
/// another project's firmware is expected to brick the phone, so it must not be
/// a reflex click.
fn confirmation_section(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let flash = &state.flash.firmware;

    let mut content = column![].spacing(10).width(Fill);

    if flash.project_mismatch {
        content = content.push(
            text(project_mismatch_warning(state))
                .size(13.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph)
                .style(iced::widget::text::danger),
        );
    }

    content = content.push(
        text(l10n.tr("firmware-flash-confirm"))
            .size(13.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph)
            .style(warning_orange),
    );

    // The countdown is what makes the warning hard to skip, so it is shown as
    // long as "Yes" is not clickable.
    if flash.confirm_countdown > 0 {
        content = content.push(
            text(l10n.tr_with_args(
                "firmware-flash-brick-countdown",
                &[("seconds", flash.confirm_countdown.to_string())],
            ))
            .size(13.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph)
            .style(iced::widget::text::danger),
        );
    }

    let yes = button(text(l10n.tr("guided-yes")));
    let yes = if flash.confirm_countdown > 0 {
        yes
    } else {
        yes.on_press(Message::FirmwareFlashConfirmed)
    };

    content.push(
        row![
            yes,
            button(text(l10n.tr("guided-no"))).on_press(Message::FirmwareFlashBack),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .into()
}

/// The two project codes of a mismatched package and phone, saying what they
/// are and what flashing them together does.
///
/// A code that is somehow missing by the time this is drawn is named as such
/// rather than dropped: the sentence has to stay readable either way.
fn project_mismatch_warning(state: &State) -> String {
    let l10n = &state.l10n;
    let flash = &state.flash.firmware;

    let package = flash
        .plan
        .as_ref()
        .and_then(|package| package.project_code.clone())
        .unwrap_or_else(|| "—".to_owned());
    let device = flash
        .device_vars
        .as_ref()
        .and_then(|variables| variables.product.clone())
        .unwrap_or_else(|| "—".to_owned());

    l10n.tr_with_args(
        "firmware-flash-project-mismatch",
        &[("package", package), ("device", device)],
    )
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
                    bar((flash.step_index + 1) as f32 / flash.step_total as f32),
                    text(format!("{}/{}", flash.step_index + 1, flash.step_total)).size(12.0),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
        }

        // Bytes of the procedure that is currently running.
        //
        // The built-in fastboot reports them, so its bar is always drawn: it
        // stays empty while a step transfers nothing (`erase`, `oem`,
        // `getvar`) instead of vanishing and letting the view jump. `mfastboot`
        // prints its own progress lines into the log and reports no bytes, so
        // it gets no bar.
        let bytes = flash
            .progress
            .map(|(done, total)| fraction(done, total))
            .unwrap_or(0.0);
        let builtin = matches!(flash.engine, Engine::Builtin);

        if builtin || flash.progress.is_some() {
            content = content.push(bar(bytes));
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
        if let Some(status) = reboot_status(state) {
            content = content.push(status);
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
                    .size(OUTPUT_SIZE)
                    .font(output_font())
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

/// What the selected device reports about itself (`securestate`, `cid`,
/// `product` and `ro.carrier`), plus a warning when the package is for another
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
        // The bootloader answers with the software channel code, so the name
        // behind it is written after it: `retcn (Retail China)`.
        info_row(
            l10n.tr("firmware-flash-device-carrier"),
            variables
                .carrier
                .as_deref()
                .map(|carrier| crate::carrier::labeled(carrier, l10n)),
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

/// The device dropdown with the Refresh button that re-scans the USB bus for
/// connected phones.
///
/// The dropdown sits where the "Select a device" label used to be, so the row
/// is one line: while the scan runs, or when there is nothing to offer, the
/// same place carries the reason instead.
fn device_row(state: &State, busy: bool) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let flash = &state.flash.firmware;

    let status: Element<'_, Message> = if flash.listing {
        busy_row(state, l10n.tr("factory-reset-reading-devices"))
    } else if let Some(error) = &flash.list_error {
        text(error.clone())
            .size(13.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph)
            .style(iced::widget::text::danger)
            .into()
    } else if flash.devices.is_empty() {
        let error_id = if flash.total == 0 {
            "retcn-fill-fastboot-no-device"
        } else {
            "retcn-fill-fastboot-unsupported-device"
        };

        text(l10n.tr(error_id))
            .size(13.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph)
            .style(iced::widget::text::danger)
            .into()
    } else {
        device_picker(state)
    };

    let refresh = button(text(l10n.tr("factory-reset-refresh")));

    // Reading the phone's own report needs a device, and has to wait until
    // whatever else is talking to it (the variable read, a flash) is done.
    let read_info = button(text(l10n.tr("firmware-flash-read-info")));
    let read_info = if flash.selected.is_some()
        && !busy
        && !flash.reading_device
        && !flash.rebooting
        && !flash.reading_info
    {
        read_info.on_press(Message::FirmwareFlashReadInfo)
    } else {
        read_info
    };

    row![
        container(status).width(Fill),
        read_info,
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

/// What the "Read Info" commands reported, with the buttons that hide the
/// IMEIs, copy the text, and go back to the setup form.
fn info_section(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let flash = &state.flash.firmware;

    let mut content = column![text(l10n.tr("firmware-flash-read-info")).size(16.0)]
        .spacing(10)
        .width(Fill);

    if flash.reading_info {
        content = content.push(busy_row(state, l10n.tr("retcn-fill-fastboot-fetching")));
    } else if let Some(error) = &flash.info_error {
        content = content.push(
            text(error.clone())
                .size(13.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph)
                .style(iced::widget::text::danger),
        );
    }

    if !flash.info_lines.is_empty() {
        // Tool output, so it is monospaced like the flashing log; the text
        // cannot be selected, hence the Copy button below.
        let showing = if flash.hide_sensitive {
            fastboot_info::hide_sensitive(&flash.info_lines)
        } else {
            flash.info_lines.clone()
        };

        let lines = iced::widget::Column::with_children(
            showing.into_iter().map(|line| {
                text(line)
                    .size(OUTPUT_SIZE)
                    .font(output_font())
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph)
                    .into()
            }),
        )
        .spacing(2)
        .align_x(Alignment::Start);

        content = content.push(
            container(scrollable(lines).width(Fill).height(280))
                .padding(6)
                .width(Fill)
                .style(container::rounded_box),
        );
    }

    // Masking is a toggle, so the button says what pressing it does next.
    let (label_id, hidden) = if flash.hide_sensitive {
        ("firmware-flash-show-sensitive", false)
    } else {
        ("firmware-flash-hide-sensitive", true)
    };

    let sensitive = button(text(l10n.tr(label_id))).width(Fill);
    let sensitive = if flash.info_lines.is_empty() {
        sensitive
    } else {
        sensitive.on_press(Message::FirmwareFlashInfoSensitiveToggled(hidden))
    };

    let copy = button(text(l10n.tr("flash-bootloader-copy"))).width(Fill);
    let copy = if flash.info_lines.is_empty() {
        copy
    } else {
        copy.on_press(Message::FirmwareFlashInfoCopy)
    };

    content
        .push(
            row![
                sensitive,
                copy,
                button(text(format!(
                    "< {}",
                    l10n.tr("flash-bootloader-return")
                )))
                .width(Fill)
                .on_press(Message::FirmwareFlashInfoClosed),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .into()
}

/// One `label: value` line of the package summary.
///
/// The value wraps: the carrier row carries a channel code plus the name behind
/// it, which is the longest thing in the box, and a translation can make it
/// longer still.
fn info_row(label: String, value: Option<String>) -> Element<'static, Message> {
    let value = value.unwrap_or_else(|| "—".to_owned());

    row![
        text(label).size(12.0).width(140),
        text(value)
            .size(12.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph),
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
