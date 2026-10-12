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

use iced::widget::rule;
use iced::widget::text::{IntoFragment, Shaping, Wrapping};
use iced::widget::{
    button, column, container, mouse_area, pick_list, row, scrollable, text_input,
};
use iced::{Alignment, Fill};

use crate::bootloader::{bright_success, darker_card, warning_orange};
use crate::compat_iced as iced;
use crate::compat_iced::*;
use crate::fastboot_info::SecureState;
use crate::firmware_flash::{bar, fraction, menu_style};
use crate::flashfile::{is_mediatek, FlashPart};
use crate::guided::spinner;
use crate::multi_flash_run;
use crate::text;
use crate::{
    load_error_text, output_font, DeviceStatus, FlashConditions, Labeled, Message,
    MultiFlashDevice, MultiFlashPreset, PresetStatus, PresetTool, State,
};
use crate::cid;

/// Id of the step checklist's scrollable, so its position survives the card
/// being rebuilt for every message.
const STEPS_SCROLL_ID: &str = "multi-flash-steps";

/// The bootloader unlock states a custom condition can accept.
/// `flashing_locked` (a bootloader that was unlocked once and relocked) is not
/// among them: it is exactly the state the default rule refuses to flash to.
const CUSTOM_STATES: [SecureState; 3] = [
    SecureState::Locked,
    SecureState::Unlocked,
    SecureState::Engineering,
];

/// The height of the flash-options card. It is the tallest dialog of the app
/// (identity, tool, checklist and conditions all show at once), so it is sized
/// to fit a package with a handful of procedures without scrolling.
const OPTIONS_CARD_HEIGHT: f32 = 640.0;

/// How tall the divider between the two groups of buttons in the preset card's
/// header is: tall enough to read as a divider between the buttons around it,
/// short enough not to look like a border.
const HEADER_SEPARATOR_HEIGHT: f32 = 20.0;

/// How wide that divider is: one logical pixel, i.e. one device pixel at 100%
/// display scaling (a thicker value would be a bar, not a divider).
const SEPARATOR_WIDTH: f32 = 1.0;

/// Whether the "Preset Manage" dialog is open.
pub(crate) fn preset_open(state: &State) -> bool {
    state.flash.multi_flash.preset_dialog
}

/// Whether the flash-options card is open.
pub(crate) fn preset_options_open(state: &State) -> bool {
    state.flash.multi_flash.pending.path.is_some()
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
    // A device of a run that was stopped is still being written to, so the page
    // stays as locked as it is while the run watches for devices.
    let busy = running || !multi.flashing.is_empty();

    // [Preset Manage]. Locked while a run goes on: what is being flashed must
    // not change under it.
    let preset = button(text(l10n.tr("multi-flash-preset-manage"))).width(Fill);
    let preset = if busy {
        preset
    } else {
        preset.on_press(Message::MultiFlashPresetManage)
    };

    // [Start] / [Stop]. Starting needs something to flash, and no device of the
    // run before it still being written to (iced greys a button without
    // `on_press` out and ignores clicks on it); stopping is the one control a
    // run must keep live.
    let start = button(text(
        l10n.tr(if running {
            "multi-flash-stop"
        } else {
            "multi-flash-start"
        }),
    ))
    .width(Fill);
    let start = if running {
        start.on_press(Message::MultiFlashStop)
    } else if !busy && !multi.presets.is_empty() {
        start.on_press(Message::MultiFlashStart)
    } else {
        start
    };

    // [Exit]. Locked while a device is being flashed, even after "Stop": its log
    // is still being written and it has to finish.
    let exit = button(text(l10n.tr("multi-flash-exit"))).width(Fill);
    let exit = if busy {
        exit
    } else {
        exit.on_press(Message::MultiFlashExit)
    };

    let mut content = column![row![preset, start, exit].spacing(8).width(Fill)]
        .spacing(12)
        .width(Fill)
        .align_x(Alignment::Start);

    // Where this run's logs go, so the user can read them afterwards.
    if let Some(directory) = &multi.log_directory {
        content = content.push(
            text(l10n.tr_with_args(
                "multi-flash-log-directory",
                &[("path", directory.display().to_string())],
            ))
            .size(12.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph),
        );
    }

    if let Some(error) = &multi.run_error {
        content = content.push(status_text(
            l10n.tr_with_args("multi-flash-run-failed", &[("error", error.clone())]),
            iced::widget::text::danger,
        ));
    }

    content = content.push(device_table(state));

    // A run that was stopped leaves its devices behind while they finish: the
    // window cannot be closed until they do, which is worth saying out loud.
    if !running && !multi.flashing.is_empty() {
        content = content.push(status_text(
            l10n.tr_with_args(
                "multi-flash-still-flashing",
                &[("count", multi.flashing.len().to_string())],
            ),
            warning_orange,
        ));
    }

    container(content)
        .width(Fill)
        .height(Fill)
        .padding(16)
        .into()
}

/// The width of the run table's "Serial No" column, in logical pixels.
///
/// The table is laid out in fixed columns, the way the Pac Repack one is: a long
/// serial must not push the progress line of the next device around.
const COLUMN_SERIAL: f32 = 150.0;
/// The width of its "XT Code" column.
const COLUMN_XT_CODE: f32 = 90.0;
/// The width of its "CID" column.
const COLUMN_CID: f32 = 80.0;
/// The width of its progress bar column.
const COLUMN_BAR: f32 = 110.0;

/// The room a table cell keeps around its content, so the columns stay apart.
const CELL_PADDING: [f32; 2] = [3.0, 6.0];

/// Id of the run's table, so its scroll position survives a repaint.
const RUN_SCROLL_ID: &str = "multi-flash-devices";

/// The devices of the run: one row per phone, as the session reports them.
fn device_table(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let multi = &state.flash.multi_flash;

    let head = container(
        row![
            value(l10n.tr("retcn-sn-label"), COLUMN_SERIAL),
            value(l10n.tr("retcn-model-label"), COLUMN_XT_CODE),
            value(l10n.tr("firmware-flash-cid"), COLUMN_CID),
            value(l10n.tr("multi-flash-col-progress"), COLUMN_BAR),
            value(l10n.tr("multi-flash-col-current"), Fill),
        ]
        .align_y(Alignment::Center),
    )
    .width(Fill);

    let mut rows: Vec<Element<'_, Message>> = Vec::new();

    for device in &multi.devices {
        rows.push(device_row(l10n, device));
    }

    // A run starts with an empty table and fills it as phones answer; saying so
    // is friendlier than a head row over nothing.
    if rows.is_empty() {
        rows.push(
            text(l10n.tr("multi-flash-idle"))
                .size(12.0)
                .width(Fill)
                .into(),
        );
    }

    let body = scrollable(
        iced::widget::Column::with_children(rows)
            .spacing(4)
            .width(Fill),
    )
    .id(iced::widget::Id::new(RUN_SCROLL_ID))
    .width(Fill)
    .height(Fill);

    container(
        column![head, horizontal_rule(1), body]
            .spacing(4)
            .width(Fill)
            .height(Fill),
    )
    .width(Fill)
    .height(Fill)
    .padding(12)
    .style(darker_card)
    .into()
}

/// One device's row: what it is, how far its flash is, and what it is doing.
fn device_row<'a>(
    l10n: &crate::l10n::Bundle,
    device: &'a MultiFlashDevice,
) -> Element<'a, Message> {
    let (message, style) = device_status(device, l10n);

    // A finished flash shows a full bar; one that failed keeps the bar where it
    // stopped (`device.fraction`), so the row says how far it got.
    let fraction = match device.status {
        DeviceStatus::Finished => 1.0,
        _ => device.fraction,
    };

    let progress: Element<'a, Message> = {
        let line = text(message)
            .size(12.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph);

        match style {
            Some(style) => line.style(style).into(),
            None => line.into(),
        }
    };

    row![
        value(device.serial.as_str(), COLUMN_SERIAL),
        value(device.xt_code.as_deref().unwrap_or("—"), COLUMN_XT_CODE),
        value(device.cid.as_deref().unwrap_or("—"), COLUMN_CID),
        run_cell(bar(fraction), COLUMN_BAR),
        run_cell(progress, Fill),
    ]
    .align_y(Alignment::Center)
    .into()
}

/// What a device's progress column says, and in which colour (`None` is the
/// ordinary text colour).
fn device_status<'a>(
    device: &'a MultiFlashDevice,
    l10n: &'a crate::l10n::Bundle,
) -> (String, Option<fn(&crate::fluent::Theme) -> iced::widget::text::Style>) {
    match &device.status {
        // The firmware is being read; the first procedure follows immediately.
        DeviceStatus::Preparing => (l10n.tr("firmware-flash-loading"), None),
        DeviceStatus::NoPreset => (l10n.tr("multi-flash-no-preset"), Some(warning_orange)),
        DeviceStatus::Flashing {
            index,
            total,
            label,
        } => (format!("[{}/{}] {label}", index + 1, total), None),
        DeviceStatus::Finished => (l10n.tr("multi-flash-device-finished"), Some(bright_success)),
        DeviceStatus::Failed(error) => (
            l10n.tr_with_args("multi-flash-device-failed", &[("error", error.clone())]),
            Some(iced::widget::text::danger),
        ),
    }
}

/// A table entry: the data lines up in columns, so it is written in the
/// platform's monospace font.
fn value<'a>(
    value: impl IntoFragment<'a>,
    width: impl Into<iced::Length>,
) -> Element<'a, Message> {
    run_cell(text(value).size(12.0).font(output_font()), width)
}

/// One table cell: as wide as its column, so the head and the rows line up (the
/// last column takes the room the others left).
fn run_cell<'a>(
    content: impl Into<Element<'a, Message>>,
    width: impl Into<iced::Length>,
) -> Element<'a, Message> {
    container(content).width(width).padding(CELL_PADDING).into()
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

/// Renders the flash-options card, or `None` when no image is being set up.
///
/// It is stacked on top of the "Preset Manage" dialog: dismissing it only
/// reveals the list again, so it is not animated out like a whole dialog.
pub(crate) fn preset_options_overlay(state: &State) -> Option<Element<'_, Message>> {
    if !preset_options_open(state) {
        return None;
    }

    Some(modal(
        options_card(state),
        Message::MultiFlashOptionsBackdropPressed,
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

/// The "Preset Manage" dialog: the firmware images the run will flash, each
/// with the options it was set up with.
fn preset_card(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let presets = &state.flash.multi_flash.presets;

    let mut list: Vec<Element<'_, Message>> = Vec::new();

    if presets.is_empty() {
        list.push(text(l10n.tr("multi-flash-preset-empty")).size(14.0).into());
    } else {
        for (index, preset) in presets.iter().enumerate() {
            let identity = &preset.identity;

            let mut details = column![
                // The number the conflict warning below names this entry by:
                // two entries set up from a folder each hold a `flashfile.xml`,
                // so the file name tells them apart no better than the path.
                text(format!(
                    "{} {}",
                    preset_number(index),
                    preset.path.display()
                ))
                .size(12.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph),
                // What the package is for: the codename is what tells the
                // entries of a run apart.
                text(labeled(
                    l10n,
                    "multi-flash-preset-codename",
                    identity.codename.as_deref()
                ))
                .size(12.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph),
            ]
            .spacing(2)
            .width(Fill);

            // What the flashfile says the package is (`arcfox_g-user 14
            // U3UXS34.56-124-1-4 … release-keys`).
            details = details.push(
                text(labeled(
                    l10n,
                    "firmware-flash-version",
                    identity.software_version.as_deref()
                ))
                .size(12.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph),
            );

            // The carrier the firmware itself is for, so the list says which
            // phones it fits without opening the entry.
            details = details.push(
                text(labeled(
                    l10n,
                    "firmware-flash-cid",
                    identity.cid.as_deref()
                ))
                .size(12.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph),
            );

            // Which tool flashes this one: a preset carries its own, so a run
            // can use a different `fastboot` per firmware.
            let tool = tool_label(l10n, &preset.tool);
            details = details.push(
                text(labeled(l10n, "firmware-flash-tool", Some(tool.as_str())))
                    .size(12.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph),
            );

            // What the image was set up to do, so the list is not just a pile
            // of file names.
            let conditions = conditions_summary(l10n, preset);
            details = details.push(
                text(labeled(
                    l10n,
                    "multi-flash-options-conditions",
                    Some(conditions.as_str()),
                ))
                .size(12.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph),
            );

            // How many of the package's procedures this preset runs.
            let enabled = preset.enabled.iter().filter(|on| **on).count();
            let steps = format!("{enabled}/{}", preset.identity.steps.len());
            details = details.push(
                text(labeled(l10n, "firmware-flash-steps", Some(steps.as_str())))
                    .size(12.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph),
            );

            if preset.reboot {
                details = details.push(text(l10n.tr("multi-flash-preset-reboot")).size(12.0));
            }

            // Skipping the checksums is not the default, so it is worth
            // pointing out: such a run writes the images without reading them
            // back.
            if !preset.verify_checksums {
                details = details.push(
                    text(l10n.tr("multi-flash-preset-no-verify"))
                        .size(12.0)
                        .width(Fill)
                        .wrapping(Wrapping::WordOrGlyph)
                        .style(warning_orange),
                );
            }
            // The buttons sit on the right of the entry, so the text takes
            // the room they do not need.
            list.push(
                container(
                    row![
                        details,
                        button(text(l10n.tr("multi-flash-preset-edit")).size(12.0))
                            .on_press(Message::MultiFlashPresetEdit(index)),
                        button(text(l10n.tr("multi-flash-preset-remove")).size(12.0))
                            .on_press(Message::MultiFlashPresetRemoved(index)),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                )
                .padding(6)
                .width(Fill)
                .style(container::rounded_box)
                .into(),
            );
        }
    }

    // The actions sit in the top-right corner of the card; the title takes the
    // room they do not need. They are two groups, told apart by a divider: the
    // whole list is written and read first, then the two buttons that change
    // what it holds.
    let save = button(text(l10n.tr("multi-flash-preset-save")).size(12.0));
    let save = if presets.is_empty() {
        save
    } else {
        save.on_press(Message::MultiFlashPresetSave)
    };

    // Two presets that both accept a phone make the run's choice a matter of
    // their order, so the list cannot be closed (a button without `on_press` is
    // greyed out and ignores clicks) until the user took one out or gave them
    // apart.
    let conflicts = multi_flash_run::conflicting_presets(presets);
    let finish = button(text(l10n.tr("multi-flash-preset-finish")).size(12.0));
    let finish = if conflicts.is_empty() {
        finish.on_press(Message::MultiFlashPresetClose)
    } else {
        finish
    };

    // A themed divider (the Fluent rule colour), one logical pixel thick —
    // which is one device pixel at 100% display scaling — and as tall as the
    // buttons beside it. `rule::vertical` takes the THICKNESS; the length comes
    // from the container around it (`Rule::size` fills the other axis).
    let separator: Element<'_, Message> = container(rule::vertical(SEPARATOR_WIDTH))
        .height(HEADER_SEPARATOR_HEIGHT)
        .into();

    let header = row![
        text(l10n.tr("multi-flash-preset-manage"))
            .size(18.0)
            .width(Fill),
        button(text(l10n.tr("multi-flash-preset-load")).size(12.0))
            .on_press(Message::MultiFlashPresetLoad),
        save,
        separator,
        button(text(l10n.tr("multi-flash-preset-add")).size(12.0))
            .on_press(Message::MultiFlashPresetAdd),
        finish,
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    // What the last save or load did, right under the buttons that did it.
    let mut content = column![header, horizontal_rule(1)]
        .spacing(10)
        .width(Fill);
    content = content.extend(status_lines(state));

    // A list two presets of which would flash the same phone is not one a run
    // can work with: which of them a device gets would be decided by their
    // order. Both the list of them and the reason Finish is shut are shown.
    if !conflicts.is_empty() {
        let involved = conflicting_presets(&conflicts);
        let numbers = involved
            .iter()
            .map(|index| preset_number(*index))
            .collect::<Vec<String>>()
            .join(", ");

        content = content.push(status_text(
            l10n.tr_with_args(
                "multi-flash-preset-conflict",
                &[
                    ("count", involved.len().to_string()),
                    ("presets", numbers),
                ],
            ),
            warning_orange,
        ));
    }

    content = content.push(column(list).spacing(4).width(Fill));

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

/// A `label: value` line for the list and the card, with a dash for a value
/// the package does not carry (the way the Firmware Flash dialog shows a
/// missing field).
fn labeled(l10n: &crate::l10n::Bundle, id: &str, value: Option<&str>) -> String {
    format!("{}: {}", l10n.tr(id), value.unwrap_or("—"))
}

/// How an entry of the preset list is pointed at: its place in the list, one
/// based — what the conflict warning names.
fn preset_number(index: usize) -> String {
    format!("#{}", index + 1)
}

/// The entries a conflict is about: every preset that appears in one of the
/// pairs, in the order of the list.
///
/// A pair is not what the user acts on — an entry can conflict with two others
/// — so the warning names each entry once, by its number.
fn conflicting_presets(conflicts: &[(usize, usize)]) -> Vec<usize> {
    let mut involved: Vec<usize> = Vec::new();

    for (first, second) in conflicts {
        for index in [*first, *second] {
            if !involved.contains(&index) {
                involved.push(index);
            }
        }
    }

    involved.sort();
    involved
}

/// What a preset's tool is called in the list and in the dropdown.
///
/// A shipped build is named by its version and Google's copy by its product
/// name (which is not translated); only the built-in engine has a name of its
/// own to localize.
fn tool_label(l10n: &crate::l10n::Bundle, tool: &PresetTool) -> String {
    match tool {
        PresetTool::Builtin => l10n.tr("firmware-flash-tool-builtin"),
        PresetTool::Shipped(version) => format!("mfastboot {version}"),
        PresetTool::PlatformTools => l10n.tr("firmware-flash-tool-platform-tools"),
    }
}

/// What the last save or load of the preset list did, or nothing while there
/// is no news.
fn status_lines(state: &State) -> Vec<Element<'_, Message>> {
    let l10n = &state.l10n;

    match &state.flash.multi_flash.preset_status {
        None => Vec::new(),
        Some(PresetStatus::Saved(path)) => vec![status_text(
            l10n.tr_with_args(
                "multi-flash-preset-saved",
                &[("path", path.display().to_string())],
            ),
            bright_success,
        )],
        Some(PresetStatus::Loaded {
            path,
            count,
            skipped,
        }) => {
            let mut lines = vec![status_text(
                l10n.tr_with_args(
                    "multi-flash-preset-loaded",
                    &[
                        ("count", count.to_string()),
                        ("path", path.display().to_string()),
                    ],
                ),
                bright_success,
            )];

            // An entry whose package is gone (or whose file names an unlock
            // state this release does not know) is left out, and the user is
            // told which ones instead of finding them missing later.
            if !skipped.is_empty() {
                lines.push(status_text(
                    l10n.tr_with_args(
                        "multi-flash-preset-load-skipped",
                        &[("count", skipped.len().to_string())],
                    ),
                    warning_orange,
                ));

                let paths = skipped
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<String>>()
                    .join("\n");
                lines.push(status_text(paths, warning_orange));
            }

            lines
        }
        Some(PresetStatus::Failed(error)) => vec![status_text(
            preset_file_error_text(l10n, error),
            iced::widget::text::danger,
        )],
    }
}

/// A status line of the preset card: small text in one of the status colours,
/// wrapped so a long path does not widen the card.
fn status_text<'a>(
    message: String,
    style: fn(&crate::fluent::Theme) -> iced::widget::text::Style,
) -> Element<'a, Message> {
    text(message)
        .size(12.0)
        .width(Fill)
        .wrapping(Wrapping::WordOrGlyph)
        .style(style)
        .into()
}

/// The message a refused preset file shows: the cases a user can act on have a
/// translation, the rest (the JSON parser, the file system) is technical text
/// shown as it came.
fn preset_file_error_text(
    l10n: &crate::l10n::Bundle,
    error: &crate::preset_file::FileError,
) -> String {
    match error {
        crate::preset_file::FileError::NotAPresetFile => l10n.tr("multi-flash-preset-invalid"),
        crate::preset_file::FileError::Newer(version) => l10n.tr_with_args(
            "multi-flash-preset-newer",
            &[("version", version.to_string())],
        ),
        crate::preset_file::FileError::Other(message) => l10n.tr_with_args(
            "multi-flash-preset-failed",
            &[("error", message.clone())],
        ),
    }
}

/// How a preset's conditions read in the list: the default rule's name, or
/// the CIDs and unlock states a custom condition accepts.
fn conditions_summary(l10n: &crate::l10n::Bundle, preset: &MultiFlashPreset) -> String {
    match &preset.conditions {
        FlashConditions::Default => {
            let rule = l10n.tr("multi-flash-options-default");

            // The one thing that tells two presets of the default rule apart,
            // so it is worth showing.
            if preset.no_engineering {
                format!(
                    "{} · {}",
                    rule,
                    l10n.tr("multi-flash-options-no-engineering")
                )
            } else {
                rule
            }
        }
        FlashConditions::Custom { cids, states } => {
            let states: Vec<String> = states
                .iter()
                .map(|state| l10n.tr(state.message_id()))
                .collect();

            format!("{} · {}", cids.join(", "), states.join(", "))
        }
    }
}

/// The "where does this ZIP go" section of the options card: the directory the
/// package's images are unpacked into, the button that picks or changes it,
/// how far the unpacking is, and why it failed.
///
/// A ZIP whose images are still nowhere (no directory picked, or the
/// unpacking failed) cannot be confirmed — a preset whose firmware is not
/// unpacked has nothing a run could flash.
fn extract_section(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let pending = &state.flash.multi_flash.pending;

    let mut content = column![].spacing(6).width(Fill);

    // The chosen directory, or the placeholder that says there is none yet.
    let mut row: Row<'_, Message> = Row::new()
        .spacing(8)
        .align_y(Alignment::Center)
        .push(
            text(labeled(
                l10n,
                "multi-flash-options-extract",
                pending
                    .extract_to
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .as_deref(),
            ))
            .size(12.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph),
        );

    let choose = button(text(l10n.tr("multi-flash-options-extract-choose")).size(12.0));
    // A directory cannot be changed while it is being written into.
    row = row.push(if pending.unpacking {
        choose
    } else {
        choose.on_press(Message::MultiFlashPresetChooseDirectory)
    });
    content = content.push(row);

    if pending.unpacking {
        content = content.push(
            row![
                spinner(state.anim_tick),
                text(l10n.tr("multi-flash-options-unpacking")).size(12.0),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );

        if let Some((done, total)) = pending.unpacked {
            content = content.push(bar(fraction(done, total)));
        }
    } else if let Some(error) = &pending.unpack_error {
        // The archive was checked before anything was written, so a failure
        // here is the file system (a full disk, no permission) or a ZIP the
        // reader could not take apart.
        content = content.push(
            text(load_error_text(l10n, error))
                .size(12.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph)
                .style(iced::widget::text::danger),
        );
    } else if pending.extract_to.is_none() {
        content = content.push(
            text(l10n.tr("multi-flash-options-extract-required"))
                .size(12.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph)
                .style(warning_orange),
        );
    }

    content.into()
}

/// The tools the options card's dropdown offers: the built-in fastboot, every
/// `mfastboot` build found next to the application (newest first), and
/// Google's copy when this machine has a download for it.
///
/// A preset naming a build this machine does not have keeps its entry, so the
/// dropdown shows what the preset says instead of quietly switching it.
fn tool_options(state: &State) -> Vec<Labeled<PresetTool>> {
    let l10n = &state.l10n;
    let multi = &state.flash.multi_flash;

    let mut options: Vec<Labeled<PresetTool>> = vec![Labeled {
        value: PresetTool::Builtin,
        label: tool_label(l10n, &PresetTool::Builtin),
    }];

    for tool in &multi.mfastboot {
        let value = PresetTool::Shipped(tool.version.clone());
        options.push(Labeled {
            label: tool_label(l10n, &value),
            value,
        });
    }

    if multi.platform_tools.is_some() {
        options.push(Labeled {
            value: PresetTool::PlatformTools,
            label: tool_label(l10n, &PresetTool::PlatformTools),
        });
    }

    let chosen = &multi.pending.tool;

    if !options.iter().any(|option| &option.value == chosen) {
        options.push(Labeled {
            value: chosen.clone(),
            label: tool_label(l10n, chosen),
        });
    }

    options
}

/// The "which tool flashes this" row of the options card: a label and the
/// dropdown that fills the rest of the line.
fn tool_row(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let options = tool_options(state);
    let selected = options
        .iter()
        .find(|option| option.value == state.flash.multi_flash.pending.tool)
        .cloned();

    // The picker's theme cannot be inferred from the closure, so it is spelled
    // out (the iced 0.14 gotcha).
    let picker: iced::widget::PickList<
        '_,
        Labeled<PresetTool>,
        Vec<Labeled<PresetTool>>,
        Labeled<PresetTool>,
        Message,
        crate::fluent::Theme,
    > = pick_list(options, selected, |option| {
        Message::MultiFlashToolSelected(option.value)
    })
    .text_shaping(Shaping::Advanced)
    .width(Fill);

    Row::new()
        .spacing(8)
        .align_y(Alignment::Center)
        .push(text(l10n.tr("firmware-flash-tool")).size(13.0))
        .push(picker)
        .into()
}

/// The Rosetta 2 / box64 hint of the selected tool, when this machine cannot
/// run it yet. `None` while it can, and for a tool the machine does not have
/// at all (there is nothing to install to get it back).
fn tool_hint(state: &State) -> Option<(&'static str, &'static str)> {
    let multi = &state.flash.multi_flash;

    let tool = match &multi.pending.tool {
        PresetTool::Shipped(version) => multi
            .mfastboot
            .iter()
            .find(|tool| &tool.version == version),
        PresetTool::PlatformTools => multi.platform_tools.as_ref(),
        PresetTool::Builtin => None,
    }?;

    tool.emulator.hint_ids()
}

/// The flash-options card: what to do with the firmware image that was just
/// picked, before it joins the preset list.
fn options_card(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let pending = &state.flash.multi_flash.pending;

    let path = pending
        .path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_default();

    let mut content = column![
        text(l10n.tr("multi-flash-options-title")).size(18.0),
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
    .spacing(10)
    .width(Fill);

    // What the package is, or why it could not be read. A file the dialog
    // cannot read is not worth adding, so the card says so and Confirm stays
    // shut until the answer is in.
    if pending.identifying {
        content = content.push(
            row![
                spinner(state.anim_tick),
                text(l10n.tr("firmware-flash-loading")).size(12.0),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );
    } else if let Some(identity) = &pending.identity {
        let line: Option<Element<'_, Message>> = match identity {
            Ok(identity) => Some(
                column![
                    text(labeled(
                        l10n,
                        "multi-flash-preset-codename",
                        identity.codename.as_deref()
                    ))
                    .size(12.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph),
                    text(labeled(
                        l10n,
                        "firmware-flash-version",
                        identity.software_version.as_deref()
                    ))
                    .size(12.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph),
                    // The CID the package itself carries: a run matches a
                    // device on it, and the condition below starts with it.
                    text(labeled(
                        l10n,
                        "firmware-flash-cid",
                        identity.cid.as_deref()
                    ))
                    .size(12.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph),
                ]
                .spacing(2)
                .width(Fill)
                .into(),
            ),
            Err(error) => Some(
                text(load_error_text(l10n, error))
                    .size(12.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph)
                    .style(iced::widget::text::danger)
                    .into(),
            ),
        };

        if let Some(line) = line {
            content = content.push(line);
        }
    }

    content = content.push(horizontal_rule(1));

    // Where a ZIP package's images go. A factory package is gigabytes, so the
    // directory is the user's to choose, and nothing is unpacked before the
    // archive was found to be a firmware package (the identity above).
    if matches!(pending.identity, Some(Ok(_))) && pending.zip() {
        content = content.push(extract_section(state));
    }

    // Which `fastboot` flashes this firmware. The list next to it is the same
    // one the single-device dialog offers, so a preset can use another build
    // than the one that happened to be selected there.
    content = content.push(tool_row(state));

    // The shipped `mfastboot` is an Intel binary: on an ARM machine it only
    // runs through an emulator (Rosetta 2, box64), which may still have to be
    // installed. Say how to get it instead of letting the first step fail.
    if let Some((needed, install)) = tool_hint(state) {
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

    // Whether the images are read back before they are written: it is the
    // single-device dialog's checkbox, and a preset can turn it off to skip
    // the check.
    content = content.push(
        checkbox(l10n.tr("firmware-flash-verify"), pending.verify_checksums)
            .text_shaping(Shaping::Advanced)
            .on_toggle(Message::MultiFlashVerifyToggled),
    );
    content = content.push(
        checkbox(l10n.tr("multi-flash-options-reboot"), pending.reboot)
            .text_shaping(Shaping::Advanced)
            .on_toggle(Message::MultiFlashOptionsRebootToggled),
    );
    content = content.push(horizontal_rule(1));
    content = content.push(text(l10n.tr("multi-flash-options-conditions")).size(14.0));
    content = content.push(conditions_row(l10n, pending.custom));

    if pending.custom {
        content = content.push(cid_section(state));
        content = content.push(state_section(state));

        // A custom condition that names no CID or no unlock state matches no
        // device, so Confirm stays shut until both are there.
        if !pending.complete() {
            content = content.push(
                text(l10n.tr("multi-flash-options-incomplete"))
                    .size(12.0)
                    .width(Fill)
                    .wrapping(Wrapping::WordOrGlyph)
                    .style(iced::widget::text::danger),
            );
        }
    } else {
        content = content.push(
            text(l10n.tr("multi-flash-options-default-desc"))
                .size(12.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph),
        );

        // The CID an engineering bootloader reports is a placeholder
        // (`0xDEAD`), so the default rule takes one whatever the firmware is
        // for. A preset set up for mass flashing usually wants the two are told
        // apart instead, which is what this switch is for.
        content = content.push(
            checkbox(
                l10n.tr("multi-flash-options-no-engineering"),
                pending.no_engineering,
            )
            .text_shaping(Shaping::Advanced)
            .on_toggle(Message::MultiFlashNoEngineeringToggled),
        );
    }

    // What the package does, and what of it this preset should run: the same
    // checklist the single-device dialog offers, so a step erasing user data
    // can be left out here.
    if matches!(pending.identity, Some(Ok(_))) {
        content = content.push(horizontal_rule(1));
        content = content.push(steps_section(state));
    }

    content = content.push(horizontal_rule(1));

    let confirm = button(text(l10n.tr("multi-flash-options-confirm"))).width(Fill);
    let confirm = if pending.complete() {
        confirm.on_press(Message::MultiFlashOptionsConfirm)
    } else {
        confirm
    };

    let cancel = button(text(l10n.tr("login-cancel"))).width(Fill);
    // Unpacking writes gigabytes into the folder the user picked, so the card
    // stays until it is done — the same way the single-device dialog does not
    // let an extraction be abandoned half way.
    let cancel = if pending.unpacking {
        cancel
    } else {
        cancel.on_press(Message::MultiFlashOptionsCancel)
    };

    content = content.push(row![confirm, cancel].spacing(8));

    // The padding sits inside the scrollable, so the card's border is the
    // viewport (the modal convention).
    container(
        scrollable(container(content).padding(16).width(Fill))
            .width(Fill)
            .height(Fill),
    )
    .width(520)
    .height(OPTIONS_CARD_HEIGHT)
    .style(darker_card)
    .into()
}

/// A button that checks every step of one firmware part without unchecking
/// anything else, mirroring the menus of the flashing scripts Motorola ships
/// with the firmware: several parts combine. Drawn disabled when the package
/// has none of them (the count in the label then says `0`).
fn part_button(state: &State, part: FlashPart) -> Element<'_, Message> {
    let l10n = &state.l10n;

    let steps = state
        .flash
        .multi_flash
        .pending
        .identity
        .as_ref()
        .and_then(|identity| identity.as_ref().ok())
        .map(|identity| identity.steps.as_slice())
        .unwrap_or_default();

    // On a MediaTek package `dtbo` counts as part of the bootloader.
    let mediatek = is_mediatek(steps);
    let count = steps
        .iter()
        .filter(|step| step.part(mediatek) == Some(part))
        .count();

    let label = l10n.tr_with_args(
        "firmware-flash-select-part",
        &[
            ("part", part.name().to_owned()),
            ("count", count.to_string()),
        ],
    );

    let select = button(text(label).size(12.0)).padding([4.0, 8.0]);

    if count == 0 {
        select.into()
    } else {
        select
            .on_press(Message::MultiFlashSelectPart(part))
            .into()
    }
}

/// The procedures of the picked package, as the checklist the single-device
/// dialog offers: every step starts checked, and the ones this preset should
/// not run (a step erasing user data, say) can be turned off.
fn steps_section(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let pending = &state.flash.multi_flash.pending;

    let steps = pending
        .identity
        .as_ref()
        .and_then(|identity| identity.as_ref().ok())
        .map(|identity| identity.steps.as_slice())
        .unwrap_or_default();

    // One line holding every control, as in the single-device dialog: the
    // whole checklist first, then one button per firmware part. A language
    // whose labels are longer wraps them onto a second line rather than
    // clipping them.
    let controls = Row::with_children(vec![
        button(text(l10n.tr("firmware-flash-select-all")).size(12.0))
            .padding([4.0, 8.0])
            .on_press(Message::MultiFlashStepsAll(true))
            .into(),
        button(text(l10n.tr("firmware-flash-select-none")).size(12.0))
            .padding([4.0, 8.0])
            .on_press(Message::MultiFlashStepsAll(false))
            .into(),
        part_button(state, FlashPart::Ap),
        part_button(state, FlashPart::Bp),
        part_button(state, FlashPart::Bl),
    ])
    .spacing(6)
    .align_y(Alignment::Center)
    .wrap();

    let mut content = column![
        text(l10n.tr("firmware-flash-steps")).size(14.0),
        controls,
    ]
    .spacing(6)
    .width(Fill);

    if steps.is_empty() {
        return content.into();
    }

    let rows: Vec<Element<'_, Message>> = steps
        .iter()
        .enumerate()
        .map(|(index, step)| -> Element<'_, Message> {
            checkbox(
                format!("{}. {}", index + 1, step.describe()),
                pending.enabled.get(index).copied().unwrap_or(true),
            )
            .text_shaping(Shaping::Advanced)
            .on_toggle(move |checked| Message::MultiFlashStepToggled(index, checked))
            .into()
        })
        .collect();

    // Its own scrollable, so a long procedure list does not push the rest of
    // the card out of view; the id keeps its position across redraws.
    content = content.push(
        container(
            scrollable(
                iced::widget::Column::with_children(rows)
                    .spacing(4)
                    .align_x(Alignment::Start),
            )
            .id(iced::widget::Id::new(STEPS_SCROLL_ID))
            .width(Fill)
            .height(160),
        )
        .padding(6)
        .width(Fill)
        .style(container::rounded_box),
    );

    // A preset that runs nothing would do nothing at all, so Confirm stays
    // shut while that is the case.
    if !pending.enabled.iter().any(|enabled| *enabled) {
        content = content.push(
            text(l10n.tr("firmware-flash-procedures-none"))
                .size(12.0)
                .width(Fill)
                .wrapping(Wrapping::WordOrGlyph)
                .style(iced::widget::text::danger),
        );
    }

    let ignored = pending
        .identity
        .as_ref()
        .and_then(|identity| identity.as_ref().ok())
        .map(|identity| identity.ignored_partitions.join(", "))
        .unwrap_or_default();

    if !ignored.is_empty() {
        content = content.push(
            text(l10n.tr_with_args(
                "firmware-flash-ignored",
                &[("partitions", ignored)],
            ))
            .size(12.0)
            .width(Fill)
            .wrapping(Wrapping::WordOrGlyph)
            .style(crate::bootloader::warning_orange),
        );
    }

    content.into()
}

/// The [Default] [Custom] choice of the options card. The picked one is drawn
/// like the selected tab, so which rule will be saved is obvious.
fn conditions_row(l10n: &crate::l10n::Bundle, custom: bool) -> Element<'_, Message> {
    let default = button(text(l10n.tr("multi-flash-options-default"))).width(Fill);
    let default = if custom {
        default
    } else {
        default.style(button::primary)
    };

    let own = button(text(l10n.tr("multi-flash-options-custom"))).width(Fill);
    let own = if custom {
        own.style(button::primary)
    } else {
        own
    };

    row![
        default.on_press(Message::MultiFlashConditionsDefault),
        own.on_press(Message::MultiFlashConditionsCustom),
    ]
    .spacing(8)
    .into()
}

/// The CID part of a custom condition: the tags picked so far, and the field
/// that adds one more.
fn cid_section(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let pending = &state.flash.multi_flash.pending;

    let tags: Vec<Element<'_, Message>> = pending
        .cids
        .iter()
        .enumerate()
        .map(|(index, cid)| -> Element<'_, Message> {
            container(
                row![
                    text(cid.clone()).size(12.0),
                    button(text("×").size(12.0))
                        .padding(0)
                        .on_press(Message::MultiFlashCidRemoved(index)),
                ]
                .spacing(4)
                .align_y(Alignment::Center),
            )
            .padding(6)
            .style(container::rounded_box)
            .into()
        })
        .collect();

    let mut content = column![text(l10n.tr("multi-flash-options-cid")).size(14.0)]
        .spacing(6)
        .width(Fill);

    if !tags.is_empty() {
        content = content.push(
            Row::with_children(tags)
                .spacing(6)
                .align_y(Alignment::Center)
                .wrap(),
        );
    }

    let add = button(text(l10n.tr("multi-flash-options-cid-add")));
    let add = if pending.cid_input.trim().is_empty() {
        add
    } else {
        add.on_press(Message::MultiFlashCidAdded)
    };

    // The codes the reference documents, so the usual ones do not have to be
    // typed: picking one fills the field, and Add (or Enter) takes it like any
    // typed one. The control always shows its `Preset` placeholder, the way
    // the reboot menu does, so it reads as a menu rather than as a value.
    let preset: iced::widget::PickList<
        '_,
        cid::CidOption,
        Vec<cid::CidOption>,
        cid::CidOption,
        Message,
        crate::fluent::Theme,
    > = pick_list(cid::options(l10n), None, |option| {
        Message::MultiFlashCidPresetSelected(option.code())
    })
    .placeholder(l10n.tr("multi-flash-options-cid-preset"))
    .text_shaping(Shaping::Advanced)
    .style(menu_style);

    content = content.push(
        row![
            preset,
            text_input("", &pending.cid_input)
                .on_input(Message::MultiFlashCidChanged)
                .on_submit(Message::MultiFlashCidAdded)
                .width(Fill),
            add,
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    );

    content.into()
}

/// The unlock-state part of a custom condition.
fn state_section(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let pending = &state.flash.multi_flash.pending;

    let states: Vec<Element<'_, Message>> = CUSTOM_STATES
        .iter()
        .map(|&securestate| -> Element<'_, Message> {
            checkbox(
                l10n.tr(securestate.message_id()),
                pending.states.contains(&securestate),
            )
            .text_shaping(Shaping::Advanced)
            .on_toggle(move |selected| Message::MultiFlashStateToggled(securestate, selected))
            .into()
        })
        .collect();

    column![
        text(l10n.tr("multi-flash-options-states")).size(14.0),
        Row::with_children(states)
            .spacing(12)
            .align_y(Alignment::Center)
            .wrap(),
    ]
    .spacing(6)
    .width(Fill)
    .into()
}

/// The prompt that a window close was refused because a run is going on.
fn close_prompt_card(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let multi = &state.flash.multi_flash;

    // Once the run was stopped, what keeps the window open is the devices that
    // are still being written to — telling the user to stop again would be
    // advice they already followed.
    let message = if multi.running {
        l10n.tr("multi-flash-close-prompt")
    } else {
        l10n.tr_with_args(
            "multi-flash-still-flashing",
            &[("count", multi.flashing.len().to_string())],
        )
    };

    let content = column![
        text(message)
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
