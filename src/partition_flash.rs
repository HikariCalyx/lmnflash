//! The "Single Partition Flash" dialog (Mode 2, Smart Device Flashing).
//!
//! Pressing the Single Partition Flash tile opens this modal: the user picks a
//! connected phone (which may be in the bootloader or in userspace fastboot —
//! the button beside the picker switches it there), picks the tool that writes
//! the image, types the partition to write to (with the three names used most
//! often a click away), picks the image file — by browsing, or by dropping it
//! onto the window — and then flashes it with one press.
//!
//! Two AVB flags can be written into the image on the way (`Disable Verity`,
//! `Disable Verification`); they only apply to an image that carries a vbmeta
//! struct, so the boxes are enabled for exactly those.
//!
//! Like the other dialogs, this module only *presents* state and sends
//! messages; the flow state and its update handlers live in `crate`
//! (`State::flash.partition`).

use iced::widget::text::Wrapping;
use iced::widget::text::Shaping;
use iced::widget::{
    button, column, container, mouse_area, pick_list, row, scrollable, text_input,
};
use iced::{Alignment, Fill};

use crate::bootloader::{bright_success, darker_card, warning_orange};
use crate::compat_iced as iced;
use crate::compat_iced::*;
use crate::flash_engine::Engine;
use crate::text;
use crate::{firmware_flash, Labeled, Message, PartitionFlashTask, State};

/// The partition names the dropdown places into the field: the three a
/// modified phone is flashed with most of the time.
///
/// They are technical names, so they are not translated.
const PARTITION_PRESETS: [&str; 3] = ["boot", "init_boot", "vbmeta"];

/// Whether the dialog is open.
pub(crate) fn is_open(state: &State) -> bool {
    state.flash.partition.open
}

/// Renders the dialog over the app, or `None` when it is closed. Clicks on
/// empty space are swallowed (they neither dismiss the dialog nor reach the UI
/// underneath); the modal is left only via Cancel.
pub(crate) fn overlay(state: &State) -> Option<Element<'_, Message>> {
    if !is_open(state) {
        return None;
    }

    let children: Vec<Element<'_, Message>> = vec![
        mouse_area(Space::new(Fill, Fill))
            .on_press(Message::PartitionFlashBackdropPressed)
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

/// Whether "Flash" can run: a device, a partition name, an image, and a tool
/// that can actually start (an Intel build whose emulator is missing cannot).
pub(crate) fn ready(state: &State) -> bool {
    let flash = &state.flash.partition;

    flash.selected.is_some()
        && !flash.partition.trim().is_empty()
        && flash.image.is_some()
        && emulator_hint(state).is_none()
}

/// Whether a job is in flight, which is what every control but the status
/// lines waits for.
pub(crate) fn busy(state: &State) -> bool {
    state.flash.partition.task.is_some()
}

fn card(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let flash = &state.flash.partition;
    let busy = busy(state);

    let mut content = column![
        text(l10n.tr("flash-partition-title")).size(18.0),
        text(l10n.tr("partition-flash-desc"))
            .size(14.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph),
        horizontal_rule(1),
        // Which phone is written to, and the two ways of changing its mode.
        device_row(state, busy),
        tool_row(state),
    ]
    .spacing(10)
    .width(Fill);

    // What the selected tool reports for `--version`.
    if let Some(version) = tool_version_row(state) {
        content = content.push(version);
    }

    // The shipped `mfastboot` is Intel-only, so an ARM machine runs it through
    // an emulator that may still have to be installed.
    if let Some((needed, install)) = emulator_hint(state) {
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

    // What the phone reports about itself, read when it is selected.
    if flash.selected.is_some() {
        if flash.reading_device {
            content = content.push(firmware_flash::busy_row(
                state,
                l10n.tr("retcn-fill-fastboot-fetching"),
            ));
        } else if let Some(error) = &flash.vars_error {
            content = content.push(
                text(error.clone())
                    .size(12.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph)
                    .style(iced::widget::text::danger),
            );
        } else {
            content = content.push(
                column![
                    firmware_flash::info_row(
                        l10n.tr("partition-flash-slot"),
                        current_slot(state),
                    ),
                    firmware_flash::info_row(
                        l10n.tr("partition-flash-mode"),
                        device_mode(state).map(|mode| match mode {
                            DeviceMode::Bootloader => l10n.tr("partition-flash-mode-bootloader"),
                            DeviceMode::Fastbootd => l10n.tr("partition-flash-mode-fastbootd"),
                        }),
                    ),
                ]
                .spacing(2)
                .align_x(Alignment::Start),
            );
        }
    }

    content = content.push(partition_row(state, busy));
    content = content.push(image_row(state, busy));

    // The two flags are written into the image's vbmeta struct, so they are
    // offered for exactly the images that have one.
    let avb = flash.image_avb;

    let verity = checkbox(
        l10n.tr("partition-flash-disable-verity"),
        flash.disable_verity,
    )
    .text_shaping(Shaping::Advanced);
    content = content.push(if avb && !busy {
        verity.on_toggle(Message::PartitionFlashVerityToggled)
    } else {
        verity
    });

    let verification = checkbox(
        l10n.tr("partition-flash-disable-verification"),
        flash.disable_verification,
    )
    .text_shaping(Shaping::Advanced);
    content = content.push(if avb && !busy {
        verification.on_toggle(Message::PartitionFlashVerificationToggled)
    } else {
        verification
    });

    // An image that carries no struct cannot take the flags, which the user
    // asked for by picking it — say so rather than leaving two dead boxes.
    if flash.image.is_some() && !avb {
        content = content.push(
            text(l10n.tr("partition-flash-no-avb"))
                .size(12.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph)
                .style(warning_orange),
        );
    }

    // What the job in flight is doing, and how the last one went.
    content = content.push(status_section(state));

    content = content.push(horizontal_rule(1));

    let flash_button = button(text(l10n.tr("partition-flash-flash"))).width(Fill);
    let flash_button = if ready(state) && !busy {
        flash_button.on_press(Message::PartitionFlashStart)
    } else {
        flash_button
    };

    // Leaving fastboot is a separate step from flashing: it clears the boot
    // mode flag first, so the phone does not come back up in fastboot.
    let reboot = button(text(l10n.tr("firmware-flash-reboot"))).width(Fill);
    let reboot = if flash.selected.is_some() && !busy {
        reboot.on_press(Message::PartitionFlashReboot)
    } else {
        reboot
    };

    // The dialog is never closed while a job runs: the phone would be left
    // half-written with nothing to say what happened.
    let cancel = button(text(l10n.tr("login-cancel"))).width(Fill);
    let cancel = if busy {
        cancel
    } else {
        cancel.on_press(Message::PartitionFlashCancel)
    };

    content = content.push(
        row![flash_button, reboot, cancel]
            .spacing(8)
            .align_y(Alignment::Center),
    );

    // The padding sits inside the scrollable, so the card's border is the
    // viewport (the modal convention).
    container(
        scrollable(container(content).padding(16).width(Fill))
            .width(Fill)
            .height(Fill),
    )
    .width(560)
    .height(580)
    .style(darker_card)
    .into()
}

/// The mode the phone is in, as far as it has reported: `is-userspace` says
/// `yes` in userspace fastboot and `no` in the bootloader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeviceMode {
    Bootloader,
    Fastbootd,
}

/// [`DeviceMode`] of the selected phone, or `None` while it is unknown.
pub(crate) fn device_mode(state: &State) -> Option<DeviceMode> {
    let value = state
        .flash
        .partition
        .vars
        .as_ref()
        .and_then(|vars| vars.is_userspace.as_deref())?;

    if value.trim().eq_ignore_ascii_case("yes") {
        Some(DeviceMode::Fastbootd)
    } else if value.trim().eq_ignore_ascii_case("no") {
        Some(DeviceMode::Bootloader)
    } else {
        None
    }
}

/// The slot the phone is running from, without the `_` some bootloaders put in
/// front of it.
pub(crate) fn current_slot(state: &State) -> Option<String> {
    let slot = state
        .flash
        .partition
        .vars
        .as_ref()
        .and_then(|vars| vars.current_slot.as_deref())?;
    let slot = slot.trim().trim_start_matches('_');

    (!slot.is_empty()).then(|| slot.to_owned())
}

/// The device dropdown, the Refresh button and the button that sends the phone
/// into userspace fastboot.
fn device_row(state: &State, busy: bool) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let flash = &state.flash.partition;

    let status: Element<'_, Message> = if flash.listing {
        firmware_flash::busy_row(state, l10n.tr("factory-reset-reading-devices"))
    } else if let Some(error) = &flash.list_error {
        text(error.clone())
            .size(13.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph)
            .style(iced::widget::text::danger)
            .into()
    } else if flash.devices.is_empty() {
        text(l10n.tr("retcn-fill-fastboot-no-device"))
            .size(13.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph)
            .style(iced::widget::text::danger)
            .into()
    } else {
        device_picker(state)
    };

    let refresh = button(text(l10n.tr("factory-reset-refresh")));
    let refresh = if busy || flash.listing {
        refresh
    } else {
        refresh.on_press(Message::PartitionFlashRescan)
    };

    // `reboot fastboot` only means something while the phone is in the
    // bootloader; a phone that does not say which mode it is in is taken as
    // one, since that is the mode the command leaves behind either way.
    let switch = button(text(l10n.tr("partition-flash-switch")));
    let switch = if flash.selected.is_some()
        && !busy
        && !matches!(device_mode(state), Some(DeviceMode::Fastbootd))
    {
        switch.on_press(Message::PartitionFlashSwitchToFastbootd)
    } else {
        switch
    };

    row![
        container(status).width(Fill),
        refresh,
        switch,
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

/// The connected phones as a dropdown — the selected one is what the flash
/// would run on.
fn device_picker(state: &State) -> Element<'_, Message> {
    let options: Vec<Labeled<String>> = state
        .flash
        .partition
        .devices
        .iter()
        .map(|device| Labeled {
            value: device.serial.clone(),
            label: device.label(),
        })
        .collect();

    let selected = state
        .flash
        .partition
        .selected
        .as_ref()
        .and_then(|serial| options.iter().find(|option| &option.value == serial).cloned());

    let picker: iced::widget::PickList<
        '_,
        Labeled<String>,
        Vec<Labeled<String>>,
        Labeled<String>,
        Message,
        crate::fluent::Theme,
    > = pick_list(options, selected, |option| {
        Message::PartitionFlashDeviceSelected(option.value)
    })
    .text_shaping(Shaping::Advanced)
    .placeholder(state.l10n.tr("factory-reset-select-device"))
    .width(Fill);

    picker.into()
}

/// The `fastboot` the image is written with: the built-in one, the `mfastboot`
/// builds shipped next to the application, and Google's copy when it was
/// already downloaded.
fn tool_options(state: &State) -> Vec<Labeled<Engine>> {
    let l10n = &state.l10n;
    let flash = &state.flash.partition;

    let mut options = vec![Labeled {
        value: Engine::Builtin,
        label: l10n.tr("firmware-flash-tool-builtin"),
    }];

    for tool in &flash.mfastboot {
        options.push(Labeled {
            value: Engine::Mfastboot(tool.clone()),
            label: tool.label(),
        });
    }

    // Google's copy is offered only once it is on disk: this dialog does not
    // download it (the Firmware Flash dialog does, and both share the file).
    if let Some(tool) = flash.platform_tools.as_ref().filter(|tool| tool.path.is_file()) {
        options.push(Labeled {
            value: Engine::Mfastboot(tool.clone()),
            label: l10n.tr("firmware-flash-tool-platform-tools"),
        });
    }

    options
}

/// The "Flashing tool" row: the label and the dropdown that fills the line.
fn tool_row(state: &State) -> Element<'_, Message> {
    let options = tool_options(state);
    let selected = options
        .iter()
        .find(|option| option.value == state.flash.partition.engine)
        .cloned()
        .unwrap_or_else(|| options[0].clone());

    let picker: iced::widget::PickList<
        '_,
        Labeled<Engine>,
        Vec<Labeled<Engine>>,
        Labeled<Engine>,
        Message,
        crate::fluent::Theme,
    > = pick_list(options, Some(selected), |option| {
        Message::PartitionFlashEngineSelected(option.value)
    })
    .text_shaping(Shaping::Advanced)
    .width(Fill);

    Row::new()
        .spacing(8)
        .align_y(Alignment::Center)
        .push(text(state.l10n.tr("firmware-flash-tool")).size(13.0))
        .push(picker)
        .into()
}

/// What the selected tool reports for `--version`. `None` for the built-in
/// engine, which has no command line to ask.
fn tool_version_row(state: &State) -> Option<Element<'_, Message>> {
    let l10n = &state.l10n;
    let flash = &state.flash.partition;

    flash.engine.tool()?;

    if flash.reading_version {
        return Some(firmware_flash::busy_row(
            state,
            l10n.tr("firmware-flash-tool-version-reading"),
        ));
    }

    match &flash.tool_version {
        Some(Ok(version)) => Some(
            text(l10n.tr_with_args(
                "firmware-flash-tool-version",
                &[("version", version.clone())],
            ))
            .size(12.0)
            .width(Fill)
            .into(),
        ),
        Some(Err(error)) => Some(
            text(l10n.tr_with_args(
                "firmware-flash-tool-version-failed",
                &[("error", error.clone())],
            ))
            .size(12.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph)
            .style(iced::widget::text::danger)
            .into(),
        ),
        None => None,
    }
}

/// The Rosetta 2 / box64 lines for a selected build that cannot start here.
fn emulator_hint(state: &State) -> Option<(&'static str, &'static str)> {
    state.flash.partition.engine.tool()?.emulator.hint_ids()
}

/// The partition to write to, with the names used most often a click away.
fn partition_row(state: &State, busy: bool) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let flash = &state.flash.partition;

    // The label sits beside the field, so the placeholder stays empty (the
    // house pattern of the labelled lookup fields).
    let mut field = text_input("", &flash.partition).width(Fill);

    if !busy {
        field = field.on_input(Message::PartitionFlashPartitionChanged);
    }

    let presets: Vec<Labeled<&'static str>> = PARTITION_PRESETS
        .iter()
        .map(|name| Labeled {
            value: *name,
            label: (*name).to_owned(),
        })
        .collect();

    // The dropdown is a menu of one-shot picks: it always shows its
    // placeholder and every pick fills the field, like the Reboot menu.
    let picker: iced::widget::PickList<
        '_,
        Labeled<&'static str>,
        Vec<Labeled<&'static str>>,
        Labeled<&'static str>,
        Message,
        crate::fluent::Theme,
    > = pick_list(presets, None, |option| {
        Message::PartitionFlashPartitionPreset(option.value)
    })
    .placeholder(l10n.tr("multi-flash-options-cid-preset"))
    .text_shaping(Shaping::Advanced)
    .style(firmware_flash::menu_style);

    let mut content = column![row![
        text(l10n.tr("partition-flash-partition")).size(13.0),
        field,
        picker,
    ]
    .spacing(8)
    .align_y(Alignment::Center)]
    .spacing(4)
    .width(Fill);

    // Flashing needs something to write to, so an empty field is said rather
    // than left to the failing command.
    if flash.selected.is_some() && flash.partition.trim().is_empty() {
        content = content.push(
            text(l10n.tr("partition-flash-partition-required"))
                .size(12.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph)
                .style(iced::widget::text::danger),
        );
    }

    content.into()
}

/// Where the image comes from: the picker, and the path a dropped file (or the
/// picker) leaves behind.
fn image_row(state: &State, busy: bool) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let flash = &state.flash.partition;

    let select = button(text(l10n.tr("partition-flash-select")));
    let select = if busy {
        select
    } else {
        select.on_press(Message::PartitionFlashPickImage)
    };

    let path = match &flash.image {
        Some(path) => path.display().to_string(),
        None => l10n.tr("partition-flash-no-image"),
    };

    column![
        row![
            text(l10n.tr("partition-flash-image")).size(13.0),
            select,
            container(
                text(path)
                    .size(12.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph),
            )
            .padding(8)
            .width(Fill)
            .style(container::rounded_box),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        // The window takes a dropped image as well, which is the quickest way
        // to flash something that was just downloaded.
        text(l10n.tr("partition-flash-drop"))
            .size(11.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph),
    ]
    .spacing(4)
    .width(Fill)
    .into()
}

/// What the job in flight is doing, how the last one went, and its log.
fn status_section(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let flash = &state.flash.partition;

    let mut content = column![].spacing(8).width(Fill);

    // The task in flight says which spinner text belongs to it.
    if let Some(task) = flash.task {
        let label = match task {
            PartitionFlashTask::Flash => l10n.tr("partition-flash-flashing"),
            PartitionFlashTask::Reboot => l10n.tr("firmware-flash-rebooting"),
            PartitionFlashTask::SwitchToFastbootd => l10n.tr("partition-flash-switching"),
        };

        content = content.push(firmware_flash::busy_row(state, label));

        // Bytes of the image being written: only the built-in engine reports
        // them (`mfastboot` prints its own progress into the log).
        if task == PartitionFlashTask::Flash && matches!(flash.engine, Engine::Builtin) {
            let bytes = flash
                .progress
                .map(|(done, total)| firmware_flash::fraction(done, total))
                .unwrap_or(0.0);

            content = content.push(firmware_flash::bar(bytes));
        }
    } else if let Some(result) = &flash.result {
        match result {
            Ok(()) => {
                content = content.push(
                    text(l10n.tr("partition-flash-done"))
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
    }

    // Switching the phone to userspace fastboot is its own step, and so is
    // the reboot that leaves fastboot for Android.
    match &flash.switch_result {
        None => {}
        Some(Ok(())) => {
            content = content.push(
                text(l10n.tr("partition-flash-switched"))
                    .size(13.0)
                    .style(bright_success),
            );
        }
        Some(Err(error)) => {
            content = content.push(
                text(error.clone())
                    .size(13.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph)
                    .style(iced::widget::text::danger),
            );
        }
    }

    match &flash.reboot_result {
        None => {}
        Some(Ok(())) => {
            content = content.push(
                text(l10n.tr("firmware-flash-reboot-sent"))
                    .size(13.0)
                    .style(bright_success),
            );
        }
        Some(Err(error)) => {
            content = content.push(
                text(format!("{}: {error}", l10n.tr("firmware-flash-reboot-failed")))
                    .size(13.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph)
                    .style(iced::widget::text::danger),
            );
        }
    }

    if !flash.log.is_empty() {
        let lines = iced::widget::Column::with_children(
            flash.log.iter().map(|line| {
                text(line.clone())
                    .size(firmware_flash::OUTPUT_SIZE)
                    .font(crate::output_font())
                    .into()
            }),
        )
        .spacing(2)
        .align_x(Alignment::Start);

        content = content.push(
            column![
                row![
                    text(l10n.tr("firmware-flash-log")).size(13.0),
                    button(text(l10n.tr("firmware-flash-locate-log")).size(12.0))
                        .on_press(Message::PartitionFlashLocateLog),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                container(
                    scrollable(lines)
                        .id(iced::widget::Id::new(firmware_flash::LOG_SCROLL_ID))
                        .width(Fill)
                        .height(160),
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
