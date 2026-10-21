// Release builds run without a console window; debug builds keep one so
// `eprintln!` diagnostics remain visible with `cargo run`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod about;
mod anim;
mod avb;
mod bloatware;
mod bootloader;
mod browser;
mod bulk;
mod captive_portal;
mod captive_portal_dialog;
mod carrier;
mod cid;
mod compat_iced;
mod config;
mod debloat;
mod decrypt;
mod decrypt_mode;
mod driver;
mod driver_install;
mod factory_reset;
mod fastboot_info;
mod firehose;
mod firmware;
mod firmware_flash;
mod flash_engine;
mod flash_log;
mod flash_script;
mod flashfile;
mod fluent;
mod guided;
mod image_unpack;
mod instance;
mod l10n;
mod lenovoubl;
mod login;
mod lookup;
mod multi_flash;
mod multi_flash_run;
mod pac;
mod pac_build;
mod pac_repack;
mod pac_unpack;
mod partition_flash;
mod platform_tools;
mod preset_file;
mod protocol;
mod qualcomm;
mod qualcomm_ports;
mod scatter;
mod smartphone;
mod spd_script;
mod tablet_bootloader;
mod tablet_unlock;
mod telemetry;
mod unpack;
mod webview;
mod wireless;
mod wireless_dialog;

use iced::widget::{
    button, column, container, mouse_area, pick_list, row, stack, text_editor,
};
use iced::widget::text::Shaping;
use iced::{Alignment, Fill, Font, Size, Task};

use crate::compat_iced as iced;
use crate::compat_iced::*;
use smartphone::SmartphoneFeature;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// The application's element type.
///
/// In iced 0.14 a theme is a *type* — it implements [`iced::theme::Base`] and
/// provides the widget `Catalog` implementations — so the Fluent theme is part
/// of every widget's type. This alias keeps a bare `Element<'_, Message>`
/// meaning the Fluent-themed element everywhere.
pub(crate) type Element<'a, M = Message> = ::iced::Element<'a, M, fluent::Theme>;

/// A Fluent-themed [`iced::widget::Text`] (see [`Element`]).
pub(crate) type Text<'a> = ::iced::widget::Text<'a, fluent::Theme>;

/// A `text` label rendered with advanced text shaping.
///
/// Advanced shaping makes missing glyphs (e.g. CJK on a system whose UI
/// language is not Chinese) fall back through the installed system fonts,
/// so text renders correctly regardless of the OS language.
fn text<'a>(
    fragment: impl iced::widget::text::IntoFragment<'a>,
) -> Text<'a> {
    iced::widget::text(fragment).shaping(Shaping::Advanced)
}

pub fn main() -> ::iced::Result {
    // When started with `--login-dialog`, this process only shows the WebView
    // login dialog and exits (winit allows one event loop per process).
    if let Some(code) = webview::run_dialog_process_if_requested() {
        std::process::exit(code);
    }

    // A `softwarefix://` callback (the browser login redirect) is handed to the
    // running instance when there is one, so the login completes in the window
    // the user is already looking at. This runs before any other startup work,
    // because a forwarding process exits again right away.
    let callback = protocol::callback_arg();
    let receiver = match instance::start(callback.as_deref()) {
        instance::Endpoint::Primary(receiver) => Some(receiver),
        instance::Endpoint::Secondary => {
            if callback.is_some() {
                // The URL was delivered to the running instance.
                return Ok(());
            }

            // A plain second launch still opens its own window.
            None
        }
        instance::Endpoint::Disabled => None,
    };

    // macOS does not start a second copy for a `softwarefix://` link: the
    // running application receives the URL in an Apple Event, which only this
    // process can handle (see `protocol`). Installing the handler this early
    // means a cold start — LaunchServices opening the application *because* of
    // the link — is caught as well. It is `None` on the platforms that forward
    // the callback instead.
    let events = protocol::listen_for_callbacks();

    // Warm the WebView2 availability cache before the first frame. The probe
    // spawns `reg.exe`; without this the render that first shows the
    // Bootloader Unlock chooser (whose "Guided" button is gated on it) would
    // stall on that process spawn.
    let _ = webview::webview_available();

    // Same for the default browser: the login page names the cookie-clearing
    // steps of the browser that is set as default, and reading that spawns a
    // helper process on macOS/Linux.
    browser::warm();

    let icon = iced::window::icon::from_file_data(include_bytes!("icon.ico"), None).ok();

    // iced 0.14 boots the application through a `Fn` closure (it may be called
    // more than once), so the callback receivers are handed over through a cell.
    let receiver = Arc::new(std::sync::Mutex::new(Some((receiver, events))));

    let application = iced::application(
        move || {
            let mut state = State::initial();

            // Only the primary instance receives forwarded callbacks, and only
            // macOS receives them inside the process. Either receiver lives as
            // long as the process: a later instance (or the browser) hands the
            // `softwarefix://` callback over here.
            let (instance, events) = receiver
                .lock()
                .expect("boot runs once")
                .take()
                .unwrap_or((None, None));

            let mut callbacks = Vec::new();

            for receiver in [instance, events].into_iter().flatten() {
                state.receives_callbacks = true;
                callbacks.push(Task::run(receiver, Message::LoginCallback));
            }

            // Ask for the OS colour scheme so the Fluent theme matches it from
            // the first frame on.
            let theme = iced::system::theme().map(Message::SystemTheme);

            (state, Task::batch([Task::batch(callbacks), theme]))
        },
        update,
        view,
    )
    .title(|state: &State| state.l10n.tr("app-title"))
    .default_font(system_ui_font())
    .theme(app_theme)
    .window(iced::window::Settings {
        icon,
        // The app answers close requests itself (see
        // `Message::WindowCloseRequested`), so a Multi Flashing run can
        // refuse to be cut short.
        exit_on_close_request: false,
        ..iced::window::Settings::default()
    })
    .window_size(Size::new(720.0, 720.0))
    .centered()
    .subscription(subscription);

    // The Fluent theme styles its text with `Inter` (see
    // `iced_fluent_theme::font`), and ships the face, so it has to be
    // registered here or every label the theme styles would fall back to an
    // arbitrary system font. Glyphs Inter does not cover — CJK, for one —
    // still fall back through the system font database, and `default_font`
    // above stays the CJK-aware system UI font for everything the app styles
    // itself.
    let application = iced_fluent_theme::font::load()
        .into_iter()
        .fold(application, |application, font| application.font(font));

    application.run()
}

/// The application theme: the Fluent variant matching the OS colour scheme.
fn app_theme(state: &State) -> fluent::Theme {
    fluent::theme_for(state.dark)
}

/// True for Traditional-Chinese OS locales (Taiwan, Hong Kong, Macau, or an
/// explicit `Hant` script tag).
fn is_traditional_chinese(locale: &str) -> bool {
    let normalized = locale.replace('_', "-").to_ascii_lowercase();

    normalized.contains("hant")
        || normalized.contains("-tw")
        || normalized.contains("-hk")
        || normalized.contains("-mo")
}

/// Picks a system font family that matches the OS display language.
///
/// Requires iced's `system` feature so OS fonts are loaded; missing glyphs
/// (e.g. emoji, CJK) then fall back through the system font database.
fn system_ui_font() -> Font {
    let locale = sys_locale::get_locale().unwrap_or_default();
    let language = locale.split(['-', '_']).next().unwrap_or_default();

    let family = match language {
        "zh" => {
            if is_traditional_chinese(&locale) {
                #[cfg(target_os = "windows")]
                {
                    "Microsoft JhengHei"
                }
                #[cfg(target_os = "macos")]
                {
                    "PingFang TC"
                }
                #[cfg(not(any(target_os = "windows", target_os = "macos")))]
                {
                    "Noto Sans CJK TC"
                }
            } else {
                #[cfg(target_os = "windows")]
                {
                    "Microsoft YaHei"
                }
                #[cfg(target_os = "macos")]
                {
                    "PingFang SC"
                }
                #[cfg(not(any(target_os = "windows", target_os = "macos")))]
                {
                    "Noto Sans CJK SC"
                }
            }
        }
        "ja" => {
            #[cfg(target_os = "windows")]
            {
                "Yu Gothic UI"
            }
            #[cfg(target_os = "macos")]
            {
                "Hiragino Sans"
            }
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            {
                "Noto Sans CJK JP"
            }
        }
        "ko" => {
            #[cfg(target_os = "windows")]
            {
                "Malgun Gothic"
            }
            #[cfg(target_os = "macos")]
            {
                "Apple SD Gothic Neo"
            }
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            {
                "Noto Sans CJK KR"
            }
        }
        _ => {
            #[cfg(target_os = "windows")]
            {
                "Segoe UI"
            }
            #[cfg(target_os = "macos")]
            {
                "Helvetica Neue"
            }
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            {
                "Noto Sans"
            }
        }
    };

    Font::with_name(family)
}

/// The font tool output and table data are written in.
///
/// iced's [`iced::Font::MONOSPACE`] is a *generic* family and iced bundles no
/// font of its own, so the system picks it — and on a machine whose monospace
/// font is a CJK one that is a trap: those fonts map ASCII `0x5C` to their own
/// character, so a Korean font shows the won sign (`₩`) where the output had a
/// backslash. Naming the platform's classic monospace font keeps the output
/// ASCII-shaped.
pub(crate) fn output_font() -> Font {
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

    Font::with_name(family)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Mode {
    #[default]
    Mode1,
    Mode2,
    Mode3,
}

impl Mode {
    const ALL: [Self; 3] = [Self::Mode1, Self::Mode2, Self::Mode3];

    fn message_id(self) -> &'static str {
        match self {
            Self::Mode1 => "mode-1",
            Self::Mode2 => "mode-2",
            Self::Mode3 => "mode-3",
        }
    }
}

/// Which mouse button triggered a login request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Click {
    Left,
    Right,
}

/// Device family selected in the firmware lookup dropdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum LookupMode {
    #[default]
    RowSmartphone,
    RetcnSmartphone,
    Tablet,
    ByModel,
    BulkImei,
}

impl LookupMode {
    const ALL: [Self; 5] = [
        Self::RowSmartphone,
        Self::RetcnSmartphone,
        Self::Tablet,
        Self::ByModel,
        Self::BulkImei,
    ];

    fn message_id(self) -> &'static str {
        match self {
            Self::RowSmartphone => "lookup-mode-row",
            Self::RetcnSmartphone => "lookup-mode-retcn",
            Self::Tablet => "lookup-mode-tablet",
            Self::ByModel => "lookup-mode-by-model",
            Self::BulkImei => "lookup-mode-bulk-imei",
        }
    }
}

/// A dropdown option pairing a value with its localized label.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Labeled<T> {
    value: T,
    label: String,
}

impl<T> std::fmt::Display for Labeled<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

#[derive(Debug, Clone)]
enum LookupResult {
    Standard(firmware::FirmwareInfo),
    CnTablet(firmware::CnTabletInfo),
}

#[derive(Debug, Clone)]
enum LookupStatus {
    Idle,
    Fetching,
    Done(LookupResult),
    Error(String),
}

impl Default for LookupStatus {
    fn default() -> Self {
        Self::Idle
    }
}

/// One row of the bulk-IMEI lookup report.
///
/// The status strings are the literal column values of the CSV report, so they
/// stay stable no matter what the UI language is.
#[derive(Debug, Clone)]
struct BulkRow {
    imei: String,
    status: &'static str,
    xt_code: String,
    carrier: String,
    build_fingerprint: String,
    download_link: String,
    lolinet_filename: String,
    assumed_directory: String,
}

/// CSV status of a row whose firmware was found.
const BULK_STATUS_OK: &str = "ok";
/// CSV status of a valid IMEI the server has no build for.
const BULK_STATUS_MISSING: &str = "not found";
/// CSV status of a line that is not a valid IMEI.
const BULK_STATUS_INVALID: &str = "invalid imei";
/// CSV status of a row whose lookup failed (network/auth error).
const BULK_STATUS_FAILED: &str = "error";

impl BulkRow {
    /// A row for a found build: every column comes from the lookup result.
    fn found(imei: &str, info: &firmware::FirmwareInfo) -> Self {
        let xt_code = if info.model_name.trim().is_empty() {
            info.sale_model.trim()
        } else {
            info.model_name.trim()
        };

        Self {
            imei: imei.to_string(),
            status: BULK_STATUS_OK,
            xt_code: xt_code.to_string(),
            carrier: info.carrier.trim().to_string(),
            build_fingerprint: info.fingerprint.trim().to_string(),
            download_link: info.rom_uri.trim().to_string(),
            lolinet_filename: firmware::lolinet_filename(info),
            assumed_directory: firmware::lolinet_directory(info).unwrap_or_default(),
        }
    }

    /// A row with only an IMEI and a status (invalid / missing / failed).
    fn skipped(imei: &str, status: &'static str) -> Self {
        Self {
            imei: imei.to_string(),
            status,
            xt_code: String::new(),
            carrier: String::new(),
            build_fingerprint: String::new(),
            download_link: String::new(),
            lolinet_filename: String::new(),
            assumed_directory: String::new(),
        }
    }
}

/// Progress of the bulk-IMEI lookup.
#[derive(Debug, Clone, Default)]
enum BulkStatus {
    #[default]
    Idle,
    /// `current` of `total` IMEIs have been (or are being) looked up.
    Running { current: usize, total: usize },
    /// The batch finished; [`BulkInput::rows`] holds its report.
    Done,
    Error(String),
}

/// Inputs and progress for the bulk-IMEI lookup.
#[derive(Default)]
struct BulkInput {
    /// The multi-line IMEI box (one IMEI per line).
    content: text_editor::Content,
    /// The non-empty lines captured when the batch started.
    queue: Vec<String>,
    /// Index of the next [`BulkInput::queue`] entry to look up.
    index: usize,
    /// The report rows collected so far (parallel to the processed lines).
    rows: Vec<BulkRow>,
    status: BulkStatus,
    /// Outcome of the last save dialog (`Ok(path)` = written, `Err` = failed).
    save_note: Option<Result<String, String>>,
}

/// A text field of the RETCN lookup form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetcnField {
    SerialNumber,
    Fingerprint,
    Model,
    Carrier,
    FsgVersion,
}

/// State of the "fill from fastboot" feature.
#[derive(Debug, Clone)]
enum FastbootStatus {
    Fetching,
    Filled(String),
    Error(String),
}

/// Modal state for choosing among multiple fastboot devices.
#[derive(Debug, Clone, Default)]
enum DevicePicker {
    #[default]
    Closed,
    Fetching,
    Open(Vec<fastboot_info::DeviceEntry>),
}

/// Status of the firmware-decrypt mode (Mode 3).
#[derive(Debug, Clone, Default)]
enum DecryptStatus {
    #[default]
    Idle,
    PickingDir,
    Working,
    Done(decrypt::DecryptSummary),
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum SimCount {
    #[default]
    Single,
    Dual,
}

impl SimCount {
    const ALL: [Self; 2] = [Self::Single, Self::Dual];

    fn message_id(self) -> &'static str {
        match self {
            Self::Single => "sim-single",
            Self::Dual => "sim-dual",
        }
    }

    fn count(self) -> u8 {
        match self {
            Self::Single => 1,
            Self::Dual => 2,
        }
    }
}

/// Inputs for the RETCN smartphone lookup.
#[derive(Default)]
struct RetcnInput {
    serial_number: String,
    fingerprint: String,
    model: String,
    carrier: String,
    platform: firmware::Platform,
    fsg_version: String,
    sim_count: SimCount,
    fastboot_status: Option<FastbootStatus>,
    device_picker: DevicePicker,
}

/// Inputs for the tablet lookup.
#[derive(Default)]
struct TabletInput {
    serial_number: String,
}

/// Inputs for the "Lookup by Model" lookup.
struct ByModelInput {
    model: String,
    /// Discriminator parameter names the server needs for this model.
    required: Vec<String>,
    /// The model `required` was fetched for (refetched when it changes).
    loaded_model: String,
    /// Current value of each required parameter (parallel to `required`).
    values: Vec<String>,
    /// Device category; auto-detected from the model until overridden.
    category: firmware::Category,
    /// False once the user picks a category manually (stops auto-detection).
    category_auto: bool,
    /// Two-letter country code, sent for tablets/smart devices (e.g. "US").
    country_code: String,
}

impl Default for ByModelInput {
    fn default() -> Self {
        Self {
            model: String::new(),
            required: Vec::new(),
            loaded_model: String::new(),
            values: Vec::new(),
            category: firmware::Category::Phone,
            category_auto: true,
            country_code: "US".to_string(),
        }
    }
}

/// Infers the device category (and sometimes the country) from a model number.
///
/// - `TB…` → L tablet
/// - `XT…` → M smartphone
/// - `PC-…` → N tablet, country JP
/// - `CD…` / `SD…` → L smart device
fn detect_category(model: &str) -> Option<(firmware::Category, Option<&'static str>)> {
    let model = model.trim();
    if model.starts_with("XT") {
        Some((firmware::Category::Phone, None))
    } else if model.starts_with("TB") {
        Some((firmware::Category::Tablet, None))
    } else if model.starts_with("PC-") {
        Some((firmware::Category::Tablet, Some("JP")))
    } else if model.starts_with("CD") || model.starts_with("SD") {
        Some((firmware::Category::Smart, None))
    } else {
        None
    }
}

/// Inputs for the firmware-decrypt mode (Mode 3).
#[derive(Default)]
struct DecryptState {
    /// Whether the "Tablet Firmware Decrypt" modal is open.
    open: bool,
    directory: Option<std::path::PathBuf>,
    custom_password: bool,
    password: String,
    status: DecryptStatus,
}

/// One firmware image of a Multi Flashing run, with what to do with it.
#[derive(Debug, Clone)]
struct MultiFlashPreset {
    /// The firmware package (a ZIP, or a `flashfile.xml` with its images).
    path: std::path::PathBuf,
    /// What the package says about itself. Its codename is what tells the
    /// entries apart (a package without a `vbmeta` image has none).
    identity: flashfile::PackageIdentity,
    /// Which of its procedures run: one flag per `identity.steps`, so a preset
    /// can leave out a step that erases user data. A missing entry counts as
    /// checked.
    enabled: Vec<bool>,
    /// Reboot the device once this preset finished.
    reboot: bool,
    /// Verify the `MD5` of every image before writing it. A preset can skip
    /// that, which is what the "skip the checksum" option is for: it costs a
    /// read of the whole image per step.
    verify_checksums: bool,
    /// The tool this preset's run uses.
    tool: PresetTool,
    /// Where the firmware's files were unpacked, for a ZIP package (a
    /// `flashfile.xml` package is already unpacked next to its images).
    extract_to: Option<std::path::PathBuf>,
    /// Which devices this firmware may be flashed to.
    conditions: FlashConditions,
    /// `true` when the default rule is told not to take an engineering unit:
    /// such a bootloader reports the placeholder `0xDEAD` instead of a real
    /// CID, so it would be flashed whatever the firmware is for. A custom
    /// condition says so itself, through the unlock states it lists.
    no_engineering: bool,
}

/// The tool a preset's run uses.
///
/// The single-device dialog picks one of these too; here it belongs to a
/// preset, so a run can use a different `fastboot` per firmware (a package its
/// own build was made for, say). The tool is stored by *name*, not by path, so
/// a saved preset still reads on a machine that keeps its builds elsewhere.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PresetTool {
    /// The built-in `fastboot` crate.
    Builtin,
    /// One of the `mfastboot` builds shipped next to the application, named by
    /// the version its directory carries.
    Shipped(String),
    /// Google's `fastboot` from the Android platform-tools.
    PlatformTools,
}

/// Why the extraction directory of a ZIP preset was asked for.
///
/// The two cases differ in what a cancelled pick means: the first one belongs
/// to the flow that adds a firmware, so cancelling it is the user saying no to
/// that firmware, while changing a directory is a correction of something that
/// is already set up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectoryAsk {
    /// The picked archive turned out to be a firmware package and has no
    /// directory yet.
    New,
    /// The card's own button, changing a directory that is set already.
    Change,
}

/// When a preset's firmware may be flashed to a device.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FlashConditions {
    /// The rule the single-device dialog applies: the CID has to match the
    /// firmware, and an engineering bootloader — which ignores the CID and
    /// reports the placeholder `0xDEAD` instead — matches as well unless
    /// [`MultiFlashPreset::no_engineering`] was set.
    Default,
    /// The user's own list of acceptable CIDs and bootloader unlock states.
    Custom {
        cids: Vec<String>,
        states: Vec<fastboot_info::SecureState>,
    },
}

/// The "flash options" card's inputs: what the firmware image that was just
/// picked is set up to do before it joins the preset list.
struct PresetOptions {
    /// The image the card configures; `None` while the card is closed.
    path: Option<std::path::PathBuf>,
    /// The entry of the preset list the card changes, when it was opened from
    /// the list; `None` sets up a new one.
    index: Option<usize>,
    /// What the package says about itself, once it was read.
    identity: Option<Result<flashfile::PackageIdentity, flashfile::LoadError>>,
    /// `true` while it is being read.
    identifying: bool,
    /// One flag per procedure of the identified package (parallel to its
    /// `steps`): only the checked ones run, so a preset can leave out a step
    /// that erases user data. A missing entry counts as checked.
    enabled: Vec<bool>,
    /// Reboot the device once this preset finished.
    reboot: bool,
    /// Verify the `MD5` of every image before writing it.
    verify_checksums: bool,
    /// The tool this preset's run uses.
    tool: PresetTool,
    /// Where the firmware's files go, for a ZIP package: a factory package is
    /// gigabytes of images, so the user picks the directory they are written
    /// to (a package that is already unpacked needs none).
    extract_to: Option<std::path::PathBuf>,
    /// `true` while that directory is being written.
    unpacking: bool,
    /// How far the unpacking is, in bytes.
    unpacked: Option<(u64, u64)>,
    /// Why it failed, when it did.
    unpack_error: Option<flashfile::LoadError>,
    /// `true` when the user defines the conditions themselves instead of
    /// taking the default rule.
    custom: bool,
    /// `true` while the default rule is told not to take an engineering unit:
    /// such a bootloader reports the placeholder `0xDEAD` instead of a real
    /// CID, so a run would flash it whatever the firmware is for.
    no_engineering: bool,
    /// The CIDs a custom condition accepts, as they were entered.
    cids: Vec<String>,
    /// What the CID field holds right now.
    cid_input: String,
    /// The bootloader unlock states a custom condition accepts.
    states: Vec<fastboot_info::SecureState>,
}

impl Default for PresetOptions {
    /// A card that was just opened runs every procedure, verifies the image
    /// checksums, reboots the phone when it is done and leaves engineering
    /// units out — the set-up a batch of retail phones is flashed with. The
    /// tool is filled in from the machine's own builds when the pick opens.
    fn default() -> Self {
        Self {
            path: None,
            index: None,
            identity: None,
            identifying: false,
            enabled: Vec::new(),
            reboot: true,
            verify_checksums: true,
            tool: PresetTool::Builtin,
            extract_to: None,
            unpacking: false,
            unpacked: None,
            unpack_error: None,
            custom: false,
            no_engineering: true,
            cids: Vec::new(),
            cid_input: String::new(),
            states: Vec::new(),
        }
    }
}

impl PresetOptions {
    /// Whether this configuration can be confirmed: the image has to be a
    /// readable package with at least one checked procedure, its files have to
    /// be in place, and a custom condition has to name at least one CID and
    /// one unlock state.
    fn complete(&self) -> bool {
        let conditions =
            !self.custom || (!self.cids.is_empty() && !self.states.is_empty());
        let identity = matches!(self.identity, Some(Ok(_)));

        conditions && identity && self.unpacked() && self.enabled.iter().any(|enabled| *enabled)
    }

    /// Whether the picked package is a ZIP, whose images have to be unpacked
    /// into [`PresetOptions::extract_to`] before a run can flash them.
    fn zip(&self) -> bool {
        self.path.as_deref().is_some_and(flashfile::is_zip)
    }

    /// Whether the firmware's files are in place: a package that is unpacked
    /// next to its `flashfile.xml` already is, a ZIP only once it finished
    /// unpacking into the directory the user chose.
    fn unpacked(&self) -> bool {
        !self.zip()
            || (self.extract_to.is_some() && !self.unpacking && self.unpack_error.is_none())
    }

    /// The CID the package itself carries, which a condition starts with: the
    /// firmware says which carrier it is for, and that is the code a phone of
    /// that carrier reports. A package that names none leaves the list to the
    /// user.
    fn default_cids(&self) -> Vec<String> {
        match &self.identity {
            Some(Ok(identity)) => identity.cid.iter().cloned().collect(),
            _ => Vec::new(),
        }
    }

    /// The card that changes an entry of the list, filled in with what that
    /// entry holds now.
    fn from_preset(preset: &MultiFlashPreset, index: usize) -> Self {
        let (custom, cids, states) = match &preset.conditions {
            FlashConditions::Default => (false, Vec::new(), Vec::new()),
            FlashConditions::Custom { cids, states } => (true, cids.clone(), states.clone()),
        };

        Self {
            path: Some(preset.path.clone()),
            index: Some(index),
            identity: Some(Ok(preset.identity.clone())),
            identifying: false,
            enabled: preset.enabled.clone(),
            reboot: preset.reboot,
            verify_checksums: preset.verify_checksums,
            tool: preset.tool.clone(),
            extract_to: preset.extract_to.clone(),
            unpacking: false,
            unpacked: None,
            unpack_error: None,
            custom,
            no_engineering: preset.no_engineering,
            cids,
            cid_input: String::new(),
            states,
        }
    }
}

/// State of the "Multi Flashing" page (Mode 2).
///
/// The page is reached from the Firmware Flash dialog and takes the place of
/// the feature grid; several devices are flashed in one run there.
#[derive(Default)]
struct MultiFlashState {
    /// Whether the page is shown instead of the feature grid.
    open: bool,
    /// The firmware images of the run, each with the options it was set up
    /// with. The "Preset Manage" dialog fills them in, and starting a run
    /// needs at least one.
    presets: Vec<MultiFlashPreset>,
    /// Whether the "Preset Manage" dialog is open.
    preset_dialog: bool,
    /// The flash-options card a picked image is configured with.
    pending: PresetOptions,
    /// The `mfastboot` builds found next to the application when the page was
    /// entered (newest first) — the tools a preset may pick from.
    mfastboot: Vec<flash_engine::Mfastboot>,
    /// Google's `fastboot`, when this machine has a download for it. Built
    /// with `mfastboot` above, so the picker does not ask the system (Rosetta
    /// 2, box64) on every frame.
    platform_tools: Option<flash_engine::Mfastboot>,
    /// What the last save or load of the preset list did; shown above the
    /// list until the dialog is closed.
    preset_status: Option<PresetStatus>,
    /// `true` while a run goes on: the workspace is locked, "Start" reads
    /// "Stop", and the window cannot be closed.
    running: bool,
    /// The devices the run found, in the order they appeared.
    devices: Vec<MultiFlashDevice>,
    /// The serials whose flash has not reported its end yet. A device that was
    /// stopped is *not* taken out of it: its flash runs to the end in the
    /// background, and the window stays unclosable until it does.
    flashing: std::collections::HashSet<String>,
    /// Counts the runs, so the events of a session that was stopped (or that
    /// ran before) are not applied to what the user sees now.
    session: u64,
    /// Stops the session's watch for devices; taken out of the way (`None`)
    /// once it was asked to stop.
    stop: Option<Arc<AtomicBool>>,
    /// Where this run's logs go, once the session said so.
    log_directory: Option<std::path::PathBuf>,
    /// Why the run could not start (its log directory could not be made).
    run_error: Option<String>,
    /// Whether a close request was refused because a run was going on; the
    /// prompt stays up until it is acknowledged.
    close_prompt: bool,
}

/// State of the "Partition Manager" page (Mode 2), in its Qualcomm form.
///
/// The page is reached from the feature grid and takes its place; it shows the
/// partition table of a phone read through Firehose. The table stays empty
/// until the programmer is injected; after that, its rows can be dumped,
/// written, erased or read again.
#[derive(Default)]
struct QualcommState {
    /// Whether the page is shown instead of the feature grid.
    open: bool,
    /// What the partition search box contains. It only filters what the table
    /// shows.
    filter: String,
    /// The port the Firehose connection will use, once the dropdown offers
    /// the ones that enumerate as 05C6:9008.
    port: Option<String>,
    /// The COM ports (or `ttyUSB*` devices) a Firehose device offers right
    /// now; the dropdown is refreshed from this list about once a second.
    ports: Vec<String>,
    /// The storage the session talks to; the default lets the phone report it.
    storage: StorageType,
    /// The storage the phone actually reported, once a table was read.
    detected: Option<StorageType>,
    /// The partition table the injection read.
    partitions: Vec<firehose::Partition>,
    /// Which rows of the table are selected, parallel to `partitions`. The
    /// table is a multi-select list: a plain click selects one row, Command
    /// (or Control) toggles a row, Shift extends a range, and Control+A takes
    /// every row on screen.
    selected: Vec<bool>,
    /// Which row a Shift-click extends a range from, as a partition index.
    anchor: Option<usize>,
    /// The modifiers of the last keyboard event, so a row click knows whether
    /// Command/Control (toggle one row) or Shift (extend a range) was held.
    modifiers: iced::keyboard::Modifiers,
    /// How far the running job is, over every selected partition: the bar
    /// under the table. `None` when nothing runs.
    progress: Option<firehose::Progress>,
    /// What the page is doing, or how the last action went.
    status: QualcommStatus,
    /// Whether a Firehose job (an injection, a dump, a write or a reset) is
    /// running.
    busy: bool,
    /// Whether the programmer is running on the phone. It stays in RAM after
    /// the injection's port was closed, so "Inject Firehose" has nothing to
    /// do until the phone is reset or another port is picked.
    programmer_uploaded: bool,
    /// The detailed log the last Firehose session wrote, so the page can show
    /// it in the file manager.
    log_path: Option<std::path::PathBuf>,
    /// The LUN picker "Dump Entire Storage" opens on a storage that is split
    /// into physical partitions.
    lun_dialog: Option<LunDialog>,
    /// The LUNs the picker settled on, kept while the save dialog runs.
    pending_luns: Vec<firehose::Lun>,
    /// The window a close request asked to close, while the prompt that asks
    /// first is up: a loaded programmer is a live Firehose session, and closing
    /// the window leaves the phone in it.
    close_prompt: Option<iced::window::Id>,
}

/// The LUN picker "Dump Entire Storage" opens on a UFS phone.
///
/// It has to ask the phone which physical partitions it has before it can list
/// them: a LUN a partition table is not in — where UFS keeps user data — has no
/// other way of being found.
enum LunDialog {
    /// The phone is being asked about its physical partitions.
    Reading,
    /// The phone answered; `checked` is parallel to `luns`.
    Luns {
        luns: Vec<firehose::Lun>,
        checked: Vec<bool>,
    },
    /// The question failed; the card says why.
    Failed(String),
}

/// What the Qualcomm page's status line says.
///
/// The error text is kept as the raw message the failing step produced, and
/// is only put into a sentence when the page is drawn — a language switch
/// then re-renders it in the new language.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum QualcommStatus {
    /// Nothing has been done yet (or the last table was discarded).
    #[default]
    Idle,
    /// The programmer is being uploaded and the table read.
    Injecting,
    /// The programmer is running and its partition table is being read.
    Reading,
    /// The table was read; the count is how many partitions it holds.
    Table { count: usize },
    /// The injection failed; the text says why.
    InjectFailed(String),
    /// The phone is being reset.
    Rebooting,
    /// The reset command went through.
    Rebooted,
    /// The reset failed; the text says why.
    RebootFailed(String),
    /// One partition is being read into a file; `done` of `total` bytes.
    Dumping { name: String, done: u64, total: u64 },
    /// One image is being written to a partition.
    Writing { name: String, done: u64, total: u64 },
    /// The sectors of one partition are being erased.
    Erasing { name: String, done: u64, total: u64 },
    /// The partitions were dumped into a folder.
    DumpDone {
        count: usize,
        directory: std::path::PathBuf,
    },
    /// The dump failed; the text says why.
    DumpFailed(String),
    /// An image was written to a partition.
    WriteDone(String),
    /// The write failed; the text says why.
    WriteFailed(String),
    /// The selected partitions were erased.
    EraseDone { count: usize },
    /// The erase failed; the text says why.
    EraseFailed(String),
    /// The partition table is being read again.
    Refreshing,
    /// Reading the table again failed; the text says why.
    RefreshFailed(String),
    /// Whole physical partitions were dumped; `count` images sit at `path`.
    StorageDumpDone {
        count: usize,
        path: std::path::PathBuf,
    },
    /// The storage dump failed; the text says why.
    StorageDumpFailed(String),
}

/// The modes the page's Reboot menu offers.
///
/// Every one of them is a Firehose session of its own: the mode decides what is
/// written into the storage — the OEM-unlocking flag in FRP/config, or a boot
/// command in `misc` — before the phone is reset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RebootMode {
    Normal,
    FactoryReset,
    OemUnlock,
    Recovery,
    Fastbootd,
}

impl RebootMode {
    /// The entries the menu shows, in order.
    const ALL: [Self; 5] = [
        Self::Normal,
        Self::FactoryReset,
        Self::OemUnlock,
        Self::Recovery,
        Self::Fastbootd,
    ];

    /// The FTL id of its label.
    fn message_id(self) -> &'static str {
        match self {
            Self::Normal => "qc-reboot-normal",
            Self::FactoryReset => "flash-factory-reset-title",
            Self::OemUnlock => "qc-reboot-oem-unlock",
            Self::Recovery => "qc-reboot-recovery",
            Self::Fastbootd => "qc-reboot-fastbootd",
        }
    }

    /// The work the mode asks the programmer for.
    fn action(self) -> firehose::RebootAction {
        match self {
            Self::Normal => firehose::RebootAction::Normal,
            Self::FactoryReset => firehose::RebootAction::FactoryReset,
            Self::OemUnlock => firehose::RebootAction::OemUnlock,
            Self::Recovery => firehose::RebootAction::Recovery,
            Self::Fastbootd => firehose::RebootAction::Fastbootd,
        }
    }
}

/// The storage a Qualcomm phone can be told to work with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum StorageType {
    /// Ask the phone which storage it has.
    #[default]
    Autodetect,
    Emmc,
    Ufs,
    SpiNor,
    Nand,
    Nvme,
}

impl StorageType {
    /// The choices the storage dropdown offers, in the order it shows them.
    const ALL: [Self; 6] = [
        Self::Autodetect,
        Self::Emmc,
        Self::Ufs,
        Self::SpiNor,
        Self::Nand,
        Self::Nvme,
    ];

    /// The FTL id of its label.
    fn message_id(self) -> &'static str {
        match self {
            Self::Autodetect => "qc-storage-autodetect",
            Self::Emmc => "qc-storage-emmc",
            Self::Ufs => "qc-storage-ufs",
            Self::SpiNor => "qc-storage-spi-nor",
            Self::Nand => "qc-storage-nand",
            Self::Nvme => "qc-storage-nvme",
        }
    }
}

/// One device of a run, as the table shows it.
struct MultiFlashDevice {
    serial: String,
    /// XT model code (`sku`), when the bootloader reports one.
    xt_code: Option<String>,
    /// Carrier/region code (`cid`), normalized.
    cid: Option<String>,
    status: DeviceStatus,
    /// How far the preset is, 0 to 1, kept as its own field so a flash that
    /// fails leaves its bar where it stopped instead of jumping back.
    fraction: f32,
}

/// What a device of a run is doing.
enum DeviceStatus {
    /// A preset was chosen and its firmware is being read; the row is created
    /// once that is known, so this is the state of a single frame.
    Preparing,
    /// No preset accepts this device: it is left alone.
    NoPreset,
    /// A procedure is running, and the progress line says which one.
    Flashing {
        /// 0-based index of the procedure.
        index: usize,
        total: usize,
        label: String,
    },
    /// The firmware was written (and the phone rebooted, when the preset asks
    /// for that).
    Finished,
    /// The flash failed; the reason is the log file's, and the row shows it.
    Failed(String),
}

impl DeviceStatus {
    /// Whether nothing more will be reported about this device: it was left
    /// alone (no preset), or its flash ended one way or another.
    ///
    /// What a run that was stopped keeps on the table is exactly the devices
    /// that are *not* done — those are the ones still being written to, and the
    /// only thing the window is waiting for.
    fn settled(&self) -> bool {
        matches!(self, Self::NoPreset | Self::Finished | Self::Failed(_))
    }
}

/// The row of a device, for a report that carries its serial.
fn device_mut<'a>(
    multi: &'a mut MultiFlashState,
    serial: &str,
) -> Option<&'a mut MultiFlashDevice> {
    multi.devices.iter_mut().find(|device| device.serial == serial)
}

/// Where a device is in its preset, as the table's fraction: the procedures
/// that are done, plus how far the running one is (`inside`, 0 to 1).
fn device_fraction(index: usize, inside: f32, total: usize) -> f32 {
    if total == 0 {
        return 0.0;
    }

    ((index as f32 + inside) / total as f32).clamp(0.0, 1.0)
}

/// What the last save or load of the preset list did.
enum PresetStatus {
    /// The whole list was written to that file.
    Saved(std::path::PathBuf),
    /// A file was read into the list; `skipped` entries could not be loaded.
    Loaded {
        path: std::path::PathBuf,
        count: usize,
        skipped: Vec<std::path::PathBuf>,
    },
    /// The file was refused (not ours, too new, or unreadable).
    Failed(preset_file::FileError),
}

/// State of the "Smart Device Flashing" mode (Mode 2) feature tiles.
#[derive(Default)]
struct SmartphoneFlashState {
    bootloader: BootloaderState,
    tablet_unlock: TabletUnlockState,
    factory_reset: FactoryResetState,
    firmware: FirmwareFlashState,
    partition: PartitionFlashState,
    driver: DriverState,
    bloatware: BloatwareState,
    wireless: WirelessState,
    captive_portal: CaptivePortalState,
    multi_flash: MultiFlashState,
    qualcomm: QualcommState,
}

/// State of the "Unpack Image" feature (Mode 3, firmware-file tools).
#[derive(Default)]
struct UnpackState {
    /// Whether the modal is open.
    open: bool,
    /// The image file chosen by the user.
    image: Option<std::path::PathBuf>,
    /// Where the files are written.
    output: Option<std::path::PathBuf>,
    /// The files the selected image holds, once its header was read.
    entries: Option<Vec<image_unpack::Entry>>,
    /// `true` while the image header is being read.
    listing: bool,
    /// Why the image could not be read.
    image_error: Option<image_unpack::UnpackError>,
    /// Id of the running extraction, so events of an abandoned job are dropped.
    job: u64,
    /// `true` while the selected image is written out.
    working: bool,
    /// The file being written right now.
    current: Option<String>,
    /// Payload bytes written so far, and the payload size of the whole image.
    done_bytes: u64,
    total_bytes: u64,
    /// How the run ended.
    result: Option<Result<image_unpack::Summary, image_unpack::UnpackError>>,
}

impl UnpackState {
    /// Whether a job is running: the dialog must not be closed then.
    fn busy(&self) -> bool {
        self.listing || self.working
    }
}

/// State of the "Unisoc PAC" package tool (Mode 3, firmware-file tools).
#[derive(Default)]
struct PacState {
    /// Whether the modal is open.
    open: bool,
    /// The package file chosen by the user.
    file: Option<std::path::PathBuf>,
    /// Where the files are written.
    output: Option<std::path::PathBuf>,
    /// The package's header and file table, once they were read.
    package: Option<pac::Package>,
    /// Which entries are checked, parallel to `package.files`.
    selected: Vec<bool>,
    /// What the search box above the file list contains. It only filters what
    /// the list shows; `selected` keeps the indexes into `package.files`, so a
    /// selection survives changing the query.
    filter: String,
    /// `true` while the package's file table is being read.
    listing: bool,
    /// Why the package could not be read.
    file_error: Option<pac::PacError>,
    /// Id of the running extraction, so events of an abandoned job are dropped.
    job: u64,
    /// `true` while the checked files are written out.
    working: bool,
    /// The file being written right now.
    current: Option<String>,
    /// Payload bytes written so far, and the payload size of the selection.
    done_bytes: u64,
    total_bytes: u64,
    /// How the run ended.
    result: Option<Result<pac::Summary, pac::PacError>>,
    /// Whether the run also writes the `spd_dump` scripts next to the files.
    spd_script: bool,
    /// The scripts the last run wrote, or why they could not be written.
    /// `None` while nothing ran, and empty when none were asked for.
    scripts: Option<Result<Vec<String>, String>>,
}

/// Progress of a package extraction.
///
/// The worker reports its outcome itself, in one place, so the dialog can never
/// wait for an event that does not come.
#[derive(Debug, Clone)]
enum PacJobEvent {
    /// A file is about to be written.
    File(String),
    /// `bytes` payload bytes have been written so far (across all files).
    Progress { bytes: u64, total: u64 },
    /// The run is over: what it wrote, and the scripts written next to the
    /// files — empty when the user asked for none.
    Finished {
        result: Result<pac::Summary, pac::PacError>,
        scripts: Result<Vec<String>, String>,
    },
}

impl PacState {
    /// Whether a job is running: the dialog must not be closed then.
    fn busy(&self) -> bool {
        self.listing || self.working
    }

    /// How many entries are checked.
    fn chosen(&self) -> usize {
        self.selected.iter().filter(|chosen| **chosen).count()
    }

    /// Indexes of the checked entries, the way [`pac::unpack`] takes them.
    fn chosen_indexes(&self) -> Vec<usize> {
        self.selected
            .iter()
            .enumerate()
            .filter(|(_, chosen)| **chosen)
            .map(|(index, _)| index)
            .collect()
    }
}

/// State of the "Repack Package" dialog (Mode 3, firmware-file tools).
///
/// Unisoc leaves a scattered firmware behind as its files, a scatter XML and a
/// `flash.cfg`; either of the two is read here and turned back into a package.
/// A `flash.cfg` names every file itself, a scatter XML only for the names its
/// product uses — what cannot be matched is pointed at by hand, or left out of
/// the package.
#[derive(Default)]
struct PacRepackState {
    /// Whether the modal is open.
    open: bool,
    /// The `flash.cfg` or scatter XML the package is described by.
    source: Option<std::path::PathBuf>,
    /// The records to write, once the source was read.
    plan: Option<pac_build::Plan>,
    /// What the search box above the record list contains. It only filters what
    /// the list shows.
    filter: String,
    /// How far the record table is scrolled sideways, so its header can be
    /// drawn above the columns the rows are scrolled to.
    offset: f32,
    /// `true` while the source is being read.
    reading: bool,
    /// Why the source could not be read.
    source_error: Option<pac_build::BuildError>,
    /// The file the package is written to, once the user named it.
    output: Option<std::path::PathBuf>,
    /// Id of the running build, so events of an abandoned job are dropped.
    job: u64,
    /// `true` while the package is written.
    working: bool,
    /// The file being written right now.
    current: Option<String>,
    /// Payload bytes written so far, and the payload size of the run.
    done_bytes: u64,
    total_bytes: u64,
    /// How the run ended.
    result: Option<Result<pac::Summary, pac_build::BuildError>>,
}

impl PacRepackState {
    /// Whether a job is running: the dialog must not be closed then.
    fn busy(&self) -> bool {
        self.reading || self.working
    }
}

/// What a running repack reports.
///
/// The package writer reports its own progress, but it does not know why a plan
/// was refused, so the outcome of the whole build is reported here instead.
#[derive(Debug, Clone)]
enum PacRepackEvent {
    /// The file being written right now.
    File(String),
    /// Payload bytes written so far, out of the total.
    Progress { bytes: u64, total: u64 },
    /// How the build ended.
    Finished(Result<pac::Summary, pac_build::BuildError>),
}

/// Which "Remove System Bloatware" dialog is currently on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum BloatwareDialog {
    #[default]
    Closed,
    /// The whole flow (device selection, app checklist, removal, result) is
    /// presented by this one modal.
    Open,
}

/// State of the "Remove System Bloatware" feature (Mode 2).
#[derive(Default)]
struct BloatwareState {
    dialog: BloatwareDialog,
    /// `true` while the platform-tools are prepared and `adb devices` runs.
    preparing: bool,
    /// Failure of that step (its download, or `adb` itself).
    prepare_error: Option<String>,
    /// Devices `adb devices` reported.
    devices: Vec<debloat::AdbDevice>,
    /// Serial of the device the dialog works on.
    selected: Option<String>,
    /// `true` while `pm list packages` runs on the selected device.
    checking: bool,
    /// Failure of that read (the phone may have been unplugged).
    check_error: Option<String>,
    /// The known bloatware found on the device; `None` until it was read.
    found: Option<Vec<debloat::Bloatware>>,
    /// One flag per entry of `found` (parallel): the apps to remove.
    checked: Vec<bool>,
    /// What the search box above the checklist contains; only the rows whose
    /// name or package contains it are shown.
    filter: String,
    /// `true` while the selected apps are removed.
    removing: bool,
    /// What the removal did, per app, once it finished.
    results: Option<Vec<debloat::RemovalOutcome>>,
}

impl BloatwareState {
    /// Whether a job is running: the dialog must not be closed then.
    fn busy(&self) -> bool {
        self.preparing || self.checking || self.removing
    }

    /// Label of the selected device (serial and model), so the dialog can
    /// show which phone it is working on.
    fn selected_label(&self) -> Option<String> {
        let serial = self.selected.as_ref()?;

        Some(
            self.devices
                .iter()
                .find(|device| &device.serial == serial)
                .map_or_else(|| serial.clone(), debloat::AdbDevice::label),
        )
    }

    /// The bloatware the user left checked, in the checklist's order.
    fn chosen_apps(&self) -> Vec<debloat::Bloatware> {
        let Some(found) = &self.found else {
            return Vec::new();
        };

        found
            .iter()
            .enumerate()
            .filter(|(index, _)| self.checked.get(*index).copied().unwrap_or(false))
            .map(|(_, app)| *app)
            .collect()
    }

    /// Whether at least one app is still checked (the view asks this on every
    /// frame, so it does not build the list above).
    fn has_chosen_apps(&self) -> bool {
        match &self.found {
            Some(found) => found
                .iter()
                .enumerate()
                .any(|(index, _)| self.checked.get(index).copied().unwrap_or(false)),
            None => false,
        }
    }
}

/// How the "Connect ADB Wireless Debugging" dialog pairs with the phone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum WirelessMethod {
    /// Show a QR code for the phone's "Pair device with QR code" screen.
    #[default]
    Qr,
    /// Take the six-digit code the phone's "Pair device with pairing code"
    /// screen shows.
    Code,
}

/// State of the "Connect ADB Wireless Debugging" feature (Mode 2).
///
/// The flow is the same for both methods: a pairing code reaches the phone (it
/// scans the QR code, or the user copies the six digits), the phone announces
/// its mDNS pairing service, and `adb pair` finishes the job. The two ways
/// differ only in where the code comes from.
#[derive(Default)]
struct WirelessState {
    /// Whether the modal is open.
    open: bool,
    /// Which pairing method the dialog shows.
    method: WirelessMethod,
    /// The code the QR code carries. A fresh one is made for every attempt:
    /// the phone adopts the service name and password from what it scans.
    pairing_code: Option<wireless::PairingCode>,
    /// The QR code of `pairing_code`, encoded once instead of per frame.
    qr: Option<wireless::QrImage>,
    /// The running network discovery, kept so it can be stopped when the
    /// dialog closes.
    discovery: Option<wireless::Discovery>,
    /// Id of the running discovery, so answers of an abandoned browse are
    /// dropped.
    job: u64,
    /// The pairing services the phones on the network announce.
    pairing_services: Vec<wireless::AdbService>,
    /// The connect services the phones on the network announce.
    connect_services: Vec<wireless::AdbService>,
    /// The wireless devices `adb` already has connected (`192.168.1.5:5555`),
    /// so "Disconnect" knows about a phone that was connected before this
    /// dialog was opened.
    devices: Vec<String>,
    /// The pairing service the manual flow works with (its mDNS full name).
    chosen: Option<String>,
    /// What the user typed into the pairing-code field.
    code_input: String,
    /// `true` while `adb pair` and the connect that follows it run.
    working: bool,
    /// `true` while `adb disconnect` runs.
    disconnecting: bool,
    /// How the last pairing attempt ended.
    result: Option<Result<wireless::PairOutcome, String>>,
    /// How the last disconnect ended.
    disconnect_result: Option<DisconnectOutcome>,
    /// Why the network discovery could not be started.
    discover_error: Option<String>,
}

impl WirelessState {
    /// Whether a job is running: the dialog must not be closed then.
    fn busy(&self) -> bool {
        self.working || self.disconnecting
    }

    /// The pairing service a phone announced after scanning the QR code that
    /// is on screen, if any.
    fn scanned(&self) -> Option<&wireless::AdbService> {
        let code = self.pairing_code.as_ref()?;

        self.pairing_services
            .iter()
            .find(|service| wireless::matches(code, &service.fullname))
    }

    /// The pairing service the manual flow will use.
    fn manual_service(&self) -> Option<&wireless::AdbService> {
        let chosen = self.chosen.as_ref()?;

        self.pairing_services
            .iter()
            .find(|service| &service.fullname == chosen)
    }

    /// Whether the pairing-code flow can start: a phone to talk to, and the
    /// code that proves the user can see its screen.
    fn ready_to_pair(&self) -> bool {
        !self.busy()
            && self.manual_service().is_some()
            && !self.code_input.trim().is_empty()
    }

    /// The first connect service: a phone with wireless debugging on announces
    /// one, so it can be connected to without pairing again.
    fn connect_service(&self) -> Option<&wireless::AdbService> {
        self.connect_services.first()
    }

    /// What the "Disconnect" menu offers: every wireless device `adb` lists,
    /// and last the entry that drops all of them.
    fn disconnect_options(&self) -> Vec<wireless::DisconnectTarget> {
        self.devices
            .iter()
            .cloned()
            .map(wireless::DisconnectTarget::Device)
            .chain(std::iter::once(wireless::DisconnectTarget::Everything))
            .collect()
    }
}

/// How the wireless dialog's last disconnect ended.
#[derive(Debug, Clone)]
enum DisconnectOutcome {
    /// The target was disconnected.
    Done(wireless::DisconnectTarget),
    /// It could not be, and this is what `adb` said.
    Failed(String),
}

/// State of the "Fix Captive Portal Server" feature (Mode 2).
///
/// The fix points the device's captive-portal check and NTP server at servers
/// that answer (see [`captive_portal`]); the steps themselves run detached on
/// the device, because the last of them cycles the network connection the
/// command came in on.
#[derive(Default)]
struct CaptivePortalState {
    /// Whether the modal is open.
    open: bool,
    /// `true` while the platform-tools are prepared and `adb devices` runs.
    listing: bool,
    /// Failure of that step (its download, or `adb` itself).
    list_error: Option<String>,
    /// Devices `adb devices` reported.
    devices: Vec<debloat::AdbDevice>,
    /// Serial of the device the fix runs on.
    selected: Option<String>,
    /// `true` while the fix is applied.
    fixing: bool,
    /// How the last fix ended.
    result: Option<Result<captive_portal::FixReport, captive_portal::FixError>>,
}

impl CaptivePortalState {
    /// Whether a job is running: the dialog must not be closed then.
    fn busy(&self) -> bool {
        self.listing || self.fixing
    }

    /// Whether the fix can be started: a device was chosen, nothing runs, and
    /// the last attempt did not already succeed (a failure can be retried).
    fn ready_to_fix(&self) -> bool {
        self.selected.is_some()
            && !self.busy()
            && !matches!(self.result, Some(Ok(_)))
    }

    /// Label of the selected device (serial and model), so the dialog can
    /// show which phone it is working on.
    fn selected_label(&self) -> Option<String> {
        let serial = self.selected.as_ref()?;

        Some(
            self.devices
                .iter()
                .find(|device| &device.serial == serial)
                .map_or_else(|| serial.clone(), debloat::AdbDevice::label),
        )
    }
}
/// Which "Install Driver" dialog is currently on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum DriverDialog {
    #[default]
    Closed,
    Open,
}

/// Step of a running driver installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum DriverStage {
    /// The Windows installer is being downloaded.
    #[default]
    Downloading,
    /// The installer (Windows) or the udev rules (Linux) are running.
    Installing,
    /// The installation is over.
    Done,
}

/// State of the "Install Driver" feature (Mode 2).
///
/// Opening the dialog only presents what is about to happen; the job itself is
/// started by the dialog's Install button, which is also where Linux is asked
/// for the password `sudo` needs.
#[derive(Default)]
struct DriverState {
    dialog: DriverDialog,
    /// The password handed to `sudo` (Linux only; never shown, never stored).
    password: String,
    /// Id of the running installation, so events of an abandoned job are
    /// dropped instead of touching a dialog that moved on.
    job: u64,
    busy: bool,
    stage: DriverStage,
    downloaded: u64,
    total: u64,
    error: Option<driver_install::DriverError>,
}

/// Which Bootloader Unlock dialog is currently on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum BootloaderDialog {
    #[default]
    Closed,
    /// Guided-vs-Manual chooser shown after pressing the tile's button.
    Choosing,
    /// The manual unlock flow.
    Manual,
    /// The guided (webview wizard) unlock flow.
    Guided,
}

/// Step shown by the guided (wizard) flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum GuidedStep {
    #[default]
    Login,
    Read,
    Request,
    Key,
}

/// State of the guided (webview-driven) unlock flow.
#[derive(Default)]
struct GuidedState {
    step: GuidedStep,
    /// Set once the Motorola portal login has completed.
    logged_in: bool,
    /// Logged-in account display name (from the portal profile page).
    account_name: Option<String>,
    /// Logged-in account e-mail (from the portal profile page).
    account_email: Option<String>,
    /// `true` while the logged-in account profile (name/e-mail) is still being
    /// fetched after a successful portal login.
    fetching_account: bool,
    /// Portal session cookie captured by the webview login.
    cookie_header: String,
    logging_in: bool,
    /// `true` while the previous session is being signed out ("Change
    /// Account"): the webview is reopened with a fresh login afterwards.
    logging_out: bool,
    login_error: Option<String>,
    /// `true` while the "request unlock key?" confirm prompt is shown.
    confirming_request: bool,
    requesting: bool,
    request_error: Option<String>,
}

/// The device operation pending a device choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BootloaderDeviceAction {
    ReadDeviceId,
    Unlock,
}

/// Device picker shown when several fastboot devices are connected.
#[derive(Debug, Clone)]
struct BootloaderPicker {
    devices: Vec<fastboot_info::DeviceEntry>,
    action: BootloaderDeviceAction,
}

/// State of the "Bootloader Unlock" feature (Mode 2).
#[derive(Default)]
struct BootloaderState {
    dialog: BootloaderDialog,
    picker: Option<BootloaderPicker>,
    /// Device ID obtained with `fastboot oem get_unlock_data`.
    device_id: String,
    /// Unlock key typed into the manual flow.
    key_input: String,
    /// Snapshot of the key used by the pending unlock command.
    pending_key: Option<String>,
    reading: bool,
    unlocking: bool,
    /// Motorola unlock-eligibility check, run automatically after the Device
    /// ID is read. Failures (e.g. no Internet connection) are kept silent.
    checking: bool,
    /// `true` = phone qualifies, `false` = not qualified (once checked).
    eligible: Option<bool>,
    /// Status line for the Device ID area (read / copy).
    read_error: Option<String>,
    read_info: Option<String>,
    /// Status line for the Unlock area.
    unlock_error: Option<String>,
    /// Neutral notice in the Unlock area (e.g. the request was cancelled).
    unlock_notice: Option<String>,
    unlock_info: Option<String>,
    /// Guided (wizard) flow state.
    guided: GuidedState,
}

/// Which tablet Bootloader Unlock dialog is currently on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum TabletUnlockDialog {
    #[default]
    Closed,
    /// Guided-vs-Manual chooser.
    Choosing,
    /// The manual (ZUI) unlock flow.
    Manual,
    /// The guided (automatic) unlock flow.
    Guided,
}

/// Fastboot devices offered by the tablet flow when several are connected.
#[derive(Debug, Clone)]
struct TabletPicker {
    /// USB serials, in discovery order.
    devices: Vec<String>,
}

/// The card the guided tablet unlock shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum TabletGuidedStep {
    /// Pick the connected tablet.
    #[default]
    Devices,
    /// The unlock attempts are running (and then their result).
    Working,
}

/// State of the guided (automatic) tablet unlock.
#[derive(Default)]
struct TabletGuidedState {
    /// Monotonic run counter, so a stopped job's events and report are ignored.
    job: u64,
    step: TabletGuidedStep,
    /// `true` while the connected devices are listed.
    listing: bool,
    devices: Vec<tablet_unlock::TabletDevice>,
    list_error: Option<String>,
    /// `true` while the unlock attempts run.
    unlocking: bool,
    /// The chipset the running job reported, so the card can show how to accept
    /// the tablet's on-device confirmation.
    platform: Option<crate::firmware::Platform>,
    /// `true` once an unlock command is waiting for the user to accept the
    /// confirmation the tablet shows.
    confirming: bool,
    /// `true` while "Cancel & Restart" was pressed and the job is stopping.
    cancelling: bool,
    /// Flags the running job to stop at its next checkpoint.
    cancel: Option<Arc<AtomicBool>>,
    /// The outcome, once the attempts are over.
    result: Option<tablet_unlock::GuidedOutcome>,
}

/// State of the tablet "Bootloader Unlock" feature (Mode 2).
#[derive(Default)]
struct TabletUnlockState {
    dialog: TabletUnlockDialog,
    picker: Option<TabletPicker>,
    /// USB serial of the device the identity was read from.
    device: Option<String>,
    /// Serial number, left-padded to eight characters.
    serial: String,
    /// `Bootloader_SN_Part1` / `_Part2`, as read from the bootloader.
    bootloader_sn_part1: String,
    bootloader_sn_part2: String,
    /// The chipset the read reported, which decides how the tablet's on-device
    /// unlock confirmation has to be accepted.
    platform: Option<crate::firmware::Platform>,
    /// The two parts concatenated, i.e. what the UI shows; empty when the
    /// bootloader does not report it.
    bootloader_sn: String,
    /// `true` once a read attempt finished (so the "Not applicable" watermark
    /// only appears after the device was actually asked).
    read_done: bool,
    reading: bool,
    read_error: Option<String>,
    /// Orange notice shown after a read (e.g. which ZUI section to use).
    read_notice: Option<String>,
    unlocking: bool,
    /// `true` while "Cancel & Restart" was pressed and the unlock is stopping.
    cancelling: bool,
    /// Flags the running unlock to stop at its next checkpoint.
    cancel: Option<Arc<AtomicBool>>,
    unlock_error: Option<String>,
    unlock_info: Option<String>,
    /// Guided (automatic) flow state.
    guided: TabletGuidedState,
}

impl TabletUnlockState {
    /// The identity the token URL is built from.
    fn info(&self) -> tablet_unlock::TabletUnlockInfo {
        tablet_unlock::TabletUnlockInfo {
            serial: self.serial.clone(),
            bootloader_sn_part1: self.bootloader_sn_part1.clone(),
            bootloader_sn_part2: self.bootloader_sn_part2.clone(),
            platform: self.platform,
        }
    }
}

/// Which Factory Reset dialog is currently on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum FactoryResetDialog {
    #[default]
    Closed,
    /// The whole factory-reset flow (device selection, requirement checks,
    /// confirmation, erase) is presented by this one modal.
    Open,
}

/// Outcome of the factory-reset requirement checks
/// (`getvar securestate`, `fdr-allowed`).
#[derive(Debug, Clone)]
enum ResetCheckOutcome {
    /// All requirements are met; the reset may be confirmed. `frp_protected`
    /// warns that the bootloader will not clear Google's Factory Reset
    /// Protection.
    Allowed { frp_protected: bool },
    /// A requirement is not met, so the device must not be reset.
    Blocked(fastboot_info::FactoryResetBlocker),
    /// The device could not be queried (USB / command failure).
    Failed(String),
}

/// State of the "Factory Reset" feature (Mode 2).
#[derive(Default)]
struct FactoryResetState {
    dialog: FactoryResetDialog,
    /// `true` while the connected devices are being listed.
    listing: bool,
    /// Supported (Motorola) devices found, offered for selection.
    devices: Vec<fastboot_info::DeviceEntry>,
    /// Total number of fastboot devices found, including unsupported ones
    /// (so the UI can tell "no device" from "unsupported device").
    total: usize,
    /// Failure of the device listing itself (e.g. a USB error).
    list_error: Option<String>,
    /// Serial of the device the user selected.
    selected: Option<String>,
    /// `true` while the requirement checks run on the selected device.
    checking: bool,
    /// Requirement-check result, once it finished.
    check: Option<ResetCheckOutcome>,
    /// `true` while `erase userdata` / `erase metadata` run.
    resetting: bool,
    /// Result of the erase, once it finished.
    result: Option<Result<(), String>>,
}

impl FactoryResetState {
    /// Label of the selected device (serial, XT code and codename), so the
    /// dialog can show which phone it is working on.
    fn selected_label(&self) -> Option<String> {
        let serial = self.selected.as_ref()?;

        Some(
            match self.devices.iter().find(|device| &device.serial == serial) {
                Some(device) => device.label(),
                None => serial.clone(),
            },
        )
    }
}

/// Which Firmware Flash dialog is currently on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum FirmwareFlashDialog {
    #[default]
    Closed,
    /// The whole flow (package selection, device selection, confirmation,
    /// flashing with progress and log) is presented by this one modal.
    Open,
}

/// State of the "Firmware Flash" feature (Mode 2).
struct FirmwareFlashState {
    dialog: FirmwareFlashDialog,
    /// Incremented for every job started by the dialog; events that carry an
    /// older id belong to an abandoned job and are ignored.
    job: u64,
    /// Selected firmware package (a ZIP archive or a `flashfile.xml`).
    package: Option<std::path::PathBuf>,
    /// `true` while the package is unpacked / parsed.
    loading: bool,
    /// Unpacking progress of the selected ZIP, in bytes.
    extracted: Option<(u64, u64)>,
    /// The parsed package, ready to flash.
    plan: Option<flashfile::FlashPackage>,
    /// One flag per procedure of `plan` (parallel to `FlashPackage::steps`):
    /// only the checked ones are executed.
    enabled: Vec<bool>,
    /// Erase groups added on top of the package ("Erase Userdata" / "Erase
    /// NV cache"); they are independent of the package's own procedures.
    erase_groups: Vec<flashfile::EraseGroup>,
    /// `true` while the procedure checklist is shown.
    editing: bool,
    /// Package-loading failure.
    error: Option<flashfile::LoadError>,
    /// `mfastboot` builds found next to the application (newest first).
    mfastboot: Vec<flash_engine::Mfastboot>,
    /// Google's `fastboot`, when this system has a download for it. Built when
    /// the dialog opens, so the picker does not ask the system (Rosetta 2,
    /// box64) on every frame.
    platform_tools: Option<flash_engine::Mfastboot>,
    /// `true` while that download runs.
    installing_tools: bool,
    /// Where the last download failed.
    tools_error: Option<String>,
    /// `true` once a download finished, so the dialog can say so.
    tools_ready: bool,
    /// `true` while the "Open Terminal" flow runs on its own (the minimal-ADB
    /// check or the launch itself).
    terminal_busy: bool,
    /// `true` while that flow has a platform-tools download running, so its
    /// result opens the terminal instead of only refreshing the dialog.
    terminal_after_install: bool,
    /// Why the terminal could not be opened.
    terminal_error: Option<TerminalError>,
    /// The directory the terminal was opened in, once it was.
    terminal_path: Option<std::path::PathBuf>,
    /// `true` while the selected tool is asked for its version
    /// (`fastboot --version`).
    reading_version: bool,
    /// What it reported, or why it could not be asked.
    tool_version: Option<Result<String, String>>,
    /// Backend that runs the package's steps.
    engine: flash_engine::Engine,
    /// Verify the `MD5` recorded for every `flash` step.
    verify_checksums: bool,
    /// `true` while the connected devices are listed.
    listing: bool,
    /// Supported (Motorola) devices found, offered for selection.
    devices: Vec<fastboot_info::DeviceEntry>,
    /// Total number of fastboot devices found, including unsupported ones.
    total: usize,
    /// Failure of the device listing itself (e.g. a USB error).
    list_error: Option<String>,
    /// Serial of the device the package is flashed to.
    selected: Option<String>,
    /// `true` while the selected device's variables are read.
    reading_device: bool,
    /// `true` while the "Read Info" commands run on the selected phone.
    reading_info: bool,
    /// `true` while what they reported is on screen.
    info_view: bool,
    /// Their output, each command line followed by what it answered.
    info_lines: Vec<String>,
    /// Failure of that read (the phone may have been unplugged).
    info_error: Option<String>,
    /// `true` while the IMEIs in that output are masked.
    hide_sensitive: bool,
    /// `securestate`, `cid` and `product` of the selected device.
    device_vars: Option<fastboot_info::DeviceVars>,
    /// Failure of that read (the phone may have been unplugged).
    device_vars_error: Option<String>,
    /// `true` while the warning before flashing is shown.
    confirming: bool,
    /// `true` when the package's project code differs from the selected
    /// phone's, i.e. when flashing it is expected to brick the phone. The
    /// warning then has to be read before it can be confirmed.
    project_mismatch: bool,
    /// Seconds left before "Yes" becomes clickable after that warning; `0`
    /// whenever there is nothing to wait for.
    confirm_countdown: u8,
    /// `true` while the steps run.
    running: bool,
    step_index: usize,
    step_total: usize,
    step_label: String,
    /// Byte progress inside the current step.
    progress: Option<(u64, u64)>,
    /// Output of the flashing tool (and the engine's own notes).
    log: Vec<String>,
    /// The file this run's log is written to as well, once it started:
    /// `Documents\lmnflash\logs\single_flash_<serial>_<moment>.log`. The
    /// dialog shows the output, the file is what the user can keep or send on.
    log_file: Option<flash_log::FlashLog>,
    /// Result of the whole package, once it finished.
    result: Option<Result<(), String>>,
    /// `true` while `oem fb_mode_clear` + `reboot` run on the phone.
    rebooting: bool,
    /// Outcome of that reboot, once it ran.
    reboot_result: Option<Result<(), String>>,
    /// `true` while the question is on screen whether the tool the script
    /// starts should be copied next to it.
    export_confirming: bool,
    /// Where the last "Export to script" wrote the plan to, or why it could
    /// not be written.
    export_result: Option<Result<std::path::PathBuf, String>>,
}

impl Default for FirmwareFlashState {
    fn default() -> Self {
        Self {
            dialog: FirmwareFlashDialog::default(),
            job: 0,
            package: None,
            loading: false,
            extracted: None,
            plan: None,
            enabled: Vec::new(),
            erase_groups: Vec::new(),
            editing: false,
            error: None,
            mfastboot: Vec::new(),
            platform_tools: None,
            installing_tools: false,
            tools_error: None,
            tools_ready: false,
            terminal_busy: false,
            terminal_after_install: false,
            terminal_error: None,
            terminal_path: None,
            reading_version: false,
            tool_version: None,
            engine: flash_engine::Engine::Builtin,
            // Checksum verification is on unless it is turned off explicitly.
            verify_checksums: true,
            listing: false,
            devices: Vec::new(),
            total: 0,
            list_error: None,
            selected: None,
            reading_device: false,
            reading_info: false,
            info_view: false,
            info_lines: Vec::new(),
            info_error: None,
            hide_sensitive: false,
            device_vars: None,
            device_vars_error: None,
            confirming: false,
            project_mismatch: false,
            confirm_countdown: 0,
            running: false,
            step_index: 0,
            step_total: 0,
            step_label: String::new(),
            progress: None,
            log: Vec::new(),
            log_file: None,
            result: None,
            rebooting: false,
            reboot_result: None,
            export_confirming: false,
            export_result: None,
        }
    }
}

/// Why the "Open Terminal" button could not open a terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TerminalError {
    /// "Minimal ADB and Fastboot" is installed: its own `adb` and `fastboot`
    /// would be used instead of the ones the dialog unpacks.
    MinimalAdb,
    /// Anything else, with the message the platform reported.
    Other(String),
}

/// A job the Single Partition Flash dialog runs.
///
/// One is in flight at a time, and the card shows a spinner for it: the flash
/// itself, the reboot that leaves fastboot for Android, or the reboot that
/// switches the phone to userspace fastboot (`fastbootd`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PartitionFlashTask {
    Flash,
    Reboot,
    SwitchToFastbootd,
}

/// State of the "Single Partition Flash" feature (Mode 2).
struct PartitionFlashState {
    /// Whether the dialog is on screen.
    open: bool,
    /// The job in flight, if any.
    task: Option<PartitionFlashTask>,
    /// Incremented for every job the dialog starts; events that carry an older
    /// id belong to an abandoned job and are ignored.
    job: u64,
    /// `mfastboot` builds found next to the application (newest first).
    mfastboot: Vec<flash_engine::Mfastboot>,
    /// Google's `fastboot`, when it was downloaded already. This dialog does
    /// not download it (the Firmware Flash dialog does, and both use the same
    /// file), so it is only offered once it is there.
    platform_tools: Option<flash_engine::Mfastboot>,
    /// The backend that writes the image.
    engine: flash_engine::Engine,
    /// `true` while the selected tool is asked for its version.
    reading_version: bool,
    /// What it reported, or why it could not be asked.
    tool_version: Option<Result<String, String>>,
    /// `true` while the connected phones are listed.
    listing: bool,
    /// The phones found, whatever vendor they are: the dialog also works on a
    /// phone it switched to `fastbootd`, which no longer reports the `cid` the
    /// Motorola-only listing filters on.
    devices: Vec<fastboot_info::DeviceSummary>,
    /// Failure of the listing itself (e.g. a USB error).
    list_error: Option<String>,
    /// Serial of the phone the image is written to.
    selected: Option<String>,
    /// `true` while that phone's variables are read.
    reading_device: bool,
    /// `is-userspace` and `current-slot` of the selected phone.
    vars: Option<fastboot_info::DeviceVars>,
    /// Failure of that read (the phone may have been unplugged).
    vars_error: Option<String>,
    /// The partition the image is written to, as typed.
    partition: String,
    /// The image file to write.
    image: Option<std::path::PathBuf>,
    /// Whether that image carries a vbmeta struct, i.e. whether the two AVB
    /// flags can be written into it.
    image_avb: bool,
    /// Write `AVB_VBMETA_IMAGE_FLAGS_HASHTREE_DISABLED` into the image.
    disable_verity: bool,
    /// Write `AVB_VBMETA_IMAGE_FLAGS_VERIFICATION_DISABLED` into the image.
    disable_verification: bool,
    /// Byte progress inside the flash that is running.
    progress: Option<(u64, u64)>,
    /// Output of the flashing tool (and the engine's own notes).
    log: Vec<String>,
    /// The file this job's log is written to as well, once it started:
    /// `Documents\lmnflash\logs\partition_flash_<serial>_<moment>.log`.
    log_file: Option<flash_log::FlashLog>,
    /// Result of the last flash.
    result: Option<Result<(), String>>,
    /// Outcome of the last reboot ([`PartitionFlashTask::Reboot`]).
    reboot_result: Option<Result<(), String>>,
    /// Outcome of the last switch to fastbootd
    /// ([`PartitionFlashTask::SwitchToFastbootd`]).
    switch_result: Option<Result<(), String>>,
}

impl Default for PartitionFlashState {
    fn default() -> Self {
        Self {
            open: false,
            task: None,
            job: 0,
            mfastboot: Vec::new(),
            platform_tools: None,
            engine: flash_engine::Engine::Builtin,
            reading_version: false,
            tool_version: None,
            listing: false,
            devices: Vec::new(),
            list_error: None,
            selected: None,
            reading_device: false,
            vars: None,
            vars_error: None,
            partition: String::new(),
            image: None,
            image_avb: false,
            disable_verity: false,
            disable_verification: false,
            progress: None,
            log: Vec::new(),
            log_file: None,
            result: None,
            reboot_result: None,
            switch_result: None,
        }
    }
}

/// Inputs captured when a lookup starts, kept until it finishes so the
/// best-effort telemetry event reflects the original request even if the user
/// edits the form while the lookup runs (see `mod telemetry`).
#[derive(Debug, Clone)]
enum TelemetryContext {
    RowSmartphone { imei: String },
    RetcnSmartphone(firmware::RetcnRequest),
    Tablet { serial_number: String },
}

#[derive(Default)]
struct LookupState {
    mode: LookupMode,
    imei_input: String,
    status: LookupStatus,
    /// Set when the token expires; cleared after the next successful login
    /// (which then re-runs the lookup automatically).
    retry_after_login: bool,
    retcn: RetcnInput,
    tablet: TabletInput,
    by_model: ByModelInput,
    bulk: BulkInput,
    /// The in-flight lookup's inputs, used for the best-effort telemetry POST
    /// once a firmware image is found; cleared when no lookup is pending.
    pending_telemetry: Option<TelemetryContext>,
}

#[derive(Debug, Clone, Default)]
enum LoginStatus {
    #[default]
    LoggedOut,
    Fetching {
        click: Click,
    },
    WebViewOpen {
        url: String,
    },
    /// Waiting for the system browser to hand the `softwarefix://` callback
    /// back through [`instance`] (LMN Flash owns the scheme).
    WaitingForBrowser {
        url: String,
    },
    Manual {
        url: String,
        input: String,
        notice: Option<String>,
    },
    Error(String),
    LoggedIn {
        token: String,
        full_name: Option<String>,
    },
}

struct State {
    mode: Mode,
    lang: l10n::Language,
    l10n: l10n::Bundle,
    client_uuid: String,
    login: LoginStatus,
    /// How `softwarefix://` links are handled on this machine
    /// (`Handler::Unsupported` where the platform has no such handling).
    protocol: protocol::Handler,
    /// The last `softwarefix://` registration failure, shown on the Manual
    /// Login page.
    protocol_error: Option<String>,
    /// Whether this instance owns the single-instance endpoint and therefore
    /// receives `softwarefix://` callbacks (see [`instance`]).
    receives_callbacks: bool,
    lookup: LookupState,
    decrypt: DecryptState,
    /// The "Unpack Image" feature of the firmware-decrypt mode (Mode 3).
    unpack: UnpackState,
    /// The "Unisoc PAC" feature of the firmware-decrypt mode (Mode 3).
    pac: PacState,
    /// The "Repack Package" feature of the firmware-decrypt mode (Mode 3).
    pac_repack: PacRepackState,
    flash: SmartphoneFlashState,
    /// Whether the About dialog (the round "i" button next to the language
    /// selector on the Firmware Lookup page) is open.
    about_open: bool,
    /// Monotonic UI animation tick, advanced by a time subscription while a
    /// busy indicator (spinner) is visible.
    anim_tick: u64,
    /// Whether the OS colour scheme is dark, which selects the Fluent theme
    /// variant. Reported at startup and on every system theme change.
    dark: bool,
    /// In-flight fade + slide of the tab content, started when the user
    /// switches to another feature (Mode 1/2/3).
    content_transition: Option<anim::Transition>,
    /// Whether that transition animates the whole window — the language
    /// selector and the mode tabs included — instead of only the content.
    /// Entering and leaving the Multi Flashing page is a full-screen change,
    /// so its chrome travels with it rather than popping.
    content_transition_covers_chrome: bool,
    /// In-flight fade + slide of the modal dialog (see `overlay_key`).
    overlay_transition: Option<anim::Transition>,
    /// In-flight fade + slide of the login page, started when the login flow
    /// moves to another step (see [`login_step`]).
    login_transition: Option<anim::Transition>,
    /// In-flight fade + slide of the lookup form, started when the lookup mode
    /// changes (see [`lookup_step`]). The selector above the form is left out:
    /// the user just picked from it.
    lookup_transition: Option<anim::Transition>,
    /// The close of a dialog, held back until it has finished animating out;
    /// `Some` while it is leaving.
    pending_close: Option<Message>,
}

impl Default for State {
    fn default() -> Self {
        let lang = default_language();

        Self {
            mode: Mode::default(),
            l10n: l10n::bundle_for(lang),
            lang,
            client_uuid: uuid::Uuid::new_v4().to_string(),
            login: LoginStatus::default(),
            protocol: protocol::Handler::Unsupported,
            protocol_error: None,
            receives_callbacks: false,
            lookup: LookupState::default(),
            decrypt: DecryptState::default(),
            unpack: UnpackState::default(),
            pac: PacState::default(),
            pac_repack: PacRepackState::default(),
            flash: SmartphoneFlashState::default(),
            about_open: false,
            anim_tick: 0,
            dark: false,
            content_transition: None,
            content_transition_covers_chrome: false,
            overlay_transition: None,
            login_transition: None,
            lookup_transition: None,
            pending_close: None,
        }
    }
}

impl State {
    /// Creates the initial state, reusing the previously selected language
    /// and cached credentials if they are fresh.
    fn initial() -> Self {
        let mut state = Self::default();

        if let Some(language) = config::load_language() {
            state.lang = language;
            state.l10n = l10n::bundle_for(language);
        }

        if let Some((token, client_uuid)) = config::load_credentials() {
            state.client_uuid = client_uuid;
            state.login = LoginStatus::LoggedIn {
                token,
                full_name: None,
            };
        }

        // Register the `softwarefix://` scheme when nothing else handles it,
        // so the browser can hand the login callback back to us.
        let (handler, error) = protocol::ensure_registered();
        state.protocol = handler;
        state.protocol_error = error;

        // Started by the browser through the `softwarefix://` callback: use the
        // URL right away instead of asking the user to paste it.
        if let Some(url) = protocol::callback_arg() {
            // No lookup can be pending before the first frame, so the task the
            // helper returns is always empty here.
            let _ = apply_login_callback(&mut state, &url);
        }

        state
    }
}

/// Picks the UI language from the OS locale (defaults to English).
fn default_language() -> l10n::Language {
    let locale = sys_locale::get_locale().unwrap_or_default();
    let language = locale.split(['-', '_']).next().unwrap_or_default();

    if language.eq_ignore_ascii_case("zh") {
        if is_traditional_chinese(&locale) {
            return l10n::Language::ZhHant;
        }

        return l10n::Language::ZhHans;
    }

    match language.to_ascii_lowercase().as_str() {
        "ja" => l10n::Language::Ja,
        "ko" => l10n::Language::Ko,
        "ru" => l10n::Language::Ru,
        "da" => l10n::Language::Da,
        "de" => l10n::Language::De,
        "fr" => l10n::Language::Fr,
        "it" => l10n::Language::It,
        "nb" | "no" => l10n::Language::Nb,
        "nl" => l10n::Language::Nl,
        "pt" => l10n::Language::PtBr,
        "fi" => l10n::Language::Fi,
        "es" => l10n::Language::Es,
        "sv" => l10n::Language::Sv,
        "uk" => l10n::Language::Uk,
        _ => l10n::Language::EnUs,
    }
}

#[derive(Debug, Clone)]
enum Message {
    ModeSelected(Mode),
    /// The OS colour scheme was reported (at startup) or changed.
    SystemTheme(iced::theme::Mode),
    /// The round "i" button beside the language selector was pressed.
    AboutPressed,
    /// The About dialog was dismissed (its button or the backdrop).
    AboutClosed,
    /// A URL shown in the About dialog was clicked.
    AboutOpenUrl(String),
    LanguageSelected(l10n::Language),
    LoginRequested(Click),
    LoginUrlFetched(Result<String, String>),
    WebViewFinished(Result<String, String>),
    ManualInputChanged(String),
    SubmitManual,
    LoginCallback(String),
    OpenBrowser,
    /// Run the login in the embedded WebView instead of the system browser.
    LoginWebViewRetry,
    CopyUrl,
    CancelLogin,
    ProtocolToggled,
    BrowserOpened,
    LookupModeSelected(LookupMode),
    ImeiInputChanged(String),
    LookupRequested,
    LookupFinished(Result<firmware::FirmwareInfo, firmware::FirmwareError>),
    RetcnPlatformSelected(firmware::Platform),
    RetcnSimCountSelected(SimCount),
    RetcnFieldChanged(RetcnField, String),
    FastbootFillRequested,
    FastbootDevicesFetched(Result<fastboot_info::DeviceList, String>),
    FastbootDeviceSelected(String),
    FastbootDevicePickerCancelled,
    FastbootFillFinished(Result<(String, fastboot_info::DeviceInfo), String>),
    RetcnLookupRequested,
    TabletSnChanged(String),
    TabletLookupRequested,
    TabletLookupFinished(Result<LookupResult, String>),
    ModelInputChanged(String),
    ModelLookupRequested,
    ModelMatchParamsFetched(Result<firmware::ModelMatchParams, firmware::FirmwareError>),
    ModelParamChanged(usize, String),
    ModelCategorySelected(firmware::Category),
    ModelCountryChanged(String),
    BulkEdit(text_editor::Action),
    BulkLookupRequested,
    BulkStepFinished(String, Result<firmware::FirmwareInfo, firmware::FirmwareError>),
    BulkSaveRequested,
    BulkSaveFinished(Result<Option<std::path::PathBuf>, String>),
    CopyCnPassword(String),
    CopyDownloadUri(String),
    CopyToolUri(String),
    CopyRawJson(String),
    DecryptPickDirRequested,
    DecryptDirPicked(Option<std::path::PathBuf>),
    DecryptCustomPasswordToggled(bool),
    DecryptPasswordChanged(String),
    DecryptRequested,
    DecryptFinished(Result<decrypt::DecryptSummary, String>),
    /// A feature tile of the firmware-decrypt mode (Mode 3) was pressed.
    DecryptFeaturePressed(decrypt_mode::DecryptFeature),
    /// Tablet Firmware Decrypt: close the dialog.
    DecryptCancel,
    /// Click on the dialog's empty space (swallowed, so it neither dismisses
    /// the dialog nor reaches the UI underneath).
    DecryptBackdropPressed,
    SmartphoneFeaturePressed(SmartphoneFeature),
    BootloaderManualSelected,
    BootloaderGuidedSelected,
    BootloaderCancel,
    BootloaderBackdropPressed,
    BootloaderReturnToChooser,
    BootloaderOpenSite,
    BootloaderCopyDeviceId,
    BootloaderReadRequested,
    BootloaderDevicesFetched(Result<fastboot_info::DeviceList, String>),
    BootloaderDeviceSelected(String),
    BootloaderPickerCancelled,
    BootloaderDeviceIdRead(Result<String, String>),
    BootloaderEligibilityChecked(Result<bool, String>),
    BootloaderKeyChanged(String),
    BootloaderPasteRequested,
    BootloaderPasteFetched(Option<String>),
    BootloaderUnlockRequested,
    BootloaderUnlockFinished(Result<String, fastboot_info::UnlockFailure>),
    /// Tablet Bootloader Unlock: the feature tile's Tablet button was pressed.
    TabletUnlockSelected,
    /// Tablet Bootloader Unlock: the manual (ZUI) flow was chosen.
    TabletUnlockManualSelected,
    /// Tablet Bootloader Unlock: go back to the chooser.
    TabletUnlockReturnToChooser,
    /// Tablet Bootloader Unlock: close the dialog.
    TabletUnlockCancel,
    /// Click on the dialog's empty space (swallowed).
    TabletUnlockBackdropPressed,
    /// Tablet Bootloader Unlock: open ZUI's unlock website.
    TabletUnlockOpenSite,
    /// Tablet Bootloader Unlock: copy the serial number.
    TabletUnlockCopySerial,
    /// Tablet Bootloader Unlock: copy the Bootloader_SN.
    TabletUnlockCopySn,
    /// Tablet Bootloader Unlock: read the identity from fastboot.
    TabletUnlockReadRequested,
    /// Tablet Bootloader Unlock: the connected fastboot devices were listed.
    TabletUnlockDevicesFetched(Result<Vec<String>, String>),
    /// Tablet Bootloader Unlock: the user picked a device to read from.
    TabletUnlockDeviceSelected(String),
    /// Tablet Bootloader Unlock: the device picker was cancelled.
    TabletUnlockPickerCancelled,
    /// Tablet Bootloader Unlock: the identity was read from a device.
    TabletUnlockInfoRead(String, Result<tablet_unlock::TabletUnlockInfo, String>),
    /// Tablet Bootloader Unlock: the user asked to unlock.
    TabletUnlockRequested,
    /// Tablet Bootloader Unlock: stop the unlock job and start over.
    TabletUnlockCancelRestart,
    /// Tablet Bootloader Unlock: the unlock job finished.
    TabletUnlockFinished(tablet_unlock::UnlockOutcome),
    /// Tablet Bootloader Unlock: the guided (automatic) flow was chosen.
    TabletGuidedSelected,
    /// Tablet Bootloader Unlock (guided): re-scan for connected devices.
    TabletGuidedRefresh,
    /// Tablet Bootloader Unlock (guided): the connected devices were listed.
    TabletGuidedDevicesFetched(Result<Vec<tablet_unlock::TabletDevice>, String>),
    /// Tablet Bootloader Unlock (guided): the device to unlock was picked.
    TabletGuidedDeviceSelected(String),
    /// Tablet Bootloader Unlock (guided): the running job reported progress.
    TabletGuidedEvent(u64, tablet_unlock::GuidedEvent),
    /// Tablet Bootloader Unlock (guided): the automatic unlock finished.
    TabletGuidedFinished(u64, tablet_unlock::GuidedReport),
    /// Tablet Bootloader Unlock (guided): stop the job and start over.
    TabletGuidedCancelRestart,
    GuidedLogin,
    GuidedLoginFinished(Result<webview::PortalLogin, String>),
    GuidedLogoutFinished(Result<(), String>),
    GuidedAccountFetched(Result<(String, String), String>),
    GuidedNext,
    GuidedRequestKey,
    GuidedRequestConfirmYes,
    GuidedRequestConfirmNo,
    GuidedKeyRequestFinished(Result<(), String>),
    /// Factory Reset: the connected fastboot devices were listed.
    FactoryResetDevicesFetched(Result<fastboot_info::DeviceList, String>),
    /// Factory Reset: the user picked the device to reset.
    FactoryResetDeviceSelected(String),
    /// Factory Reset: the requirement checks finished.
    FactoryResetCheckFinished(Result<fastboot_info::FactoryResetCheck, String>),
    /// Factory Reset: the user confirmed the destructive reset.
    FactoryResetConfirmed,
    /// Factory Reset: re-scan for connected devices.
    FactoryResetRescan,
    /// Factory Reset: go back to the device list.
    FactoryResetBack,
    /// Factory Reset: close the dialog.
    FactoryResetCancel,
    /// Click on the dialog's empty space (swallowed, so it neither dismisses
    /// the dialog nor reaches the UI underneath).
    FactoryResetBackdropPressed,
    /// Factory Reset: `erase userdata` / `erase metadata` finished.
    FactoryResetFinished(Result<(), String>),
    /// Firmware Flash: the user asked for the package picker.
    FirmwareFlashPickRequested,
    /// Firmware Flash: the package picker closed (`None` = cancelled).
    FirmwareFlashPicked(Option<std::path::PathBuf>),
    /// Firmware Flash: the backend that runs the steps was switched.
    FirmwareFlashEngineSelected(flash_engine::Engine),
    /// Firmware Flash: Google's platform-tools were downloaded, or could not
    /// be.
    FirmwareFlashToolsInstalled(Result<std::path::PathBuf, String>),
    /// Firmware Flash: download Google's platform-tools again, to get the
    /// current build.
    FirmwareFlashToolsUpdate,
    /// Firmware Flash: open a terminal in the platform-tools directory.
    FirmwareFlashOpenTerminal,
    /// Firmware Flash: the check for a "Minimal ADB and Fastboot" install
    /// finished (`true` = it is installed and has to be removed first).
    FirmwareFlashTerminalChecked(bool),
    /// Firmware Flash: the terminal was opened in the carried directory, or
    /// could not be.
    FirmwareFlashTerminalOpened(Result<std::path::PathBuf, String>),
    /// Firmware Flash: the selected tool answered `--version` (the path tells
    /// an answer for a tool that is not selected any more apart).
    FirmwareFlashToolVersionRead(std::path::PathBuf, Result<String, String>),
    /// Firmware Flash: the "verify checksums" box was toggled.
    FirmwareFlashVerifyToggled(bool),
    /// Firmware Flash: read what the selected phone reports about itself
    /// (`getvar all`, `oem hw`, `oem build-signature`, `oem read_sv`,
    /// `oem config`).
    FirmwareFlashReadInfo,
    /// Firmware Flash: those commands finished.
    FirmwareFlashInfoRead(Result<Vec<String>, String>),
    /// Firmware Flash: mask (or unmask) the IMEIs in that output.
    FirmwareFlashInfoSensitiveToggled(bool),
    /// Firmware Flash: copy that output.
    FirmwareFlashInfoCopy,
    /// Firmware Flash: leave the output.
    FirmwareFlashInfoClosed,
    /// Firmware Flash: re-scan the USB bus for connected phones.
    FirmwareFlashRescan,
    /// Firmware Flash: the connected devices were listed.
    FirmwareFlashDevicesFetched(Result<fastboot_info::DeviceList, String>),
    /// Firmware Flash: the device to flash was selected.
    FirmwareFlashDeviceSelected(String),
    /// Firmware Flash: the selected device's `securestate`/`cid`/`product`
    /// were read (the serial tells stale answers from a previous selection
    /// apart).
    FirmwareFlashDeviceVarsRead(String, Result<fastboot_info::DeviceVars, String>),
    /// Firmware Flash: "Start Flashing" was pressed (asks for confirmation).
    FirmwareFlashStart,
    /// Firmware Flash: one second of the countdown elapsed that unlocks "Yes"
    /// after the warning that this package belongs to another phone.
    FirmwareFlashCountdownTick,
    /// Firmware Flash: the warning was confirmed; run the package.
    FirmwareFlashConfirmed,
    /// Firmware Flash: back from the warning to the setup form.
    FirmwareFlashBack,
    /// Firmware Flash: open the procedure checklist.
    FirmwareFlashEditProcedures,
    /// Firmware Flash: one procedure of the checklist was (de)selected.
    FirmwareFlashProcedureToggled(usize, bool),
    /// Firmware Flash: check or uncheck every procedure.
    FirmwareFlashSelectAllProcedures(bool),
    /// Firmware Flash: check only the procedures of one firmware part (AP/BP/BL).
    FirmwareFlashSelectPart(flashfile::FlashPart),
    /// Firmware Flash: add or remove an erase group (user data / NV cache).
    FirmwareFlashEraseGroupToggled(flashfile::EraseGroup),
    /// Firmware Flash: write the selected procedures into a script
    /// (`*.cmd` on Windows, `*.sh` elsewhere).
    FirmwareFlashExport,
    /// Firmware Flash: whether the tool the script starts is copied next to
    /// it, as the question before the export asked.
    FirmwareFlashExportCopy(bool),
    /// Firmware Flash: that script was written (or the save dialog was
    /// cancelled, or the write failed).
    FirmwareFlashExported(Result<Option<std::path::PathBuf>, String>),
    /// Firmware Flash: show the file this run's flashing log is written to in
    /// the system's file manager (the folder both modes write into when the
    /// dialog has no file of its own yet).
    FirmwareFlashLocateLog,
    /// Firmware Flash: the log was handed to the file manager (only logs a
    /// failure).
    FirmwareFlashLogOpened,
    /// Firmware Flash: leave fastboot for Android once the flash is done.
    FirmwareFlashReboot,
    /// Firmware Flash: a mode was picked from the Reboot dropdown.
    FirmwareFlashRebootSelected(flash_engine::RebootMode),
    /// Firmware Flash: that reboot finished (or failed).
    FirmwareFlashRebooted(u64, Result<(), String>),
    /// Firmware Flash: leave the procedure checklist.
    FirmwareFlashEditDone,
    /// Firmware Flash: close the dialog.
    FirmwareFlashCancel,
    /// Single Partition Flash: re-scan the USB bus for connected phones.
    PartitionFlashRescan,
    /// Single Partition Flash: the connected phones were listed.
    PartitionFlashDevicesFetched(Result<Vec<fastboot_info::DeviceSummary>, String>),
    /// Single Partition Flash: the phone to write to was selected.
    PartitionFlashDeviceSelected(String),
    /// Single Partition Flash: that phone's variables were read (the serial
    /// tells stale answers from a previous selection apart).
    PartitionFlashDeviceVarsRead(String, Result<fastboot_info::DeviceVars, String>),
    /// Single Partition Flash: the tool that writes the image was switched.
    PartitionFlashEngineSelected(flash_engine::Engine),
    /// Single Partition Flash: the selected tool answered `--version` (the
    /// path tells an answer for a tool that is not selected any more apart).
    PartitionFlashToolVersionRead(std::path::PathBuf, Result<String, String>),
    /// Single Partition Flash: the partition name was edited.
    PartitionFlashPartitionChanged(String),
    /// Single Partition Flash: one of the usual partition names was picked
    /// from the dropdown, which fills the field.
    PartitionFlashPartitionPreset(&'static str),
    /// Single Partition Flash: ask for the image file.
    PartitionFlashPickImage,
    /// Single Partition Flash: the picker closed (`None` = cancelled); a file
    /// arrives with whether it carries a vbmeta struct.
    PartitionFlashImagePicked(Option<(std::path::PathBuf, bool)>),
    /// Single Partition Flash: an image file was dropped onto the window.
    PartitionFlashImageDropped(std::path::PathBuf),
    /// Single Partition Flash: the "disable verity" box was toggled.
    PartitionFlashVerityToggled(bool),
    /// Single Partition Flash: the "disable verification" box was toggled.
    PartitionFlashVerificationToggled(bool),
    /// Single Partition Flash: write the image to the partition.
    PartitionFlashStart,
    /// Single Partition Flash: progress and output of the running flash.
    PartitionFlashEvent(u64, flash_engine::FlashEvent),
    /// Single Partition Flash: leave fastboot for Android.
    PartitionFlashReboot,
    /// Single Partition Flash: the reboot that leaves fastboot for Android
    /// finished (or failed).
    PartitionFlashRebooted(u64, Result<(), String>),
    /// Single Partition Flash: send the phone into userspace fastboot.
    PartitionFlashSwitchToFastbootd,
    /// Single Partition Flash: that switch finished, and the phones found
    /// afterwards are carried back so the dialog can pick the same one up.
    PartitionFlashSwitched(u64, Result<Vec<fastboot_info::DeviceSummary>, String>),
    /// A background Single Partition Flash job ended (only completes the task;
    /// the results arrive through the messages above).
    PartitionFlashWorkerDone,
    /// Single Partition Flash: show this run's log file in the file manager
    /// (the folder the logs are written into when there is no such file yet).
    PartitionFlashLocateLog,
    /// Single Partition Flash: the log was handed to the file manager (only
    /// logs a failure).
    PartitionFlashLogOpened,
    /// Single Partition Flash: close the dialog.
    PartitionFlashCancel,
    /// Click on the dialog's empty space (swallowed, so it neither dismisses
    /// the dialog nor reaches the UI underneath).
    PartitionFlashBackdropPressed,
    /// Multi Flashing: open the workspace that flashes a whole batch of
    /// phones, which takes the place of the Mode 2 feature grid.
    MultiFlashingSelected,
    /// Qualcomm Partition Manager: open the workspace that lists the
    /// partitions of a phone read through Firehose, which takes the place of
    /// the Mode 2 feature grid.
    QualcommManagerSelected,
    /// Qualcomm Partition Manager: leave the workspace.
    QualcommExit,
    /// Qualcomm Partition Manager: the partition search box was edited.
    QualcommFilterChanged(String),
    /// Qualcomm Partition Manager: a port of the dropdown was picked.
    QualcommPortSelected(String),
    /// Qualcomm Partition Manager: a row of the partition table was clicked.
    /// Command/Control toggles it, Shift extends a range from the row clicked
    /// before, and a plain click selects it alone.
    QualcommPartitionClicked(usize),
    /// Qualcomm Partition Manager: the modifier keys changed.
    QualcommModifiersChanged(iced::keyboard::Modifiers),
    /// Qualcomm Partition Manager: Control+A asked for every row on screen.
    QualcommSelectAll,
    /// Qualcomm Partition Manager: "Erase" was pressed — a destructive action
    /// asks first.
    QualcommErase,
    /// Qualcomm Partition Manager: the erase confirmation was answered.
    QualcommEraseConfirmed(bool),
    /// Qualcomm Partition Manager: progress of an erase.
    QualcommEraseProgress(firehose::Progress),
    /// Qualcomm Partition Manager: the erase ended with how many partitions it
    /// cleared (or the reason it could not).
    QualcommEraseFinished(Result<usize, String>),
    /// Qualcomm Partition Manager: the refresh button was pressed — the table
    /// is cleared and read again.
    QualcommRefresh,
    /// Qualcomm Partition Manager: the table was read again (or the reason it
    /// could not be).
    QualcommRefreshed(Result<firehose::Table, String>),
    /// Qualcomm Partition Manager: a storage type was picked.
    QualcommStorageTypeSelected(StorageType),
    /// Qualcomm Partition Manager: time to look the Firehose ports up again
    /// (the page polls them while it is open).
    QualcommPortsPolled,
    /// Qualcomm Partition Manager: the ports a Firehose device offers now.
    QualcommPortsScanned(Vec<String>),
    /// Qualcomm Partition Manager: open Windows' Device Manager, where the
    /// phone appears as a serial port.
    QualcommOpenDeviceManager,
    /// Qualcomm Partition Manager: show the last Firehose session's log in the
    /// file manager.
    QualcommLocateLog,
    /// Qualcomm Partition Manager: an entry of the danger-zone menu was
    /// picked. The entries are placeholders, so the pick is ignored.
    QualcommDangerZoneSelected,
    /// Qualcomm Partition Manager: "Inject Firehose" was pressed — ask for
    /// the programmer file.
    QualcommInjectFirehose,
    /// Qualcomm Partition Manager: the programmer file the picker returned.
    QualcommProgrammerPicked(Option<std::path::PathBuf>),
    /// Qualcomm Partition Manager: the injection moved on from the upload to
    /// reading the partition table.
    QualcommFirehoseReading,
    /// Qualcomm Partition Manager: the injection ended with the table it read
    /// (or the reason it could not).
    QualcommFirehoseReady(Result<firehose::Table, String>),
    /// Qualcomm Partition Manager: an entry of the reboot menu was picked.
    /// Only a normal reboot is implemented so far.
    QualcommRebootModeSelected(RebootMode),
    /// Qualcomm Partition Manager: the reset command was sent (or failed).
    QualcommResetFinished(Result<(), String>),
    /// Qualcomm Partition Manager: progress of the writes a reboot mode does
    /// before it resets.
    QualcommRebootProgress(firehose::Progress),
    /// Qualcomm Partition Manager: "Dump" was pressed — ask where the files
    /// should go: a file for one selected row, a folder for several.
    QualcommDump,
    /// Qualcomm Partition Manager: the folder the dump should be written to.
    QualcommDumpFolderPicked(Option<std::path::PathBuf>),
    /// Qualcomm Partition Manager: the file a single-partition dump should be
    /// written to.
    QualcommDumpFilePicked(Option<std::path::PathBuf>),
    /// Qualcomm Partition Manager: progress of a dump (partition, bytes done,
    /// total bytes of the job).
    QualcommDumpProgress(firehose::Progress),
    /// Qualcomm Partition Manager: the dump ended with the number of files it
    /// wrote and where they are (or the reason it could not).
    QualcommDumpFinished(Result<(usize, std::path::PathBuf), String>),
    /// Qualcomm Partition Manager: "Dump Entire Storage" was pressed.
    QualcommDumpStorage,
    /// Qualcomm Partition Manager: the phone answered with the physical
    /// partitions the storage is split into.
    QualcommLunsRead(Result<Vec<firehose::Lun>, String>),
    /// Qualcomm Partition Manager: one LUN of the picker was checked or
    /// unchecked.
    QualcommLunToggled(usize, bool),
    /// Qualcomm Partition Manager: every LUN of the picker was checked or
    /// unchecked.
    QualcommLunsAll(bool),
    /// Qualcomm Partition Manager: the picker's Dump button.
    QualcommLunsConfirmed,
    /// Qualcomm Partition Manager: a click beside the LUN picker's card. It is
    /// swallowed, so it neither dismisses the card nor reaches the page.
    QualcommLunBackdropPressed,
    /// Qualcomm Partition Manager: the picker was dismissed.
    QualcommLunDialogClosed,
    /// Qualcomm Partition Manager: the window's close button was pressed while
    /// a Firehose session was live. The prompt that asks first is showing.
    QualcommClosePromptCancelled,
    /// Qualcomm Partition Manager: a click beside the close prompt. It is
    /// swallowed, so it neither dismisses the prompt nor reaches the page.
    QualcommClosePromptBackdropPressed,
    /// Qualcomm Partition Manager: "Close Anyway" in the prompt.
    QualcommCloseConfirmed,
    /// Qualcomm Partition Manager: the folder a multi-LUN dump should be
    /// written to.
    QualcommStorageFolderPicked(Option<std::path::PathBuf>),
    /// Qualcomm Partition Manager: the file a single-LUN dump should be
    /// written to.
    QualcommStorageFilePicked(Option<std::path::PathBuf>),
    /// Qualcomm Partition Manager: the storage dump ended with the number of
    /// images it wrote and where they are (or the reason it could not).
    QualcommStorageDumpFinished(Result<(usize, std::path::PathBuf), String>),
    /// Qualcomm Partition Manager: "Write" was pressed — ask for the image.
    QualcommWrite,
    /// Qualcomm Partition Manager: the image the picker returned.
    QualcommWriteFilePicked(Option<std::path::PathBuf>),
    /// Qualcomm Partition Manager: progress of a write (bytes done, total).
    QualcommWriteProgress(firehose::Progress),
    /// Qualcomm Partition Manager: the write ended with the partition it wrote
    /// to (or the reason it could not).
    QualcommWriteFinished(Result<String, String>),
    /// Click on the dialog's empty space (swallowed, so it neither dismisses
    /// the dialog nor reaches the UI underneath).
    FirmwareFlashBackdropPressed,
    /// Firmware Flash: progress of the package extraction / parsing.
    FirmwareFlashPackageEvent(u64, flash_engine::PackageEvent),
    /// Firmware Flash: progress of the flashing itself.
    FirmwareFlashEvent(u64, flash_engine::FlashEvent),
    /// A background firmware-flash job ended (only completes the task; the
    /// results arrive through the event messages above).
    FirmwareFlashWorkerDone,
    /// Install Driver: the tile's button — opens the dialog, where the
    /// installation is started by pressing Install.
    DriverInstallRequested,
    /// Install Driver: open Lenovo's "Software Fix" page for tablet firmware.
    DriverTabletSite,
    /// Install Driver: the sudo password was edited.
    DriverPasswordChanged(String),
    /// Install Driver: the dialog's Install button (or Enter in the password
    /// field) — start the installation. Linux asks for the password before
    /// this can do anything; Windows needs nothing but the click.
    DriverInstallConfirmed,
    /// Install Driver: close the dialog.
    DriverCancel,
    /// Install Driver: progress of the running installation.
    DriverEvent(u64, driver_install::DriverEvent),
    /// A background driver installation ended (only completes the task; the
    /// result arrives through the event message above).
    DriverWorkerDone,
    /// Click on the dialog's empty space (swallowed, so it neither dismisses
    /// the dialog nor reaches the UI underneath).
    DriverBackdropPressed,
    /// Remove System Bloatware: the connected devices were listed
    /// (`adb devices -l`), which also prepared ADB itself.
    BloatwareDevicesFetched(Result<Vec<debloat::AdbDevice>, String>),
    /// Remove System Bloatware: the user picked the device to work on.
    BloatwareDeviceSelected(String),
    /// Remove System Bloatware: reconnect to a device whose "Allow USB
    /// debugging?" prompt was dismissed, so it is shown again.
    BloatwareRetry(String),
    /// Remove System Bloatware: re-scan for connected devices.
    BloatwareRescan,
    /// Remove System Bloatware: the device's installed bloatware was read.
    BloatwarePackagesFetched(Result<Vec<debloat::Bloatware>, String>),
    /// Remove System Bloatware: one app of the checklist was (de)selected.
    BloatwareAppToggled(usize, bool),
    /// Remove System Bloatware: the search box above the checklist changed.
    BloatwareFilterChanged(String),
    /// Remove System Bloatware: check or uncheck every app.
    BloatwareSelectAll(bool),
    /// Remove System Bloatware: remove the checked apps.
    BloatwareRemove,
    /// Remove System Bloatware: the removal finished, one outcome per app.
    BloatwareRemoved(Vec<debloat::RemovalOutcome>),
    /// Remove System Bloatware: leave the result and go back to the (now
    /// shorter) checklist.
    BloatwareDone,
    /// Remove System Bloatware: go back to the device list.
    BloatwareBackToDevices,
    /// Remove System Bloatware: close the dialog.
    BloatwareCancel,
    /// Click on the dialog's empty space (swallowed, so it neither dismisses
    /// the dialog nor reaches the UI underneath).
    BloatwareBackdropPressed,
    /// Connect ADB Wireless Debugging: the pairing method was switched.
    WirelessMethodSelected(WirelessMethod),
    /// Connect ADB Wireless Debugging: a fresh QR code was asked for.
    WirelessCodeRegenerated,
    /// Connect ADB Wireless Debugging: the pairing service to work with was
    /// picked from the list.
    WirelessServiceSelected(String),
    /// Connect ADB Wireless Debugging: the pairing code was edited.
    WirelessCodeChanged(String),
    /// Connect ADB Wireless Debugging: pair with the chosen phone by hand.
    WirelessPairRequested,
    /// Connect ADB Wireless Debugging: drop the selected wireless connection
    /// (one phone, or every one of them).
    WirelessDisconnectSelected(wireless::DisconnectTarget),
    /// Connect ADB Wireless Debugging: the network discovery reported
    /// something (the id tells a browse that was replaced apart).
    WirelessDiscovered(u64, wireless::DiscoverEvent),
    /// Connect ADB Wireless Debugging: the pairing, and the connect that
    /// follows it, finished.
    WirelessFinished(Result<wireless::PairOutcome, String>),
    /// Connect ADB Wireless Debugging: `adb disconnect` finished (the target
    /// is carried back so the status line can name it).
    WirelessDisconnected(wireless::DisconnectTarget, Result<(), String>),
    /// Connect ADB Wireless Debugging: the wireless devices `adb` lists as
    /// connected were read.
    WirelessDevicesFetched(Vec<String>),
    /// Connect ADB Wireless Debugging: close the dialog.
    WirelessCancel,
    /// Click on the dialog's empty space (swallowed, so it neither dismisses
    /// the dialog nor reaches the UI underneath).
    WirelessBackdropPressed,
    /// Fix Captive Portal Server: the devices `adb devices` lists were read.
    CaptivePortalDevicesFetched(Result<Vec<debloat::AdbDevice>, String>),
    /// Fix Captive Portal Server: look for devices again.
    CaptivePortalRescan,
    /// Fix Captive Portal Server: the device to fix was chosen.
    CaptivePortalDeviceSelected(String),
    /// Fix Captive Portal Server: reconnect to a device whose "Allow USB
    /// debugging?" prompt was dismissed, so it is shown again.
    CaptivePortalRetry(String),
    /// Fix Captive Portal Server: apply the fix to the chosen device.
    CaptivePortalFix,
    /// Fix Captive Portal Server: the fix finished.
    CaptivePortalFixed(Result<captive_portal::FixReport, captive_portal::FixError>),
    /// Fix Captive Portal Server: close the dialog.
    CaptivePortalCancel,
    /// Click on the dialog's empty space (swallowed, so it neither dismisses
    /// the dialog nor reaches the UI underneath).
    CaptivePortalBackdropPressed,
    /// Unpack Image: ask for the image file to unpack.
    UnpackPickImageRequested,
    /// Unpack Image: the file picker closed (`None` = cancelled).
    UnpackImagePicked(Option<std::path::PathBuf>),
    /// Unpack Image: the picked image's directory was read (the carried path
    /// tells a late answer for another file apart).
    UnpackListed(
        std::path::PathBuf,
        Result<Vec<image_unpack::Entry>, image_unpack::UnpackError>,
    ),
    /// Unpack Image: ask for the folder the files are written to.
    UnpackPickFolderRequested,
    /// Unpack Image: the folder picker closed (`None` = cancelled).
    UnpackFolderPicked(Option<std::path::PathBuf>),
    /// Unpack Image: start extracting the selected image.
    UnpackStart,
    /// Unpack Image: show the output folder in the system's file manager.
    UnpackLocate,
    /// Unpack Image: the output folder was handed to the file manager (only
    /// logs a failure).
    UnpackFolderOpened,
    /// Unpack Image: progress of the running extraction.
    UnpackEvent(u64, image_unpack::UnpackEvent),
    /// A background extraction ended (only completes the task; the outcome
    /// arrives through the event message above).
    UnpackWorkerDone,
    /// Unpack Image: close the dialog.
    UnpackCancel,
    /// Click on the dialog's empty space (swallowed, so it neither dismisses
    /// the dialog nor reaches the UI underneath).
    UnpackBackdropPressed,
    /// Unisoc PAC: ask for the package file to unpack.
    PacPickFileRequested,
    /// Unisoc PAC: the file picker closed (`None` = cancelled).
    PacFilePicked(Option<std::path::PathBuf>),
    /// Unisoc PAC: the picked package's file table was read (the carried path
    /// tells a late answer for another file apart).
    PacListed(std::path::PathBuf, Result<pac::Package, pac::PacError>),
    /// Unisoc PAC: ask for the folder the files are written to.
    PacPickFolderRequested,
    /// Unisoc PAC: the folder picker closed (`None` = cancelled).
    PacFolderPicked(Option<std::path::PathBuf>),
    /// Unisoc PAC: check or uncheck one entry.
    PacEntryToggled(usize, bool),
    /// Unisoc PAC: check or uncheck every entry.
    PacSelectAll(bool),
    /// Unisoc PAC: the search box above the file list changed.
    PacFilterChanged(String),
    /// Unisoc PAC: write the `spd_dump` scripts next to the files, or not.
    PacScriptToggled(bool),
    /// Unisoc PAC: start extracting the checked files.
    PacStart,
    /// Unisoc PAC: show the output folder in the system's file manager.
    PacLocate,
    /// Unisoc PAC: the output folder was handed to the file manager (only
    /// logs a failure).
    PacFolderOpened,
    /// Unisoc PAC: progress of the running extraction.
    PacEvent(u64, PacJobEvent),
    /// A background extraction ended (only completes the task; the outcome
    /// arrives through the event message above).
    PacWorkerDone,
    /// Unisoc PAC: close the dialog.
    PacCancel,
    /// Click on the dialog's empty space (swallowed, so it neither dismisses
    /// the dialog nor reaches the UI underneath).
    PacBackdropPressed,
    /// Repack Package: ask for the `flash.cfg` or scatter XML to build from.
    PacRepackPickSourceRequested,
    /// Repack Package: the source picker closed (`None` = cancelled).
    PacRepackSourcePicked(Option<std::path::PathBuf>),
    /// Repack Package: the picked source was read (the carried path tells a
    /// late answer for another file apart).
    PacRepackPlanned(
        std::path::PathBuf,
        Result<pac_build::Plan, pac_build::BuildError>,
    ),
    /// Repack Package: the product alias the package should carry changed.
    PacRepackAliasChanged(String),
    /// Repack Package: the product version the package should carry changed.
    PacRepackVersionChanged(String),
    /// Repack Package: the search box above the record list changed.
    PacRepackFilterChanged(String),
    /// Repack Package: the record table was scrolled sideways — the header of
    /// the table follows it.
    PacRepackScrolled(f32),
    /// Repack Package: leave one record in the package, or out of it.
    PacRepackToggle(usize, bool),
    /// Repack Package: leave in, or out, every record the search box shows.
    PacRepackSelectAll(bool),
    /// Repack Package: leave out every record that still has no file.
    PacRepackLeaveOut,
    /// Repack Package: ask for the file of one record.
    PacRepackSelectFile(usize),
    /// Repack Package: that picker closed (`None` = cancelled).
    PacRepackFilePicked(usize, Option<std::path::PathBuf>),
    /// Repack Package: ask where the package is written, and build it.
    PacRepackStart,
    /// Repack Package: the save dialog closed (`None` = cancelled).
    PacRepackOutputChosen(Option<std::path::PathBuf>),
    /// Repack Package: show the written package in the system's file manager.
    PacRepackLocate,
    /// Repack Package: the folder was handed to the file manager (only logs a
    /// failure).
    PacRepackFolderOpened,
    /// Repack Package: progress of the running build.
    PacRepackEvent(u64, PacRepackEvent),
    /// A background build ended (only completes the task; the outcome arrives
    /// through the event message above).
    PacRepackWorkerDone,
    /// Repack Package: close the dialog.
    PacRepackCancel,
    /// Click on the dialog's empty space (swallowed, so it neither dismisses
    /// the dialog nor reaches the UI underneath).
    PacRepackBackdropPressed,
    /// Multi Flashing: leave the page and return to the Mode 2 feature grid.
    MultiFlashExit,
    /// Multi Flashing: open the "Preset Manage" dialog.
    MultiFlashPresetManage,
    /// Multi Flashing: add a firmware image to the run (file picker).
    MultiFlashPresetAdd,
    /// Multi Flashing: that picker closed (`None` = cancelled). The picked
    /// image is then configured by the flash-options card.
    MultiFlashPresetPicked(Option<std::path::PathBuf>),
    /// Multi Flashing: close the "Preset Manage" dialog.
    MultiFlashPresetClose,
    /// Multi Flashing: the flash-options card's reboot switch changed.
    MultiFlashOptionsRebootToggled(bool),
    /// Multi Flashing: the flash-options card's checksum switch changed
    /// (`false` skips the MD5 check of every image).
    MultiFlashVerifyToggled(bool),
    /// Multi Flashing: the flashing tool of the options card changed.
    MultiFlashToolSelected(PresetTool),
    /// Multi Flashing: choose (or change) the directory a ZIP package is
    /// unpacked into.
    MultiFlashPresetChooseDirectory,
    /// Multi Flashing: that picker closed (`None` = cancelled; a cancelled
    /// first pick drops the firmware being added, see [`DirectoryAsk`]).
    MultiFlashPresetDirectoryPicked(Option<std::path::PathBuf>, DirectoryAsk),
    /// Multi Flashing: what the unpacking of a package reports (progress, the
    /// finished package, or why it failed).
    MultiFlashUnpackEvent(std::path::PathBuf, flash_engine::PackageEvent),
    /// Multi Flashing: the unpacking worker is done (only used to keep its
    /// task alive, so a panic is reported instead of waiting forever).
    MultiFlashUnpackWorkerDone,
    /// Multi Flashing: write the preset list to a file (save dialog).
    MultiFlashPresetSave,
    /// Multi Flashing: that dialog closed (`None` = cancelled), or the file
    /// could not be written.
    MultiFlashPresetSaved(
        Result<Option<std::path::PathBuf>, preset_file::FileError>,
    ),
    /// Multi Flashing: read a preset list back from a file (file picker).
    MultiFlashPresetLoad,
    /// Multi Flashing: that picker closed (`None` = cancelled), or the file
    /// could not be read.
    MultiFlashPresetLoaded(Result<Option<preset_file::Loaded>, preset_file::FileError>),
    /// Multi Flashing: what the picked firmware package says about itself.
    MultiFlashPresetIdentified(
        std::path::PathBuf,
        Result<flashfile::PackageIdentity, flashfile::LoadError>,
    ),
    /// Multi Flashing: change an entry of the preset list.
    MultiFlashPresetEdit(usize),
    /// Multi Flashing: drop an entry of the preset list.
    MultiFlashPresetRemoved(usize),
    /// Multi Flashing: the options card takes the default conditions.
    MultiFlashConditionsDefault,
    /// Multi Flashing: the options card takes the user's own conditions.
    MultiFlashConditionsCustom,
    /// Multi Flashing: the options card's default rule was told (not) to take
    /// an engineering unit as well (`true` = do not match one).
    MultiFlashNoEngineeringToggled(bool),
    /// Multi Flashing: the CID field of a custom condition changed.
    MultiFlashCidChanged(String),
    /// Multi Flashing: one of the documented CIDs was picked from the options
    /// card's dropdown, which puts it into the field (`None` is the rule that
    /// separates its groups).
    MultiFlashCidPresetSelected(Option<&'static str>),
    /// Multi Flashing: add what the CID field holds as one more tag.
    MultiFlashCidAdded,
    /// Multi Flashing: drop one of those tags.
    MultiFlashCidRemoved(usize),
    /// Multi Flashing: one of the accepted unlock states was (de)selected.
    MultiFlashStateToggled(fastboot_info::SecureState, bool),
    /// Multi Flashing: one procedure of the options card's checklist was
    /// (de)selected.
    MultiFlashStepToggled(usize, bool),
    /// Multi Flashing: check or uncheck every procedure of that checklist.
    MultiFlashStepsAll(bool),
    /// Multi Flashing: check the procedures of one firmware part (AP/BP/BL)
    /// without unchecking anything else.
    MultiFlashSelectPart(flashfile::FlashPart),
    /// Multi Flashing: the configured image joins the preset list.
    MultiFlashOptionsConfirm,
    /// Multi Flashing: the options card is dismissed; the image is not added.
    MultiFlashOptionsCancel,
    /// Click on the options card's empty space (swallowed).
    MultiFlashOptionsBackdropPressed,
    /// Click on the "Preset Manage" dialog's empty space (swallowed, so it
    /// neither dismisses the dialog nor reaches the UI underneath).
    MultiFlashPresetBackdropPressed,
    /// Multi Flashing: start the run of the loaded firmware images.
    MultiFlashStart,
    /// Multi Flashing: stop the running job.
    MultiFlashStop,
    /// Multi Flashing: show the folder this run writes its per-device logs
    /// into in the system's file manager (the button that takes "Preset
    /// Manage"'s place while a run goes on).
    MultiFlashLocateLog,
    /// Multi Flashing: that folder was handed to the file manager (only logs a
    /// failure).
    MultiFlashLogOpened,
    /// Multi Flashing: what the running session reports (a device it found, the
    /// progress of a flash, its end). The number is the run the report belongs
    /// to, so a session that was stopped never writes into the table of the
    /// next one.
    MultiFlashSessionEvent(u64, multi_flash_run::SessionEvent),
    /// Multi Flashing: the session's worker is done (only used to keep its task
    /// alive, so a panic is reported instead of waiting forever).
    MultiFlashSessionDone,
    /// The window was asked to close. The app answers it itself (the window
    /// settings turn iced's automatic close off), so a job can refuse to be
    /// cut short.
    WindowCloseRequested(iced::window::Id),
    /// Multi Flashing: acknowledge the prompt that blocked closing the
    /// window while a run was going on.
    MultiFlashClosePromptDismissed,
    /// Click on that prompt's empty space (swallowed).
    MultiFlashClosePromptBackdropPressed,
    /// The best-effort telemetry POST finished (only logged; telemetry must
    /// never affect the app).
    TelemetrySent(Result<(), String>),
    /// UI animation tick, emitted while a busy indicator is shown (drives the
    /// spinners without blocking the UI thread).
    AnimTick,
    /// Advances an in-flight feature-switch transition. Emitted every frame
    /// while one is running (see `anim`).
    TransitionTick,
    /// Click on a dialog that is animating out: swallowed, so nothing can be
    /// started while its card is leaving.
    OverlayLeavingPressed,
}

/// Drives the small UI spinner animations while a busy status is shown.
/// Returns an idle stream when nothing needs to animate, so the UI only ticks
/// (and redraws) while a spinner is actually visible.
fn subscription(state: &State) -> iced::Subscription<Message> {
    let guided = &state.flash.bootloader.guided;
    let reset = &state.flash.factory_reset;
    let firmware = &state.flash.firmware;
    let partition = &state.flash.partition;
    let driver = &state.flash.driver;
    let tablet = &state.flash.tablet_unlock;
    let bloatware = &state.flash.bloatware;
    let wireless = &state.flash.wireless;
    let portal = &state.flash.captive_portal;
    let unpack = &state.unpack;
    let pac = &state.pac;
    let repack = &state.pac_repack;
    let animating = guided.logging_in
        || guided.logging_out
        || guided.fetching_account
        || guided.requesting
        || reset.listing
        || reset.checking
        || reset.resetting
        || tablet.reading
        || tablet.unlocking
        || tablet.guided.listing
        || tablet.guided.unlocking
        || firmware.listing
        || firmware.loading
        || firmware.running
        || firmware.rebooting
        || firmware.reading_info
        || firmware.reading_device
        || firmware.installing_tools
        || firmware.terminal_busy
        || firmware.reading_version
        // The Single Partition Flash dialog spins for every job it runs, and
        // while it lists devices or asks a tool for its version.
        || partition.task.is_some()
        || partition.listing
        || partition.reading_device
        || partition.reading_version
        || driver.busy
        || bloatware.preparing
        || bloatware.checking
        || bloatware.removing
        || wireless.busy()
        || portal.busy()
        || unpack.listing
        || unpack.working
        || pac.listing
        || pac.working
        || repack.reading
        || repack.working
        // A firmware lookup shows a spinner while it runs, and the form eases
        // from one status to the next; the multi-IMEI batch spins as well.
        || matches!(state.lookup.status, LookupStatus::Fetching)
        || matches!(state.lookup.bulk.status, BulkStatus::Running { .. })
        // Reading a picked preset's package is quick, but it is still work
        // with a spinner — and unpacking one is not quick at all.
        || state.flash.multi_flash.pending.identifying
        || state.flash.multi_flash.pending.unpacking
        // The decrypt dialog only has text while it works, but its spinner
        // ticks along with the others.
        || matches!(state.decrypt.status, DecryptStatus::Working)
        // Injecting a programmer — and resetting the phone afterwards — keep
        // the status line's spinner turning.
        || state.flash.qualcomm.busy;
    let ticks = if animating {
        iced::time::every(std::time::Duration::from_millis(64)).map(|_| Message::AnimTick)
    } else {
        iced::Subscription::none()
    };
    // The countdown that unlocks "Yes" after the brick warning runs on whole
    // seconds, so it does not need the spinner's fast tick.
    let countdown = if firmware.confirm_countdown > 0 {
        iced::time::every(std::time::Duration::from_secs(1))
            .map(|_| Message::FirmwareFlashCountdownTick)
    } else {
        iced::Subscription::none()
    };
    // While the Qualcomm Partition Manager is open, the ports a Firehose
    // device offers are looked up once a second: plugging the phone in or out
    // has to show up in its dropdown without the user refreshing anything.
    let port_poll = if qualcomm::is_open(state) {
        iced::time::every(std::time::Duration::from_secs(1))
            .map(|_| Message::QualcommPortsPolled)
    } else {
        iced::Subscription::none()
    };
    // The table's rows read the modifier keys, and Control+A takes every row on
    // screen: the page listens to the keyboard while it is open.
    let page_input = if qualcomm::is_open(state) {
        iced::event::listen_with(|event, status, _window| match event {
            iced::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(modifiers)) => {
                Some(Message::QualcommModifiersChanged(modifiers))
            }
            iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, modifiers, .. })
                if status == iced::event::Status::Ignored =>
            {
                table_shortcut(&key, modifiers)
            }
            _ => None,
        })
    } else {
        iced::Subscription::none()
    };
    // A feature-switch transition animates at full frame rate, but only while
    // one is actually running; iced redraws once per tick.
    let transition = if state.content_transition.is_some()
        || state.overlay_transition.is_some()
        || state.login_transition.is_some()
        || state.lookup_transition.is_some()
        || state.pending_close.is_some()
    {
        iced::time::every(anim::FRAME).map(|_| Message::TransitionTick)
    } else {
        iced::Subscription::none()
    };

    // A window close request is answered by the app (iced's automatic close
    // is turned off in the window settings), so a running job can keep the
    // window open until it is stopped.
    let close_requests =
        iced::window::close_requests().map(Message::WindowCloseRequested);

    // The Fluent theme follows the OS light/dark setting; the subscription
    // also fires once at startup (iced reports the current mode).
    let system_theme =
        iced::system::theme_changes().map(Message::SystemTheme);

    // An image file dropped onto the window is for the Single Partition Flash
    // dialog: it is the only feature that takes one that way. (The window's
    // drag-and-drop support is on by default; a message that arrives with no
    // such dialog open is ignored by its handler.)
    let dropped_files = iced::event::listen_with(
        |event, _status, _window| match event {
            iced::Event::Window(iced::window::Event::FileDropped(path)) => {
                Some(Message::PartitionFlashImageDropped(path))
            }
            _ => None,
        },
    );

    iced::Subscription::batch([
        ticks,
        countdown,
        port_poll,
        page_input,
        transition,
        close_requests,
        system_theme,
        dropped_files,
    ])
}

/// The keys the partition table answers to.
///
/// Control (Command on macOS) plus A takes every row on screen. A key a focused
/// widget already took — the search box's own Control+A — never reaches here.
fn table_shortcut(
    key: &iced::keyboard::Key,
    modifiers: iced::keyboard::Modifiers,
) -> Option<Message> {
    match key {
        iced::keyboard::Key::Character(character)
            if modifiers.command() && character.eq_ignore_ascii_case("a") =>
        {
            Some(Message::QualcommSelectAll)
        }
        _ => None,
    }
}

fn update(state: &mut State, message: Message) -> Task<Message> {
    // A dialog that is animating out has already queued the close that removes
    // it, and its buttons are blocked: a second close request would only
    // restart the animation.
    if state.pending_close.is_some() && closes_overlay(&message) {
        return Task::none();
    }

    let before = overlay_key(state);
    let login_before = login_step(state);
    let lookup_before = lookup_step(state);

    // The user closing the dialog is held back for the length of the exit
    // transition, so the dialog can be seen to leave; the close itself runs
    // when that transition ends (see `Message::TransitionTick`).
    if before != OVERLAY_NONE && closes_overlay(&message) {
        // Start from wherever the card is now, so closing a card that is still
        // moving (a step change the user interrupted) does not jump.
        let from = state
            .overlay_transition
            .as_ref()
            .map_or_else(anim::dialog_rest, |transition| transition.pose());

        state.pending_close = Some(message);
        state.overlay_transition =
            Some(anim::Transition::leave(from, anim::dialog_exit()));
        return Task::none();
    }

    let task = handle(state, message);

    // The login flow eases from one step to the next (asking for the URL, the
    // browser/WebView wait, the result) instead of swapping the page.
    start_login_transition(state, login_before, login_step(state));

    // The lookup form does the same when the mode above it changes.
    start_lookup_transition(state, lookup_before, lookup_step(state));

    // A dialog that appeared, or whose card changed, eases in. A dialog that
    // closed without the user asking for it just disappears.
    if state.pending_close.is_none() {
        start_overlay_transition(state, before, overlay_key(state));
    }

    task
}

/// Plays the transition that matches a change of the dialog card on screen.
fn start_overlay_transition(state: &mut State, before: u8, after: u8) {
    if after == before {
        return;
    }

    // A card that is still moving is picked up wherever it is. A step that
    // resolves into the next one before the previous change has settled — the
    // bloatware dialog goes device list → checking → checklist as soon as ADB
    // answers — then carries that movement on instead of jumping back to the
    // start of the slide and playing it a second time. The close does the same
    // (see `update`).
    let running = state
        .overlay_transition
        .as_ref()
        .map(|transition| transition.pose());

    state.overlay_transition = match (before, after) {
        (_, OVERLAY_NONE) => None,
        (OVERLAY_NONE, _) => Some(anim::Transition::settle(
            anim::dialog_enter(),
            anim::dialog_rest(),
        )),
        // A card stacked on top of this one is dismissed: the card below was
        // never gone, so it is revealed rather than played in again.
        _ if is_stacked(before) && !is_stacked(after) => None,
        _ => Some(match running {
            Some(pose) => anim::Transition::settle(pose, anim::dialog_rest()),
            None => anim::Transition::settle(
                anim::dialog_switch(switch_forward(before, after)),
                anim::dialog_rest(),
            ),
        }),
    };
}

/// Whether the message is the user closing the dialog, rather than a step
/// inside it. Only these are held back for the exit animation.
fn closes_overlay(message: &Message) -> bool {
    matches!(
        message,
        Message::BootloaderCancel
            | Message::TabletUnlockCancel
            | Message::FactoryResetCancel
            | Message::FirmwareFlashCancel
            | Message::PartitionFlashCancel
            | Message::DriverCancel
            | Message::BloatwareCancel
            | Message::WirelessCancel
            | Message::CaptivePortalCancel
            | Message::UnpackCancel
            | Message::PacCancel
            | Message::PacRepackCancel
            | Message::DecryptCancel
            | Message::MultiFlashPresetClose
            | Message::MultiFlashClosePromptDismissed
            | Message::AboutClosed
            | Message::QualcommLunDialogClosed
            | Message::QualcommClosePromptCancelled
            | Message::FastbootDevicePickerCancelled
    )
}

/// Which way the card moved when one dialog card replaced another. A card of
/// another dialog always comes in from the right; inside one dialog the step
/// decides, except that a card stacked on top counts as one step deeper.
fn switch_forward(before: u8, after: u8) -> bool {
    if is_stacked(after) {
        return true;
    }

    if is_stacked(before) {
        return false;
    }

    before & 0xF0 != after & 0xF0 || after > before
}

/// A card that is drawn on top of another dialog card.
fn is_stacked(key: u8) -> bool {
    key == OVERLAY_BOOTLOADER_PICKER
        || key == OVERLAY_GUIDED_CONFIRM
        || key == OVERLAY_TABLET_PICKER
        || key == OVERLAY_MULTI_FLASH_PRESET_OPTIONS
}

/// No dialog is open.
const OVERLAY_NONE: u8 = 0x00;
/// The Qualcomm Partition Manager's LUN picker; its low nibble is the card it
/// shows (asking the phone, the list, or a question that failed).
const OVERLAY_QUALCOMM_LUNS: u8 = 0x11;
/// The Partition Manager's "close LMN Flash?" prompt, which a window close
/// request opens while a Firehose session is live.
const OVERLAY_QUALCOMM_CLOSE_PROMPT: u8 = 0x14;
/// The dialog cards, keyed by the dialog they belong to and the step they show.
const OVERLAY_BOOTLOADER_CHOOSER: u8 = 0x10;
const OVERLAY_BOOTLOADER_MANUAL: u8 = 0x20;
const OVERLAY_GUIDED: u8 = 0x30;
const OVERLAY_BOOTLOADER_PICKER: u8 = 0x40;
const OVERLAY_GUIDED_CONFIRM: u8 = 0x50;
const OVERLAY_FACTORY_RESET: u8 = 0x60;
/// The tablet unlock chooser, the manual (ZUI) flow, its device picker and the
/// guided (automatic) flow.
const OVERLAY_TABLET_CHOOSER: u8 = 0x70;
const OVERLAY_TABLET_MANUAL: u8 = 0x71;
const OVERLAY_TABLET_PICKER: u8 = 0x72;
const OVERLAY_TABLET_GUIDED_DEVICES: u8 = 0x73;
const OVERLAY_TABLET_GUIDED_WORKING: u8 = 0x74;
const OVERLAY_FIRMWARE_FLASH: u8 = 0x80;
/// The Single Partition Flash dialog. It shares the Firmware Flash nibble —
/// the two are never open at once, and the card is the only one it has.
const OVERLAY_PARTITION_FLASH: u8 = 0x81;
/// The About dialog opened from the Firmware Lookup page.
const OVERLAY_ABOUT: u8 = 0x90;
const OVERLAY_DRIVER: u8 = 0xA0;
/// The "Remove System Bloatware" dialog; its low nibble is the step it shows
/// (device list, package check, checklist, removal).
const OVERLAY_BLOATWARE: u8 = 0xB0;
/// The "Connect ADB Wireless Debugging" dialog; its low nibble is the pairing
/// method it shows. It shares the RETCN device picker's nibble, since the two
/// are never open at once.
const OVERLAY_WIRELESS: u8 = 0xC1;
/// The "Fix Captive Portal Server" dialog; its low nibble is the step it shows
/// (device selection, the chosen device, the running fix with its result).
const OVERLAY_CAPTIVE_PORTAL: u8 = 0xC2;
/// The "Unpack Image" dialog; its low nibble is the step it shows (the setup
/// form, or the running extraction and its result).
const OVERLAY_UNPACK: u8 = 0xD0;
/// The "Unisoc PAC" dialog; its low nibble is the step it shows (the setup
/// form with the file list, or the running extraction and its result). It
/// shares the tablet-decrypt nibble, since the two dialogs are never open at
/// once.
const OVERLAY_PAC: u8 = 0xE1;
/// The "Repack Package" dialog; its low nibble is the step it shows (the setup
/// form with the record list, or the running build and its result).
const OVERLAY_PAC_REPACK: u8 = 0xE3;
/// The "Tablet Firmware Decrypt" dialog.
const OVERLAY_DECRYPT: u8 = 0xE0;
const OVERLAY_RETCN_PICKER: u8 = 0xC0;
/// The Multi Flashing page's modals: its "Preset Manage" dialog, and the
/// prompt that a close request was refused because a run was going on.
const OVERLAY_MULTI_FLASH_PRESET: u8 = 0xF0;
const OVERLAY_MULTI_FLASH_CLOSE_PROMPT: u8 = 0xF1;
/// The flash-options card a picked firmware image is configured with, stacked
/// on top of the "Preset Manage" dialog.
const OVERLAY_MULTI_FLASH_PRESET_OPTIONS: u8 = 0xF2;

/// Identifies the dialog card that is on screen (or [`OVERLAY_NONE`]), so that
/// opening, closing, stepping through and replacing a dialog can be noticed
/// and animated.
///
/// The order mirrors `view`, so the reported card is the one that is actually
/// drawn; a card stacked on top of a dialog (a device picker, the guided
/// request prompt) takes the place of the card below it. A step number must
/// only change when the card really changes: everything that keeps updating
/// while a dialog stays on screen (flashing log lines, progress, a spinner)
/// has to leave its key alone.
fn overlay_key(state: &State) -> u8 {
    // The Partition Manager's own prompts are the last thing on screen; its
    // workspace is only reachable with every other dialog closed.
    if qualcomm::close_prompt_open(state) {
        return OVERLAY_QUALCOMM_CLOSE_PROMPT;
    }

    // The LUN picker belongs to the Partition Manager workspace, which is only
    // reachable with every other dialog closed.
    if let Some(step) = qualcomm::lun_card_step(state) {
        return OVERLAY_QUALCOMM_LUNS + step;
    }

    // The Multi Flashing page's own modals are drawn above everything else:
    // the page can only be reached with every other dialog closed.
    if multi_flash::close_prompt_open(state) {
        return OVERLAY_MULTI_FLASH_CLOSE_PROMPT;
    }

    if multi_flash::preset_options_open(state) {
        return OVERLAY_MULTI_FLASH_PRESET_OPTIONS;
    }

    if multi_flash::preset_open(state) {
        return OVERLAY_MULTI_FLASH_PRESET;
    }

    if bootloader::is_open(state) {
        let bootloader = &state.flash.bootloader;
        if bootloader.picker.is_some() {
            return OVERLAY_BOOTLOADER_PICKER;
        }

        return match bootloader.dialog {
            BootloaderDialog::Closed => OVERLAY_NONE,
            BootloaderDialog::Choosing => OVERLAY_BOOTLOADER_CHOOSER,
            BootloaderDialog::Manual => OVERLAY_BOOTLOADER_MANUAL,
            BootloaderDialog::Guided if bootloader.guided.confirming_request => {
                OVERLAY_GUIDED_CONFIRM
            }
            BootloaderDialog::Guided => OVERLAY_GUIDED + guided_step(bootloader.guided.step),
        };
    }

    if tablet_bootloader::is_open(state) {
        let tablet = &state.flash.tablet_unlock;
        if tablet.picker.is_some() {
            return OVERLAY_TABLET_PICKER;
        }

        return match tablet.dialog {
            TabletUnlockDialog::Closed => OVERLAY_NONE,
            TabletUnlockDialog::Choosing => OVERLAY_TABLET_CHOOSER,
            TabletUnlockDialog::Manual => OVERLAY_TABLET_MANUAL,
            TabletUnlockDialog::Guided => match tablet.guided.step {
                TabletGuidedStep::Devices => OVERLAY_TABLET_GUIDED_DEVICES,
                TabletGuidedStep::Working => OVERLAY_TABLET_GUIDED_WORKING,
            },
        };
    }

    if factory_reset::is_open(state) {
        // The one modal shows the erase running, its result, the requirement
        // check, the confirmation, or the device list: in that order of
        // precedence, numbered the way the flow reaches them.
        let reset = &state.flash.factory_reset;
        let step = if reset.resetting {
            3
        } else if reset.result.is_some() {
            4
        } else if reset.checking {
            1
        } else if reset.check.is_some() {
            2
        } else {
            0
        };

        return OVERLAY_FACTORY_RESET + step;
    }

    if firmware_flash::is_open(state) {
        // The running flash and its result share a card; the procedure
        // checklist, the confirmation, the read-info view and the question
        // before an export each have one.
        let flash = &state.flash.firmware;
        let step = if flash.running || flash.result.is_some() {
            3
        } else if flash.editing {
            1
        } else if flash.confirming {
            2
        } else if flash.info_view {
            4
        } else if flash.export_confirming {
            5
        } else {
            0
        };

        return OVERLAY_FIRMWARE_FLASH + step;
    }

    if driver::is_open(state) {
        return OVERLAY_DRIVER;
    }

    if partition_flash::is_open(state) {
        return OVERLAY_PARTITION_FLASH;
    }

    if bloatware::is_open(state) {
        // The one modal shows the removal, its result, the checklist, the
        // package check, or the device list: in that order of precedence,
        // numbered the way the flow reaches them.
        let bloatware = &state.flash.bloatware;
        let step = if bloatware.removing || bloatware.results.is_some() {
            3
        } else if bloatware.found.is_some() {
            2
        } else if bloatware.checking || bloatware.check_error.is_some() {
            1
        } else {
            0
        };

        return OVERLAY_BLOATWARE + step;
    }

    if wireless_dialog::is_open(state) {
        // The card only changes when the pairing method does; the QR code, the
        // code field and the status update while it stays on screen.
        return OVERLAY_WIRELESS + wireless_step(state.flash.wireless.method);
    }

    if captive_portal_dialog::is_open(state) {
        // The device selection, the chosen device it will work on, and the
        // running fix with its result, in that order of precedence.
        let portal = &state.flash.captive_portal;
        let step = if portal.fixing || portal.result.is_some() {
            2
        } else if portal.selected.is_some() {
            1
        } else {
            0
        };

        return OVERLAY_CAPTIVE_PORTAL + step;
    }

    if unpack::is_open(state) {
        // The setup form, and the running extraction with its result.
        let unpack = &state.unpack;
        let step = if unpack.working || unpack.result.is_some() {
            1
        } else {
            0
        };

        return OVERLAY_UNPACK + step;
    }

    if decrypt::is_open(state) {
        return OVERLAY_DECRYPT;
    }

    if pac_unpack::is_open(state) {
        // The setup form with the file list, and the running extraction with
        // its result.
        let pac = &state.pac;
        let step = if pac.working || pac.result.is_some() {
            1
        } else {
            0
        };

        return OVERLAY_PAC + step;
    }

    if pac_repack::is_open(state) {
        // The setup form with the record list, and the running build with its
        // result.
        let repack = &state.pac_repack;
        let step = if repack.working || repack.result.is_some() {
            1
        } else {
            0
        };

        return OVERLAY_PAC_REPACK + step;
    }

    if state.about_open {
        return OVERLAY_ABOUT;
    }

    if matches!(state.lookup.retcn.device_picker, DevicePicker::Open(_)) {
        return OVERLAY_RETCN_PICKER;
    }

    OVERLAY_NONE
}

/// The card the guided wizard shows for a step, as a step number.
fn guided_step(step: GuidedStep) -> u8 {
    match step {
        GuidedStep::Login => 0,
        GuidedStep::Read => 1,
        GuidedStep::Request => 2,
        GuidedStep::Key => 3,
    }
}

/// The card the wireless dialog shows for a pairing method, as a step number.
fn wireless_step(method: WirelessMethod) -> u8 {
    match method {
        WirelessMethod::Qr => 0,
        WirelessMethod::Code => 1,
    }
}

/// The index of a tab, used to tell which way the user is moving through the
/// feature tabs (and therefore which way the content should slide in).
fn mode_index(mode: Mode) -> usize {
    Mode::ALL.iter().position(|&tab| tab == mode).unwrap_or(0)
}

/// The login page currently shown, as a step number, used to tell which way
/// the user is moving through the flow (and therefore which way the page
/// should slide in).
///
/// The numbers follow the flow, so a later step means "forward" (the page
/// comes in from the right): making the request, the WebView or browser wait,
/// the manual fallback — and the two outcomes, the login and the error, last.
fn login_step(state: &State) -> u8 {
    match state.login {
        LoginStatus::LoggedOut => 0,
        LoginStatus::Fetching { .. } => 1,
        LoginStatus::WebViewOpen { .. } => 2,
        LoginStatus::WaitingForBrowser { .. } => 3,
        LoginStatus::Manual { .. } => 4,
        LoginStatus::LoggedIn { .. } => 5,
        LoginStatus::Error(_) => 6,
    }
}

/// Plays the transition that matches a change of the login page on screen.
fn start_login_transition(state: &mut State, before: u8, after: u8) {
    if after == before {
        return;
    }

    // Like the dialog cards, a step that resolves before the previous change
    // settled carries that movement on instead of jumping back to the start of
    // the slide (the login URL answers well before the 200 ms are up).
    let running = state
        .login_transition
        .as_ref()
        .map(|transition| transition.pose());
    let from = running.unwrap_or_else(|| anim::content_start(after > before));

    state.login_transition = Some(anim::Transition::settle(from, anim::content_rest()));
}

/// The lookup form currently shown, as a step number, used to tell which way
/// the user is moving through it (and therefore which way the form should
/// slide in).
///
/// Both what the selector picks (the mode) and what the request did (the
/// status below the form) count, so switching the mode, a request starting,
/// and its result each ease the form the way the login steps do. The mode is
/// the higher digit, so the numbers still follow [`LookupMode::ALL`], and a
/// later mode or a later status means "forward" (the form comes in from the
/// right).
fn lookup_step(state: &State) -> u8 {
    let mode = LookupMode::ALL
        .iter()
        .position(|&mode| mode == state.lookup.mode)
        .unwrap_or(0) as u8;

    let status = match state.lookup.status {
        LookupStatus::Idle => 0,
        LookupStatus::Fetching => 1,
        // A result and an error are both the end of the request, so they take
        // the form to the same (last) step.
        LookupStatus::Done(_) | LookupStatus::Error(_) => 2,
    };

    mode * 4 + status
}

/// Plays the transition that matches a change of the lookup mode or status.
///
/// Only the form under the selector moves: the user just picked from that
/// selector, so it stays where it is while the form eases in behind it.
fn start_lookup_transition(state: &mut State, before: u8, after: u8) {
    if after == before {
        return;
    }

    // As with the login steps, a change that comes before the previous one
    // settled carries that movement on instead of jumping back to the start of
    // the slide.
    let running = state
        .lookup_transition
        .as_ref()
        .map(|transition| transition.pose());
    let from = running.unwrap_or_else(|| anim::content_start(after > before));

    state.lookup_transition = Some(anim::Transition::settle(from, anim::content_rest()));
}

fn handle(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::ModeSelected(mode) => {
            if mode != state.mode {
                state.content_transition = Some(anim::Transition::settle(
                    anim::content_start(mode_index(mode) >= mode_index(state.mode)),
                    anim::content_rest(),
                ));
            }

            // A tab move animates the content only: the tabs stay put.
            state.content_transition_covers_chrome = false;

            state.mode = mode;
            Task::none()
        }
        Message::AnimTick => {
            state.anim_tick = state.anim_tick.wrapping_add(1);
            Task::none()
        }
        Message::SystemTheme(mode) => {
            state.dark = mode == iced::theme::Mode::Dark;
            Task::none()
        }
        Message::TransitionTick => {
            anim::tick(&mut state.content_transition);
            anim::tick(&mut state.overlay_transition);
            anim::tick(&mut state.login_transition);
            anim::tick(&mut state.lookup_transition);

            // A finished transition is followed by ordinary ones again
            // (the Multi Flashing page is the only full-window one so far).
            if state.content_transition.is_none() {
                state.content_transition_covers_chrome = false;
            }

            // A dialog that has finished leaving now runs the close that was
            // held back for it.
            if state.overlay_transition.is_none() {
                if let Some(close) = state.pending_close.take() {
                    let task = handle(state, close);

                    // Either the close was refused (a busy dialog ignores it)
                    // or another dialog took this one's place: ease a card
                    // back in rather than leaving one half gone. A dialog that
                    // really closed just disappears.
                    if overlay_key(state) != OVERLAY_NONE {
                        state.overlay_transition = Some(anim::Transition::settle(
                            anim::dialog_enter(),
                            anim::dialog_rest(),
                        ));
                    }

                    return task;
                }
            }

            Task::none()
        }
        Message::OverlayLeavingPressed => Task::none(),
        Message::AboutPressed => {
            state.about_open = true;
            Task::none()
        }
        Message::AboutClosed => {
            state.about_open = false;
            Task::none()
        }
        Message::AboutOpenUrl(url) => open_browser(&url),
        Message::SmartphoneFeaturePressed(feature) => match feature {
            SmartphoneFeature::BootloaderUnlock => {
                let bootloader = &mut state.flash.bootloader;
                bootloader.dialog = BootloaderDialog::Choosing;
                bootloader.picker = None;
                bootloader.pending_key = None;
                bootloader.reading = false;
                bootloader.unlocking = false;
                bootloader.checking = false;
                bootloader.eligible = None;
                bootloader.read_error = None;
                bootloader.read_info = None;
                bootloader.unlock_error = None;
                bootloader.unlock_notice = None;
                bootloader.unlock_info = None;
                bootloader.guided = GuidedState::default();
                Task::none()
            }
            SmartphoneFeature::FactoryReset => start_factory_reset_dialog(state),
            SmartphoneFeature::FirmwareFlash => start_firmware_flash_dialog(state),
            SmartphoneFeature::PartitionFlash => start_partition_flash_dialog(state),
            SmartphoneFeature::MultiFlashing => start_multi_flashing_page(state),
            SmartphoneFeature::PartitionManager => start_qualcomm_page(state),
            SmartphoneFeature::InstallDriver => start_driver_dialog(state),
            SmartphoneFeature::RemoveBloatware => start_bloatware_dialog(state),
            SmartphoneFeature::WirelessDebugging => start_wireless_dialog(state),
            SmartphoneFeature::FixCaptivePortal => start_captive_portal_dialog(state),
        },
        Message::DecryptFeaturePressed(feature) => match feature {
            decrypt_mode::DecryptFeature::TabletDecrypt => start_decrypt_dialog(state),
            decrypt_mode::DecryptFeature::UnpackImage => start_unpack_dialog(state),
            decrypt_mode::DecryptFeature::UnisocPac => start_pac_dialog(state),
            decrypt_mode::DecryptFeature::UnisocPacRepack => start_pac_repack_dialog(state),
        },
        Message::DecryptCancel => {
            // Never close the dialog while a batch is running: its result would
            // have nowhere to go.
            if matches!(
                state.decrypt.status,
                DecryptStatus::PickingDir | DecryptStatus::Working
            ) {
                return Task::none();
            }

            state.decrypt = DecryptState::default();
            Task::none()
        }
        Message::DecryptBackdropPressed => Task::none(),
        Message::FactoryResetDevicesFetched(result) => {
            let reset = &mut state.flash.factory_reset;
            reset.listing = false;

            match result {
                Ok(list) => {
                    reset.total = list.total;
                    reset.devices = list.devices;
                    reset.list_error = None;
                }
                Err(error) => {
                    reset.total = 0;
                    reset.devices.clear();
                    reset.list_error = Some(error);
                }
            }

            Task::none()
        }
        Message::FactoryResetDeviceSelected(serial) => {
            let reset = &mut state.flash.factory_reset;
            reset.selected = Some(serial.clone());
            reset.checking = true;
            reset.check = None;
            reset.result = None;

            start_factory_reset_check(serial)
        }
        Message::FactoryResetCheckFinished(result) => {
            let reset = &mut state.flash.factory_reset;
            if !reset.checking {
                return Task::none();
            }

            reset.checking = false;
            reset.check = Some(match result {
                Ok(check) => match check.blocker() {
                    Some(blocker) => ResetCheckOutcome::Blocked(blocker),
                    None => ResetCheckOutcome::Allowed {
                        frp_protected: check.frp_protected(),
                    },
                },
                Err(error) => ResetCheckOutcome::Failed(error),
            });

            Task::none()
        }
        Message::FactoryResetConfirmed => {
            let reset = &mut state.flash.factory_reset;
            let Some(serial) = reset.selected.clone() else {
                return Task::none();
            };

            // Only erase once the requirement checks passed and nothing else
            // is running.
            if reset.resetting
                || !matches!(reset.check, Some(ResetCheckOutcome::Allowed { .. }))
            {
                return Task::none();
            }

            reset.resetting = true;
            reset.result = None;

            start_factory_reset(serial)
        }
        Message::FactoryResetFinished(result) => {
            let reset = &mut state.flash.factory_reset;
            if !reset.resetting {
                return Task::none();
            }

            reset.resetting = false;
            reset.result = Some(result);

            Task::none()
        }
        Message::FactoryResetRescan => {
            let reset = &mut state.flash.factory_reset;
            if reset.listing || reset.checking || reset.resetting {
                return Task::none();
            }

            reset.listing = true;
            reset.devices.clear();
            reset.total = 0;
            reset.list_error = None;
            reset.selected = None;
            reset.check = None;
            reset.result = None;

            start_factory_reset_list_devices()
        }
        Message::FactoryResetBack => {
            let reset = &mut state.flash.factory_reset;
            if reset.listing || reset.checking || reset.resetting {
                return Task::none();
            }

            reset.selected = None;
            reset.check = None;
            reset.result = None;

            Task::none()
        }
        Message::FactoryResetCancel => {
            let reset = &mut state.flash.factory_reset;
            // Never close the dialog while an operation is in flight, so the
            // device is not left half-erased.
            if reset.listing || reset.checking || reset.resetting {
                return Task::none();
            }

            *reset = FactoryResetState::default();

            Task::none()
        }
        // Clicks on empty modal space are swallowed here so they neither
        // dismiss the dialog nor reach the UI underneath.
        Message::FactoryResetBackdropPressed => Task::none(),
        Message::FirmwareFlashPickRequested => {
            let flash = &mut state.flash.firmware;
            if flash.loading || flash.running {
                return Task::none();
            }

            let title = state.l10n.tr("firmware-flash-select");

            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        rfd::FileDialog::new()
                            .set_title(&title)
                            .add_filter("Firmware package", &FIRMWARE_PACKAGE_EXTENSIONS)
                            .pick_file()
                    })
                    .await
                    .unwrap_or(None)
                },
                Message::FirmwareFlashPicked,
            )
        }
        Message::FirmwareFlashPicked(path) => {
            let Some(path) = path else {
                return Task::none();
            };

            if state.flash.firmware.loading || state.flash.firmware.running {
                return Task::none();
            }

            state.flash.firmware.package = Some(path.clone());
            start_firmware_flash_load(state, path)
        }
        Message::FirmwareFlashEngineSelected(engine) => {
            let flash = &mut state.flash.firmware;
            flash.engine = engine;
            flash.tools_error = None;
            flash.tools_ready = false;
            flash.tool_version = None;

            // Google's platform-tools are fetched the first time they are
            // picked and used as they are afterwards.
            if flash.engine.is_platform_tools()
                && !flash.installing_tools
                && engine_tool_missing(flash)
            {
                return start_platform_tools_install(state);
            }

            // The versions differ per tool, so the new one is asked too.
            start_tool_version_read(state)
        }
        Message::FirmwareFlashToolsUpdate => {
            // Only the downloaded platform-tools can be updated, and not while
            // a download is already running.
            if !state.flash.firmware.engine.is_platform_tools()
                || state.flash.firmware.installing_tools
            {
                return Task::none();
            }

            start_platform_tools_install(state)
        }
        Message::FirmwareFlashOpenTerminal => start_open_terminal(state),
        Message::FirmwareFlashTerminalChecked(minimal_adb) => {
            let flash = &mut state.flash.firmware;

            // A minimal ADB install owns the `adb`/`fastboot` names on the
            // `PATH`; the terminal would hand the user those instead of the
            // tools here, so it is not opened at all.
            if minimal_adb {
                flash.terminal_busy = false;
                flash.terminal_error = Some(TerminalError::MinimalAdb);
                return Task::none();
            }

            flash.terminal_busy = false;

            // The tools have to be on disk before there is anything to open a
            // terminal for; the download reports back through
            // `FirmwareFlashToolsInstalled`.
            if platform_tools_missing(flash) {
                flash.terminal_after_install = true;
                return start_platform_tools_install(state);
            }

            open_platform_tools_terminal(state)
        }
        Message::FirmwareFlashTerminalOpened(result) => {
            let flash = &mut state.flash.firmware;
            flash.terminal_busy = false;

            match result {
                Ok(path) => {
                    flash.terminal_error = None;
                    flash.terminal_path = Some(path);
                }
                Err(error) => {
                    flash.terminal_path = None;
                    flash.terminal_error = Some(TerminalError::Other(error));
                }
            }

            Task::none()
        }
        Message::FirmwareFlashToolVersionRead(path, result) => {
            let flash = &mut state.flash.firmware;

            // A late answer about a tool that is not selected any more is
            // dropped, like the device variables' serial guard.
            if flash.engine.tool().map(|tool| tool.path.as_path()) != Some(path.as_path()) {
                return Task::none();
            }

            flash.reading_version = false;
            flash.tool_version = Some(result);
            Task::none()
        }
        Message::FirmwareFlashToolsInstalled(result) => {
            let flash = &mut state.flash.firmware;
            flash.installing_tools = false;
            // A download the terminal button started opens the terminal once
            // it worked; one the engine started only refreshes the dialog.
            let for_terminal = std::mem::take(&mut flash.terminal_after_install);

            match result {
                Ok(_path) => {
                    flash.tools_error = None;
                    flash.tools_ready = true;

                    #[cfg(debug_assertions)]
                    eprintln!(
                        "[firmware-flash] platform-tools installed: {}",
                        _path.display()
                    );

                    if for_terminal {
                        return open_platform_tools_terminal(state);
                    }

                    // What the fresh download reports is what the dialog
                    // shows as the version from now on.
                    return start_tool_version_read(state);
                }
                Err(error) => {
                    if for_terminal {
                        flash.terminal_error = Some(TerminalError::Other(error.clone()));
                    }

                    flash.tools_error = Some(error);
                }
            }

            Task::none()
        }
        Message::FirmwareFlashVerifyToggled(verify_checksums) => {
            state.flash.firmware.verify_checksums = verify_checksums;
            Task::none()
        }
        Message::FirmwareFlashRescan => {
            let flash = &mut state.flash.firmware;
            if flash.listing || flash.loading || flash.running {
                return Task::none();
            }

            flash.listing = true;
            flash.devices.clear();
            flash.total = 0;
            flash.list_error = None;
            flash.selected = None;
            flash.reading_device = false;
            flash.device_vars = None;
            flash.device_vars_error = None;

            start_firmware_flash_list_devices()
        }
        Message::FirmwareFlashDevicesFetched(result) => {
            let flash = &mut state.flash.firmware;
            flash.listing = false;

            match result {
                Ok(list) => {
                    flash.total = list.total;
                    flash.devices = list.devices;
                    flash.list_error = None;
                }
                Err(error) => {
                    flash.total = 0;
                    flash.devices.clear();
                    flash.list_error = Some(error);
                }
            }

            // A single phone is what the dialog would flash anyway, so it is
            // selected right away (and its variables are read).
            let only = match flash.devices.as_slice() {
                [device] => Some(device.serial.clone()),
                _ => None,
            };

            match only {
                Some(serial) => select_firmware_flash_device(state, serial),
                None => Task::none(),
            }
        }
        Message::FirmwareFlashDeviceSelected(serial) => {
            select_firmware_flash_device(state, serial)
        }
        Message::FirmwareFlashDeviceVarsRead(serial, result) => {
            let flash = &mut state.flash.firmware;
            // Ignore an answer for a device that is no longer selected.
            if flash.selected.as_deref() != Some(serial.as_str()) {
                return Task::none();
            }

            flash.reading_device = false;

            match result {
                Ok(variables) => {
                    flash.device_vars = Some(variables);
                    flash.device_vars_error = None;
                }
                Err(error) => {
                    flash.device_vars = None;
                    flash.device_vars_error = Some(error);
                }
            }

            Task::none()
        }
        Message::FirmwareFlashStart => {
            let flash = &mut state.flash.firmware;
            if flash.running
                || flash.loading
                || flash.plan.is_none()
                || flash.selected.is_none()
                || enabled_procedures(flash) == 0
            {
                return Task::none();
            }

            flash.editing = false;
            flash.confirming = true;
            // Flashing a package that belongs to another phone is expected to
            // brick it, so that case gets its own warning and a countdown
            // before it can be confirmed.
            flash.project_mismatch = project_mismatch(flash);
            flash.confirm_countdown = if flash.project_mismatch {
                MISMATCH_COUNTDOWN_SECONDS
            } else {
                0
            };
            Task::none()
        }
        Message::FirmwareFlashCountdownTick => {
            let flash = &mut state.flash.firmware;

            // Only the warning counts down, and only while it is on screen.
            if flash.confirming && flash.confirm_countdown > 0 {
                flash.confirm_countdown -= 1;
            }

            Task::none()
        }
        Message::FirmwareFlashConfirmed => {
            let flash = &state.flash.firmware;

            // The button is disabled while the warning counts down; ignoring
            // the message as well means no click can skip the wait.
            if flash.confirm_countdown > 0 {
                return Task::none();
            }

            start_firmware_flash(state)
        }
        Message::FirmwareFlashEditProcedures => {
            let flash = &mut state.flash.firmware;
            if flash.loading || flash.running || flash.plan.is_none() {
                return Task::none();
            }

            flash.editing = true;
            flash.confirming = false;
            flash.confirm_countdown = 0;
            Task::none()
        }
        Message::FirmwareFlashProcedureToggled(index, enabled) => {
            let flash = &mut state.flash.firmware;
            if flash.loading || flash.running {
                return Task::none();
            }

            if let Some(flag) = flash.enabled.get_mut(index) {
                *flag = enabled;
            }

            Task::none()
        }
        Message::FirmwareFlashSelectAllProcedures(enabled) => {
            let flash = &mut state.flash.firmware;
            if flash.loading || flash.running {
                return Task::none();
            }

            for flag in &mut flash.enabled {
                *flag = enabled;
            }

            Task::none()
        }
        Message::FirmwareFlashSelectPart(part) => {
            let flash = &mut state.flash.firmware;
            if flash.loading || flash.running {
                return Task::none();
            }

            // Selecting a part ADDS its procedures to the selection; nothing
            // is unchecked, so the parts can be combined. `erase`/`oem`
            // commands belong to no part and are left as they are.
            let matched: Vec<usize> = {
                let Some(package) = &flash.plan else {
                    return Task::none();
                };

                // On a MediaTek package `dtbo` belongs to the bootloader.
                let mediatek = package.is_mediatek();

                package
                    .steps
                    .iter()
                    .enumerate()
                    .filter(|(_, step)| step.part(mediatek) == Some(part))
                    .map(|(index, _)| index)
                    .collect()
            };

            for index in matched {
                if let Some(enabled) = flash.enabled.get_mut(index) {
                    *enabled = true;
                }
            }

            Task::none()
        }
        Message::FirmwareFlashEditDone => {
            state.flash.firmware.editing = false;
            Task::none()
        }
        Message::FirmwareFlashEraseGroupToggled(group) => {
            let flash = &mut state.flash.firmware;
            if flash.loading || flash.running {
                return Task::none();
            }

            // Unlike the part buttons these are standalone commands, so the
            // button toggles: pressing it again takes the group back out.
            let Some(index) = flash
                .erase_groups
                .iter()
                .position(|active| *active == group)
            else {
                flash.erase_groups.push(group);
                check_erase_rows(flash, group, true);
                return Task::none();
            };

            flash.erase_groups.remove(index);
            check_erase_rows(flash, group, false);

            Task::none()
        }
        Message::FirmwareFlashBack => {
            let flash = &mut state.flash.firmware;
            if flash.running || flash.loading || flash.rebooting {
                return Task::none();
            }

            // Returning from the warning or from the result keeps the loaded
            // package and the selected device, so a retry does not have to
            // unpack the firmware all over again.
            flash.confirming = false;
            flash.project_mismatch = false;
            flash.confirm_countdown = 0;
            flash.result = None;
            flash.reboot_result = None;
            flash.export_confirming = false;
            flash.export_result = None;
            flash.progress = None;
            flash.step_index = 0;
            flash.step_label.clear();

            Task::none()
        }
        Message::FirmwareFlashCancel => {
            let flash = &mut state.flash.firmware;
            // Never close the dialog while a job is in flight: the phone must
            // not be left half-flashed, and the unpacked package is still
            // being read.
            if flash.loading || flash.running || flash.rebooting {
                return Task::none();
            }

            let job = flash.job + 1;
            *flash = FirmwareFlashState {
                job,
                ..FirmwareFlashState::default()
            };

            cleanup_firmware_flash_package()
        }
        Message::MultiFlashingSelected => {
            // Never step into the workspace while a job is in flight: the phone
            // must not be left half-flashed. The tile that sends this sits
            // behind the Firmware Flash dialog, so nothing was ever flashed
            // from the grid's point of view — but a stray event must not skip
            // the check either.
            let flash = &state.flash.firmware;

            if flash.loading || flash.running || flash.rebooting {
                return Task::none();
            }

            start_multi_flashing_page(state)
        }
        Message::MultiFlashExit => {
            // A run has to be stopped first; the button is locked while it
            // goes on, but a stray event must not leave one behind either. A
            // device that is still being flashed counts the same way: its log
            // is still being written.
            if state.flash.multi_flash.running || !state.flash.multi_flash.flashing.is_empty() {
                return Task::none();
            }

            // The table belongs to the run that was just left; the firmware
            // images do not (they are what the page is set up with).
            state.flash.multi_flash.devices.clear();
            state.flash.multi_flash.log_directory = None;
            state.flash.multi_flash.run_error = None;

            state.flash.multi_flash.open = false;
            // Leaving plays the same slide the other way round, so the
            // feature grid comes back the way the page went in.
            state.content_transition = Some(anim::Transition::settle(
                anim::content_start(false),
                anim::content_rest(),
            ));
            // The chrome comes back with the grid, fading in over the same
            // pose as the content around it.
            state.content_transition_covers_chrome = true;
            Task::none()
        }
        Message::QualcommManagerSelected => start_qualcomm_page(state),
        Message::QualcommExit => {
            // A loaded programmer is a live Firehose session running inside the
            // phone: leaving the page would leave it there with nothing to talk
            // to, so the phone has to be reset — or the window closed — first.
            if state.flash.qualcomm.busy || state.flash.qualcomm.programmer_uploaded {
                return Task::none();
            }

            state.flash.qualcomm.open = false;
            // Leaving plays the same slide the other way round, so the
            // feature grid comes back the way the page went in.
            state.content_transition = Some(anim::Transition::settle(
                anim::content_start(false),
                anim::content_rest(),
            ));
            // The chrome comes back with the grid, fading in over the same
            // pose as the content around it.
            state.content_transition_covers_chrome = true;
            Task::none()
        }
        Message::QualcommFilterChanged(value) => {
            state.flash.qualcomm.filter = value;
            Task::none()
        }
        Message::QualcommPortSelected(port) => {
            state.flash.qualcomm.port = Some(port);
            // Another phone (or another port of the same one) says nothing
            // about the table that was read from the previous one, and the
            // programmer does not run on it either — so "Inject Firehose" is
            // offered again.
            state.flash.qualcomm.partitions.clear();
            state.flash.qualcomm.selected.clear();
            state.flash.qualcomm.anchor = None;
            state.flash.qualcomm.progress = None;
            state.flash.qualcomm.detected = None;
            state.flash.qualcomm.status = QualcommStatus::Idle;
            state.flash.qualcomm.programmer_uploaded = false;
            Task::none()
        }
        Message::QualcommPartitionClicked(index) => {
            if index >= state.flash.qualcomm.partitions.len() || state.flash.qualcomm.busy {
                return Task::none();
            }

            let visible = qualcomm_visible_indices(&state.flash.qualcomm);
            let modifiers = state.flash.qualcomm.modifiers;
            select_qualcomm_row(&mut state.flash.qualcomm, &visible, index, modifiers);

            Task::none()
        }
        Message::QualcommModifiersChanged(modifiers) => {
            state.flash.qualcomm.modifiers = modifiers;
            Task::none()
        }
        Message::QualcommSelectAll => {
            if state.flash.qualcomm.busy || state.flash.qualcomm.partitions.is_empty() {
                return Task::none();
            }

            // Every row the table shows — the search box narrows what "all"
            // means, the way a file list does.
            let visible = qualcomm_visible_indices(&state.flash.qualcomm);
            let qualcomm = &mut state.flash.qualcomm;
            qualcomm.selected.fill(false);
            for index in &visible {
                if let Some(row) = qualcomm.selected.get_mut(*index) {
                    *row = true;
                }
            }
            qualcomm.anchor = visible.first().copied();

            Task::none()
        }
        Message::QualcommStorageTypeSelected(storage) => {
            state.flash.qualcomm.storage = storage;
            // The table was read from the storage the phone reported, which
            // the pick does not change.
            Task::none()
        }
        Message::QualcommPortsPolled => start_qualcomm_port_scan(),
        Message::QualcommPortsScanned(ports) => {
            let qualcomm = &mut state.flash.qualcomm;

            // A port that went away cannot stay selected: the dropdown would
            // name a phone that is no longer there. The programmer went with
            // it, so the next session has to upload it again.
            if let Some(selected) = &qualcomm.port {
                if !ports.iter().any(|port| port == selected) {
                    qualcomm.port = None;
                    qualcomm.programmer_uploaded = false;
                }
            }

            // The first port a scan finds is selected on its own, so plugging
            // the phone in is all it takes; a port that is already selected is
            // left alone (the table read from it stays).
            if qualcomm.port.is_none() && !qualcomm.busy {
                if let Some(first) = ports.first() {
                    qualcomm.port = Some(first.clone());
                    // Nothing has been read from a port that was just found.
                    qualcomm.partitions.clear();
                    qualcomm.selected.clear();
                    qualcomm.anchor = None;
                    qualcomm.progress = None;
                    qualcomm.detected = None;
                    qualcomm.status = QualcommStatus::Idle;
                }
            }

            qualcomm.ports = ports;
            Task::none()
        }
        Message::QualcommOpenDeviceManager => {
            if let Err(error) = qualcomm_ports::open_device_manager() {
                eprintln!("[qualcomm] could not open Device Manager: {error}");
            }

            Task::none()
        }
        Message::QualcommLocateLog => {
            if let Some(path) = state.flash.qualcomm.log_path.clone() {
                if let Err(error) = flash_log::reveal(&path) {
                    eprintln!("[qualcomm] could not open the log: {error}");
                }
            }

            Task::none()
        }
        // Nothing of the danger zone is implemented yet: the pick is ignored
        // and the control keeps showing its placeholder, the way the page's
        // greyed-out buttons do.
        Message::QualcommDangerZoneSelected => Task::none(),
        Message::QualcommInjectFirehose => start_qualcomm_injection(state),
        Message::QualcommProgrammerPicked(path) => {
            let Some(path) = path else {
                // The picker was cancelled; nothing was touched.
                return Task::none();
            };
            let Some(port) = state.flash.qualcomm.port.clone() else {
                state.flash.qualcomm.status =
                    QualcommStatus::InjectFailed(state.l10n.tr("qc-select-port"));
                return Task::none();
            };

            let storage = state.flash.qualcomm.storage;
            // Every session gets its own file, named after the port and the
            // moment it started: an injection that fails is exactly what the
            // log is for, so the file is created before anything is sent.
            let log = flash_log::FlashLog::create(flash_log::partition_manager_path(&port));
            state.flash.qualcomm.log_path = Some(log.path().to_path_buf());
            state.flash.qualcomm.busy = true;
            state.flash.qualcomm.status = QualcommStatus::Injecting;

            // The worker reports when the upload is over, so the status line
            // can say "Reading Partition Table…" for the second half of the
            // job instead of claiming an upload that has finished.
            let (progress, events) = iced::futures::channel::mpsc::unbounded();

            Task::batch([
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            let mut log = log;
                            let reading = move || {
                                let _ = progress.unbounded_send(Message::QualcommFirehoseReading);
                            };

                            firehose::inject(&port, &path, storage, &mut log, &reading)
                        })
                        .await
                        .unwrap_or_else(|error| {
                            Err(format!("the Firehose worker stopped: {error}"))
                        })
                    },
                    Message::QualcommFirehoseReady,
                ),
                Task::run(events, |message| message),
            ])
        }
        Message::QualcommFirehoseReading => {
            // A report from a job that already ended is dropped, like every
            // other late answer.
            if state.flash.qualcomm.busy {
                state.flash.qualcomm.status = QualcommStatus::Reading;
                // The programmer is running now: uploading it again would be
                // a no-op at best, so the button waits for another port.
                state.flash.qualcomm.programmer_uploaded = true;
            }

            Task::none()
        }
        Message::QualcommFirehoseReady(result) => {
            let qualcomm = &mut state.flash.qualcomm;
            qualcomm.busy = false;

            match result {
                Ok(table) => apply_qualcomm_table(qualcomm, table),
                Err(error) => {
                    qualcomm.partitions.clear();
                    qualcomm.selected.clear();
                    qualcomm.anchor = None;
                    qualcomm.detected = None;
                    qualcomm.status = QualcommStatus::InjectFailed(error);
                }
            }

            Task::none()
        }
        Message::QualcommRebootModeSelected(mode) => {
            if state.flash.qualcomm.busy {
                return Task::none();
            }
            let Some(port) = state.flash.qualcomm.port.clone() else {
                state.flash.qualcomm.status =
                    QualcommStatus::RebootFailed(state.l10n.tr("qc-select-port"));
                return Task::none();
            };

            // The modes act on the table the injection read: the partitions
            // they clear and the `misc` block they write are named in it.
            let partitions = state.flash.qualcomm.partitions.clone();
            let action = mode.action();

            state.flash.qualcomm.busy = true;
            state.flash.qualcomm.status = QualcommStatus::Rebooting;
            state.flash.qualcomm.progress = Some(firehose::Progress::default());

            // A reboot is a Firehose command like any other, so it gets its own
            // logged session: the phone may have been left in a strange state
            // by whatever ran before it.
            let log = flash_log::FlashLog::create(flash_log::partition_manager_path(&port));
            state.flash.qualcomm.log_path = Some(log.path().to_path_buf());

            let (progress, events) = iced::futures::channel::mpsc::unbounded();

            Task::batch([
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            let mut log = log;
                            let report = move |step: firehose::Progress| {
                                let _ = progress
                                    .unbounded_send(Message::QualcommRebootProgress(step));
                            };

                            firehose::reboot(&port, &partitions, action, &mut log, &report)
                        })
                        .await
                        .unwrap_or_else(|error| {
                            Err(format!("the Firehose worker stopped: {error}"))
                        })
                    },
                    Message::QualcommResetFinished,
                ),
                Task::run(events, |message| message),
            ])
        }
        Message::QualcommRebootProgress(progress) => {
            if state.flash.qualcomm.busy {
                state.flash.qualcomm.progress = Some(progress);
            }

            Task::none()
        }
        Message::QualcommResetFinished(result) => {
            let qualcomm = &mut state.flash.qualcomm;
            qualcomm.busy = false;
            // The phone leaves Firehose mode as it resets, so the next session
            // has to upload the programmer again — and the table it answered
            // with belonged to the session that just ended.
            qualcomm.programmer_uploaded = false;
            qualcomm.partitions.clear();
            qualcomm.selected.clear();
            qualcomm.anchor = None;
            qualcomm.progress = None;
            qualcomm.detected = None;
            qualcomm.status = match result {
                Ok(()) => QualcommStatus::Rebooted,
                Err(error) => QualcommStatus::RebootFailed(error),
            };

            Task::none()
        }
        Message::QualcommDump => {
            if state.flash.qualcomm.busy {
                return Task::none();
            }

            let regions = qualcomm_selected_regions(state);
            if regions.is_empty() {
                state.flash.qualcomm.status =
                    QualcommStatus::DumpFailed(state.l10n.tr("qc-select-partition"));
                return Task::none();
            }

            // One selected row is one file, so the save dialog lets it be
            // named and placed where the user wants it. Several rows need a
            // folder that holds one file each.
            if let [region] = regions.as_slice() {
                let title = state.l10n.tr("qc-dump-file-title");
                let filter = state.l10n.tr("qc-image-filter");
                let name = format!("{}.img", flash_log::file_name_part(&region.name));

                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            rfd::FileDialog::new()
                                .set_title(&title)
                                .set_file_name(&name)
                                .add_filter(&filter, &["img"])
                                .save_file()
                        })
                        .await
                        .unwrap_or(None)
                    },
                    Message::QualcommDumpFilePicked,
                );
            }

            let title = state.l10n.tr("qc-dump-folder-title");

            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        rfd::FileDialog::new().set_title(&title).pick_folder()
                    })
                    .await
                    .unwrap_or(None)
                },
                Message::QualcommDumpFolderPicked,
            )
        }
        Message::QualcommDumpFolderPicked(directory) => match directory {
            Some(directory) => start_qualcomm_dump(state, firehose::DumpTarget::Folder(directory)),
            // The picker was cancelled; nothing was touched.
            None => Task::none(),
        },
        Message::QualcommDumpFilePicked(file) => match file {
            Some(file) => start_qualcomm_dump(state, firehose::DumpTarget::File(file)),
            None => Task::none(),
        },
        Message::QualcommDumpProgress(progress) => {
            if state.flash.qualcomm.busy {
                state.flash.qualcomm.status = QualcommStatus::Dumping {
                    name: progress.name.clone(),
                    done: progress.done,
                    total: progress.total,
                };
                state.flash.qualcomm.progress = Some(progress);
            }

            Task::none()
        }
        Message::QualcommDumpFinished(result) => {
            let qualcomm = &mut state.flash.qualcomm;
            qualcomm.busy = false;
            qualcomm.progress = None;
            qualcomm.status = match result {
                Ok((count, directory)) => QualcommStatus::DumpDone { count, directory },
                Err(error) => QualcommStatus::DumpFailed(error),
            };

            Task::none()
        }
        Message::QualcommDumpStorage => {
            if state.flash.qualcomm.busy || state.flash.qualcomm.partitions.is_empty() {
                return Task::none();
            }
            let Some(port) = state.flash.qualcomm.port.clone() else {
                state.flash.qualcomm.status =
                    QualcommStatus::StorageDumpFailed(state.l10n.tr("qc-select-port"));
                return Task::none();
            };

            let storage = state
                .flash
                .qualcomm
                .detected
                .unwrap_or(state.flash.qualcomm.storage);

            // A storage that is one physical partition needs no picker: the
            // save dialog is the only question to ask.
            if storage != StorageType::Ufs {
                let title = state.l10n.tr("qc-dump-storage-file-title");
                let filter = state.l10n.tr("qc-image-filter");
                let name = format!("{}.img", storage_stem(storage));

                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            rfd::FileDialog::new()
                                .set_title(&title)
                                .set_file_name(&name)
                                .add_filter(&filter, &["img"])
                                .save_file()
                        })
                        .await
                        .unwrap_or(None)
                    },
                    Message::QualcommStorageFilePicked,
                );
            }

            // UFS splits the storage into LUNs, and only the phone can say
            // which ones there are — one of them has no partition table at all.
            let partitions = state.flash.qualcomm.partitions.clone();
            let log = flash_log::FlashLog::create(flash_log::partition_manager_path(&port));
            state.flash.qualcomm.log_path = Some(log.path().to_path_buf());
            state.flash.qualcomm.busy = true;
            state.flash.qualcomm.lun_dialog = Some(LunDialog::Reading);

            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        let mut log = log;
                        firehose::storage_luns(&port, &partitions, &mut log)
                    })
                    .await
                    .unwrap_or_else(|error| {
                        Err(format!("the Firehose worker stopped: {error}"))
                    })
                },
                Message::QualcommLunsRead,
            )
        }
        Message::QualcommLunsRead(result) => {
            let qualcomm = &mut state.flash.qualcomm;
            qualcomm.busy = false;
            qualcomm.lun_dialog = Some(match result {
                // Every LUN starts checked: the button says "entire storage",
                // and unchecking the ones that are not wanted is one click
                // away.
                Ok(luns) => LunDialog::Luns {
                    checked: vec![true; luns.len()],
                    luns,
                },
                Err(error) => LunDialog::Failed(error),
            });

            Task::none()
        }
        Message::QualcommLunToggled(index, checked) => {
            if let Some(LunDialog::Luns { checked: choices, .. }) =
                &mut state.flash.qualcomm.lun_dialog
            {
                if let Some(choice) = choices.get_mut(index) {
                    *choice = checked;
                }
            }

            Task::none()
        }
        Message::QualcommLunsAll(every) => {
            if let Some(LunDialog::Luns { checked, .. }) =
                &mut state.flash.qualcomm.lun_dialog
            {
                checked.fill(every);
            }

            Task::none()
        }
        Message::QualcommLunBackdropPressed => Task::none(),
        Message::QualcommLunDialogClosed => {
            state.flash.qualcomm.lun_dialog = None;
            state.flash.qualcomm.pending_luns.clear();

            Task::none()
        }
        Message::QualcommLunsConfirmed => {
            let picked: Vec<firehose::Lun> = match &state.flash.qualcomm.lun_dialog {
                Some(LunDialog::Luns { luns, checked }) => luns
                    .iter()
                    .zip(checked)
                    .filter(|(_, checked)| **checked)
                    .map(|(lun, _)| lun.clone())
                    .collect(),
                _ => return Task::none(),
            };
            if picked.is_empty() {
                return Task::none();
            }

            // One LUN is one file, so it can be named and placed where the
            // user wants it; several need a folder that holds one file each.
            let single = picked[0].number;
            let one = picked.len() == 1;
            state.flash.qualcomm.pending_luns = picked;

            if one {
                let title = state.l10n.tr("qc-dump-storage-file-title");
                let filter = state.l10n.tr("qc-image-filter");
                let name = format!("lun{single}.img");

                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            rfd::FileDialog::new()
                                .set_title(&title)
                                .set_file_name(&name)
                                .add_filter(&filter, &["img"])
                                .save_file()
                        })
                        .await
                        .unwrap_or(None)
                    },
                    Message::QualcommStorageFilePicked,
                );
            }

            let title = state.l10n.tr("qc-dump-storage-folder-title");

            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        rfd::FileDialog::new().set_title(&title).pick_folder()
                    })
                    .await
                    .unwrap_or(None)
                },
                Message::QualcommStorageFolderPicked,
            )
        }
        Message::QualcommStorageFilePicked(file) => match file {
            Some(file) => start_qualcomm_storage_dump(state, firehose::DumpTarget::File(file)),
            // The picker was cancelled; the LUN list stays up so it can be
            // tried again.
            None => {
                state.flash.qualcomm.pending_luns.clear();
                Task::none()
            }
        },
        Message::QualcommStorageFolderPicked(directory) => match directory {
            Some(directory) => {
                start_qualcomm_storage_dump(state, firehose::DumpTarget::Folder(directory))
            }
            None => {
                state.flash.qualcomm.pending_luns.clear();
                Task::none()
            }
        },
        Message::QualcommStorageDumpFinished(result) => {
            let qualcomm = &mut state.flash.qualcomm;
            qualcomm.busy = false;
            qualcomm.progress = None;
            qualcomm.status = match result {
                Ok((count, path)) => QualcommStatus::StorageDumpDone { count, path },
                Err(error) => QualcommStatus::StorageDumpFailed(error),
            };

            Task::none()
        }
        Message::QualcommWrite => {
            if state.flash.qualcomm.busy {
                return Task::none();
            }

            let selected = qualcomm_selected_regions(state);
            let status = match selected.len() {
                0 => Some("qc-select-partition"),
                1 => None,
                _ => Some("qc-select-one-partition"),
            };
            if let Some(id) = status {
                state.flash.qualcomm.status = QualcommStatus::WriteFailed(state.l10n.tr(id));
                return Task::none();
            }

            let title = state.l10n.tr("qc-write-file-title");
            let filter = state.l10n.tr("qc-image-filter");

            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        rfd::FileDialog::new()
                            .set_title(&title)
                            .add_filter(&filter, &["img", "bin"])
                            .pick_file()
                    })
                    .await
                    .unwrap_or(None)
                },
                Message::QualcommWriteFilePicked,
            )
        }
        Message::QualcommWriteFilePicked(image) => {
            let Some(image) = image else {
                return Task::none();
            };
            if state.flash.qualcomm.busy {
                return Task::none();
            }
            let selected = qualcomm_selected_regions(state);
            let [region] = selected.as_slice() else {
                return Task::none();
            };
            let Some(port) = state.flash.qualcomm.port.clone() else {
                state.flash.qualcomm.status =
                    QualcommStatus::WriteFailed(state.l10n.tr("qc-select-port"));
                return Task::none();
            };

            let region = region.clone();
            let name = region.name.clone();
            let log = flash_log::FlashLog::create(flash_log::partition_manager_path(&port));
            state.flash.qualcomm.log_path = Some(log.path().to_path_buf());
            state.flash.qualcomm.busy = true;
            state.flash.qualcomm.progress = Some(firehose::Progress::default());
            state.flash.qualcomm.status = QualcommStatus::Writing {
                name: name.clone(),
                done: 0,
                total: 0,
            };

            let (progress, events) = iced::futures::channel::mpsc::unbounded();

            Task::batch([
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            let mut log = log;
                            let report = move |step: firehose::Progress| {
                                let _ =
                                    progress.unbounded_send(Message::QualcommWriteProgress(step));
                            };

                            firehose::write_partition(
                                &port,
                                &region,
                                &image,
                                &mut log,
                                &report,
                            )
                            .map(|()| name)
                        })
                        .await
                        .unwrap_or_else(|error| {
                            Err(format!("the Firehose worker stopped: {error}"))
                        })
                    },
                    Message::QualcommWriteFinished,
                ),
                Task::run(events, |message| message),
            ])
        }
        Message::QualcommWriteProgress(progress) => {
            if let QualcommStatus::Writing { name, .. } = &state.flash.qualcomm.status {
                let name = name.clone();
                state.flash.qualcomm.status = QualcommStatus::Writing {
                    name,
                    done: progress.done,
                    total: progress.total,
                };
                state.flash.qualcomm.progress = Some(progress);
            }

            Task::none()
        }
        Message::QualcommWriteFinished(result) => {
            let qualcomm = &mut state.flash.qualcomm;
            qualcomm.busy = false;
            qualcomm.progress = None;
            qualcomm.status = match result {
                Ok(name) => QualcommStatus::WriteDone(name),
                Err(error) => QualcommStatus::WriteFailed(error),
            };

            Task::none()
        }
        Message::QualcommErase => {
            if state.flash.qualcomm.busy {
                return Task::none();
            }

            let regions = qualcomm_selected_regions(state);
            if regions.is_empty() {
                state.flash.qualcomm.status =
                    QualcommStatus::EraseFailed(state.l10n.tr("qc-select-partition"));
                return Task::none();
            }

            // Erasing cannot be undone, so it asks first. The dialog is a
            // native one, so its buttons carry the system's language.
            let title = state.l10n.tr("qc-erase-confirm-title");
            let question = state
                .l10n
                .tr_with_args("qc-erase-confirm", &[("count", regions.len().to_string())]);

            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        matches!(
                            rfd::MessageDialog::new()
                                .set_title(&title)
                                .set_description(&question)
                                .set_level(rfd::MessageLevel::Warning)
                                .set_buttons(rfd::MessageButtons::YesNo)
                                .show(),
                            rfd::MessageDialogResult::Yes
                        )
                    })
                    .await
                    .unwrap_or(false)
                },
                Message::QualcommEraseConfirmed,
            )
        }
        Message::QualcommEraseConfirmed(confirmed) => {
            if confirmed {
                start_qualcomm_erase(state)
            } else {
                Task::none()
            }
        }
        Message::QualcommEraseProgress(progress) => {
            if state.flash.qualcomm.busy {
                state.flash.qualcomm.status = QualcommStatus::Erasing {
                    name: progress.name.clone(),
                    done: progress.done,
                    total: progress.total,
                };
                state.flash.qualcomm.progress = Some(progress);
            }

            Task::none()
        }
        Message::QualcommEraseFinished(result) => {
            let qualcomm = &mut state.flash.qualcomm;
            qualcomm.busy = false;
            qualcomm.progress = None;
            qualcomm.status = match result {
                Ok(count) => QualcommStatus::EraseDone { count },
                Err(error) => QualcommStatus::EraseFailed(error),
            };

            Task::none()
        }
        Message::QualcommRefresh => start_qualcomm_refresh(state),
        Message::QualcommRefreshed(result) => {
            let qualcomm = &mut state.flash.qualcomm;
            qualcomm.busy = false;

            match result {
                Ok(table) => apply_qualcomm_table(qualcomm, table),
                Err(error) => {
                    // The table was cleared when the refresh started, so the
                    // page keeps its rows until a read goes through again.
                    qualcomm.status = QualcommStatus::RefreshFailed(error);
                }
            }

            Task::none()
        }
        Message::MultiFlashPresetManage => {
            state.flash.multi_flash.preset_dialog = true;
            // A result from an earlier visit says nothing about this one.
            state.flash.multi_flash.preset_status = None;
            Task::none()
        }
        Message::MultiFlashPresetBackdropPressed => Task::none(),
        Message::MultiFlashPresetClose => {
            // A list holding two presets that would flash the same phone is not
            // one a run can work with — the first of them would win, by their
            // order — so the card cannot be left. The button is shut for the
            // same reason; this refuses a stray event.
            if !multi_flash_run::conflicting_presets(&state.flash.multi_flash.presets).is_empty() {
                return Task::none();
            }

            state.flash.multi_flash.preset_dialog = false;
            Task::none()
        }
        Message::MultiFlashPresetAdd => {
            let title = state.l10n.tr("multi-flash-preset-add");

            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        // The same formats the single-device Firmware Flash
                        // dialog takes: a package ZIP, or a `flashfile.xml`
                        // with its image files next to it.
                        rfd::FileDialog::new()
                            .set_title(&title)
                            .add_filter("Firmware package", &FIRMWARE_PACKAGE_EXTENSIONS)
                            .pick_file()
                    })
                    .await
                    .unwrap_or(None)
                },
                Message::MultiFlashPresetPicked,
            )
        }
        Message::MultiFlashPresetPicked(path) => {
            let Some(path) = path else {
                return Task::none();
            };

            // The image is configured before it joins the list: the options
            // card decides what happens to it, and cancelling it discards the
            // pick. What the package is for is read while the card opens.
            let tool = default_preset_tool(&state.flash.multi_flash);
            state.flash.multi_flash.pending = PresetOptions {
                path: Some(path.clone()),
                identifying: true,
                tool,
                ..PresetOptions::default()
            };

            start_multi_flash_identify(path)
        }
        Message::MultiFlashPresetIdentified(path, identity) => {
            let ask = {
                let pending = &mut state.flash.multi_flash.pending;

                // An answer for a package the card no longer shows.
                if pending.path.as_deref() != Some(path.as_path()) {
                    return Task::none();
                }

                pending.identifying = false;
                // Every procedure starts checked; the card offers the same
                // checklist the single-device dialog does.
                pending.enabled = match &identity {
                    Ok(identity) => vec![true; identity.steps.len()],
                    Err(_) => Vec::new(),
                };
                pending.identity = Some(identity);

                // The firmware names the CID it is for, so a custom condition
                // starts with it instead of an empty list.
                pending.cids = pending.default_cids();

                // A ZIP holds images that have to be unpacked before anything
                // can be flashed, so its directory is asked for now. The
                // check above runs FIRST: a file that is no firmware package
                // never gets a folder picked for it, and nothing is ever
                // unpacked on the strength of its name.
                matches!(pending.identity, Some(Ok(_)))
                    && pending.zip()
                    && pending.extract_to.is_none()
            };

            if ask {
                return start_multi_flash_choose_directory(
                    state,
                    path,
                    None,
                    DirectoryAsk::New,
                );
            }

            Task::none()
        }
        Message::MultiFlashStepToggled(index, checked) => {
            let enabled = &mut state.flash.multi_flash.pending.enabled;

            if let Some(flag) = enabled.get_mut(index) {
                *flag = checked;
            }

            Task::none()
        }
        Message::MultiFlashStepsAll(checked) => {
            for enabled in &mut state.flash.multi_flash.pending.enabled {
                *enabled = checked;
            }

            Task::none()
        }
        Message::MultiFlashSelectPart(part) => {
            let pending = &mut state.flash.multi_flash.pending;

            let Some(Ok(identity)) = &pending.identity else {
                return Task::none();
            };

            // Additive, like the single-device dialog: the steps of that part
            // are checked and nothing is unchecked, so parts can be combined.
            let mediatek = flashfile::is_mediatek(&identity.steps);
            let selected: Vec<usize> = identity
                .steps
                .iter()
                .enumerate()
                .filter(|(_, step)| step.part(mediatek) == Some(part))
                .map(|(index, _)| index)
                .collect();

            for index in selected {
                if let Some(enabled) = pending.enabled.get_mut(index) {
                    *enabled = true;
                }
            }

            Task::none()
        }
        Message::MultiFlashPresetEdit(index) => {
            let multi = &mut state.flash.multi_flash;

            let Some(preset) = multi.presets.get(index).cloned() else {
                return Task::none();
            };

            multi.pending = PresetOptions::from_preset(&preset, index);

            // The folder a preset was unpacked into may be gone by now (moved
            // or deleted). The card asks for one again rather than confirming
            // a preset whose files are nowhere.
            if multi
                .pending
                .extract_to
                .as_deref()
                .is_some_and(|directory| !directory.is_dir())
            {
                multi.pending.extract_to = None;
            }

            Task::none()
        }
        Message::MultiFlashPresetRemoved(index) => {
            let presets = &mut state.flash.multi_flash.presets;

            if index < presets.len() {
                presets.remove(index);
            }

            Task::none()
        }
        Message::MultiFlashOptionsBackdropPressed => Task::none(),
        Message::MultiFlashOptionsCancel => {
            // A preset whose files are still being written is not dropped:
            // the card does not offer this while that runs, and a stray event
            // must not leave the user's folder half filled either.
            if state.flash.multi_flash.pending.unpacking {
                return Task::none();
            }

            state.flash.multi_flash.pending = PresetOptions::default();
            Task::none()
        }
        Message::MultiFlashOptionsConfirm => {
            let multi = &mut state.flash.multi_flash;

            // The card only offers this once the image and the conditions are
            // in order.
            let preset = {
                let pending = &multi.pending;

                let (Some(path), true) = (pending.path.clone(), pending.complete()) else {
                    return Task::none();
                };

                let Some(Ok(identity)) = pending.identity.clone() else {
                    return Task::none();
                };

                let conditions = if pending.custom {
                    FlashConditions::Custom {
                        cids: pending.cids.clone(),
                        states: pending.states.clone(),
                    }
                } else {
                    FlashConditions::Default
                };

                MultiFlashPreset {
                    path,
                    identity,
                    enabled: pending.enabled.clone(),
                    reboot: pending.reboot,
                    verify_checksums: pending.verify_checksums,
                    tool: pending.tool.clone(),
                    extract_to: pending.extract_to.clone(),
                    conditions,
                    no_engineering: pending.no_engineering,
                }
            };

            // A card opened from the list changes that entry; a new pick joins
            // the end of it.
            match multi.pending.index {
                Some(index) if index < multi.presets.len() => multi.presets[index] = preset,
                _ => multi.presets.push(preset),
            }

            multi.pending = PresetOptions::default();

            Task::none()
        }
        Message::MultiFlashOptionsRebootToggled(reboot) => {
            state.flash.multi_flash.pending.reboot = reboot;
            Task::none()
        }
        Message::MultiFlashVerifyToggled(verify) => {
            state.flash.multi_flash.pending.verify_checksums = verify;
            Task::none()
        }
        Message::MultiFlashToolSelected(tool) => {
            state.flash.multi_flash.pending.tool = tool;
            Task::none()
        }
        Message::MultiFlashPresetChooseDirectory => {
            let (path, current, ask) = {
                let pending = &state.flash.multi_flash.pending;
                (
                    pending.path.clone(),
                    pending.extract_to.clone(),
                    // A package that is unpacked next to its flashfile already
                    // needs no directory, and one that is being written must
                    // not be moved under the extraction.
                    pending.zip() && !pending.unpacking,
                )
            };

            match (path, ask) {
                (Some(path), true) => {
                    start_multi_flash_choose_directory(state, path, current, DirectoryAsk::Change)
                }
                _ => Task::none(),
            }
        }
        Message::MultiFlashPresetDirectoryPicked(directory, ask) => {
            let Some(directory) = directory else {
                // A cancelled pick. The first directory belongs to the flow
                // that adds a firmware — without one there is nothing to
                // confirm — so cancelling it drops the card and brings the
                // preset list back, exactly as cancelling the file pick does.
                // Changing a directory that is set already only leaves the
                // card as it was.
                if ask == DirectoryAsk::New {
                    state.flash.multi_flash.pending = PresetOptions::default();
                }

                return Task::none();
            };

            state.flash.multi_flash.pending.extract_to = Some(directory);

            start_multi_flash_unpack(state)
        }
        Message::MultiFlashUnpackEvent(path, event) => {
            let pending = &mut state.flash.multi_flash.pending;

            // An answer for a package the card no longer shows.
            if pending.path.as_deref() != Some(path.as_path()) {
                return Task::none();
            }

            match event {
                flash_engine::PackageEvent::Extract { done, total } => {
                    pending.unpacked = Some((done, total));
                }
                flash_engine::PackageEvent::Ready(_) => {
                    // The card works from what `identify` read out of the
                    // archive, so the unpacked package itself is not needed
                    // here: what matters is that its files are in place.
                    pending.unpacking = false;
                    pending.unpack_error = None;
                }
                flash_engine::PackageEvent::Failed(error) => {
                    pending.unpacking = false;
                    pending.unpack_error = Some(error);
                }
                // The lines an unpack logs are the engine's own (English)
                // notes, and the card has no log to show them in.
                flash_engine::PackageEvent::Log(_) => {}
            }

            Task::none()
        }
        Message::MultiFlashUnpackWorkerDone => Task::none(),
        Message::MultiFlashPresetSave => {
            let title = state.l10n.tr("multi-flash-preset-save");
            let presets = state.flash.multi_flash.presets.clone();

            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        let Some(path) = rfd::FileDialog::new()
                            .set_title(&title)
                            .set_file_name(preset_file::DEFAULT_FILE_NAME)
                            .add_filter("Preset file", &preset_file::EXTENSIONS)
                            .save_file()
                        else {
                            return Ok(None);
                        };

                        preset_file::save(&presets, &path).map(|()| Some(path))
                    })
                    .await
                    .unwrap_or_else(|error| {
                        Err(preset_file::FileError::Other(format!(
                            "background task failed: {error}"
                        )))
                    })
                },
                Message::MultiFlashPresetSaved,
            )
        }
        Message::MultiFlashPresetSaved(result) => {
            let multi = &mut state.flash.multi_flash;

            multi.preset_status = match result {
                Ok(Some(path)) => Some(PresetStatus::Saved(path)),
                // The save dialog was cancelled: nothing happened.
                Ok(None) => None,
                Err(error) => Some(PresetStatus::Failed(error)),
            };

            Task::none()
        }
        Message::MultiFlashPresetLoad => {
            let title = state.l10n.tr("multi-flash-preset-load");

            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        let Some(path) = rfd::FileDialog::new()
                            .set_title(&title)
                            .add_filter("Preset file", &preset_file::EXTENSIONS)
                            .pick_file()
                        else {
                            return Ok(None);
                        };

                        // Every firmware package is identified again on this
                        // thread, so a slow disk does not hold up the UI.
                        preset_file::load(&path).map(Some)
                    })
                    .await
                    .unwrap_or_else(|error| {
                        Err(preset_file::FileError::Other(format!(
                            "background task failed: {error}"
                        )))
                    })
                },
                Message::MultiFlashPresetLoaded,
            )
        }
        Message::MultiFlashPresetLoaded(result) => {
            let multi = &mut state.flash.multi_flash;

            match result {
                // The file replaces the list: that is what loading a preset
                // file means, and entries that could not be read are reported
                // instead of being kept as half-usable rows.
                Ok(Some(loaded)) => {
                    let count = loaded.presets.len();
                    multi.presets = loaded.presets;
                    multi.preset_status = Some(PresetStatus::Loaded {
                        path: loaded.path,
                        count,
                        skipped: loaded.skipped,
                    });
                }
                Ok(None) => {}
                Err(error) => multi.preset_status = Some(PresetStatus::Failed(error)),
            }

            Task::none()
        }
        Message::MultiFlashConditionsDefault => {
            state.flash.multi_flash.pending.custom = false;
            Task::none()
        }
        Message::MultiFlashConditionsCustom => {
            let pending = &mut state.flash.multi_flash.pending;
            pending.custom = true;

            // A condition written by hand starts with the CID the firmware is
            // for, so the usual case is one click away from being complete.
            if pending.cids.is_empty() {
                pending.cids = pending.default_cids();
            }

            Task::none()
        }
        Message::MultiFlashNoEngineeringToggled(no_engineering) => {
            state.flash.multi_flash.pending.no_engineering = no_engineering;
            Task::none()
        }
        Message::MultiFlashCidChanged(value) => {
            state.flash.multi_flash.pending.cid_input = value;
            Task::none()
        }
        Message::MultiFlashCidPresetSelected(code) => {
            // The rule between two groups of the dropdown is there to be read,
            // not to be picked — and it names no CID to put into the field.
            let Some(code) = code else {
                return Task::none();
            };

            state.flash.multi_flash.pending.cid_input = flashfile::normalize_cid(code);

            Task::none()
        }
        Message::MultiFlashCidAdded => {
            let pending = &mut state.flash.multi_flash.pending;

            // The same normalization the firmware dialog uses, so `50`,
            // `0x32` and `0x0032` are one CID here too.
            let cid = flashfile::normalize_cid(&pending.cid_input);
            pending.cid_input.clear();

            if !cid.is_empty() && !pending.cids.contains(&cid) {
                pending.cids.push(cid);
            }

            Task::none()
        }
        Message::MultiFlashCidRemoved(index) => {
            let cids = &mut state.flash.multi_flash.pending.cids;

            if index < cids.len() {
                cids.remove(index);
            }

            Task::none()
        }
        Message::MultiFlashStateToggled(securestate, selected) => {
            let states = &mut state.flash.multi_flash.pending.states;

            if selected {
                if !states.contains(&securestate) {
                    states.push(securestate);
                }
            } else {
                states.retain(|state| *state != securestate);
            }

            Task::none()
        }
        Message::MultiFlashStart => {
            let multi = &mut state.flash.multi_flash;

            // A run needs something to flash; the button is only drawn live
            // once an image was loaded. A device of the previous run that is
            // still being flashed has to finish first: its log is still being
            // written, and it belongs to the run that started it.
            if multi.running || !multi.flashing.is_empty() || multi.presets.is_empty() {
                return Task::none();
            }

            multi.running = true;
            multi.devices.clear();
            multi.run_error = None;
            multi.log_directory = None;
            multi.session += 1;

            start_multi_flash_session(state)
        }
        Message::MultiFlashStop => {
            let multi = &mut state.flash.multi_flash;

            // Only the watch for devices is stopped. A device that is being
            // written right now runs to the end — pulling the cable out of a
            // phone mid-flash is not what the button should do — so `flashing`
            // is left as it is, its row stays on the table to be watched (and
            // keeps being updated as it goes), and the window stays unclosable
            // until every one of them is done. Only what the run is finished
            // with goes away: a stopped run keeps exactly the devices it is
            // still writing to.
            if let Some(stop) = multi.stop.take() {
                stop.store(true, Ordering::Relaxed);
            }

            multi.running = false;
            multi.devices.retain(|device| !device.status.settled());

            Task::none()
        }
        Message::MultiFlashLocateLog => {
            // The button is drawn once the session said where it writes, so a
            // stray event (or a run that could not make its directory) has
            // nothing to show.
            let Some(directory) = state.flash.multi_flash.log_directory.clone() else {
                return Task::none();
            };

            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || flash_log::reveal(&directory))
                        .await
                        .unwrap_or_else(|error| Err(error.to_string()))
                },
                |result| {
                    if let Err(error) = result {
                        eprintln!("failed to open the log folder: {error}");
                    }

                    Message::MultiFlashLogOpened
                },
            )
        }
        Message::MultiFlashLogOpened => Task::none(),
        Message::MultiFlashSessionEvent(session, event) => {
            let multi = &mut state.flash.multi_flash;

            // What a run that is no longer the one on the page reports (a new
            // "Start" took the table over, see [`MultiFlashState::session`]).
            if session != multi.session {
                return Task::none();
            }

            match event {
                multi_flash_run::SessionEvent::LogDirectory(directory) => {
                    multi.log_directory = Some(directory);
                }
                multi_flash_run::SessionEvent::Failed(error) => {
                    multi.running = false;
                    multi.run_error = Some(error);
                }
                multi_flash_run::SessionEvent::Finished { serial, result } => {
                    multi.flashing.remove(&serial);

                    if let Some(device) = device_mut(multi, &serial) {
                        device.status = match result {
                            Ok(()) => DeviceStatus::Finished,
                            Err(error) => DeviceStatus::Failed(error),
                        };
                    }
                }
                multi_flash_run::SessionEvent::Stopped => {
                    multi.running = false;
                    multi.stop = None;
                }
                multi_flash_run::SessionEvent::Identified {
                    serial,
                    xt_code,
                    cid,
                    steps,
                } => {
                    // A device with procedures to run is being written to from
                    // here on, which is what keeps the window from being closed
                    // while this run goes on — even after it was stopped. Its
                    // row is shown for the same reason: a device the run is
                    // still writing to is what the wait is about.
                    if steps.is_some() {
                        multi.flashing.insert(serial.clone());
                    }

                    let status = match steps {
                        Some(_) => DeviceStatus::Preparing,
                        None => DeviceStatus::NoPreset,
                    };

                    multi.devices.push(MultiFlashDevice {
                        serial,
                        xt_code,
                        cid,
                        status,
                        fraction: 0.0,
                    });
                }
                multi_flash_run::SessionEvent::Step {
                    serial,
                    index,
                    total,
                    label,
                } => {
                    if let Some(device) = device_mut(multi, &serial) {
                        // The bar starts over for every procedure: the fraction
                        // is about the preset as a whole.
                        device.fraction = device_fraction(index, 0.0, total);
                        device.status = DeviceStatus::Flashing {
                            index,
                            total,
                            label,
                        };
                    }
                }
                multi_flash_run::SessionEvent::Progress {
                    serial,
                    done,
                    total,
                } => {
                    if let Some(device) = device_mut(multi, &serial) {
                        if let DeviceStatus::Flashing {
                            index,
                            total: steps,
                            ..
                        } = &device.status
                        {
                            let inside = if total == 0 {
                                0.0
                            } else {
                                done as f32 / total as f32
                            };

                            device.fraction = device_fraction(*index, inside, *steps);
                        }
                    }
                }
            }

            Task::none()
        }
        Message::MultiFlashSessionDone => Task::none(),
        Message::WindowCloseRequested(id) => {
            // The Partition Manager holds a live Firehose session of its own:
            // the programmer is running inside the phone, and closing the
            // window would leave it in Firehose mode with no way back but a
            // power cycle. The prompt offers both ways out.
            if state.flash.qualcomm.open
                && (state.flash.qualcomm.busy || state.flash.qualcomm.programmer_uploaded)
            {
                state.flash.qualcomm.close_prompt = Some(id);
                return Task::none();
            }

            // Closing the window would cut the run short, so it is refused
            // and the user is told to stop the operation first. A device that
            // is still being flashed keeps the window open even after "Stop":
            // its log is still being written, and it has to finish.
            if state.flash.multi_flash.running || !state.flash.multi_flash.flashing.is_empty() {
                state.flash.multi_flash.close_prompt = true;
                return Task::none();
            }

            iced::window::close::<Message>(id)
        }
        Message::QualcommClosePromptBackdropPressed => Task::none(),
        Message::QualcommClosePromptCancelled => {
            state.flash.qualcomm.close_prompt = None;
            Task::none()
        }
        Message::QualcommCloseConfirmed => {
            // The window the close request named is the one that closes: the
            // prompt only ever stands in for that request.
            match state.flash.qualcomm.close_prompt.take() {
                Some(id) => iced::window::close::<Message>(id),
                None => Task::none(),
            }
        }
        Message::MultiFlashClosePromptDismissed => {
            state.flash.multi_flash.close_prompt = false;
            Task::none()
        }
        Message::MultiFlashClosePromptBackdropPressed => Task::none(),
        Message::FirmwareFlashBackdropPressed => Task::none(),
        Message::FirmwareFlashPackageEvent(job, event) => {
            let flash = &mut state.flash.firmware;
            // Events from a package this dialog no longer works on.
            if job != flash.job {
                return Task::none();
            }

            match event {
                flash_engine::PackageEvent::Extract { done, total } => {
                    flash.extracted = Some((done, total));
                }
                flash_engine::PackageEvent::Log(line) => push_flash_log(flash, line),
                flash_engine::PackageEvent::Ready(package) => {
                    flash.loading = false;
                    flash.extracted = None;
                    flash.error = None;
                    // Every procedure is checked by default; the user can
                    // uncheck the ones they do not want to run.
                    flash.enabled = vec![true; package.steps.len()];
                    flash.plan = Some(*package);
                    flash.export_result = None;
                }
                flash_engine::PackageEvent::Failed(error) => {
                    flash.loading = false;
                    flash.extracted = None;
                    flash.plan = None;
                    flash.error = Some(error);
                }
            }

            Task::none()
        }
        Message::FirmwareFlashEvent(job, event) => {
            let flash = &mut state.flash.firmware;
            // Events from a flash this dialog no longer runs.
            if job != flash.job {
                return Task::none();
            }

            // New log lines pull the log viewport down, so the newest output
            // stays visible without the user having to scroll.
            let mut scroll_log = false;

            match event {
                flash_engine::FlashEvent::StepStarted {
                    index,
                    total,
                    label,
                } => {
                    push_flash_log(
                        flash,
                        format!("[{}/{}] {}", index + 1, total, label),
                    );
                    flash.step_index = index;
                    flash.step_total = total;
                    flash.step_label = label;
                    flash.progress = None;
                    scroll_log = true;
                }
                flash_engine::FlashEvent::Progress { done, total } => {
                    flash.progress = Some((done, total));
                }
                flash_engine::FlashEvent::Log(line) => {
                    push_flash_log(flash, line);
                    scroll_log = true;
                }
                flash_engine::FlashEvent::Finished(result) => {
                    flash.running = false;
                    flash.progress = None;
                    flash.result = Some(result);
                }
            }

            if scroll_log {
                scroll_flash_log_to_end()
            } else {
                Task::none()
            }
        }
        Message::FirmwareFlashWorkerDone => Task::none(),
        Message::PartitionFlashRescan => {
            let flash = &mut state.flash.partition;
            if flash.listing || flash.task.is_some() {
                return Task::none();
            }

            flash.listing = true;
            flash.devices.clear();
            flash.list_error = None;
            flash.selected = None;
            flash.reading_device = false;
            flash.vars = None;
            flash.vars_error = None;

            start_partition_flash_list_devices()
        }
        Message::PartitionFlashDevicesFetched(result) => {
            let flash = &mut state.flash.partition;
            flash.listing = false;

            match result {
                Ok(devices) => {
                    flash.devices = devices;
                    flash.list_error = None;
                }
                Err(error) => {
                    flash.devices.clear();
                    flash.list_error = Some(error);
                }
            }

            // A single phone is what the dialog would write to anyway, so it
            // is selected right away (and its variables are read).
            let only = match flash.devices.as_slice() {
                [device] => Some(device.serial.clone()),
                _ => None,
            };

            match only {
                Some(serial) => select_partition_flash_device(state, serial),
                None => Task::none(),
            }
        }
        Message::PartitionFlashDeviceSelected(serial) => {
            select_partition_flash_device(state, serial)
        }
        Message::PartitionFlashDeviceVarsRead(serial, result) => {
            let flash = &mut state.flash.partition;
            // Ignore an answer for a phone that is not selected any more.
            if flash.selected.as_deref() != Some(serial.as_str()) {
                return Task::none();
            }

            flash.reading_device = false;

            match result {
                Ok(vars) => {
                    flash.vars = Some(vars);
                    flash.vars_error = None;
                }
                Err(error) => {
                    flash.vars = None;
                    flash.vars_error = Some(error);
                }
            }

            Task::none()
        }
        Message::PartitionFlashEngineSelected(engine) => {
            let flash = &mut state.flash.partition;
            flash.engine = engine;
            flash.tool_version = None;

            // The versions differ per tool, so the new one is asked too.
            start_partition_flash_tool_version(state)
        }
        Message::PartitionFlashToolVersionRead(path, result) => {
            let flash = &mut state.flash.partition;

            // A late answer about a tool that is not selected any more is
            // dropped, like the device variables' serial guard.
            if flash.engine.tool().map(|tool| tool.path.as_path()) != Some(path.as_path()) {
                return Task::none();
            }

            flash.reading_version = false;
            flash.tool_version = Some(result);
            Task::none()
        }
        Message::PartitionFlashPartitionChanged(partition) => {
            // The field cannot be typed into while a job runs; a change that
            // arrives anyway would name a partition the run is not writing.
            if state.flash.partition.task.is_some() {
                return Task::none();
            }

            state.flash.partition.partition = partition;
            Task::none()
        }
        Message::PartitionFlashPartitionPreset(partition) => {
            if state.flash.partition.task.is_some() {
                return Task::none();
            }

            state.flash.partition.partition = partition.to_owned();
            Task::none()
        }
        Message::PartitionFlashPickImage => {
            if partition_flash::busy(state) {
                return Task::none();
            }

            let title = state.l10n.tr("partition-flash-select");

            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        rfd::FileDialog::new()
                            .set_title(&title)
                            .add_filter("Image", &PARTITION_IMAGE_EXTENSIONS)
                            .pick_file()
                            // Whether the image can take the AVB flags is read
                            // once, here, rather than per frame or per flash.
                            .map(|path| {
                                let avb = avb::image_vbmeta_offset(&path).is_some();
                                (path, avb)
                            })
                    })
                    .await
                    .unwrap_or(None)
                },
                Message::PartitionFlashImagePicked,
            )
        }
        Message::PartitionFlashImagePicked(picked) => {
            let Some((path, avb)) = picked else {
                return Task::none();
            };

            if partition_flash::busy(state) {
                return Task::none();
            }

            set_partition_flash_image(state, path, avb);
            Task::none()
        }
        Message::PartitionFlashImageDropped(path) => {
            // A dropped file belongs to the Single Partition Flash dialog: it
            // is the only feature that takes an image that way.
            if !partition_flash::is_open(state) || partition_flash::busy(state) {
                return Task::none();
            }

            let avb = avb::image_vbmeta_offset(&path).is_some();
            set_partition_flash_image(state, path, avb);
            Task::none()
        }
        Message::PartitionFlashVerityToggled(disable) => {
            let flash = &mut state.flash.partition;

            // The boxes are drawn disabled for an image without a vbmeta
            // struct; a stale click must not set the flag behind them.
            if !flash.image_avb || flash.task.is_some() {
                return Task::none();
            }

            flash.disable_verity = disable;
            Task::none()
        }
        Message::PartitionFlashVerificationToggled(disable) => {
            let flash = &mut state.flash.partition;

            if !flash.image_avb || flash.task.is_some() {
                return Task::none();
            }

            flash.disable_verification = disable;
            Task::none()
        }
        Message::PartitionFlashStart => start_partition_flash(state),
        Message::PartitionFlashEvent(job, event) => {
            let flash = &mut state.flash.partition;
            // Events of a job this dialog no longer runs.
            if job != flash.job {
                return Task::none();
            }

            let mut scroll_log = false;

            match event {
                // A single image has no steps: the tool's own lines are the
                // progress that matters here.
                flash_engine::FlashEvent::StepStarted { .. } => {}
                flash_engine::FlashEvent::Progress { done, total } => {
                    flash.progress = Some((done, total));
                }
                flash_engine::FlashEvent::Log(line) => {
                    push_partition_flash_log(flash, line);
                    scroll_log = true;
                }
                flash_engine::FlashEvent::Finished(result) => {
                    flash.task = None;
                    flash.progress = None;
                    flash.result = Some(result);
                }
            }

            if scroll_log {
                scroll_flash_log_to_end()
            } else {
                Task::none()
            }
        }
        Message::PartitionFlashReboot => start_partition_flash_reboot(state),
        Message::PartitionFlashRebooted(job, result) => {
            let flash = &mut state.flash.partition;
            if job != flash.job || flash.task != Some(PartitionFlashTask::Reboot) {
                return Task::none();
            }

            if let Err(error) = &result {
                push_partition_flash_log(flash, format!("reboot failed: {error}"));
            }

            flash.task = None;
            flash.reboot_result = Some(result);

            scroll_flash_log_to_end()
        }
        Message::PartitionFlashSwitchToFastbootd => start_partition_flash_switch(state),
        Message::PartitionFlashSwitched(job, result) => {
            let flash = &mut state.flash.partition;
            if job != flash.job || flash.task != Some(PartitionFlashTask::SwitchToFastbootd) {
                return Task::none();
            }

            flash.task = None;

            let devices = match result {
                Ok(devices) => devices,
                Err(error) => {
                    flash.switch_result = Some(Err(error));
                    return Task::none();
                }
            };

            flash.devices = devices;
            flash.list_error = None;
            flash.switch_result = Some(Ok(()));
            flash.vars = None;
            flash.vars_error = None;

            // The same phone is on the bus again, under the serial userspace
            // fastboot reports; when only one phone is there at all, that is
            // the one.
            let previous = flash.selected.clone();
            let next = previous
                .filter(|serial| {
                    flash
                        .devices
                        .iter()
                        .any(|device| &device.serial == serial)
                })
                .or_else(|| match flash.devices.as_slice() {
                    [device] => Some(device.serial.clone()),
                    _ => None,
                });

            if next.is_none() {
                flash.selected = None;
            }

            match next {
                Some(serial) => select_partition_flash_device(state, serial),
                None => Task::none(),
            }
        }
        Message::PartitionFlashWorkerDone => Task::none(),
        Message::PartitionFlashLocateLog => {
            // The file this job writes; a dialog that was closed before the
            // flash started has none of its own, so the folder all the logs
            // are written into is opened for those.
            let path = state
                .flash
                .partition
                .log_file
                .as_ref()
                .map(|file| file.path().to_path_buf())
                .filter(|path| path.is_file())
                .unwrap_or_else(flash_log::logs_root);

            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || flash_log::reveal(&path))
                        .await
                        .unwrap_or_else(|error| Err(error.to_string()))
                },
                |result| {
                    if let Err(error) = result {
                        eprintln!("failed to open the log folder: {error}");
                    }

                    Message::PartitionFlashLogOpened
                },
            )
        }
        Message::PartitionFlashLogOpened => Task::none(),
        Message::PartitionFlashCancel => {
            let flash = &mut state.flash.partition;

            // Never close the dialog while a job runs, so the phone is not
            // left half-written with nothing to say what happened.
            if flash.task.is_some() {
                return Task::none();
            }

            *flash = PartitionFlashState::default();
            Task::none()
        }
        // Clicks on empty modal space are swallowed here so they neither
        // dismiss the dialog nor reach the UI underneath.
        Message::PartitionFlashBackdropPressed => Task::none(),
        Message::DriverInstallRequested => start_driver_dialog(state),
        Message::DriverTabletSite => open_browser(driver_install::TABLET_SITE),
        Message::DriverPasswordChanged(password) => {
            state.flash.driver.password = password;
            Task::none()
        }
        Message::DriverInstallConfirmed => {
            let driver = &state.flash.driver;

            // An installation is already running, or the password this system
            // installs with is still missing.
            if driver.busy || (driver_install::uses_password() && driver.password.is_empty()) {
                return Task::none();
            }

            start_driver_job(state)
        }
        Message::DriverCancel => {
            let driver = &mut state.flash.driver;

            // The dialog never closes while it works: the password would be
            // gone while `sudo` still runs.
            if driver.busy {
                return Task::none();
            }

            driver.dialog = DriverDialog::Closed;
            driver.password.clear();
            driver.error = None;
            driver.stage = DriverStage::default();
            driver.downloaded = 0;
            driver.total = 0;
            Task::none()
        }
        Message::DriverEvent(job, event) => {
            let driver = &mut state.flash.driver;

            // Progress of an installation this dialog no longer runs.
            if job != driver.job {
                return Task::none();
            }

            match event {
                driver_install::DriverEvent::Progress { downloaded, total } => {
                    // The segments report in the order they happen to finish;
                    // the bar must not walk backwards.
                    driver.downloaded = driver.downloaded.max(downloaded);

                    if total > 0 {
                        driver.total = total;
                    }
                }
                driver_install::DriverEvent::Installing => {
                    driver.stage = DriverStage::Installing;
                }
                driver_install::DriverEvent::Finished(result) => {
                    driver.busy = false;

                    match result {
                        Ok(()) => driver.stage = DriverStage::Done,
                        Err(error) => driver.error = Some(error),
                    }
                }
            }

            Task::none()
        }
        Message::DriverWorkerDone => Task::none(),
        Message::DriverBackdropPressed => Task::none(),
        Message::BloatwareDevicesFetched(result) => {
            let bloatware = &mut state.flash.bloatware;
            bloatware.preparing = false;

            match result {
                Ok(devices) => {
                    bloatware.prepare_error = None;
                    bloatware.devices = devices;
                }
                Err(error) => {
                    bloatware.devices.clear();
                    bloatware.prepare_error = Some(error);
                }
            }

            // One ready device is the one the user means; with none (or
            // several) the dialog waits for a pick or the next scan.
            let single = match bloatware
                .devices
                .iter()
                .filter(|device| device.state == debloat::AdbDeviceState::Ready)
                .collect::<Vec<_>>()
                .as_slice()
            {
                [device] => Some(device.serial.clone()),
                _ => None,
            };

            match single {
                Some(serial) => start_bloatware_check(state, serial),
                None => Task::none(),
            }
        }
        Message::BloatwareDeviceSelected(serial) => start_bloatware_check(state, serial),
        Message::BloatwareRetry(serial) => start_bloatware_retry(state, serial),
        Message::BloatwareRescan => {
            if state.flash.bloatware.busy() {
                return Task::none();
            }

            reset_bloatware(&mut state.flash.bloatware);
            state.flash.bloatware.preparing = true;

            start_bloatware_devices()
        }
        Message::BloatwarePackagesFetched(result) => {
            let bloatware = &mut state.flash.bloatware;
            if !bloatware.checking {
                return Task::none();
            }

            bloatware.checking = false;

            match result {
                Ok(apps) => {
                    // Everything found is offered for removal; the user
                    // unchecks what should stay.
                    bloatware.checked = vec![true; apps.len()];
                    bloatware.found = Some(apps);
                }
                Err(error) => bloatware.check_error = Some(error),
            }

            Task::none()
        }
        Message::BloatwareAppToggled(index, checked) => {
            if let Some(flag) = state.flash.bloatware.checked.get_mut(index) {
                *flag = checked;
            }

            Task::none()
        }
        Message::BloatwareSelectAll(checked) => {
            let l10n = &state.l10n;
            let bloatware = &mut state.flash.bloatware;

            // While a search filters the list, these buttons act on the rows
            // that are on screen; without one, on all of them.
            let targets: Vec<usize> = match &bloatware.found {
                Some(found) if !bloatware.filter.trim().is_empty() => found
                    .iter()
                    .enumerate()
                    .filter(|(_, app)| {
                        debloat::matches(&app.label(l10n), app.package, &bloatware.filter)
                    })
                    .map(|(index, _)| index)
                    .collect(),
                _ => (0..bloatware.checked.len()).collect(),
            };

            for index in targets {
                if let Some(flag) = bloatware.checked.get_mut(index) {
                    *flag = checked;
                }
            }

            Task::none()
        }
        Message::BloatwareFilterChanged(filter) => {
            state.flash.bloatware.filter = filter;
            Task::none()
        }
        Message::BloatwareRemove => start_bloatware_remove(state),
        Message::BloatwareRemoved(outcomes) => {
            let bloatware = &mut state.flash.bloatware;
            if !bloatware.removing {
                return Task::none();
            }

            bloatware.removing = false;
            bloatware.results = Some(outcomes);

            Task::none()
        }
        Message::BloatwareDone => {
            let bloatware = &mut state.flash.bloatware;

            // Drop what was removed from the checklist, so it matches the
            // device again without asking it a second time.
            if let Some(results) = bloatware.results.take() {
                if let Some(found) = bloatware.found.take() {
                    let removed: Vec<&str> = results
                        .iter()
                        .filter(|outcome| outcome.removed())
                        .map(|outcome| outcome.app.package)
                        .collect();

                    let mut kept = Vec::new();
                    let mut flags = Vec::new();

                    for (index, app) in found.into_iter().enumerate() {
                        if !removed.contains(&app.package) {
                            flags.push(bloatware.checked.get(index).copied().unwrap_or(true));
                            kept.push(app);
                        }
                    }

                    bloatware.checked = flags;
                    bloatware.found = Some(kept);
                }
            }

            Task::none()
        }
        Message::BloatwareBackToDevices => {
            let bloatware = &mut state.flash.bloatware;
            if bloatware.busy() {
                return Task::none();
            }

            bloatware.selected = None;
            bloatware.check_error = None;
            bloatware.found = None;
            bloatware.checked.clear();
            bloatware.results = None;
            Task::none()
        }
        Message::BloatwareCancel => {
            // Never close the dialog while it is working, so the phone is not
            // left with a job nobody is watching.
            if state.flash.bloatware.busy() {
                return Task::none();
            }

            state.flash.bloatware = BloatwareState::default();
            Task::none()
        }
        Message::BloatwareBackdropPressed => Task::none(),
        Message::WirelessMethodSelected(method) => {
            let wireless = &mut state.flash.wireless;

            // The status belongs to the method that produced it.
            wireless.method = method;
            wireless.result = None;
            wireless.disconnect_result = None;
            Task::none()
        }
        Message::WirelessCodeRegenerated => {
            if state.flash.wireless.busy() {
                return Task::none();
            }

            regenerate_wireless_code(state);
            Task::none()
        }
        Message::WirelessServiceSelected(fullname) => {
            state.flash.wireless.chosen = Some(fullname);
            Task::none()
        }
        Message::WirelessCodeChanged(value) => {
            state.flash.wireless.code_input = value;
            Task::none()
        }
        Message::WirelessPairRequested => {
            let wireless = &state.flash.wireless;

            let Some(service) = wireless.manual_service() else {
                return Task::none();
            };

            let address = service.address_port();
            let code = wireless.code_input.trim().to_owned();

            if code.is_empty() {
                return Task::none();
            }

            start_wireless_pair(state, address, code)
        }
        Message::WirelessDiscovered(job, event) => {
            let wireless = &mut state.flash.wireless;

            if !wireless.open || wireless.job != job {
                return Task::none();
            }

            match event {
                wireless::DiscoverEvent::Found(service) => match service.kind {
                    wireless::ServiceKind::Pairing => {
                        upsert_wireless_service(&mut wireless.pairing_services, service)
                    }
                    wireless::ServiceKind::Connect => {
                        upsert_wireless_service(&mut wireless.connect_services, service)
                    }
                },
                wireless::DiscoverEvent::Lost(kind, fullname) => {
                    let services = match kind {
                        wireless::ServiceKind::Pairing => &mut wireless.pairing_services,
                        wireless::ServiceKind::Connect => &mut wireless.connect_services,
                    };

                    services.retain(|service| service.fullname != fullname);
                }
                wireless::DiscoverEvent::Failed(error) => {
                    wireless.discover_error = Some(error);
                    return Task::none();
                }
            }

            // A single phone offering a pairing is the one the user means, so
            // it is chosen for them.
            if wireless.chosen.is_none() {
                if let [only] = wireless.pairing_services.as_slice() {
                    wireless.chosen = Some(only.fullname.clone());
                }
            }

            // The phone that scanned the QR code announces the pairing service
            // the code named, so pairing needs no further click. A phone whose
            // name does not match scanned somebody else's code.
            if wireless.method != WirelessMethod::Qr
                || wireless.busy()
                || wireless.result.is_some()
            {
                return Task::none();
            }

            let attempt = wireless
                .scanned()
                .zip(wireless.pairing_code.as_ref())
                .map(|(service, code)| (service.address_port(), code.password.clone()));

            let Some((address, password)) = attempt else {
                return Task::none();
            };

            start_wireless_pair(state, address, password)
        }
        Message::WirelessFinished(result) => {
            let wireless = &mut state.flash.wireless;

            wireless.working = false;
            wireless.disconnect_result = None;

            // A connected phone can be dropped right away; the listing is
            // refreshed from `adb` straight after, and that is what the menu is
            // built from either way.
            if let Ok(wireless::PairOutcome::Connected(device)) = &result {
                let serial = device.serial.clone();

                if !wireless.devices.contains(&serial) {
                    wireless.devices.push(serial);
                }
            }

            wireless.result = Some(result);

            // The new connection changes what "Disconnect" can drop.
            start_wireless_devices()
        }
        Message::WirelessDisconnectSelected(target) => start_wireless_disconnect(state, target),
        Message::WirelessDisconnected(target, result) => {
            let wireless = &mut state.flash.wireless;

            wireless.disconnecting = false;
            wireless.result = None;
            wireless.disconnect_result = Some(match result {
                Ok(()) => DisconnectOutcome::Done(target),
                Err(error) => DisconnectOutcome::Failed(error),
            });

            // The listing decides what is left to drop.
            start_wireless_devices()
        }
        Message::WirelessDevicesFetched(devices) => {
            let wireless = &mut state.flash.wireless;

            if !wireless.open {
                return Task::none();
            }

            wireless.devices = devices;
            Task::none()
        }
        Message::WirelessCancel => {
            // Never close the dialog while a pairing runs, so the phone is not
            // left with a job nobody is watching.
            if state.flash.wireless.busy() {
                return Task::none();
            }

            stop_wireless(state);
            Task::none()
        }
        Message::WirelessBackdropPressed => Task::none(),
        Message::CaptivePortalDevicesFetched(result) => {
            let portal = &mut state.flash.captive_portal;

            if !portal.listing {
                return Task::none();
            }

            portal.listing = false;

            match result {
                Ok(devices) => {
                    portal.devices = devices;
                    portal.list_error = None;
                }
                Err(error) => {
                    portal.devices.clear();
                    portal.list_error = Some(error);
                }
            }

            Task::none()
        }
        Message::CaptivePortalRescan => {
            if state.flash.captive_portal.busy() {
                return Task::none();
            }

            state.flash.captive_portal = CaptivePortalState {
                open: true,
                listing: true,
                ..CaptivePortalState::default()
            };

            start_captive_portal_devices()
        }
        Message::CaptivePortalDeviceSelected(serial) => {
            let portal = &mut state.flash.captive_portal;

            if portal.busy() {
                return Task::none();
            }

            portal.selected = Some(serial);
            portal.result = None;
            Task::none()
        }
        Message::CaptivePortalRetry(serial) => start_captive_portal_retry(state, serial),
        Message::CaptivePortalFix => {
            let Some(serial) = state.flash.captive_portal.selected.clone() else {
                return Task::none();
            };

            start_captive_portal_fix(state, serial)
        }
        Message::CaptivePortalFixed(result) => {
            let portal = &mut state.flash.captive_portal;

            if !portal.fixing {
                return Task::none();
            }

            // The job runs detached on the device, so its result only says
            // that it was started — which is all the phone is asked for.
            portal.fixing = false;
            portal.result = Some(result);
            Task::none()
        }
        Message::CaptivePortalCancel => {
            // Never close the dialog while a fix runs: the start of the job is
            // what reports back, and the dialog would have nowhere to show it.
            if state.flash.captive_portal.busy() {
                return Task::none();
            }

            state.flash.captive_portal = CaptivePortalState::default();
            Task::none()
        }
        Message::CaptivePortalBackdropPressed => Task::none(),
        Message::UnpackPickImageRequested => {
            if state.unpack.busy() {
                return Task::none();
            }

            let title = state.l10n.tr("unpack-select-image");

            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        rfd::FileDialog::new()
                            .set_title(&title)
                            .add_filter("Image", &["img", "bin"])
                            .pick_file()
                    })
                    .await
                    .unwrap_or(None)
                },
                Message::UnpackImagePicked,
            )
        }
        Message::UnpackImagePicked(path) => {
            let Some(path) = path else {
                return Task::none();
            };

            let unpack = &mut state.unpack;
            if unpack.busy() {
                return Task::none();
            }

            // A new image starts over: the old table (and any result) belongs
            // to another file.
            unpack.image = Some(path.clone());
            unpack.entries = None;
            unpack.image_error = None;
            unpack.result = None;
            unpack.listing = true;

            // The path guards the answer: a file picked while an earlier one
            // is still being read must not be overwritten by its result.
            Task::perform(
                async move {
                    let listing = path.clone();
                    let result =
                        tokio::task::spawn_blocking(move || image_unpack::list(&listing))
                            .await
                            .unwrap_or_else(|error| {
                                Err(image_unpack::UnpackError::Read(format!(
                                    "background task failed: {error}"
                                )))
                            });

                    (path, result)
                },
                |(path, result)| Message::UnpackListed(path, result),
            )
        }
        Message::UnpackListed(path, result) => {
            let unpack = &mut state.unpack;

            if unpack.image.as_deref() != Some(path.as_path()) {
                return Task::none();
            }

            unpack.listing = false;

            match result {
                Ok(entries) => {
                    unpack.total_bytes = entries.iter().map(|entry| entry.size).sum();
                    unpack.done_bytes = 0;
                    unpack.entries = Some(entries);
                    unpack.image_error = None;
                }
                Err(error) => {
                    unpack.entries = None;
                    unpack.total_bytes = 0;
                    unpack.image_error = Some(error);
                }
            }

            Task::none()
        }
        Message::UnpackPickFolderRequested => {
            if state.unpack.busy() {
                return Task::none();
            }

            let title = state.l10n.tr("unpack-select-folder");

            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        rfd::FileDialog::new().set_title(&title).pick_folder()
                    })
                    .await
                    .unwrap_or(None)
                },
                Message::UnpackFolderPicked,
            )
        }
        Message::UnpackFolderPicked(folder) => {
            if let Some(folder) = folder {
                state.unpack.output = Some(folder);
            }

            Task::none()
        }
        Message::UnpackStart => start_unpack_job(state),
        Message::UnpackLocate => {
            // Only a finished run has files to find; the button is drawn in
            // that state.
            let Some(directory) = state.unpack.output.clone() else {
                return Task::none();
            };

            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || open::that(&directory))
                        .await
                        .unwrap_or_else(|error| {
                            Err(std::io::Error::new(
                                std::io::ErrorKind::Other,
                                error.to_string(),
                            ))
                        })
                },
                |result| {
                    if let Err(error) = result {
                        eprintln!("failed to open the output folder: {error}");
                    }

                    Message::UnpackFolderOpened
                },
            )
        }
        Message::UnpackFolderOpened => Task::none(),
        Message::UnpackEvent(job, event) => {
            let unpack = &mut state.unpack;

            // Stale events (a job the user left behind) are dropped.
            if job != unpack.job {
                return Task::none();
            }

            match event {
                image_unpack::UnpackEvent::File { name, .. } => {
                    unpack.current = Some(name);
                }
                image_unpack::UnpackEvent::Progress { bytes, total } => {
                    unpack.done_bytes = unpack.done_bytes.max(bytes);
                    unpack.total_bytes = unpack.total_bytes.max(total);
                }
                image_unpack::UnpackEvent::Finished(result) => {
                    unpack.working = false;
                    unpack.current = None;
                    unpack.result = Some(result);
                }
            }

            Task::none()
        }
        Message::UnpackWorkerDone => Task::none(),
        Message::UnpackCancel => {
            // Never close the dialog while the image is being read or written.
            if state.unpack.busy() {
                return Task::none();
            }

            state.unpack = UnpackState::default();
            Task::none()
        }
        Message::UnpackBackdropPressed => Task::none(),
        Message::PacPickFileRequested => {
            if state.pac.busy() {
                return Task::none();
            }

            let title = state.l10n.tr("pac-select-file");

            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        rfd::FileDialog::new()
                            .set_title(&title)
                            .add_filter("PAC", &["pac"])
                            .pick_file()
                    })
                    .await
                    .unwrap_or(None)
                },
                Message::PacFilePicked,
            )
        }
        Message::PacFilePicked(path) => {
            let Some(path) = path else {
                return Task::none();
            };

            let pac = &mut state.pac;
            if pac.busy() {
                return Task::none();
            }

            // A new package starts over: the old table (and any result) belongs
            // to another file.
            pac.file = Some(path.clone());
            pac.package = None;
            pac.selected.clear();
            pac.filter.clear();
            pac.file_error = None;
            pac.result = None;
            pac.listing = true;

            // The path guards the answer: a file picked while an earlier one is
            // still being read must not be overwritten by its result.
            Task::perform(
                async move {
                    let listing = path.clone();
                    let result = tokio::task::spawn_blocking(move || pac::list(&listing))
                        .await
                        .unwrap_or_else(|error| {
                            Err(pac::PacError::Read(format!(
                                "background task failed: {error}"
                            )))
                        });

                    (path, result)
                },
                |(path, result)| Message::PacListed(path, result),
            )
        }
        Message::PacListed(path, result) => {
            let pac = &mut state.pac;

            if pac.file.as_deref() != Some(path.as_path()) {
                return Task::none();
            }

            pac.listing = false;

            match result {
                Ok(package) => {
                    pac.total_bytes = package.files.iter().map(|entry| entry.size).sum();
                    pac.done_bytes = 0;
                    // Every file starts out checked: unpacking the whole
                    // package is the common case. Files a partial package cut
                    // short stay unchecked, so writing half an image takes a
                    // deliberate click.
                    pac.selected = package.files.iter().map(|entry| entry.complete).collect();
                    pac.package = Some(package);
                    pac.file_error = None;
                }
                Err(error) => {
                    pac.package = None;
                    pac.selected.clear();
                    pac.total_bytes = 0;
                    pac.file_error = Some(error);
                }
            }

            // Another package may leave the scripts without their download
            // agents.
            drop_spd_script_when_impossible(pac);

            Task::none()
        }
        Message::PacPickFolderRequested => {
            if state.pac.busy() {
                return Task::none();
            }

            let title = state.l10n.tr("unpack-select-folder");

            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        rfd::FileDialog::new().set_title(&title).pick_folder()
                    })
                    .await
                    .unwrap_or(None)
                },
                Message::PacFolderPicked,
            )
        }
        Message::PacFolderPicked(folder) => {
            if let Some(folder) = folder {
                state.pac.output = Some(folder);
            }

            Task::none()
        }
        Message::PacEntryToggled(index, checked) => {
            // The list is locked while the package is read or written.
            if state.pac.busy() {
                return Task::none();
            }

            if let Some(entry) = state.pac.selected.get_mut(index) {
                *entry = checked;
            }

            drop_spd_script_when_impossible(&mut state.pac);

            Task::none()
        }
        Message::PacSelectAll(checked) => {
            if state.pac.busy() {
                return Task::none();
            }

            // The buttons act on what the search box shows, like the bloatware
            // dialog's checklist does. Without a query that is every file.
            let visible: Vec<usize> = state
                .pac
                .package
                .as_ref()
                .map(|package| {
                    package
                        .files
                        .iter()
                        .enumerate()
                        .filter(|(_, entry)| pac::matches(&entry.name, &state.pac.filter))
                        .map(|(index, _)| index)
                        .collect()
                })
                .unwrap_or_default();

            for index in visible {
                if let Some(entry) = state.pac.selected.get_mut(index) {
                    *entry = checked;
                }
            }

            drop_spd_script_when_impossible(&mut state.pac);

            Task::none()
        }
        Message::PacFilterChanged(filter) => {
            state.pac.filter = filter;
            Task::none()
        }
        Message::PacStart => start_pac_job(state),
        Message::PacLocate => {
            // Only a finished run has files to find; the button is drawn in
            // that state.
            let Some(directory) = state.pac.output.clone() else {
                return Task::none();
            };

            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || open::that(&directory))
                        .await
                        .unwrap_or_else(|error| {
                            Err(std::io::Error::new(
                                std::io::ErrorKind::Other,
                                error.to_string(),
                            ))
                        })
                },
                |result| {
                    if let Err(error) = result {
                        eprintln!("failed to open the output folder: {error}");
                    }

                    Message::PacFolderOpened
                },
            )
        }
        Message::PacFolderOpened => Task::none(),
        Message::PacEvent(job, event) => {
            let pac = &mut state.pac;

            // Stale events (a job the user left behind) are dropped.
            if job != pac.job {
                return Task::none();
            }

            match event {
                PacJobEvent::File(name) => {
                    pac.current = Some(name);
                }
                PacJobEvent::Progress { bytes, total } => {
                    pac.done_bytes = pac.done_bytes.max(bytes);
                    pac.total_bytes = pac.total_bytes.max(total);
                }
                PacJobEvent::Finished { result, scripts } => {
                    pac.working = false;
                    pac.current = None;
                    pac.result = Some(result);
                    pac.scripts = Some(scripts);
                }
            }

            Task::none()
        }
        Message::PacWorkerDone => Task::none(),
        Message::PacScriptToggled(asked) => {
            if state.pac.busy() {
                return Task::none();
            }

            // The scripts name the two download agents, so they have to be part
            // of what the run writes: the option cannot be switched on without
            // them.
            let possible = state
                .pac
                .package
                .as_ref()
                .is_some_and(|package| spd_script::can_generate(package, &state.pac.selected));

            state.pac.spd_script = asked && possible;
            state.pac.scripts = None;

            Task::none()
        }
        Message::PacCancel => {
            // Never close the dialog while the package is being read or its
            // files are being written.
            if state.pac.busy() {
                return Task::none();
            }

            state.pac = PacState::default();
            Task::none()
        }
        Message::PacBackdropPressed => Task::none(),
        Message::PacRepackPickSourceRequested => {
            if state.pac_repack.busy() {
                return Task::none();
            }

            let title = state.l10n.tr("pac-repack-select-source");

            // The two files the Unisoc tools describe a firmware with, in one
            // dialog: which one it is, is told by its extension. Both kinds are
            // listed at once, so neither has to be hunted for by switching the
            // dialog's own filter.
            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        rfd::FileDialog::new()
                            .set_title(&title)
                            .add_filter("Flash configuration or scatter XML", &["cfg", "xml"])
                            .pick_file()
                    })
                    .await
                    .unwrap_or(None)
                },
                Message::PacRepackSourcePicked,
            )
        }
        Message::PacRepackSourcePicked(path) => {
            let Some(path) = path else {
                return Task::none();
            };

            let repack = &mut state.pac_repack;
            if repack.busy() {
                return Task::none();
            }

            // Another source starts over: the old records (and any result)
            // belong to another firmware.
            repack.source = Some(path.clone());
            repack.plan = None;
            repack.filter.clear();
            repack.offset = 0.0;
            repack.source_error = None;
            repack.output = None;
            repack.result = None;
            repack.reading = true;

            // The path guards the answer: a source picked while an earlier one
            // is still being read must not be overwritten by its result.
            Task::perform(
                async move {
                    let listing = path.clone();
                    let result = tokio::task::spawn_blocking(move || pac_build::plan(&listing))
                        .await
                        .unwrap_or_else(|error| {
                            Err(pac_build::BuildError::Read(format!(
                                "background task failed: {error}"
                            )))
                        });

                    (path, result)
                },
                |(path, result)| Message::PacRepackPlanned(path, result),
            )
        }
        Message::PacRepackPlanned(path, result) => {
            let repack = &mut state.pac_repack;

            if repack.source.as_deref() != Some(path.as_path()) {
                return Task::none();
            }

            repack.reading = false;

            match result {
                Ok(plan) => {
                    repack.done_bytes = 0;
                    repack.total_bytes = 0;
                    repack.plan = Some(plan);
                    repack.source_error = None;
                }
                Err(error) => {
                    repack.plan = None;
                    repack.source_error = Some(error);
                }
            }

            // The table of the source that was replaced is left where it was
            // scrolled to: another firmware starts at its first column.
            iced::widget::operation::scroll_to(
                iced::widget::Id::new(pac_repack::TABLE_ID),
                iced::widget::scrollable::AbsoluteOffset { x: 0.0, y: 0.0 },
            )
        }
        Message::PacRepackAliasChanged(alias) => {
            // The fields are drawn from the plan, so they can only be typed
            // into while one is loaded.
            if let Some(plan) = state.pac_repack.plan.as_mut() {
                plan.header.product_alias = alias;
            }

            Task::none()
        }
        Message::PacRepackVersionChanged(version) => {
            if let Some(plan) = state.pac_repack.plan.as_mut() {
                plan.header.product_version = version;
            }

            Task::none()
        }
        Message::PacRepackFilterChanged(filter) => {
            state.pac_repack.filter = filter;
            Task::none()
        }
        Message::PacRepackScrolled(offset) => {
            // The header of the table is drawn at this offset; nothing is
            // scrolled here, so no task is started.
            state.pac_repack.offset = offset;
            Task::none()
        }
        Message::PacRepackToggle(index, included) => {
            // The list is locked while the source is read or the package is
            // written.
            if state.pac_repack.busy() {
                return Task::none();
            }

            if let Some(plan) = state.pac_repack.plan.as_mut() {
                plan.set_included(index, included);
            }

            Task::none()
        }
        Message::PacRepackSelectAll(included) => {
            if state.pac_repack.busy() {
                return Task::none();
            }

            // The buttons act on what the search box shows, like the package
            // dialog's list does. Without a query that is every record.
            let filter = state.pac_repack.filter.clone();

            if let Some(plan) = state.pac_repack.plan.as_mut() {
                for index in pac_repack::visible(plan, &filter) {
                    plan.set_included(index, included);
                }
            }

            Task::none()
        }
        Message::PacRepackLeaveOut => {
            if state.pac_repack.busy() {
                return Task::none();
            }

            // What the source could not match would hold the build back: it is
            // still listed, just not written. The configuration XML is not the
            // user's to leave out either.
            if let Some(plan) = state.pac_repack.plan.as_mut() {
                for entry in &mut plan.entries {
                    if entry.is_missing() && !entry.is_configuration() {
                        entry.include = false;
                    }
                }
            }

            Task::none()
        }
        Message::PacRepackSelectFile(index) => {
            if state.pac_repack.busy() {
                return Task::none();
            }

            // The files of a scattered firmware sit next to the source, so the
            // picker opens there.
            let folder = state
                .pac_repack
                .plan
                .as_ref()
                .and_then(|plan| plan.source.parent())
                .map(std::path::Path::to_path_buf);
            let title = state.l10n.tr("pac-repack-select-file");

            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        let mut dialog = rfd::FileDialog::new().set_title(&title);

                        if let Some(folder) = folder {
                            dialog = dialog.set_directory(folder);
                        }

                        dialog.pick_file()
                    })
                    .await
                    .unwrap_or(None)
                },
                move |path| Message::PacRepackFilePicked(index, path),
            )
        }
        Message::PacRepackFilePicked(index, path) => {
            let Some(path) = path else {
                return Task::none();
            };

            if state.pac_repack.busy() {
                return Task::none();
            }

            if let Some(plan) = state.pac_repack.plan.as_mut() {
                if let Some(entry) = plan.entries.get_mut(index) {
                    // The table shows what the file is called and how large it
                    // is, so the record takes both from it.
                    entry.choose(path);
                }
            }

            Task::none()
        }
        Message::PacRepackStart => {
            let repack = &state.pac_repack;

            let Some(plan) = repack.plan.as_ref() else {
                return Task::none();
            };

            // The button is only drawn when all of this holds; asking again
            // guards against a build starting while the picker was open.
            if repack.busy()
                || repack.result.is_some()
                || plan.missing() > 0
                || plan.needs_metadata()
                || plan.chosen() == 0
            {
                return Task::none();
            }

            let name = plan.suggested_name();
            let folder = plan.source.parent().map(std::path::Path::to_path_buf);
            let title = state.l10n.tr("pac-repack-save");

            // The package is one file: the save dialog asks for its name, and
            // the build starts with the answer, so no run can start without a
            // destination — replacing the file it writes is the user's call.
            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        let mut dialog = rfd::FileDialog::new()
                            .set_title(&title)
                            .set_file_name(&name)
                            .add_filter("PAC", &["pac"]);

                        if let Some(folder) = folder {
                            dialog = dialog.set_directory(folder);
                        }

                        dialog.save_file()
                    })
                    .await
                    .unwrap_or(None)
                },
                Message::PacRepackOutputChosen,
            )
        }
        Message::PacRepackOutputChosen(path) => {
            let Some(path) = path else {
                return Task::none();
            };

            if state.pac_repack.busy() {
                return Task::none();
            }

            state.pac_repack.output = Some(path);
            start_pac_repack_job(state)
        }
        Message::PacRepackLocate => {
            // Only a finished run has a package to find; the button is drawn in
            // that state.
            let Some(folder) = state
                .pac_repack
                .output
                .as_deref()
                .and_then(std::path::Path::parent)
                .map(std::path::Path::to_path_buf)
            else {
                return Task::none();
            };

            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || open::that(&folder))
                        .await
                        .unwrap_or_else(|error| {
                            Err(std::io::Error::new(
                                std::io::ErrorKind::Other,
                                error.to_string(),
                            ))
                        })
                },
                |result| {
                    if let Err(error) = result {
                        eprintln!("failed to open the output folder: {error}");
                    }

                    Message::PacRepackFolderOpened
                },
            )
        }
        Message::PacRepackFolderOpened => Task::none(),
        Message::PacRepackEvent(job, event) => {
            let repack = &mut state.pac_repack;

            // Stale events (a job the user left behind) are dropped.
            if job != repack.job {
                return Task::none();
            }

            match event {
                PacRepackEvent::File(name) => repack.current = Some(name),
                PacRepackEvent::Progress { bytes, total } => {
                    repack.done_bytes = repack.done_bytes.max(bytes);
                    repack.total_bytes = repack.total_bytes.max(total);
                }
                PacRepackEvent::Finished(result) => {
                    repack.working = false;
                    repack.current = None;
                    repack.result = Some(result);
                }
            }

            Task::none()
        }
        Message::PacRepackWorkerDone => Task::none(),
        Message::PacRepackCancel => {
            // Never close the dialog while the source is being read or the
            // package is written.
            if state.pac_repack.busy() {
                return Task::none();
            }

            state.pac_repack = PacRepackState::default();
            Task::none()
        }
        Message::PacRepackBackdropPressed => Task::none(),
        Message::FirmwareFlashExport => request_firmware_flash_export(state),
        Message::FirmwareFlashExportCopy(copy) => {
            start_firmware_flash_export(state, copy)
        }
        Message::FirmwareFlashExported(result) => {
            let flash = &mut state.flash.firmware;

            match result {
                // Nothing to say about a cancelled save dialog.
                Ok(None) => {}
                Ok(Some(path)) => flash.export_result = Some(Ok(path)),
                Err(error) => flash.export_result = Some(Err(error)),
            }

            Task::none()
        }
        Message::FirmwareFlashReadInfo => start_firmware_flash_read_info(state),
        Message::FirmwareFlashInfoRead(result) => {
            let flash = &mut state.flash.firmware;
            flash.reading_info = false;

            match result {
                Ok(lines) => flash.info_lines = lines,
                Err(error) => flash.info_error = Some(error),
            }

            Task::none()
        }
        Message::FirmwareFlashInfoSensitiveToggled(hidden) => {
            state.flash.firmware.hide_sensitive = hidden;
            Task::none()
        }
        Message::FirmwareFlashInfoCopy => {
            let flash = &state.flash.firmware;
            let text = info_text(flash);

            if text.is_empty() {
                return Task::none();
            }

            iced::clipboard::write::<Message>(text)
        }
        Message::FirmwareFlashInfoClosed => {
            let flash = &mut state.flash.firmware;
            flash.info_view = false;
            flash.info_lines.clear();
            flash.info_error = None;
            flash.hide_sensitive = false;
            Task::none()
        }
        Message::FirmwareFlashReboot => {
            start_firmware_flash_reboot(state, flash_engine::RebootMode::System)
        }
        Message::FirmwareFlashRebootSelected(mode) => start_firmware_flash_reboot(state, mode),
        Message::FirmwareFlashRebooted(job, result) => {
            let flash = &mut state.flash.firmware;
            // Outcome of a reboot this dialog no longer runs.
            if job != flash.job {
                return Task::none();
            }

            flash.rebooting = false;

            if let Err(error) = &result {
                push_flash_log(flash, format!("reboot failed: {error}"));
            }

            flash.reboot_result = Some(result);
            scroll_flash_log_to_end()
        }
        Message::FirmwareFlashLocateLog => {
            // The file this run writes; a dialog that only rebooted a phone has
            // a log on screen but no file of its own — and a log file that
            // could not be created has none either — so the folder both modes
            // write into is opened for those.
            let path = state
                .flash
                .firmware
                .log_file
                .as_ref()
                .map(|file| file.path().to_path_buf())
                .filter(|path| path.is_file())
                .unwrap_or_else(flash_log::logs_root);

            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || flash_log::reveal(&path))
                        .await
                        .unwrap_or_else(|error| Err(error.to_string()))
                },
                |result| {
                    if let Err(error) = result {
                        eprintln!("failed to open the log folder: {error}");
                    }

                    Message::FirmwareFlashLogOpened
                },
            )
        }
        Message::FirmwareFlashLogOpened => Task::none(),
        Message::BootloaderManualSelected => {
            state.flash.bootloader.dialog = BootloaderDialog::Manual;
            Task::none()
        }
        Message::BootloaderGuidedSelected => {
            let bootloader = &mut state.flash.bootloader;
            bootloader.dialog = BootloaderDialog::Guided;
            bootloader.picker = None;
            bootloader.pending_key = None;
            bootloader.reading = false;
            bootloader.unlocking = false;
            bootloader.checking = false;
            bootloader.eligible = None;
            bootloader.read_error = None;
            bootloader.read_info = None;
            bootloader.unlock_error = None;
            bootloader.unlock_notice = None;
            bootloader.unlock_info = None;
            bootloader.device_id.clear();
            bootloader.key_input.clear();
            bootloader.guided = GuidedState::default();
            Task::none()
        }
        Message::BootloaderReturnToChooser => {
            let bootloader = &mut state.flash.bootloader;
            bootloader.dialog = BootloaderDialog::Choosing;
            bootloader.picker = None;
            Task::none()
        }
        Message::BootloaderCancel => {
            let bootloader = &mut state.flash.bootloader;
            bootloader.dialog = BootloaderDialog::Closed;
            bootloader.picker = None;
            bootloader.pending_key = None;
            bootloader.reading = false;
            bootloader.unlocking = false;
            Task::none()
        }
        // Clicks on empty modal space are swallowed here so they neither
        // dismiss the dialog nor reach the UI underneath.
        Message::BootloaderBackdropPressed => Task::none(),
        Message::BootloaderOpenSite => open_browser(fastboot_info::UNLOCK_PAGE_URL),
        Message::BootloaderCopyDeviceId => {
            let device_id = state.flash.bootloader.device_id.clone();
            if device_id.is_empty() {
                Task::none()
            } else {
                iced::clipboard::write::<Message>(device_id)
            }
        }
        Message::BootloaderReadRequested => {
            start_bootloader_list_devices(state, BootloaderDeviceAction::ReadDeviceId)
        }
        Message::BootloaderUnlockRequested => {
            start_bootloader_list_devices(state, BootloaderDeviceAction::Unlock)
        }
        Message::BootloaderDevicesFetched(result) => {
            finish_bootloader_device_list(state, result)
        }
        Message::BootloaderDeviceSelected(serial) => run_bootloader_picked_device(state, serial),
        Message::BootloaderPickerCancelled => {
            state.flash.bootloader.picker = None;
            Task::none()
        }
        Message::BootloaderDeviceIdRead(result) => finish_bootloader_read(state, result),
        Message::BootloaderEligibilityChecked(result) => {
            let bootloader = &mut state.flash.bootloader;
            if !bootloader.checking {
                return Task::none();
            }

            bootloader.checking = false;
            match result {
                Ok(eligible) => bootloader.eligible = Some(eligible),
                // No Internet connection (or any other check failure): stay
                // silent rather than showing a misleading error banner.
                Err(error) => {
                    eprintln!("[bootloader] unlock eligibility check failed: {error}");
                }
            }
            Task::none()
        }
        Message::BootloaderKeyChanged(input) => {
            let bootloader = &mut state.flash.bootloader;
            bootloader.key_input = input;
            bootloader.unlock_error = None;
            Task::none()
        }
        // Right-click on the unlock-key field: read the clipboard and put the
        // text in (for users who don't use Ctrl+V).
        Message::BootloaderPasteRequested => {
            if !matches!(
                state.flash.bootloader.dialog,
                BootloaderDialog::Manual | BootloaderDialog::Guided
            ) {
                return Task::none();
            }
            iced::clipboard::read().map(Message::BootloaderPasteFetched)
        }
        Message::BootloaderPasteFetched(clipboard) => {
            if let Some(text) = clipboard {
                let bootloader = &mut state.flash.bootloader;
                bootloader.key_input = text;
                bootloader.unlock_error = None;
            }
            Task::none()
        }
        Message::BootloaderUnlockFinished(result) => finish_bootloader_unlock(state, result),
        Message::TabletUnlockSelected => {
            // Keep the (monotonic) guided job counter so a still-running job's
            // events and report stay stale.
            let job = state.flash.tablet_unlock.guided.job;
            state.flash.tablet_unlock = TabletUnlockState {
                dialog: TabletUnlockDialog::Choosing,
                ..TabletUnlockState::default()
            };
            state.flash.tablet_unlock.guided.job = job;
            Task::none()
        }
        Message::TabletUnlockManualSelected => {
            state.flash.tablet_unlock.dialog = TabletUnlockDialog::Manual;
            Task::none()
        }
        Message::TabletUnlockReturnToChooser => {
            let tablet = &mut state.flash.tablet_unlock;
            tablet.dialog = TabletUnlockDialog::Choosing;
            tablet.picker = None;
            tablet.guided.listing = false;
            tablet.guided.unlocking = false;
            stop_tablet_unlock(tablet);
            stop_tablet_guided(&mut tablet.guided);
            Task::none()
        }
        Message::TabletUnlockCancel => {
            let tablet = &mut state.flash.tablet_unlock;
            tablet.dialog = TabletUnlockDialog::Closed;
            tablet.picker = None;
            tablet.reading = false;
            tablet.guided.listing = false;
            tablet.guided.unlocking = false;
            stop_tablet_unlock(tablet);
            stop_tablet_guided(&mut tablet.guided);
            Task::none()
        }
        // Clicks on empty modal space are swallowed here so they neither
        // dismiss the dialog nor reach the UI underneath.
        Message::TabletUnlockBackdropPressed => Task::none(),
        Message::TabletUnlockOpenSite => open_browser(tablet_unlock::UNLOCK_SITE_URL),
        Message::TabletUnlockCopySerial => {
            let serial = state.flash.tablet_unlock.serial.clone();
            if serial.is_empty() {
                Task::none()
            } else {
                iced::clipboard::write::<Message>(serial)
            }
        }
        Message::TabletUnlockCopySn => {
            let sn = state.flash.tablet_unlock.bootloader_sn.clone();
            if sn.is_empty() {
                Task::none()
            } else {
                iced::clipboard::write::<Message>(sn)
            }
        }
        Message::TabletUnlockReadRequested => start_tablet_read(state),
        Message::TabletUnlockDevicesFetched(result) => finish_tablet_device_list(state, result),
        Message::TabletUnlockDeviceSelected(serial) => start_tablet_read_device(state, serial),
        Message::TabletUnlockPickerCancelled => {
            state.flash.tablet_unlock.picker = None;
            Task::none()
        }
        Message::TabletUnlockInfoRead(device, result) => {
            finish_tablet_read(state, device, result)
        }
        Message::TabletUnlockRequested => start_tablet_unlock(state),
        Message::TabletUnlockCancelRestart => cancel_tablet_unlock(state),
        Message::TabletUnlockFinished(outcome) => finish_tablet_unlock(state, outcome),
        Message::TabletGuidedSelected => {
            let tablet = &mut state.flash.tablet_unlock;
            let job = tablet.guided.job;
            tablet.dialog = TabletUnlockDialog::Guided;
            tablet.picker = None;
            tablet.guided = TabletGuidedState {
                job,
                step: TabletGuidedStep::Devices,
                listing: true,
                ..TabletGuidedState::default()
            };
            start_tablet_guided_listing()
        }
        Message::TabletGuidedRefresh => {
            let guided = &mut state.flash.tablet_unlock.guided;
            if guided.listing || guided.unlocking {
                return Task::none();
            }
            guided.step = TabletGuidedStep::Devices;
            guided.listing = true;
            guided.list_error = None;
            guided.devices.clear();
            start_tablet_guided_listing()
        }
        Message::TabletGuidedDevicesFetched(result) => finish_tablet_guided_list(state, result),
        Message::TabletGuidedDeviceSelected(serial) => start_tablet_guided_unlock(state, serial),
        Message::TabletGuidedEvent(job, event) => {
            handle_tablet_guided_event(state, job, event);
            Task::none()
        }
        Message::TabletGuidedFinished(job, report) => finish_tablet_guided(state, job, report),
        Message::TabletGuidedCancelRestart => cancel_tablet_guided(state),
        Message::GuidedLogin => {
            let guided = &mut state.flash.bootloader.guided;
            // Only one portal window (or logout) at a time. When already
            // logged in the button means "change account": the current session
            // is signed out first so the portal shows its login page again
            // instead of bouncing straight to the logged-in account profile.
            if guided.logging_in || guided.logging_out {
                return Task::none();
            }
            guided.login_error = None;

            if guided.logged_in && !guided.cookie_header.trim().is_empty() {
                guided.logging_out = true;
                let cookie = guided.cookie_header.clone();
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || firmware::logout_portal(&cookie))
                            .await
                            .unwrap_or_else(|e| Err(format!("background task failed: {e}")))
                    },
                    Message::GuidedLogoutFinished,
                );
            }

            guided.logging_in = true;
            let title = state.l10n.tr("guided-login-dialog-title");
            let url = firmware::PORTAL_LOGIN_URL.to_string();

            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        webview::show_portal_login(&title, &url)
                    })
                    .await
                    .unwrap_or_else(|e| Err(format!("background task failed: {e}")))
                },
                Message::GuidedLoginFinished,
            )
        }
        Message::GuidedLogoutFinished(result) => {
            let guided = &mut state.flash.bootloader.guided;
            guided.logging_out = false;
            match result {
                Ok(()) => {
                    // The old session is invalid now. Drop it and open the
                    // portal login page fresh so a different account can be
                    // chosen.
                    guided.logged_in = false;
                    guided.account_name = None;
                    guided.account_email = None;
                    guided.cookie_header.clear();
                    guided.logging_in = true;
                    let title = state.l10n.tr("guided-login-dialog-title");
                    let url = firmware::PORTAL_LOGIN_URL.to_string();
                    Task::perform(
                        async move {
                            tokio::task::spawn_blocking(move || {
                                webview::show_portal_login(&title, &url)
                            })
                            .await
                            .unwrap_or_else(|e| Err(format!("background task failed: {e}")))
                        },
                        Message::GuidedLoginFinished,
                    )
                }
                Err(error) => {
                    // Logout failed: keep the current session and surface the
                    // error next to the "Change Account" button.
                    guided.login_error = Some(error);
                    Task::none()
                }
            }
        }
        Message::GuidedLoginFinished(result) => match result {
            Ok(login) => {
                let guided = &mut state.flash.bootloader.guided;
                guided.logging_in = false;
                // The profile fetch and the unlock-key request both need the
                // portal session cookie. On platforms where the webview can't
                // expose cookies (e.g. Linux GTK) a "successful" login is
                // useless, so report it as a failure instead of pretending the
                // user is signed in.
                if login.cookie_header.trim().is_empty() {
                    let message = state.l10n.tr("guided-login-required");
                    guided.logged_in = false;
                    guided.login_error = Some(message);
                    return Task::none();
                }
                guided.logged_in = true;
                guided.login_error = None;
                guided.cookie_header = login.cookie_header.clone();
                guided.fetching_account = true;

                #[cfg(debug_assertions)]
                {
                    eprintln!("[guided] portal login landed on: {}", login.url);
                    let names: Vec<&str> = login
                        .cookie_header
                        .split(';')
                        .filter_map(|c| c.trim().split_once('=').map(|(n, _)| n))
                        .collect();
                    eprintln!(
                        "[guided] portal session cookies ({}): {}",
                        names.len(),
                        names.join(", ")
                    );
                }

                let cookie = login.cookie_header;
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            firmware::fetch_portal_profile(&cookie)
                        })
                        .await
                        .unwrap_or_else(|e| Err(format!("background task failed: {e}")))
                    },
                    Message::GuidedAccountFetched,
                )
            }
            Err(error) => {
                let guided = &mut state.flash.bootloader.guided;
                guided.logging_in = false;
                guided.login_error = Some(error);
                Task::none()
            }
        },
        Message::GuidedAccountFetched(result) => {
            let guided = &mut state.flash.bootloader.guided;
            guided.fetching_account = false;
            match result {
                Ok((name, email)) => {
                    guided.account_name = Some(name);
                    guided.account_email = Some(email);
                    guided.login_error = None;
                }
                Err(error) => {
                    // Login itself succeeded; only the profile info is missing.
                    eprintln!("[guided] failed to fetch portal profile: {error}");
                }
            }
            Task::none()
        }
        Message::GuidedNext => {
            let guided = &mut state.flash.bootloader.guided;
            // Don't advance past the login step while a sign-in window or a
            // "change account" sign-out is still in progress.
            if guided.step == GuidedStep::Login
                && guided.logged_in
                && !guided.logging_in
                && !guided.logging_out
            {
                guided.step = GuidedStep::Read;
            }
            Task::none()
        }
        Message::GuidedRequestKey => {
            let bootloader = &mut state.flash.bootloader;
            let guided = &mut bootloader.guided;
            // Triggered from the Read step (first request) or, after a failed
            // request, from the "Try Again" button on the Request step.
            let can_request = match guided.step {
                GuidedStep::Read => !bootloader.device_id.trim().is_empty(),
                GuidedStep::Request => guided.request_error.is_some(),
                _ => false,
            };
            if can_request && !guided.confirming_request && !guided.requesting {
                guided.step = GuidedStep::Request;
                guided.confirming_request = true;
                guided.request_error = None;
            }
            Task::none()
        }
        Message::GuidedRequestConfirmYes => {
            let bootloader = &mut state.flash.bootloader;
            let device_id = bootloader.device_id.clone();
            let cookie = bootloader.guided.cookie_header.clone();
            let guided = &mut bootloader.guided;

            guided.confirming_request = false;
            if cookie.is_empty() {
                // No usable portal session: send the user back to sign in
                // rather than looping on the confirm prompt.
                let message = state.l10n.tr("guided-login-required");
                guided.logged_in = false;
                guided.account_name = None;
                guided.account_email = None;
                guided.login_error = Some(message);
                guided.step = GuidedStep::Login;
                return Task::none();
            }

            guided.requesting = true;
            guided.request_error = None;

            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        firmware::request_unlock_key(&device_id, &cookie)
                    })
                    .await
                    .unwrap_or_else(|e| Err(format!("background task failed: {e}")))
                },
                Message::GuidedKeyRequestFinished,
            )
        }
        Message::GuidedRequestConfirmNo => {
            // "No": do not request the key — go back to Step 2 (Read) so the
            // user can re-read or change their mind.
            let bootloader = &mut state.flash.bootloader;
            let guided = &mut bootloader.guided;
            guided.confirming_request = false;
            guided.request_error = None;
            guided.step = GuidedStep::Read;
            Task::none()
        }
        Message::GuidedKeyRequestFinished(result) => {
            let guided = &mut state.flash.bootloader.guided;
            guided.requesting = false;
            match result {
                Ok(()) => {
                    guided.request_error = None;
                    guided.step = GuidedStep::Key;
                }
                Err(error) => guided.request_error = Some(error),
            }
            Task::none()
        }
        Message::LanguageSelected(language) => {
            state.lang = language;
            state.l10n = l10n::bundle_for(language);

            if let Err(error) = config::save_language(language) {
                eprintln!("failed to save language: {error}");
            }

            Task::none()
        }
        Message::LoginRequested(click) => request_login(state, click),
        Message::LoginUrlFetched(result) => {
            let click = match state.login {
                LoginStatus::Fetching { click } => click,
                _ => return Task::none(),
            };

            match result {
                Err(error) => {
                    state.login = LoginStatus::Error(error);
                    Task::none()
                }
                Ok(url) => match click {
                    Click::Right => {
                        state.login = LoginStatus::Manual {
                            url: url.clone(),
                            input: String::new(),
                            notice: None,
                        };
                        Task::none()
                    }
                    Click::Left => {
                        // When LMN Flash owns the `softwarefix://` scheme the
                        // browser can hand the callback straight back to us, so
                        // the embedded WebView is not needed.
                        if browser_login_available(state) {
                            state.login = LoginStatus::WaitingForBrowser { url: url.clone() };
                            return open_browser(&url);
                        }

                        start_webview_login(state, url)
                    }
                },
            }
        }
        Message::WebViewFinished(result) => {
            let url = match &state.login {
                LoginStatus::WebViewOpen { url } => url.clone(),
                _ => return Task::none(),
            };

            match result {
                Ok(callback) => match login::extract_login(&callback) {
                    Ok(info) => {
                        if let Err(error) =
                            config::save_credentials(&info.token, &state.client_uuid)
                        {
                            eprintln!("failed to save credentials: {error}");
                        }
                        state.login = LoginStatus::LoggedIn {
                            token: info.token,
                            full_name: info.full_name,
                        };
                        if state.lookup.retry_after_login {
                            state.lookup.retry_after_login = false;
                            return request_lookup(state);
                        }
                    }
                    Err(error) => state.login = LoginStatus::Error(error),
                },
                Err(error) => {
                    // WebView unavailable or dialog closed: fall back to
                    // manual login, showing the login URL in the UI.
                    eprintln!("[webview] login dialog failed: {error}");
                    state.login = LoginStatus::Manual {
                        url: url.clone(),
                        input: String::new(),
                        notice: Some(error),
                    };
                }
            }

            Task::none()
        }
        Message::ManualInputChanged(input) => {
            if let LoginStatus::Manual { input: field, .. } = &mut state.login {
                *field = input;
            }
            Task::none()
        }
        Message::SubmitManual => {
            let input = match &state.login {
                LoginStatus::Manual { input, .. } => input.clone(),
                _ => return Task::none(),
            };

            match login::extract_login(&input) {
                Ok(info) => {
                    if let Err(error) = config::save_credentials(&info.token, &state.client_uuid) {
                        eprintln!("failed to save credentials: {error}");
                    }
                    state.login = LoginStatus::LoggedIn {
                        token: info.token,
                        full_name: info.full_name,
                    };
                    if state.lookup.retry_after_login {
                        state.lookup.retry_after_login = false;
                        return request_lookup(state);
                    }
                }
                Err(error) => state.login = LoginStatus::Error(error),
            }

            Task::none()
        }
        Message::LoginWebViewRetry => {
            // The browser login is stuck (typically an old lenovo.com session
            // answering the sign-in page with HTTP 400), so run the login in
            // the embedded WebView instead: it has its own cookie jar.
            let url = match &state.login {
                LoginStatus::WaitingForBrowser { url } => url.clone(),
                _ => return Task::none(),
            };

            start_webview_login(state, url)
        }
        Message::OpenBrowser => {
            let url = match &state.login {
                LoginStatus::Manual { url, .. } | LoginStatus::WaitingForBrowser { url } => {
                    url.clone()
                }
                _ => return Task::none(),
            };
            open_browser(&url)
        }
        Message::LoginCallback(url) => apply_login_callback(state, &url),
        Message::CopyUrl => {
            let url = match &state.login {
                LoginStatus::Manual { url, .. } => url.clone(),
                _ => return Task::none(),
            };
            iced::clipboard::write::<Message>(url)
        }
        Message::CopyDownloadUri(uri) => iced::clipboard::write::<Message>(uri),
        Message::CopyToolUri(uri) => iced::clipboard::write::<Message>(uri),
        Message::CopyRawJson(raw) => iced::clipboard::write::<Message>(raw),
        Message::TelemetrySent(result) => {
            if cfg!(debug_assertions) {
                if let Err(error) = result {
                    eprintln!("[telemetry] {error}");
                }
            }
            Task::none()
        }
        Message::CancelLogin => {
            state.login = LoginStatus::LoggedOut;
            state.lookup.retry_after_login = false;
            Task::none()
        }
        Message::LookupModeSelected(mode) => {
            state.lookup.mode = mode;
            state.lookup.status = LookupStatus::Idle;
            state.lookup.pending_telemetry = None;
            Task::none()
        }
        Message::ImeiInputChanged(input) => {
            state.lookup.imei_input = input;
            Task::none()
        }
        Message::BulkEdit(action) => {
            if let Some(action) = bulk::bulk_digits_only(action) {
                state.lookup.bulk.content.perform(action);
            }
            Task::none()
        }
        Message::RetcnPlatformSelected(platform) => {
            state.lookup.retcn.platform = platform;
            Task::none()
        }
        Message::RetcnSimCountSelected(sim_count) => {
            state.lookup.retcn.sim_count = sim_count;
            Task::none()
        }
        Message::RetcnFieldChanged(field, value) => {
            match field {
                RetcnField::SerialNumber => state.lookup.retcn.serial_number = value,
                RetcnField::Fingerprint => state.lookup.retcn.fingerprint = value,
                RetcnField::Model => state.lookup.retcn.model = value,
                RetcnField::Carrier => state.lookup.retcn.carrier = value,
                RetcnField::FsgVersion => state.lookup.retcn.fsg_version = value,
            }
            Task::none()
        }
        Message::FastbootFillRequested => {
            if !matches!(state.lookup.retcn.device_picker, DevicePicker::Closed) {
                return Task::none();
            }

            state.lookup.retcn.device_picker = DevicePicker::Fetching;
            state.lookup.retcn.fastboot_status = Some(FastbootStatus::Fetching);

            Task::perform(
                async {
                    tokio::task::spawn_blocking(fastboot_info::list_devices)
                        .await
                        .unwrap_or_else(|e| {
                            Err(format!("background task failed: {e}"))
                        })
                },
                Message::FastbootDevicesFetched,
            )
        }
        Message::FastbootDevicesFetched(result) => match result {
            Ok(list) if list.devices.len() == 1 => {
                state.lookup.retcn.device_picker = DevicePicker::Closed;
                let serial = list.devices[0].serial.clone();
                start_fastboot_fill(serial)
            }
            Ok(list) if list.devices.len() > 1 => {
                state.lookup.retcn.fastboot_status = None;
                state.lookup.retcn.device_picker = DevicePicker::Open(list.devices);
                Task::none()
            }
            Ok(list) => {
                // No supported device: distinguish "nothing at all" from
                // "only unsupported (non-Motorola) devices".
                state.lookup.retcn.device_picker = DevicePicker::Closed;

                let error_id = if list.total == 0 {
                    "retcn-fill-fastboot-no-device"
                } else {
                    "retcn-fill-fastboot-unsupported-device"
                };
                state.lookup.retcn.fastboot_status =
                    Some(FastbootStatus::Error(state.l10n.tr(error_id)));
                Task::none()
            }
            Err(error) => {
                state.lookup.retcn.device_picker = DevicePicker::Closed;
                state.lookup.retcn.fastboot_status = Some(FastbootStatus::Error(error));
                Task::none()
            }
        },
        Message::FastbootDeviceSelected(serial) => {
            state.lookup.retcn.device_picker = DevicePicker::Closed;
            state.lookup.retcn.fastboot_status = Some(FastbootStatus::Fetching);
            start_fastboot_fill(serial)
        }
        Message::FastbootDevicePickerCancelled => {
            state.lookup.retcn.device_picker = DevicePicker::Closed;
            state.lookup.retcn.fastboot_status = None;
            Task::none()
        }
        Message::FastbootFillFinished(result) => {
            match result {
                Ok((serial, info)) => {
                    // Only overwrite fields the device actually reported,
                    // so partial data does not wipe user input.
                    if !info.serial_number.is_empty() {
                        state.lookup.retcn.serial_number = info.serial_number;
                    }
                    if !info.imei.is_empty() {
                        state.lookup.imei_input = info.imei;
                    }
                    if !info.model.is_empty() {
                        state.lookup.retcn.model = info.model;
                    }
                    if !info.carrier.is_empty() {
                        state.lookup.retcn.carrier = info.carrier;
                    }
                    if !info.fingerprint.is_empty() {
                        state.lookup.retcn.fingerprint = info.fingerprint;
                    }
                    if !info.fsg_version.is_empty() {
                        state.lookup.retcn.fsg_version = info.fsg_version;
                    }
                    if let Some(platform) = info.platform {
                        state.lookup.retcn.platform = platform;
                    }
                    if let Some(sim_count) = info.sim_count {
                        state.lookup.retcn.sim_count = match sim_count {
                            2 => SimCount::Dual,
                            _ => SimCount::Single,
                        };
                    }

                    state.lookup.retcn.fastboot_status =
                        Some(FastbootStatus::Filled(serial));
                }
                Err(error) => {
                    state.lookup.retcn.fastboot_status =
                        Some(FastbootStatus::Error(error));
                }
            }

            Task::none()
        }
        Message::RetcnLookupRequested => request_retcn_lookup(state),
        Message::TabletSnChanged(input) => {
            state.lookup.tablet.serial_number = input;
            Task::none()
        }
        Message::TabletLookupRequested => request_tablet_lookup(state),
        Message::TabletLookupFinished(result) => {
            if !matches!(state.lookup.status, LookupStatus::Fetching) {
                return Task::none();
            }

            match result {
                Ok(result) => {
                    let telemetry =
                        tablet_telemetry(state.lookup.pending_telemetry.take(), &result);
                    state.lookup.status = LookupStatus::Done(result);
                    telemetry
                }
                Err(error) => {
                    state.lookup.status = LookupStatus::Error(error);
                    Task::none()
                }
            }
        }
        Message::ModelInputChanged(input) => {
            let by_model = &mut state.lookup.by_model;
            by_model.model = input;
            if by_model.category_auto {
                if let Some((category, country)) = detect_category(&by_model.model) {
                    by_model.category = category;
                    if let Some(country) = country {
                        by_model.country_code = country.to_string();
                    }
                }
            }
            Task::none()
        }
        Message::ModelCategorySelected(category) => {
            state.lookup.by_model.category = category;
            state.lookup.by_model.category_auto = false;
            Task::none()
        }
        Message::ModelCountryChanged(input) => {
            state.lookup.by_model.country_code = input;
            Task::none()
        }
        Message::ModelLookupRequested => request_model_lookup(state),
        Message::ModelParamChanged(index, value) => {
            if let Some(slot) = state.lookup.by_model.values.get_mut(index) {
                *slot = value;
            }
            Task::none()
        }
        Message::ModelMatchParamsFetched(result) => {
            match result {
                Ok(match_params) => {
                    let by_model = &mut state.lookup.by_model;
                    by_model.loaded_model = by_model.model.clone();
                    by_model.required = match_params.params;
                    by_model.values = vec![String::new(); by_model.required.len()];
                    state.lookup.status = LookupStatus::Idle;
                }
                Err(firmware::FirmwareError::AuthExpired(message)) => {
                    // Token expired: forget it, log in again, and re-run the
                    // lookup automatically once the new login succeeds.
                    eprintln!("[lookup] token expired: {message}");
                    state.login = LoginStatus::LoggedOut;
                    state.lookup.status = LookupStatus::Idle;
                    state.lookup.retry_after_login = true;
                    return request_login(state, Click::Left);
                }
                Err(firmware::FirmwareError::Other(error)) => {
                    state.lookup.status = LookupStatus::Error(error);
                }
            }

            Task::none()
        }
        Message::CopyCnPassword(password) => iced::clipboard::write::<Message>(password),
        Message::BulkLookupRequested => bulk::start_bulk_lookup(state),
        Message::BulkStepFinished(imei, result) => match result {
            Ok(info) => {
                state.lookup.bulk.rows.push(BulkRow::found(&imei, &info));
                state.lookup.bulk.index += 1;
                bulk::bulk_next(state)
            }
            Err(firmware::FirmwareError::AuthExpired(message)) => {
                // Token expired mid-batch: stop, but keep the rows collected
                // so far (the Save button stays) for the user to save.
                eprintln!("[lookup] token expired during bulk lookup: {message}");
                state.lookup.bulk.status = BulkStatus::Error(message);
                Task::none()
            }
            Err(error) => {
                let status = if firmware::is_no_resource(&error) {
                    BULK_STATUS_MISSING
                } else {
                    BULK_STATUS_FAILED
                };
                state.lookup.bulk.rows.push(BulkRow::skipped(&imei, status));
                state.lookup.bulk.index += 1;
                bulk::bulk_next(state)
            }
        },
        Message::BulkSaveRequested => bulk::start_bulk_save(state),
        Message::BulkSaveFinished(result) => {
            state.lookup.bulk.save_note = match result {
                Ok(Some(path)) => Some(Ok(path.display().to_string())),
                Ok(None) => None,
                Err(error) => Some(Err(error)),
            };
            Task::none()
        }
        Message::DecryptPickDirRequested => {
            if matches!(
                state.decrypt.status,
                DecryptStatus::PickingDir | DecryptStatus::Working
            ) {
                return Task::none();
            }

            state.decrypt.status = DecryptStatus::PickingDir;
            let title = state.l10n.tr("decrypt-select-dir");

            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        rfd::FileDialog::new().set_title(&title).pick_folder()
                    })
                    .await
                    .unwrap_or(None)
                },
                Message::DecryptDirPicked,
            )
        }
        Message::DecryptDirPicked(directory) => {
            state.decrypt.directory = directory;
            state.decrypt.status = DecryptStatus::Idle;
            Task::none()
        }
        Message::DecryptCustomPasswordToggled(custom_password) => {
            state.decrypt.custom_password = custom_password;
            Task::none()
        }
        Message::DecryptPasswordChanged(password) => {
            state.decrypt.password = password;
            Task::none()
        }
        Message::DecryptRequested => start_decrypt(state),
        Message::DecryptFinished(result) => {
            if !matches!(state.decrypt.status, DecryptStatus::Working) {
                return Task::none();
            }

            match result {
                Ok(summary) => state.decrypt.status = DecryptStatus::Done(summary),
                Err(error) => state.decrypt.status = DecryptStatus::Error(error),
            }

            Task::none()
        }
        Message::LookupRequested => request_lookup(state),
        Message::LookupFinished(result) => {
            if !matches!(state.lookup.status, LookupStatus::Fetching) {
                return Task::none();
            }

            match result {
                Ok(info) => {
                    let telemetry =
                        standard_lookup_telemetry(state.lookup.pending_telemetry.take(), &info);
                    state.lookup.status = LookupStatus::Done(LookupResult::Standard(info));
                    telemetry
                }
                Err(firmware::FirmwareError::AuthExpired(message)) => {
                    // Token expired: forget it, log in again, and re-run the
                    // lookup automatically once the new login succeeds.
                    eprintln!("[lookup] token expired: {message}");
                    state.login = LoginStatus::LoggedOut;
                    state.lookup.status = LookupStatus::Idle;
                    state.lookup.retry_after_login = true;
                    return request_login(state, Click::Left);
                }
                Err(firmware::FirmwareError::Other(error)) => {
                    state.lookup.status = LookupStatus::Error(error);
                    Task::none()
                }
            }
        }
        Message::ProtocolToggled => {
            let result = if matches!(state.protocol, protocol::Handler::Ours) {
                protocol::restore()
            } else {
                protocol::register()
            };

            match result {
                Ok(()) => {
                    state.protocol = protocol::current();
                    state.protocol_error = None;
                }
                Err(error) => state.protocol_error = Some(error),
            }

            Task::none()
        }
        Message::BrowserOpened => Task::none(),
    }
}

/// True when a browser login can complete through the `softwarefix://`
/// callback: LMN Flash owns the scheme and this instance receives the
/// forwarded callback.
fn browser_login_available(state: &State) -> bool {
    matches!(state.protocol, protocol::Handler::Ours) && state.receives_callbacks
}

/// Applies a `softwarefix://` callback URL: this is how a browser login
/// completes, both when this process was started with the URL and when a later
/// instance forwards it over [`instance`].
fn apply_login_callback(state: &mut State, url: &str) -> Task<Message> {
    match login::extract_login(url) {
        Ok(info) => {
            if let Err(error) = config::save_credentials(&info.token, &state.client_uuid) {
                eprintln!("failed to save credentials: {error}");
            }

            state.login = LoginStatus::LoggedIn {
                token: info.token,
                full_name: info.full_name,
            };

            if state.lookup.retry_after_login {
                state.lookup.retry_after_login = false;
                return request_lookup(state);
            }
        }
        Err(error) => {
            // A failure is only shown when a browser login was expected;
            // otherwise it is a stray connection on the local endpoint.
            if matches!(state.login, LoginStatus::WaitingForBrowser { .. }) {
                state.login = LoginStatus::Error(error);
            } else {
                eprintln!("[instance] ignored callback: {error}");
            }
        }
    }

    Task::none()
}

/// Starts the login flow (fetches the login URL; the actual dialog/browser
/// step happens when [`Message::LoginUrlFetched`] arrives).
fn request_login(state: &mut State, click: Click) -> Task<Message> {
    state.login = LoginStatus::Fetching { click };
    let uuid = state.client_uuid.clone();

    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || login::fetch_login_url(&uuid))
                .await
                .unwrap_or_else(|e| Err(format!("background task failed: {e}")))
        },
        Message::LoginUrlFetched,
    )
}

/// Validates the tablet serial number and starts a tablet lookup.
///
/// Both regions are attempted: the CN service first (no auth), then the ROW
/// service. The result of whichever region has a match is shown. An expired
/// ROW token is ignored (no login prompt); the CN result still shows.
fn request_tablet_lookup(state: &mut State) -> Task<Message> {
    let token = match &state.login {
        LoginStatus::LoggedIn { token, .. } => token.clone(),
        _ => return Task::none(),
    };

    let sn = state.lookup.tablet.serial_number.trim().to_string();
    if sn.is_empty() {
        let message = state.l10n.tr("tablet-error-sn");
        state.lookup.status = LookupStatus::Error(message);
        return Task::none();
    }

    // Remember the submitted serial so a found image can report telemetry
    // even if the serial box is edited while the lookup runs.
    state.lookup.pending_telemetry =
        Some(TelemetryContext::Tablet { serial_number: sn.clone() });
    state.lookup.status = LookupStatus::Fetching;
    let uuid = state.client_uuid.clone();

    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                let cn = firmware::fetch_cn_tablet(&sn);
                let row = firmware::fetch_firmware_by_sn(&sn, &token, &uuid);

                match (cn, row) {
                    (Ok(Some(cn)), _) => Ok(LookupResult::CnTablet(cn)),
                    (Ok(None), Ok(row)) => Ok(LookupResult::Standard(row)),
                    (Ok(None), Err(firmware::FirmwareError::AuthExpired(_))) => {
                        // Requirement: ignore ROW region auth expiry here.
                        eprintln!("[tablet] ROW token expired; ignoring ROW result");
                        Err("no firmware found in either region".to_string())
                    }
                    (Ok(None), Err(firmware::FirmwareError::Other(row_error))) => Err(format!(
                        "CN region has no matching firmware; ROW lookup failed: {row_error}"
                    )),
                    (Err(cn_error), Ok(row)) => {
                        eprintln!("[tablet] CN lookup failed: {cn_error}");
                        Ok(LookupResult::Standard(row))
                    }
                    (Err(cn_error), Err(firmware::FirmwareError::AuthExpired(_))) => {
                        eprintln!("[tablet] CN lookup failed: {cn_error}; ROW token expired");
                        Err(format!("CN lookup failed: {cn_error}"))
                    }
                    (Err(cn_error), Err(firmware::FirmwareError::Other(row_error))) => Err(
                        format!("CN lookup failed: {cn_error}; ROW lookup failed: {row_error}"),
                    ),
                }
            })
            .await
            .unwrap_or_else(|e| Err(format!("background task failed: {e}")))
        },
        Message::TabletLookupFinished,
    )
}

/// Validates the IMEI and starts a firmware lookup.
fn request_lookup(state: &mut State) -> Task<Message> {
    let token = match &state.login {
        LoginStatus::LoggedIn { token, .. } => token.clone(),
        _ => return Task::none(),
    };

    let imei = match firmware::validate_imei(&state.lookup.imei_input) {
        Ok(imei) => imei,
        Err(error) => {
            let message_id = match error {
                firmware::ImeiError::NotDigits => "imei-error-digits",
                firmware::ImeiError::WrongLength => "imei-error-length",
                firmware::ImeiError::BadChecksum => "imei-error-checksum",
            };
            state.lookup.status = LookupStatus::Error(state.l10n.tr(message_id));
            return Task::none();
        }
    };

    // Remember which request this lookup is for, so a found firmware image
    // can report best-effort telemetry even if the IMEI box is edited while
    // the request runs.
    state.lookup.pending_telemetry =
        Some(TelemetryContext::RowSmartphone { imei: imei.clone() });
    state.lookup.status = LookupStatus::Fetching;
    let uuid = state.client_uuid.clone();

    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || firmware::fetch_firmware(&imei, &token, &uuid))
                .await
                .unwrap_or_else(|e| {
                    Err(firmware::FirmwareError::Other(format!(
                        "background task failed: {e}"
                    )))
                })
        },
        Message::LookupFinished,
    )
}

/// Opens the "Tablet Firmware Decrypt" dialog. Nothing is decrypted yet: the
/// dialog asks for the directory and starts the batch from its own button.
fn start_decrypt_dialog(state: &mut State) -> Task<Message> {
    state.decrypt = DecryptState {
        open: true,
        ..DecryptState::default()
    };

    Task::none()
}

/// Validates the decrypt form and starts decrypting the selected directory.
fn start_decrypt(state: &mut State) -> Task<Message> {
    let Some(directory) = state.decrypt.directory.clone() else {
        return Task::none();
    };

    let password = if state.decrypt.custom_password {
        let password = state.decrypt.password.trim().to_string();
        if password.is_empty() {
            let message = state.l10n.tr("decrypt-password-required");
            state.decrypt.status = DecryptStatus::Error(message);
            return Task::none();
        }
        password
    } else {
        decrypt::DEFAULT_PASSWORD.to_string()
    };

    state.decrypt.status = DecryptStatus::Working;

    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || decrypt::decrypt_directory(&directory, &password))
                .await
                .unwrap_or_else(|e| Err(format!("background task failed: {e}")))
        },
        Message::DecryptFinished,
    )
}

/// Reads device info from the selected fastboot device in the background.
fn start_fastboot_fill(serial: String) -> Task<Message> {
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                fastboot_info::read_device_info(&serial).map(|info| (serial, info))
            })
            .await
            .unwrap_or_else(|e| Err(format!("background task failed: {e}")))
        },
        Message::FastbootFillFinished,
    )
}

/// Validates the RETCN form and starts a RETCN firmware lookup.
fn request_retcn_lookup(state: &mut State) -> Task<Message> {
    let token = match &state.login {
        LoginStatus::LoggedIn { token, .. } => token.clone(),
        _ => return Task::none(),
    };

    let imei = match firmware::validate_imei_digits(&state.lookup.imei_input) {
        Ok(imei) => imei,
        Err(_) => {
            let message = state.l10n.tr("imei-error-digits");
            state.lookup.status = LookupStatus::Error(message);
            return Task::none();
        }
    };

    let retcn = &state.lookup.retcn;

    let error_id = if retcn.model.trim().is_empty() {
        Some("retcn-error-model")
    } else if retcn.fingerprint.trim().is_empty() {
        Some("retcn-error-fingerprint")
    } else if retcn.carrier.trim().is_empty() {
        Some("retcn-error-carrier")
    } else if retcn.serial_number.trim().is_empty() {
        Some("retcn-error-sn")
    } else if retcn.platform == firmware::Platform::Qualcomm
        && retcn.fsg_version.trim().is_empty()
    {
        Some("retcn-error-fsg")
    } else {
        None
    };

    if let Some(error_id) = error_id {
        let message = state.l10n.tr(error_id);
        state.lookup.status = LookupStatus::Error(message);
        return Task::none();
    }

    let request = firmware::RetcnRequest {
        imei,
        serial_number: retcn.serial_number.trim().to_string(),
        fingerprint: retcn.fingerprint.trim().to_string(),
        model: retcn.model.trim().to_string(),
        carrier: retcn.carrier.trim().to_string(),
        platform: retcn.platform,
        fsg_version: (retcn.platform == firmware::Platform::Qualcomm)
            .then(|| retcn.fsg_version.trim().to_string()),
        sim_count: (retcn.platform == firmware::Platform::MediaTek)
            .then(|| retcn.sim_count.count()),
    };

    // Remember the request so a found image can report telemetry even if the
    // form is edited while the lookup runs.
    state.lookup.pending_telemetry =
        Some(TelemetryContext::RetcnSmartphone(request.clone()));
    state.lookup.status = LookupStatus::Fetching;
    let uuid = state.client_uuid.clone();

    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                firmware::fetch_retcn_firmware(&request, &token, &uuid)
            })
            .await
            .unwrap_or_else(|e| {
                Err(firmware::FirmwareError::Other(format!(
                    "background task failed: {e}"
                )))
            })
        },
        Message::LookupFinished,
    )
}

/// Starts a "Lookup by Model" request.
///
/// The first request asks `getRomMatchParams` which discriminator parameters
/// separate this model's builds; once the user fills those in, a second
/// request submits the actual lookup through `getNewResource`.
fn request_model_lookup(state: &mut State) -> Task<Message> {
    let token = match &state.login {
        LoginStatus::LoggedIn { token, .. } => token.clone(),
        _ => return Task::none(),
    };

    let model = state.lookup.by_model.model.trim().to_string();
    if model.is_empty() {
        let message = state.l10n.tr("by-model-error-required");
        state.lookup.status = LookupStatus::Error(message);
        return Task::none();
    }

    // Model-based lookups have no telemetry schema; drop any stale context.
    state.lookup.pending_telemetry = None;

    let uuid = state.client_uuid.clone();

    if state.lookup.by_model.loaded_model != model {
        // The server still owes us the list of discriminator parameters.
        state.lookup.status = LookupStatus::Fetching;
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    firmware::fetch_model_match_params(&model, &token, &uuid)
                })
                .await
                .unwrap_or_else(|e| {
                    Err(firmware::FirmwareError::Other(format!(
                        "background task failed: {e}"
                    )))
                })
            },
            Message::ModelMatchParamsFetched,
        )
    } else {
        // Parameters are known; submit the lookup with the entered values.
        let params: Vec<(String, String)> = state
            .lookup
            .by_model
            .required
            .iter()
            .cloned()
            .zip(state.lookup.by_model.values.iter().cloned())
            .collect();
        let category = state.lookup.by_model.category;
        let country_code = state.lookup.by_model.country_code.clone();

        state.lookup.status = LookupStatus::Fetching;
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    firmware::fetch_firmware_by_model(
                        &model,
                        category,
                        &country_code,
                        &params,
                        &token,
                        &uuid,
                    )
                })
                .await
                .unwrap_or_else(|e| {
                    Err(firmware::FirmwareError::Other(format!(
                        "background task failed: {e}"
                    )))
                })
            },
            Message::LookupFinished,
        )
    }
}

/// Returns the best-effort telemetry POST for a successful smartphone-style
/// lookup (ROW/RETCN, which arrive through [`Message::LookupFinished`]). Only
/// lookups that found a downloadable firmware image are reported; "Lookup by
/// Model" and tablet lookups have no ROW/RETCN event, so no task is returned.
fn standard_lookup_telemetry(
    context: Option<TelemetryContext>,
    info: &firmware::FirmwareInfo,
) -> Task<Message> {
    // Report only "firmware image found" lookups (the Android implementation
    // also gated telemetry on a nonblank download URL).
    if info.rom_uri.trim().is_empty() {
        return Task::none();
    }

    let lookup = match context {
        Some(TelemetryContext::RowSmartphone { imei }) => telemetry::Lookup::RowSmartphone {
            imei,
        },
        Some(TelemetryContext::RetcnSmartphone(request)) => {
            telemetry::Lookup::RetcnSmartphone(request)
        }
        // "Lookup by Model" has no telemetry schema, and tablets report
        // through the tablet result path.
        Some(TelemetryContext::Tablet { .. }) | None => return Task::none(),
    };

    submit_telemetry(lookup, telemetry::Found::Standard(info.clone()))
}

/// Returns the best-effort telemetry POST for a successful tablet lookup
/// (either the CN-tablet result or the ROW fallback, which arrive through
/// [`Message::TabletLookupFinished`]), gated on a firmware image being
/// present.
fn tablet_telemetry(context: Option<TelemetryContext>, result: &LookupResult) -> Task<Message> {
    let Some(TelemetryContext::Tablet { serial_number }) = context else {
        return Task::none();
    };

    let found = match result {
        LookupResult::Standard(info) if !info.rom_uri.trim().is_empty() => {
            telemetry::Found::Standard(info.clone())
        }
        LookupResult::CnTablet(info) if !info.download_url.trim().is_empty() => {
            telemetry::Found::CnTablet(info.clone())
        }
        _ => return Task::none(),
    };

    submit_telemetry(telemetry::Lookup::Tablet { serial_number }, found)
}

/// Launches the best-effort telemetry POST in the background. The payload is
/// built up front (cheap) and only the blocking HTTP request runs on a worker
/// thread; its outcome reaches [`Message::TelemetrySent`], which only logs it.
fn submit_telemetry(lookup: telemetry::Lookup, found: telemetry::Found) -> Task<Message> {
    let payload = telemetry::build(&lookup, &found);

    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || telemetry::post(payload))
                .await
                .unwrap_or_else(|e| Err(format!("telemetry background task failed: {e}")))
        },
        Message::TelemetrySent,
    )
}

/// Opens the login URL in the embedded WebView instead of the system browser.
///
/// The WebView keeps its own cookie jar, so it is the way around a browser
/// session that answers the sign-in page with HTTP 400. The result comes back
/// as [`Message::WebViewFinished`]; a failure falls back to the manual login
/// page with the URL shown.
fn start_webview_login(state: &mut State, url: String) -> Task<Message> {
    state.login = LoginStatus::WebViewOpen { url: url.clone() };
    let title = state.l10n.tr("login-dialog-title");

    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || webview::show_login_dialog(&title, &url))
                .await
                .unwrap_or_else(|e| Err(format!("background task failed: {e}")))
        },
        Message::WebViewFinished,
    )
}

/// Opens the login URL in the system web browser without blocking the UI.
fn open_browser(url: &str) -> Task<Message> {
    let url = url.to_owned();

    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || open::that(&url).map_err(|e| e.to_string()))
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
        },
        |result| {
            if let Err(error) = result {
                eprintln!("failed to open web browser: {error}");
            }
            Message::BrowserOpened
        },
    )
}

/// Opens the Factory Reset dialog and lists the connected devices so the user
/// can pick the one to reset.
fn start_factory_reset_dialog(state: &mut State) -> Task<Message> {
    state.flash.factory_reset = FactoryResetState {
        dialog: FactoryResetDialog::Open,
        listing: true,
        ..FactoryResetState::default()
    };

    start_factory_reset_list_devices()
}

/// Lists the connected fastboot devices for the Factory Reset dialog.
fn start_factory_reset_list_devices() -> Task<Message> {
    Task::perform(
        async {
            tokio::task::spawn_blocking(fastboot_info::list_devices)
                .await
                .unwrap_or_else(|e| Err(format!("background task failed: {e}")))
        },
        Message::FactoryResetDevicesFetched,
    )
}

/// Checks the factory-reset requirements (`securestate`, `fdr-allowed`) on
/// the selected device in the background.
fn start_factory_reset_check(serial: String) -> Task<Message> {
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || fastboot_info::check_factory_reset(&serial))
                .await
                .unwrap_or_else(|e| Err(format!("background task failed: {e}")))
        },
        Message::FactoryResetCheckFinished,
    )
}

/// Erases `userdata` and `metadata` on the selected device in the background.
fn start_factory_reset(serial: String) -> Task<Message> {
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || fastboot_info::factory_reset(&serial))
                .await
                .unwrap_or_else(|e| Err(format!("background task failed: {e}")))
        },
        Message::FactoryResetFinished,
    )
}

/// Opens the Multi Flashing workspace: the page takes the place of the Mode 2
/// feature grid, so the whole window is its own.
fn start_multi_flashing_page(state: &mut State) -> Task<Message> {
    // A preset picks the tool its run uses, so the builds of this machine are
    // looked up here as well as in the Firmware Flash dialog: the page is
    // reached from the grid, without that dialog ever opening.
    state.flash.multi_flash.mfastboot = flash_engine::discover_mfastboot();
    state.flash.multi_flash.platform_tools = platform_tools::engine();
    state.flash.multi_flash.open = true;

    // The workspace slides in the way a tab does, and the whole window is
    // animated: the tab bar and the language selector disappear together with
    // the grid instead of popping while the page fades in.
    state.content_transition = Some(anim::Transition::settle(
        anim::content_start(true),
        anim::content_rest(),
    ));
    state.content_transition_covers_chrome = true;

    Task::none()
}

/// Opens the Qualcomm Partition Manager: the page takes the place of the Mode 2
/// feature grid, so the whole window is its own.
fn start_qualcomm_page(state: &mut State) -> Task<Message> {
    state.flash.qualcomm.open = true;
    // A result from the last visit says nothing about this one — but a job
    // that is still running (a dump can take a while) keeps the page busy.
    // The table itself is kept: re-reading it is a deliberate action
    // ("Inject Firehose").
    if !state.flash.qualcomm.busy {
        state.flash.qualcomm.status = QualcommStatus::Idle;
    }

    // The workspace slides in the way a tab does, and the whole window is
    // animated: the tab bar and the language selector disappear together with
    // the grid instead of popping while the page fades in.
    state.content_transition = Some(anim::Transition::settle(
        anim::content_start(true),
        anim::content_rest(),
    ));
    state.content_transition_covers_chrome = true;

    // The table is empty, but the port dropdown is filled at once: the page
    // polls the ports while it is open (see `subscription`).
    start_qualcomm_port_scan()
}

/// Asks for the Firehose programmer that "Inject Firehose" uploads.
///
/// The picker takes `mbn`, `elf` and `melf` files — a plain programmer or the
/// multi-image archive newer platforms ship — and the injection itself runs
/// on a worker thread once a file was picked.
fn start_qualcomm_injection(state: &State) -> Task<Message> {
    if state.flash.qualcomm.busy || state.flash.qualcomm.port.is_none() {
        return Task::none();
    }
    let title = state.l10n.tr("qc-firehose-programmer");
    let filter = state.l10n.tr("qc-firehose-programmer-filter");

    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                rfd::FileDialog::new()
                    .set_title(&title)
                    .add_filter(&filter, &firehose::PROGRAMMER_EXTENSIONS)
                    .pick_file()
            })
            .await
            .unwrap_or(None)
        },
        Message::QualcommProgrammerPicked,
    )
}

/// The partitions the table has selected, as regions a dump or a write can
/// work on.
fn qualcomm_selected_regions(state: &State) -> Vec<firehose::Region> {
    let qualcomm = &state.flash.qualcomm;

    qualcomm
        .partitions
        .iter()
        .enumerate()
        .filter(|(index, _)| qualcomm.selected.get(*index).copied().unwrap_or(false))
        .map(|(_, partition)| firehose::Region::of(partition))
        .collect()
}

/// The rows the table shows, as partition indices: the search box narrows the
/// table down, and a Shift-click extends over the rows that are on screen.
fn qualcomm_visible_indices(qualcomm: &QualcommState) -> Vec<usize> {
    let query = qualcomm.filter.trim().to_lowercase();

    qualcomm
        .partitions
        .iter()
        .enumerate()
        .filter(|(_, partition)| query.is_empty() || partition.name.to_lowercase().contains(&query))
        .map(|(index, _)| index)
        .collect()
}

/// Applies one click of the table's rows.
///
/// Command (Control on Windows and Linux) toggles the row and makes it the
/// anchor; Shift extends the selection from the anchor to the clicked row over
/// the rows the table shows; a plain click selects that row alone. The anchor
/// stays where it is while extending, so a second Shift-click extends from the
/// same row again.
fn select_qualcomm_row(
    qualcomm: &mut QualcommState,
    visible: &[usize],
    index: usize,
    modifiers: iced::keyboard::Modifiers,
) {
    if index >= qualcomm.partitions.len() {
        return;
    }
    qualcomm.selected.resize(qualcomm.partitions.len(), false);

    if modifiers.command() {
        let row = &mut qualcomm.selected[index];
        *row = !*row;
        qualcomm.anchor = Some(index);
        return;
    }

    // A range only exists between two rows the table shows: an anchor that the
    // search box filtered away is no anchor.
    if modifiers.shift() {
        if let (Some(anchor), Some(from), Some(to)) = (
            qualcomm.anchor,
            qualcomm.anchor.and_then(|anchor| visible.iter().position(|row| *row == anchor)),
            visible.iter().position(|row| *row == index),
        ) {
            let (from, to) = (from.min(to), from.max(to));
            qualcomm.selected.fill(false);
            for row in &visible[from..=to] {
                qualcomm.selected[*row] = true;
            }
            qualcomm.anchor = Some(anchor);
            return;
        }
    }

    qualcomm.selected.fill(false);
    qualcomm.selected[index] = true;
    qualcomm.anchor = Some(index);
}

/// Hands a table that was just read to the page: the rows, and a selection that
/// starts empty because a new table is a new list of rows.
fn apply_qualcomm_table(qualcomm: &mut QualcommState, table: firehose::Table) {
    qualcomm.detected = Some(table.storage);
    qualcomm.status = QualcommStatus::Table {
        count: table.partitions.len(),
    };
    qualcomm.selected = vec![false; table.partitions.len()];
    qualcomm.partitions = table.partitions;
    qualcomm.anchor = None;
}

/// Dumps the selected partitions into `target` on a worker thread, reporting
/// every chunk as it goes.
///
/// The programmer is still running in the phone, so this only reopens the port
/// the table was read from.
fn start_qualcomm_dump(state: &mut State, target: firehose::DumpTarget) -> Task<Message> {
    if state.flash.qualcomm.busy {
        return Task::none();
    }
    let regions = qualcomm_selected_regions(state);
    if regions.is_empty() {
        return Task::none();
    }
    let Some(port) = state.flash.qualcomm.port.clone() else {
        state.flash.qualcomm.status = QualcommStatus::DumpFailed(state.l10n.tr("qc-select-port"));
        return Task::none();
    };

    let log = flash_log::FlashLog::create(flash_log::partition_manager_path(&port));
    state.flash.qualcomm.log_path = Some(log.path().to_path_buf());
    state.flash.qualcomm.busy = true;
    // The bar is there from the first moment, empty until the first chunk.
    state.flash.qualcomm.progress = Some(firehose::Progress::default());
    state.flash.qualcomm.status = QualcommStatus::Dumping {
        name: regions[0].name.clone(),
        done: 0,
        total: 0,
    };

    let (progress, events) = iced::futures::channel::mpsc::unbounded();
    // Where the dump went, for the status line: the folder, or the one file.
    let place = target.path().to_path_buf();

    Task::batch([
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let mut log = log;
                    let report = move |step: firehose::Progress| {
                        let _ = progress.unbounded_send(Message::QualcommDumpProgress(step));
                    };

                    firehose::dump(&port, &regions, &target, &mut log, &report)
                        .map(|paths| (paths.len(), place))
                })
                .await
                .unwrap_or_else(|error| Err(format!("the Firehose worker stopped: {error}")))
            },
            Message::QualcommDumpFinished,
        ),
        Task::run(events, |message| message),
    ])
}

/// Dumps whole physical partitions into `target` on a worker thread.
///
/// The LUN list is the one the picker settled on; a storage with a single
/// physical partition never had a picker, so the worker reads the list itself
/// as the dump starts.
fn start_qualcomm_storage_dump(
    state: &mut State,
    target: firehose::DumpTarget,
) -> Task<Message> {
    if state.flash.qualcomm.busy {
        return Task::none();
    }
    let Some(port) = state.flash.qualcomm.port.clone() else {
        state.flash.qualcomm.pending_luns.clear();
        state.flash.qualcomm.lun_dialog = None;
        state.flash.qualcomm.status =
            QualcommStatus::StorageDumpFailed(state.l10n.tr("qc-select-port"));
        return Task::none();
    };

    let partitions = state.flash.qualcomm.partitions.clone();
    let picked = std::mem::take(&mut state.flash.qualcomm.pending_luns);
    let luns = (!picked.is_empty()).then_some(picked);

    let log = flash_log::FlashLog::create(flash_log::partition_manager_path(&port));
    state.flash.qualcomm.log_path = Some(log.path().to_path_buf());
    state.flash.qualcomm.busy = true;
    state.flash.qualcomm.lun_dialog = None;
    state.flash.qualcomm.progress = Some(firehose::Progress::default());
    // The partition the first report names is not known yet: the LUN list may
    // still have to be read, and the bar is empty until then either way.
    state.flash.qualcomm.status = QualcommStatus::Dumping {
        name: String::new(),
        done: 0,
        total: 0,
    };

    let (progress, events) = iced::futures::channel::mpsc::unbounded();
    let place = target.path().to_path_buf();

    Task::batch([
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let mut log = log;
                    let report = move |step: firehose::Progress| {
                        let _ = progress.unbounded_send(Message::QualcommDumpProgress(step));
                    };

                    firehose::dump_storage(
                        &port,
                        &partitions,
                        luns.as_deref(),
                        &target,
                        &mut log,
                        &report,
                    )
                    .map(|paths| (paths.len(), place))
                })
                .await
                .unwrap_or_else(|error| Err(format!("the Firehose worker stopped: {error}")))
            },
            Message::QualcommStorageDumpFinished,
        ),
        Task::run(events, |message| message),
    ])
}

/// The image name a media that is a single physical partition is saved as.
fn storage_stem(storage: StorageType) -> &'static str {
    match storage {
        StorageType::Autodetect => "storage",
        StorageType::Emmc => "emmc",
        StorageType::Ufs => "ufs",
        StorageType::SpiNor => "spinor",
        StorageType::Nand => "nand",
        StorageType::Nvme => "nvme",
    }
}

/// Erases the sectors of the selected partitions on a worker thread.
///
/// Like a dump, this reuses the programmer that is still running: the storage
/// the table came from is already configured.
fn start_qualcomm_erase(state: &mut State) -> Task<Message> {
    if state.flash.qualcomm.busy {
        return Task::none();
    }
    let regions = qualcomm_selected_regions(state);
    if regions.is_empty() {
        return Task::none();
    }
    let Some(port) = state.flash.qualcomm.port.clone() else {
        state.flash.qualcomm.status = QualcommStatus::EraseFailed(state.l10n.tr("qc-select-port"));
        return Task::none();
    };

    let log = flash_log::FlashLog::create(flash_log::partition_manager_path(&port));
    state.flash.qualcomm.log_path = Some(log.path().to_path_buf());
    state.flash.qualcomm.busy = true;
    state.flash.qualcomm.progress = Some(firehose::Progress::default());
    state.flash.qualcomm.status = QualcommStatus::Erasing {
        name: regions[0].name.clone(),
        done: 0,
        total: 0,
    };

    let (progress, events) = iced::futures::channel::mpsc::unbounded();

    Task::batch([
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let mut log = log;
                    let report = move |step: firehose::Progress| {
                        let _ = progress.unbounded_send(Message::QualcommEraseProgress(step));
                    };

                    firehose::erase(&port, &regions, &mut log, &report)
                })
                .await
                .unwrap_or_else(|error| Err(format!("the Firehose worker stopped: {error}")))
            },
            Message::QualcommEraseFinished,
        ),
        Task::run(events, |message| message),
    ])
}

/// Clears the table and reads it again from the phone.
///
/// The programmer is still running, so this only re-opens the port and repeats
/// the `<configure>` and the GPT reads — the storage that was detected is the
/// one that is read again.
fn start_qualcomm_refresh(state: &mut State) -> Task<Message> {
    let qualcomm = &mut state.flash.qualcomm;
    if qualcomm.busy {
        return Task::none();
    }

    // Whatever the table showed is about to be replaced, so the rows and the
    // selection go first: a stale row must not be acted on while the read runs.
    let port = qualcomm.port.clone();
    let storage = qualcomm.detected;
    qualcomm.partitions.clear();
    qualcomm.selected.clear();
    qualcomm.anchor = None;
    qualcomm.progress = None;
    qualcomm.status = QualcommStatus::Idle;

    // A table can only be read while the programmer runs: without it, the
    // button is disabled, and the cleared page is all a refresh can do.
    let (Some(port), Some(storage), true) = (port, storage, qualcomm.programmer_uploaded) else {
        return Task::none();
    };

    let log = flash_log::FlashLog::create(flash_log::partition_manager_path(&port));
    qualcomm.log_path = Some(log.path().to_path_buf());
    qualcomm.busy = true;
    qualcomm.status = QualcommStatus::Refreshing;

    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                let mut log = log;
                firehose::refresh(&port, storage, &mut log)
            })
            .await
            .unwrap_or_else(|error| Err(format!("the Firehose worker stopped: {error}")))
        },
        Message::QualcommRefreshed,
    )
}

/// Looks up the 05C6:9008 serial ports in the background.
fn start_qualcomm_port_scan() -> Task<Message> {
    Task::perform(
        async {
            tokio::task::spawn_blocking(qualcomm_ports::enumerate)
                .await
                .unwrap_or_default()
        },
        Message::QualcommPortsScanned,
    )
}

/// Opens the Firmware Flash dialog: finds the `mfastboot` builds shipped next
/// to the application, notes whether Google's platform-tools can be offered,
/// and lists the connected devices.
fn start_firmware_flash_dialog(state: &mut State) -> Task<Message> {
    // A fresh job id makes the events of a previous dialog stale.
    let job = state.flash.firmware.job + 1;
    let mfastboot = flash_engine::discover_mfastboot();
    // Prefer the newest shipped mfastboot (the Windows release bundles three
    // versions); the built-in fastboot covers platforms without one.
    let engine = mfastboot
        .first()
        .cloned()
        .map(flash_engine::Engine::Mfastboot)
        .unwrap_or(flash_engine::Engine::Builtin);

    state.flash.firmware = FirmwareFlashState {
        dialog: FirmwareFlashDialog::Open,
        job,
        mfastboot,
        platform_tools: platform_tools::engine(),
        engine,
        listing: true,
        ..FirmwareFlashState::default()
    };

    Task::batch([
        start_firmware_flash_list_devices(),
        // A previous dialog may have left a multi-gigabyte package behind.
        cleanup_firmware_flash_package(),
        // The engine picked above is the newest `mfastboot`, so its version is
        // asked for as well.
        start_tool_version_read(state),
    ])
}

/// Asks the selected tool what version it reports (`fastboot --version`).
///
/// Nothing is asked of the built-in engine, which has no command line, or of a
/// tool whose file is not there yet — the download that is running reports its
/// own result and this is started again once it finished.
fn start_tool_version_read(state: &mut State) -> Task<Message> {
    let flash = &mut state.flash.firmware;
    flash.tool_version = None;
    flash.reading_version = false;

    let Some(tool) = flash.engine.tool().filter(|tool| tool.path.is_file()) else {
        return Task::none();
    };

    flash.reading_version = true;
    let tool = tool.clone();
    let path = tool.path.clone();

    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || flash_engine::tool_version(&tool))
                .await
                .unwrap_or_else(|e| Err(format!("background task failed: {e}")))
        },
        // The mapper runs more than once, so the path it carries is cloned.
        move |result| Message::FirmwareFlashToolVersionRead(path.clone(), result),
    )
}

/// Whether the tool the selected engine runs is not on disk yet: only the
/// platform-tools are downloaded, the shipped builds and the built-in fastboot
/// are already there.
fn engine_tool_missing(flash: &FirmwareFlashState) -> bool {
    flash
        .engine
        .tool()
        .is_some_and(|tool| !tool.path.is_file())
}

/// Downloads Google's platform-tools into the configuration directory.
fn start_platform_tools_install(state: &mut State) -> Task<Message> {
    let flash = &mut state.flash.firmware;
    flash.installing_tools = true;
    flash.tools_error = None;
    flash.tools_ready = false;

    Task::perform(
        async {
            tokio::task::spawn_blocking(platform_tools::install)
                .await
                .unwrap_or_else(|e| Err(format!("background task failed: {e}")))
        },
        Message::FirmwareFlashToolsInstalled,
    )
}

/// Starts the "Open Terminal" flow, which checks for a competing minimal ADB
/// install before anything is downloaded.
fn start_open_terminal(state: &mut State) -> Task<Message> {
    let flash = &mut state.flash.firmware;

    if flash.terminal_busy || flash.installing_tools || flash.loading || flash.running {
        return Task::none();
    }

    flash.terminal_busy = true;
    flash.terminal_error = None;
    flash.terminal_path = None;

    Task::perform(
        async {
            // A registry read on Windows (a search through the uninstall
            // entries), so it does not belong on the UI thread. A check that
            // cannot run at all is treated as "not installed": the terminal
            // itself reports what it cannot do.
            tokio::task::spawn_blocking(platform_tools::minimal_adb_fastboot_installed)
                .await
                .unwrap_or(false)
        },
        Message::FirmwareFlashTerminalChecked,
    )
}

/// Whether the platform-tools the terminal needs are not on disk yet.
fn platform_tools_missing(flash: &FirmwareFlashState) -> bool {
    match &flash.platform_tools {
        // Google publishes nothing for this system: the download reports that
        // itself, in the terminal's own status row.
        None => true,
        Some(tool) => !tool.path.is_file(),
    }
}

/// Opens a terminal in the platform-tools directory, with `fastboot` aliased
/// to the build the dialog is set to when it is an external one.
fn open_platform_tools_terminal(state: &mut State) -> Task<Message> {
    let flash = &mut state.flash.firmware;
    flash.terminal_busy = true;

    // Only Motorola's `mfastboot` is worth aliasing: the built-in engine has
    // no binary to point `fastboot` at, and Google's platform-tools *are* the
    // `fastboot` of that directory (so the alias would shadow it with itself).
    let alias = flash
        .engine
        .tool()
        .filter(|tool| {
            tool.origin == flash_engine::Origin::Shipped && tool.path.is_file()
        })
        .cloned();

    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || platform_tools::open_terminal(alias.as_ref()))
                .await
                .unwrap_or_else(|e| Err(format!("background task failed: {e}")))
        },
        Message::FirmwareFlashTerminalOpened,
    )
}

/// Lists the connected fastboot devices for the Firmware Flash dialog.
fn start_firmware_flash_list_devices() -> Task<Message> {
    Task::perform(
        async {
            tokio::task::spawn_blocking(fastboot_info::list_devices)
                .await
                .unwrap_or_else(|e| Err(format!("background task failed: {e}")))
        },
        Message::FirmwareFlashDevicesFetched,
    )
}

/// Marks a device as the one to flash and reads the variables the dialog
/// shows for it (`securestate`, `cid`, `product`).
fn select_firmware_flash_device(state: &mut State, serial: String) -> Task<Message> {
    let flash = &mut state.flash.firmware;
    if flash.loading || flash.running {
        return Task::none();
    }

    flash.selected = Some(serial.clone());
    flash.reading_device = true;
    flash.device_vars = None;
    flash.device_vars_error = None;

    let worker_serial = serial.clone();

    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || fastboot_info::read_device_vars(&worker_serial))
                .await
                .unwrap_or_else(|e| Err(format!("background task failed: {e}")))
        },
        move |result| Message::FirmwareFlashDeviceVarsRead(serial.clone(), result),
    )
}

/// Unpacks and parses the selected package on a worker thread, streaming the
/// progress into [`Message::FirmwareFlashPackageEvent`].
fn start_firmware_flash_load(state: &mut State, path: std::path::PathBuf) -> Task<Message> {
    let flash = &mut state.flash.firmware;
    flash.loading = true;
    flash.extracted = None;
    flash.plan = None;
    flash.enabled.clear();
    flash.editing = false;
    flash.error = None;
    // The log of a previous run is not this package's, on screen or on disk:
    // the file of the run that is now over is left as it was written.
    flash.log.clear();
    flash.log_file = None;
    flash.result = None;

    let job = flash.job;
    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    // Kept to report a worker panic, which would otherwise leave the dialog
    // waiting for an event that never comes.
    let failure_sender = sender.clone();
    let emit = move |event| {
        let _ = sender.unbounded_send(event);
    };

    let producer = Task::perform(
        async move {
            let worker =
                tokio::task::spawn_blocking(move || flash_engine::load_package(&path, &emit));

            if let Err(error) = worker.await {
                let _ = failure_sender.unbounded_send(flash_engine::PackageEvent::Failed(
                    format!("background task failed: {error}").into(),
                ));
            }
        },
        |()| Message::FirmwareFlashWorkerDone,
    );

    let consumer = Task::run(receiver, move |event| {
        Message::FirmwareFlashPackageEvent(job, event)
    });

    Task::batch([producer, consumer])
}

/// Runs every step of the loaded package on the selected device, streaming the
/// progress into [`Message::FirmwareFlashEvent`].
fn start_firmware_flash(state: &mut State) -> Task<Message> {
    let flash = &mut state.flash.firmware;
    let (Some(mut package), Some(serial)) = (flash.plan.clone(), flash.selected.clone()) else {
        return Task::none();
    };

    if flash.running || flash.loading {
        return Task::none();
    }

    // Without the file there is nothing to run: the dialog keeps Start
    // disabled while the platform-tools are downloaded.
    if engine_tool_missing(flash) {
        return Task::none();
    }

    // Only the procedures left checked in the checklist are executed; the
    // loaded `plan` keeps the full list so the selection survives a retry.
    let enabled: Vec<flashfile::FlashOp> = package
        .steps
        .iter()
        .enumerate()
        .filter(|(index, _)| flash.enabled.get(*index).copied().unwrap_or(true))
        .map(|(_, step)| step.clone())
        .collect();

    if enabled.is_empty() {
        return Task::none();
    }

    package.steps = enabled;

    // The "Erase …" groups add the erases the package has no step for; the
    // partitions it does erase are handled by their (checked) rows.
    let extra_erases = extra_erase_partitions(flash);

    flash.running = true;
    flash.editing = false;
    flash.confirming = false;
    // The warning is gone, so its countdown stops ticking too.
    flash.confirm_countdown = 0;
    flash.result = None;
    flash.reboot_result = None;
    flash.progress = None;
    flash.step_index = 0;
    flash.step_total = package.steps.len() + extra_erases.len();
    flash.step_label.clear();

    let job_id = flash.job;
    let job = flash_engine::FlashJob {
        package,
        extra_erases,
        engine: flash.engine.clone(),
        serial,
        verify_checksums: flash.verify_checksums,
    };

    // The run's own log file, next to the multi-device mode's logs: every line
    // the dialog shows from here on goes into it as well, so what was flashed
    // can be read (and sent on) after the dialog is closed. The two header
    // lines say which package went to which phone — the file name has the
    // serial, not the firmware.
    let mut log_file = flash_log::FlashLog::create(flash_log::single_flash_path(&job.serial));
    let device = match &job.package.model {
        Some(model) => format!("{} ({model})", job.serial),
        None => job.serial.clone(),
    };
    log_file.line(&format!(
        "LMN Flash {} - flashing {}",
        env!("CARGO_PKG_VERSION"),
        job.package.directory.display()
    ));
    log_file.line(&format!("device {device}"));
    flash.log_file = Some(log_file);

    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    let failure_sender = sender.clone();
    let emit = move |event| {
        let _ = sender.unbounded_send(event);
    };

    let producer = Task::perform(
        async move {
            let worker = tokio::task::spawn_blocking(move || flash_engine::run(job, &emit));

            if let Err(error) = worker.await {
                let _ = failure_sender.unbounded_send(flash_engine::FlashEvent::Finished(Err(
                    format!("background task failed: {error}"),
                )));
            }
        },
        |()| Message::FirmwareFlashWorkerDone,
    );

    let consumer = Task::run(receiver, move |event| {
        Message::FirmwareFlashEvent(job_id, event)
    });

    Task::batch([producer, consumer])
}

/// Starts the export, or asks first whether the tool the script starts should
/// be copied next to it.
///
/// Only an external `fastboot` is a file that can be copied: the built-in
/// engine has no binary, so its script always runs the `fastboot` of the
/// Android platform-tools and there is nothing to ask.
fn request_firmware_flash_export(state: &mut State) -> Task<Message> {
    if !firmware_flash::export_ready(state) {
        return Task::none();
    }

    let flash = &mut state.flash.firmware;

    if flash.engine.tool().is_some() {
        flash.export_confirming = true;
        flash.export_result = None;
        Task::none()
    } else {
        start_firmware_flash_export(state, false)
    }
}

/// Writes the checked procedures (and the added erases) into a script the
/// user picks the name of: `*.cmd` on Windows, `*.sh` elsewhere.
///
/// The script is written from the same selection `start_firmware_flash` would
/// run, so both describe the same flash. A device is not required — the serial
/// is only written into the script when one is selected.
///
/// `copy_tool` writes the selected `fastboot` (and the DLLs it loads on
/// Windows) into the folder the script is saved in, and points the script at
/// that copy — so the script also works where the tool is not installed. The
/// built-in engine has no file to copy, so the flag is ignored for it.
fn start_firmware_flash_export(state: &mut State, copy_tool: bool) -> Task<Message> {
    let flash = &mut state.flash.firmware;

    if flash.loading || flash.running || flash.rebooting {
        return Task::none();
    }

    let Some(package) = flash.plan.clone() else {
        return Task::none();
    };

    // The checked procedures, in package order — the same list the flash runs.
    let steps: Vec<flashfile::FlashOp> = package
        .steps
        .iter()
        .enumerate()
        .filter(|(index, _)| flash.enabled.get(*index).copied().unwrap_or(true))
        .map(|(_, step)| step.clone())
        .collect();

    if steps.is_empty() {
        return Task::none();
    }

    let extra_erases = extra_erase_partitions(flash);
    let engine = flash.engine.clone();
    let copy = if copy_tool { engine.tool().cloned() } else { None };
    let serial = flash.selected.clone().unwrap_or_default();
    let shell = flash_script::Shell::for_host();
    let suggested = shell.file_name(package.model.as_deref());
    let extension = shell.extension();

    flash.export_confirming = false;
    flash.export_result = None;

    // The save dialog blocks, so it runs on a worker thread; the script is
    // written there too, before the dialog's answer is handed back.
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                let Some(path) = rfd::FileDialog::new()
                    .set_file_name(&suggested)
                    .add_filter("Script", &[extension])
                    .save_file()
                else {
                    return Ok(None);
                };

                // The tool is written first: a script that starts a copy that
                // is not there would be worse than no script at all.
                let folder = path.parent().unwrap_or_else(|| std::path::Path::new("."));
                let copied = match &copy {
                    Some(tool) => Some(flash_script::copy_tool(tool, folder)?),
                    None => None,
                };

                let job = flash_script::ScriptJob {
                    package: &package,
                    steps: &steps,
                    extra_erases: &extra_erases,
                    engine: &engine,
                    copied_tool: copied.as_deref(),
                    serial: &serial,
                    shell,
                };
                let script = flash_script::render(&job);

                std::fs::write(&path, script)
                    .map_err(|error| format!("{}: {error}", path.display()))?;

                Ok(Some(path))
            })
            .await
            .unwrap_or_else(|error| Err(format!("background task failed: {error}")))
        },
        Message::FirmwareFlashExported,
    )
}

/// Reads everything the "Read Info" view shows from the selected phone, on a
/// worker thread (the bootloader takes a moment to answer five commands).
///
/// The view opens before the read finishes, so the spinner is what the user
/// sees while it runs.
fn start_firmware_flash_read_info(state: &mut State) -> Task<Message> {
    let flash = &mut state.flash.firmware;
    let Some(serial) = flash.selected.clone() else {
        return Task::none();
    };

    if flash.loading
        || flash.running
        || flash.rebooting
        || flash.reading_device
        || flash.reading_info
    {
        return Task::none();
    }

    flash.reading_info = true;
    flash.info_view = true;
    flash.info_lines.clear();
    flash.info_error = None;
    flash.hide_sensitive = false;

    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || fastboot_info::read_info_lines(&serial))
                .await
                .unwrap_or_else(|error| Err(format!("background task failed: {error}")))
        },
        Message::FirmwareFlashInfoRead,
    )
}

/// The "Read Info" output as it is shown — masked when the user asked for
/// that, so copying never hands out more than the screen does.
fn info_text(flash: &FirmwareFlashState) -> String {
    let lines = if flash.hide_sensitive {
        fastboot_info::hide_sensitive(&flash.info_lines)
    } else {
        flash.info_lines.clone()
    };

    lines.join("\n")
}

/// Sends the phone into `mode` (out of fastboot, into the bootloader, the
/// recovery, userspace fastboot, or recovery's sideload mode).
///
/// The commands go to the selected device with the engine the dialog is set
/// to, and their output is streamed into the log like during a flash.
fn start_firmware_flash_reboot(
    state: &mut State,
    mode: flash_engine::RebootMode,
) -> Task<Message> {
    let flash = &mut state.flash.firmware;
    let Some(serial) = flash.selected.clone() else {
        return Task::none();
    };

    if flash.running || flash.loading || flash.rebooting {
        return Task::none();
    }

    flash.rebooting = true;
    flash.reboot_result = None;

    let engine = flash.engine.clone();
    let job_id = flash.job;

    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    let emit = move |event| {
        let _ = sender.unbounded_send(event);
    };

    let producer = Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                flash_engine::reboot(&engine, &serial, mode, &emit)
            })
            .await
            .unwrap_or_else(|error| Err(format!("background task failed: {error}")))
        },
        move |result| Message::FirmwareFlashRebooted(job_id, result),
    );

    let consumer = Task::run(receiver, move |event| {
        Message::FirmwareFlashEvent(job_id, event)
    });

    Task::batch([producer, consumer])
}

/// Opens the Single Partition Flash dialog: finds the `mfastboot` builds
/// shipped next to the application, notes whether Google's copy is already
/// downloaded, and lists the connected phones.
fn start_partition_flash_dialog(state: &mut State) -> Task<Message> {
    // A fresh job id makes the events of a previous dialog stale.
    let job = state.flash.partition.job + 1;
    let mfastboot = flash_engine::discover_mfastboot();
    // The newest shipped `mfastboot` is what both flashing dialogs prefer.
    let engine = mfastboot
        .first()
        .cloned()
        .map(flash_engine::Engine::Mfastboot)
        .unwrap_or(flash_engine::Engine::Builtin);

    state.flash.partition = PartitionFlashState {
        open: true,
        job,
        mfastboot,
        platform_tools: platform_tools::engine(),
        engine,
        listing: true,
        ..PartitionFlashState::default()
    };

    Task::batch([
        start_partition_flash_list_devices(),
        start_partition_flash_tool_version(state),
    ])
}

/// Lists the connected fastboot devices for the Single Partition Flash dialog.
///
/// Every vendor is kept (see [`fastboot_info::list_all_devices`]), so a phone
/// the dialog switched to userspace fastboot stays in the list.
fn start_partition_flash_list_devices() -> Task<Message> {
    Task::perform(
        async {
            tokio::task::spawn_blocking(fastboot_info::list_all_devices)
                .await
                .unwrap_or_else(|e| Err(format!("background task failed: {e}")))
        },
        Message::PartitionFlashDevicesFetched,
    )
}

/// Asks the selected tool what version it reports (`fastboot --version`).
///
/// Nothing is asked of the built-in engine, which has no command line, or of a
/// tool whose file is not there.
fn start_partition_flash_tool_version(state: &mut State) -> Task<Message> {
    let flash = &mut state.flash.partition;
    flash.tool_version = None;
    flash.reading_version = false;

    let Some(tool) = flash.engine.tool().filter(|tool| tool.path.is_file()) else {
        return Task::none();
    };

    flash.reading_version = true;
    let tool = tool.clone();
    let path = tool.path.clone();

    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || flash_engine::tool_version(&tool))
                .await
                .unwrap_or_else(|e| Err(format!("background task failed: {e}")))
        },
        // The mapper runs more than once, so the path it carries is cloned.
        move |result| Message::PartitionFlashToolVersionRead(path.clone(), result),
    )
}

/// Marks a phone as the one the image is written to and reads the variables
/// the dialog shows for it (`current-slot`, `is-userspace`).
fn select_partition_flash_device(state: &mut State, serial: String) -> Task<Message> {
    let flash = &mut state.flash.partition;
    if flash.task.is_some() {
        return Task::none();
    }

    flash.selected = Some(serial.clone());
    flash.reading_device = true;
    flash.vars = None;
    flash.vars_error = None;

    let worker_serial = serial.clone();

    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || fastboot_info::read_device_vars(&worker_serial))
                .await
                .unwrap_or_else(|e| Err(format!("background task failed: {e}")))
        },
        move |result| Message::PartitionFlashDeviceVarsRead(serial.clone(), result),
    )
}

/// Notes the image the flash will write, and whether the two AVB flags apply
/// to it.
///
/// A new image starts without the flags: they are only written into one that
/// carries a vbmeta struct, and the boxes are disabled for the others.
fn set_partition_flash_image(
    state: &mut State,
    path: std::path::PathBuf,
    avb: bool,
) {
    let flash = &mut state.flash.partition;

    flash.image = Some(path);
    flash.image_avb = avb;
    flash.disable_verity = false;
    flash.disable_verification = false;
    flash.result = None;
}

/// Writes the selected image to the selected partition, streaming the progress
/// into [`Message::PartitionFlashEvent`].
fn start_partition_flash(state: &mut State) -> Task<Message> {
    if partition_flash::busy(state) || !partition_flash::ready(state) {
        return Task::none();
    }

    let flash = &mut state.flash.partition;
    let (Some(serial), Some(image)) = (flash.selected.clone(), flash.image.clone()) else {
        return Task::none();
    };

    let job = flash_engine::PartitionJob {
        engine: flash.engine.clone(),
        serial: serial.clone(),
        partition: flash.partition.trim().to_owned(),
        image,
        disable_verity: flash.disable_verity,
        disable_verification: flash.disable_verification,
    };

    flash.task = Some(PartitionFlashTask::Flash);
    flash.progress = None;
    flash.result = None;
    flash.reboot_result = None;
    flash.switch_result = None;
    // The log of a previous job is not this one's, on screen or on disk.
    flash.log.clear();
    flash.log_file = None;

    // The run's own log file, next to the other flashes' logs.
    let mut log_file = flash_log::FlashLog::create(flash_log::partition_flash_path(&job.serial));
    log_file.line(&format!(
        "LMN Flash {} - flashing {} to {}",
        env!("CARGO_PKG_VERSION"),
        job.image.display(),
        job.partition
    ));
    log_file.line(&format!("device {serial}"));
    flash.log_file = Some(log_file);

    let job_id = flash.job;
    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    // Kept to report a worker panic, which would otherwise leave the dialog
    // waiting for an event that never comes.
    let failure_sender = sender.clone();
    let emit = move |event| {
        let _ = sender.unbounded_send(event);
    };

    let producer = Task::perform(
        async move {
            let worker =
                tokio::task::spawn_blocking(move || flash_engine::flash_partition(&job, &emit));

            if let Err(error) = worker.await {
                let _ = failure_sender.unbounded_send(flash_engine::FlashEvent::Finished(Err(
                    format!("background task failed: {error}"),
                )));
            }
        },
        |()| Message::PartitionFlashWorkerDone,
    );

    let consumer = Task::run(receiver, move |event| {
        Message::PartitionFlashEvent(job_id, event)
    });

    Task::batch([producer, consumer])
}

/// How long the bus waits between two lookups while a phone is switching to
/// userspace fastboot, and how many times it is asked.
///
/// The phone leaves as one USB device and returns as another (userspace
/// fastboot) under the same serial, which takes a few seconds; a phone that
/// needs longer than this leaves the dialog with whatever the last lookup
/// found, and the Refresh button picks it up once it is there.
const SWITCH_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
const SWITCH_ATTEMPTS: usize = 15;

/// Sends the phone to userspace fastboot (`reboot fastboot`), waits for it to
/// come back, and reports the phones found afterwards.
///
/// The reboot is "send and forget" — a phone leaves fastboot while answering —
/// so waiting for the device to reappear is what tells whether it worked.
fn partition_flash_switch(
    engine: &flash_engine::Engine,
    serial: &str,
    emit: &(dyn Fn(flash_engine::FlashEvent) + Send + Sync),
) -> Result<Vec<fastboot_info::DeviceSummary>, String> {
    flash_engine::reboot(engine, serial, flash_engine::RebootMode::Fastbootd, emit)?;

    let mut devices = Vec::new();

    for _ in 0..SWITCH_ATTEMPTS {
        std::thread::sleep(SWITCH_POLL_INTERVAL);
        devices = fastboot_info::list_all_devices().unwrap_or_default();

        if devices.iter().any(|device| device.serial == serial) {
            break;
        }
    }

    Ok(devices)
}

/// Starts that switch, streaming its output into the same log a flash writes.
fn start_partition_flash_switch(state: &mut State) -> Task<Message> {
    let flash = &mut state.flash.partition;
    let Some(serial) = flash.selected.clone() else {
        return Task::none();
    };

    if flash.task.is_some() {
        return Task::none();
    }

    flash.task = Some(PartitionFlashTask::SwitchToFastbootd);
    flash.switch_result = None;
    flash.result = None;
    flash.reboot_result = None;

    let engine = flash.engine.clone();
    let job_id = flash.job;

    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    let emit = move |event| {
        let _ = sender.unbounded_send(event);
    };

    let producer = Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                partition_flash_switch(&engine, &serial, &emit)
            })
            .await
            .unwrap_or_else(|error| Err(format!("background task failed: {error}")))
        },
        move |result| Message::PartitionFlashSwitched(job_id, result),
    );

    let consumer = Task::run(receiver, move |event| {
        Message::PartitionFlashEvent(job_id, event)
    });

    Task::batch([producer, consumer])
}

/// Reboots the phone out of fastboot, through the engine the dialog is set to:
/// the boot-mode flag is cleared first, so it does not come back up in fastboot.
fn start_partition_flash_reboot(state: &mut State) -> Task<Message> {
    let flash = &mut state.flash.partition;
    let Some(serial) = flash.selected.clone() else {
        return Task::none();
    };

    if flash.task.is_some() {
        return Task::none();
    }

    flash.task = Some(PartitionFlashTask::Reboot);
    flash.reboot_result = None;
    flash.result = None;
    flash.switch_result = None;

    let engine = flash.engine.clone();
    let job_id = flash.job;

    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    let emit = move |event| {
        let _ = sender.unbounded_send(event);
    };

    let producer = Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                flash_engine::reboot(&engine, &serial, flash_engine::RebootMode::System, &emit)
            })
            .await
            .unwrap_or_else(|error| Err(format!("background task failed: {error}")))
        },
        move |result| Message::PartitionFlashRebooted(job_id, result),
    );

    let consumer = Task::run(receiver, move |event| {
        Message::PartitionFlashEvent(job_id, event)
    });

    Task::batch([producer, consumer])
}

/// Reconnects to `serial`, then lists the devices `adb devices` reports,
/// which is what turns a device whose "Allow USB debugging?" prompt was
/// dismissed into one the dialog can work on.
///
/// The listing is the one the device step starts from, so a device that was
/// authorized this time is picked up by the auto-selection a single ready
/// device triggers.
fn start_bloatware_retry(state: &mut State, serial: String) -> Task<Message> {
    let bloatware = &mut state.flash.bloatware;

    if bloatware.busy() {
        return Task::none();
    }

    bloatware.preparing = true;
    bloatware.prepare_error = None;
    bloatware.devices.clear();

    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                debloat::reconnect(&serial)?;
                debloat::list_devices()
            })
            .await
            .unwrap_or_else(|error| Err(format!("background task failed: {error}")))
        },
        Message::BloatwareDevicesFetched,
    )
}

/// Clears every working field of the bloatware dialog, keeping only its open
/// state (the caller sets what the new step needs).
fn reset_bloatware(bloatware: &mut BloatwareState) {
    bloatware.preparing = false;
    bloatware.prepare_error = None;
    bloatware.devices.clear();
    bloatware.selected = None;
    bloatware.checking = false;
    bloatware.check_error = None;
    bloatware.found = None;
    bloatware.checked.clear();
    bloatware.filter.clear();
    bloatware.removing = false;
    bloatware.results = None;
}

/// Opens the "Remove System Bloatware" dialog and looks for a device.
fn start_bloatware_dialog(state: &mut State) -> Task<Message> {
    state.flash.bloatware = BloatwareState {
        dialog: BloatwareDialog::Open,
        preparing: true,
        ..BloatwareState::default()
    };

    start_bloatware_devices()
}

/// Lists the devices `adb devices` reports (downloading `adb` on first use).
fn start_bloatware_devices() -> Task<Message> {
    Task::perform(
        async {
            tokio::task::spawn_blocking(debloat::list_devices)
                .await
                .unwrap_or_else(|e| Err(format!("background task failed: {e}")))
        },
        Message::BloatwareDevicesFetched,
    )
}

/// Reads which of the known bloatware apps are installed on `serial`.
fn start_bloatware_check(state: &mut State, serial: String) -> Task<Message> {
    let bloatware = &mut state.flash.bloatware;
    if bloatware.checking || bloatware.removing {
        return Task::none();
    }

    bloatware.selected = Some(serial.clone());
    bloatware.checking = true;
    bloatware.check_error = None;
    bloatware.found = None;
    bloatware.checked.clear();
    bloatware.filter.clear();
    bloatware.results = None;

    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || debloat::list_installed(&serial))
                .await
                .unwrap_or_else(|e| Err(format!("background task failed: {e}")))
        },
        Message::BloatwarePackagesFetched,
    )
}

/// Opens the "Connect ADB Wireless Debugging" dialog and starts looking for
/// the phones' pairing and connect services.
fn start_wireless_dialog(state: &mut State) -> Task<Message> {
    let job = state.flash.wireless.job.wrapping_add(1);

    stop_wireless(state);
    state.flash.wireless = WirelessState {
        open: true,
        job,
        ..WirelessState::default()
    };

    // The QR code is what tells the phone which service to announce, so it is
    // made with the dialog.
    regenerate_wireless_code(state);

    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    let emit = move |event| {
        let _ = sender.unbounded_send(event);
    };

    if let Some(discovery) = wireless::start(emit) {
        state.flash.wireless.discovery = Some(discovery);
    }

    let discovery = Task::run(receiver, move |event| {
        Message::WirelessDiscovered(job, event)
    });

    // What `adb` already has connected decides whether "Disconnect" is
    // available from the first frame on, so it is read along with the
    // discovery.
    Task::batch([discovery, start_wireless_devices()])
}

/// Stops the wireless dialog's network discovery and closes it.
fn stop_wireless(state: &mut State) {
    if let Some(discovery) = state.flash.wireless.discovery.take() {
        discovery.stop();
    }

    state.flash.wireless = WirelessState::default();
}

/// Puts a fresh pairing code, and the QR code carrying it, into the state.
fn regenerate_wireless_code(state: &mut State) {
    let code = wireless::PairingCode::new();
    let qr = wireless::QrImage::encode(&code.payload());

    let wireless = &mut state.flash.wireless;
    wireless.pairing_code = Some(code);
    wireless.qr = qr;
    wireless.result = None;
    wireless.disconnect_result = None;
}

/// Pairs with the phone at `address` using `code`, on a worker thread.
fn start_wireless_pair(state: &mut State, address: String, code: String) -> Task<Message> {
    let wireless = &mut state.flash.wireless;

    if wireless.busy() {
        return Task::none();
    }

    // The connect service the phone announces while wireless debugging is on.
    // `adb` also connects on its own through mDNS, so this only makes the
    // connection come about a little sooner.
    let connect = wireless
        .connect_service()
        .map(wireless::AdbService::address_port);

    wireless.working = true;
    wireless.result = None;
    wireless.disconnect_result = None;

    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                wireless::pair_and_connect(&address, &code, connect.as_deref())
            })
            .await
            .unwrap_or_else(|error| Err(format!("background task failed: {error}")))
        },
        Message::WirelessFinished,
    )
}

/// Drops the selected wireless connection on a worker thread.
///
/// A pick from the menu cannot be refused by greying it out (iced has no
/// `on_select` that can be turned off), so a pick while a job runs is dropped
/// here instead.
fn start_wireless_disconnect(
    state: &mut State,
    target: wireless::DisconnectTarget,
) -> Task<Message> {
    let wireless = &mut state.flash.wireless;

    if wireless.busy() {
        return Task::none();
    }

    wireless.disconnecting = true;
    wireless.result = None;
    wireless.disconnect_result = None;

    // The target is echoed back, so the status line can name what was dropped
    // without the state having to remember it.
    let echoed = target.clone();

    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || wireless::disconnect(&target))
                .await
                .unwrap_or_else(|error| Err(format!("background task failed: {error}")))
        },
        move |result| Message::WirelessDisconnected(echoed.clone(), result),
    )
}

/// Reads the wireless devices `adb` already has connected, so "Disconnect"
/// knows about a phone that was connected before this dialog was opened (or by
/// another tool).
///
/// `adb` is only asked when it is installed: opening the dialog must not start
/// the platform-tools download.
fn start_wireless_devices() -> Task<Message> {
    Task::perform(
        async {
            tokio::task::spawn_blocking(wireless::connected_devices)
                .await
                .unwrap_or_default()
        },
        Message::WirelessDevicesFetched,
    )
}

/// Opens the "Fix Captive Portal Server" dialog and looks for a device.
fn start_captive_portal_dialog(state: &mut State) -> Task<Message> {
    state.flash.captive_portal = CaptivePortalState {
        open: true,
        listing: true,
        ..CaptivePortalState::default()
    };

    start_captive_portal_devices()
}

/// Lists the devices `adb devices` reports (downloading `adb` on first use).
fn start_captive_portal_devices() -> Task<Message> {
    Task::perform(
        async {
            tokio::task::spawn_blocking(debloat::list_devices)
                .await
                .unwrap_or_else(|e| Err(format!("background task failed: {e}")))
        },
        Message::CaptivePortalDevicesFetched,
    )
}

/// Reconnects to `serial`, then reads the devices `adb devices` lists.
///
/// Reconnecting is what makes a device whose "Allow USB debugging?" prompt was
/// dismissed show it again; the listing that follows is what says whether the
/// prompt was accepted.
fn start_captive_portal_retry(state: &mut State, serial: String) -> Task<Message> {
    let portal = &mut state.flash.captive_portal;

    if portal.busy() {
        return Task::none();
    }

    portal.listing = true;
    portal.list_error = None;
    portal.devices.clear();

    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                debloat::reconnect(&serial)?;
                debloat::list_devices()
            })
            .await
            .unwrap_or_else(|error| Err(format!("background task failed: {error}")))
        },
        Message::CaptivePortalDevicesFetched,
    )
}

/// Applies the captive-portal fix to `serial` on a worker thread.
fn start_captive_portal_fix(state: &mut State, serial: String) -> Task<Message> {
    let portal = &mut state.flash.captive_portal;

    if !portal.ready_to_fix() {
        return Task::none();
    }

    portal.fixing = true;
    portal.result = None;

    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || captive_portal::fix(&serial))
                .await
                .unwrap_or_else(|error| {
                    Err(captive_portal::FixError::Failed(format!(
                        "background task failed: {error}"
                    )))
                })
        },
        Message::CaptivePortalFixed,
    )
}

/// Adds a discovered service to a list, or updates the entry it is already in
/// (a phone re-announces a service with the same mDNS name).
fn upsert_wireless_service(
    services: &mut Vec<wireless::AdbService>,
    service: wireless::AdbService,
) {
    match services
        .iter_mut()
        .find(|existing| existing.fullname == service.fullname)
    {
        Some(existing) => *existing = service,
        None => services.push(service),
    }
}

/// Removes the checked apps from the selected device, one after another.
fn start_bloatware_remove(state: &mut State) -> Task<Message> {
    let bloatware = &mut state.flash.bloatware;

    let Some(serial) = bloatware.selected.clone() else {
        return Task::none();
    };
    let apps = bloatware.chosen_apps();

    if apps.is_empty() || bloatware.removing {
        return Task::none();
    }

    bloatware.removing = true;
    bloatware.results = None;

    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || debloat::remove(&serial, &apps))
                .await
                .unwrap_or_default()
        },
        Message::BloatwareRemoved,
    )
}

/// Opens the "Unpack Image" dialog. Nothing is read yet: the dialog asks for
/// the image (whose directory is listed as soon as it is picked) and for the
/// folder the files are written into.
fn start_unpack_dialog(state: &mut State) -> Task<Message> {
    state.unpack = UnpackState {
        open: true,
        ..UnpackState::default()
    };

    Task::none()
}

/// Extracts the selected image on a worker thread, streaming its progress into
/// the dialog.
fn start_unpack_job(state: &mut State) -> Task<Message> {
    let Some(image) = state.unpack.image.clone() else {
        return Task::none();
    };
    let Some(output) = state.unpack.output.clone() else {
        return Task::none();
    };

    let unpack = &mut state.unpack;
    if unpack.busy() {
        return Task::none();
    }

    unpack.working = true;
    unpack.job = unpack.job.wrapping_add(1);
    unpack.current = None;
    unpack.done_bytes = 0;
    unpack.result = None;

    let job_id = unpack.job;

    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    let failure_sender = sender.clone();
    let emit = move |event| {
        let _ = sender.unbounded_send(event);
    };

    let producer = Task::perform(
        async move {
            let worker =
                tokio::task::spawn_blocking(move || image_unpack::unpack(&image, &output, &emit));

            // `unpack` reports the outcome itself; only a worker that died
            // without getting that far has to be turned into an event here, or
            // the dialog would wait for one that never comes (the sender it
            // held is gone with it).
            if let Err(error) = worker.await {
                let _ = failure_sender.unbounded_send(image_unpack::UnpackEvent::Finished(Err(
                    image_unpack::UnpackError::Read(format!("background task failed: {error}")),
                )));
            }
        },
        |()| Message::UnpackWorkerDone,
    );

    let consumer = Task::run(receiver, move |event| Message::UnpackEvent(job_id, event));

    Task::batch([producer, consumer])
}

/// Opens the "Unisoc PAC" dialog. Nothing is read yet: the dialog asks for the
/// package (whose file table is listed as soon as it is picked) and for the
/// folder the files are written into.
fn start_pac_dialog(state: &mut State) -> Task<Message> {
    state.pac = PacState {
        open: true,
        ..PacState::default()
    };

    Task::none()
}

/// Extracts the checked files of the selected package on a worker thread,
/// streaming its progress into the dialog.
fn start_pac_job(state: &mut State) -> Task<Message> {
    let Some(file) = state.pac.file.clone() else {
        return Task::none();
    };
    let Some(output) = state.pac.output.clone() else {
        return Task::none();
    };

    let selected = state.pac.chosen_indexes();
    let script = state.pac.spd_script;

    if selected.is_empty() {
        return Task::none();
    }

    // A run of nothing but the download agents is not a scattered firmware: it
    // prepares a factory reset, so it leaves the flash configuration out.
    let configuration = if state
        .pac
        .package
        .as_ref()
        .is_some_and(|package| spd_script::only_loaders(package, &selected))
    {
        pac::Configuration::Skip
    } else {
        pac::Configuration::Write
    };

    let pac = &mut state.pac;
    if pac.busy() {
        return Task::none();
    }

    pac.working = true;
    pac.job = pac.job.wrapping_add(1);
    pac.current = None;
    pac.done_bytes = 0;
    pac.result = None;
    pac.scripts = None;

    let job_id = pac.job;

    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    let failure_sender = sender.clone();
    let emit = move |event| {
        let event = match event {
            pac::PacEvent::File { name, .. } => PacJobEvent::File(name),
            pac::PacEvent::Progress { bytes, total } => PacJobEvent::Progress { bytes, total },
            // The outcome is reported once, by the worker below.
            pac::PacEvent::Finished(_) => return,
        };

        let _ = sender.unbounded_send(event);
    };

    let producer = Task::perform(
        async move {
            let worker = tokio::task::spawn_blocking(move || {
                let result = pac::unpack(&file, &output, &selected, configuration, &emit);

                // The scripts describe the firmware the extraction just wrote,
                // so they follow it — and only when it worked, and only when
                // they were asked for.
                let scripts = if script && result.is_ok() {
                    write_spd_scripts(&file, &output, &selected)
                } else {
                    Ok(Vec::new())
                };

                (result, scripts)
            });

            // A worker that died without reporting anything has to be turned
            // into an event here, or the dialog would wait for one that never
            // comes (the sender it held is gone with it).
            let event = match worker.await {
                Ok((result, scripts)) => PacJobEvent::Finished { result, scripts },
                Err(error) => PacJobEvent::Finished {
                    result: Err(pac::PacError::Read(format!(
                        "background task failed: {error}"
                    ))),
                    scripts: Ok(Vec::new()),
                },
            };

            let _ = failure_sender.unbounded_send(event);
        },
        |()| Message::PacWorkerDone,
    );

    let consumer = Task::run(receiver, move |event| Message::PacEvent(job_id, event));

    Task::batch([producer, consumer])
}

/// Writes the `spd_dump` scripts next to the files a package was unpacked into.
///
/// They are rendered from the package and its configuration XML: the partition
/// of every record and the addresses of the two download agents come from that
/// XML, which is why a package that carries none cannot have them. The XML is
/// read from the package itself — the user may well not have unpacked it, and
/// the reset script does not need it to be there at all. The names of the files
/// written are returned, so the dialog can say what it wrote.
fn write_spd_scripts(
    package: &std::path::Path,
    output: &std::path::Path,
    selected: &[usize],
) -> Result<Vec<String>, String> {
    let parsed = pac::list(package).map_err(|error| error.to_string())?;
    let configuration = pac::config_name(&parsed).to_string();

    let entry = parsed
        .files
        .iter()
        .find(|entry| entry.name.eq_ignore_ascii_case(&configuration))
        .ok_or_else(|| format!("{configuration}: the package does not carry it"))?;

    let xml = pac::payload(package, entry).map_err(|error| error.to_string())?;
    let xml = String::from_utf8_lossy(&xml).into_owned();

    // The scripts are named after the product, or after the package when it
    // does not name one.
    let product = parsed.header.product_name.trim().to_string();
    let base = if product.is_empty() {
        package
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        product
    };

    let scripts = spd_script::scripts(
        &parsed,
        selected,
        &xml,
        flash_script::Shell::for_host(),
        &base,
    )
    .map_err(|error| error.to_string())?;

    let mut written = Vec::new();

    // Only the scripts that describe this run are there: with nothing left to
    // write, the flash script would not.
    for (name, contents) in [scripts.flash, Some(scripts.reset)].into_iter().flatten() {
        let path = output.join(&name);

        std::fs::write(&path, contents).map_err(|error| format!("{name}: {error}"))?;

        // A script is meant to be started, so it has to be executable.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
        }

        written.push(name);
    }

    Ok(written)
}

/// Turns the "Generate SPD Dump Script" option off when the selection can no
/// longer produce the scripts: they name the two download agents, so leaving
/// one of them out makes the option impossible rather than wrong.
fn drop_spd_script_when_impossible(pac: &mut PacState) {
    if !pac.spd_script {
        return;
    }

    let possible = pac
        .package
        .as_ref()
        .is_some_and(|package| spd_script::can_generate(package, &pac.selected));

    if !possible {
        pac.spd_script = false;
    }
}

/// Opens the "Repack Package" dialog. Nothing is read yet: the dialog asks for
/// the `flash.cfg` or scatter XML the package is described by.
fn start_pac_repack_dialog(state: &mut State) -> Task<Message> {
    state.pac_repack = PacRepackState {
        open: true,
        ..PacRepackState::default()
    };

    Task::none()
}

/// Writes the package the loaded plan describes on a worker thread, streaming
/// its progress into the dialog.
fn start_pac_repack_job(state: &mut State) -> Task<Message> {
    let Some(plan) = state.pac_repack.plan.clone() else {
        return Task::none();
    };
    let Some(output) = state.pac_repack.output.clone() else {
        return Task::none();
    };

    let repack = &mut state.pac_repack;

    if repack.busy() {
        return Task::none();
    }

    repack.working = true;
    repack.job = repack.job.wrapping_add(1);
    repack.current = None;
    repack.done_bytes = 0;
    repack.total_bytes = 0;
    repack.result = None;

    let job_id = repack.job;

    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    let failure_sender = sender.clone();

    // The writer reports the files it copies and how far it is. The outcome of
    // the whole build is reported once, after the worker returned: a build that
    // refuses a plan never gets to write anything, so its own events cannot be
    // the only word.
    let emit = move |event| {
        let event = match event {
            pac::PacEvent::File { name, .. } => PacRepackEvent::File(name),
            pac::PacEvent::Progress { bytes, total } => PacRepackEvent::Progress { bytes, total },
            pac::PacEvent::Finished(_) => return,
        };

        let _ = sender.unbounded_send(event);
    };

    let producer = Task::perform(
        async move {
            let worker =
                tokio::task::spawn_blocking(move || pac_build::build(&plan, &output, &emit));

            let outcome = match worker.await {
                Ok(result) => result,
                Err(error) => Err(pac_build::BuildError::Read(format!(
                    "background task failed: {error}"
                ))),
            };

            let _ = failure_sender.unbounded_send(PacRepackEvent::Finished(outcome));
        },
        |()| Message::PacRepackWorkerDone,
    );

    let consumer = Task::run(receiver, move |event| Message::PacRepackEvent(job_id, event));

    Task::batch([producer, consumer])
}

/// Opens the "Install Driver" dialog. Nothing is downloaded or installed yet:
/// the dialog explains what will happen and starts the job from its Install
/// button (on Linux only once it has the password `sudo` needs).
fn start_driver_dialog(state: &mut State) -> Task<Message> {
    state.flash.driver = DriverState {
        dialog: DriverDialog::Open,
        ..DriverState::default()
    };

    Task::none()
}

/// Runs the installation on a worker thread, streaming its progress into the
/// dialog.
fn start_driver_job(state: &mut State) -> Task<Message> {
    let driver = &mut state.flash.driver;

    driver.busy = true;
    driver.job = driver.job.wrapping_add(1);
    driver.stage = if driver_install::uses_password() {
        DriverStage::Installing
    } else {
        DriverStage::Downloading
    };
    driver.downloaded = 0;
    driver.total = 0;
    driver.error = None;

    let job_id = driver.job;
    let target = driver_install::target();
    let password = driver.password.clone();

    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    let failure_sender = sender.clone();
    let emit = move |event| {
        let _ = sender.unbounded_send(event);
    };

    let producer = Task::perform(
        async move {
            let worker =
                tokio::task::spawn_blocking(move || driver_install::run(target, &password, &emit));

            // `run` reports the outcome itself; only a worker that died
            // without getting that far has to be turned into an event here,
            // or the dialog would wait for one that never comes (the sender
            // it held is gone with it).
            if let Err(error) = worker.await {
                let _ = failure_sender.unbounded_send(driver_install::DriverEvent::Finished(
                    Err(driver_install::DriverError::Failed(format!(
                        "background task failed: {error}"
                    ))),
                ));
            }
        },
        |()| Message::DriverWorkerDone,
    );

    let consumer = Task::run(receiver, move |event| Message::DriverEvent(job_id, event));

    Task::batch([producer, consumer])
}

/// Deletes the unpacked firmware package (several gigabytes) on a worker
/// thread; a failure is irrelevant, the directory is cleared on the next run.
fn cleanup_firmware_flash_package() -> Task<Message> {
    Task::perform(
        async {
            let _ = tokio::task::spawn_blocking(flashfile::cleanup_work_directory).await;
        },
        |()| Message::FirmwareFlashWorkerDone,
    )
}

/// Appends a line to the flashing log: to the file this run writes (when it
/// has one) and to the log on screen, which is bounded in length because a
/// factory firmware produces thousands of lines.
fn push_flash_log(flash: &mut FirmwareFlashState, line: String) {
    push_log_line(&mut flash.log, &mut flash.log_file, line);
}

/// [`push_flash_log`] for the Single Partition Flash dialog, whose log lives
/// in its own state: the file keeps every line, only what is shown is trimmed.
fn push_partition_flash_log(flash: &mut PartitionFlashState, line: String) {
    push_log_line(&mut flash.log, &mut flash.log_file, line);
}

/// One line into the on-screen log and the file behind it.
fn push_log_line(
    log: &mut Vec<String>,
    file: &mut Option<flash_log::FlashLog>,
    line: String,
) {
    const MAX_LINES: usize = 500;

    if let Some(file) = file.as_mut() {
        file.line(&line);
    }

    if log.len() >= MAX_LINES {
        log.remove(0);
    }

    log.push(line);
}

/// Pins the flashing log to its last line. A build without the log on screen
/// has no widget with that id, and the operation is then a no-op.
fn scroll_flash_log_to_end() -> Task<Message> {
    iced::widget::operation::snap_to(
        iced::widget::Id::new(firmware_flash::LOG_SCROLL_ID),
        iced::widget::scrollable::RelativeOffset::END,
    )
}

/// How many procedures of the loaded package are checked in the checklist.
fn enabled_procedures(flash: &FirmwareFlashState) -> usize {
    match &flash.plan {
        Some(package) => package
            .steps
            .iter()
            .enumerate()
            .filter(|(index, _)| flash.enabled.get(*index).copied().unwrap_or(true))
            .count(),
        None => 0,
    }
}

/// How long the warning that the package belongs to another phone has to be
/// read before "Yes" can be clicked.
const MISMATCH_COUNTDOWN_SECONDS: u8 = 10;

/// The firmware formats the file pickers offer: the factory ZIP, or the
/// `flashfile.xml` of a package that is unpacked next to its image files.
/// Shared, so the Multi Flashing presets take exactly what the single-device
/// Firmware Flash dialog does.
const FIRMWARE_PACKAGE_EXTENSIONS: [&str; 2] = ["zip", "xml"];

/// The image formats the Single Partition Flash picker offers: a partition
/// image is written to a partition as it is, so only the usual extension is
/// worth filtering on.
const PARTITION_IMAGE_EXTENSIONS: [&str; 1] = ["img"];

/// The message a package failure shows: the one the user has to act on has a
/// translation, the rest (the ZIP reader, the XML parser, the file system) is
/// technical text shown as it came.
fn load_error_text(l10n: &l10n::Bundle, error: &flashfile::LoadError) -> String {
    match error {
        flashfile::LoadError::NotAPackage => l10n.tr("firmware-flash-invalid-package"),
        flashfile::LoadError::Other(message) => message.clone(),
    }
}

/// The tool a preset that was just picked starts with.
///
/// The newest shipped `mfastboot` is what the single-device dialog prefers;
/// Google's copy is the fallback on a machine that ships no build at all, and
/// the built-in fastboot is the last resort.
fn default_preset_tool(multi: &MultiFlashState) -> PresetTool {
    if let Some(tool) = multi.mfastboot.first() {
        return PresetTool::Shipped(tool.version.clone());
    }

    if multi.platform_tools.is_some() {
        return PresetTool::PlatformTools;
    }

    PresetTool::Builtin
}

/// Asks where a ZIP package's images should be unpacked.
///
/// The picker opens where the directory is set already (someone changing their
/// mind), or in the one the archive lies in, which is where a firmware is
/// usually unpacked next to. Nothing is unpacked until a directory is picked:
/// a factory package is gigabytes, and where they go is the user's choice.
///
/// `ask` says which of the two cases this is, so the answer can be acted on
/// (see [`DirectoryAsk`]).
fn start_multi_flash_choose_directory(
    state: &State,
    path: std::path::PathBuf,
    current: Option<std::path::PathBuf>,
    ask: DirectoryAsk,
) -> Task<Message> {
    let title = state.l10n.tr("multi-flash-options-extract");
    let start = current.or_else(|| path.parent().map(std::path::Path::to_path_buf));

    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                let mut dialog = rfd::FileDialog::new().set_title(&title);

                if let Some(start) = start {
                    dialog = dialog.set_directory(start);
                }

                dialog.pick_folder()
            })
            .await
            .unwrap_or(None)
        },
        move |directory| Message::MultiFlashPresetDirectoryPicked(directory, ask),
    )
}

/// Unpacks the picked ZIP into the directory the user chose, streaming its
/// progress into [`Message::MultiFlashUnpackEvent`].
///
/// The same loader the single-device dialog uses does the work, so the archive
/// is checked for a `flashfile.xml` before a single byte is written.
fn start_multi_flash_unpack(state: &mut State) -> Task<Message> {
    let pending = &mut state.flash.multi_flash.pending;

    let (Some(path), Some(destination)) = (pending.path.clone(), pending.extract_to.clone()) else {
        return Task::none();
    };

    if pending.unpacking {
        return Task::none();
    }

    pending.unpacking = true;
    pending.unpacked = None;
    pending.unpack_error = None;

    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    // Kept to report a worker panic, which would otherwise leave the card
    // waiting for an event that never comes.
    let failure_sender = sender.clone();
    let emit = move |event| {
        let _ = sender.unbounded_send(event);
    };

    let answer_path = path.clone();
    let producer = Task::perform(
        async move {
            let worker = tokio::task::spawn_blocking(move || {
                flash_engine::load_package_into(&path, Some(&destination), &emit)
            });

            if let Err(error) = worker.await {
                let _ = failure_sender.unbounded_send(flash_engine::PackageEvent::Failed(
                    format!("background task failed: {error}").into(),
                ));
            }
        },
        |()| Message::MultiFlashUnpackWorkerDone,
    );

    let consumer = Task::run(receiver, move |event| {
        Message::MultiFlashUnpackEvent(answer_path.clone(), event)
    });

    Task::batch([producer, consumer])
}

/// Watches for devices and flashes the preset that fits each of them.
///
/// The session runs on a blocking worker thread (it sleeps between looks at the
/// bus) and serves every device it finds on a thread of its own; what it reports
/// is streamed into [`Message::MultiFlashSessionEvent`].
///
/// The log directory is named here, when the user pressed "Start", so the folder
/// carries the moment the run began rather than the moment its thread happened
/// to start.
fn start_multi_flash_session(state: &mut State) -> Task<Message> {
    let multi = &mut state.flash.multi_flash;

    let presets = multi.presets.clone();
    let tools = multi.mfastboot.clone();
    let platform_tools = multi.platform_tools.clone();
    let session = multi.session;

    // The session is asked to stop through this flag; it is kept so "Stop" can
    // reach a thread that is sleeping between two looks at the bus.
    let stop = Arc::new(AtomicBool::new(false));
    multi.stop = Some(Arc::clone(&stop));

    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    // Kept to report a worker panic, which would otherwise leave the page
    // running forever with a table that never changes.
    let failure_sender = sender.clone();
    let emit: Arc<dyn Fn(multi_flash_run::SessionEvent) + Send + Sync> =
        Arc::new(move |event| {
            let _ = sender.unbounded_send(event);
        });

    let directory = multi_flash_run::log_directory();
    let producer = Task::perform(
        async move {
            let worker = tokio::task::spawn_blocking(move || {
                multi_flash_run::run(presets, tools, platform_tools, directory, emit, stop)
            });

            if let Err(error) = worker.await {
                let _ = failure_sender
                    .unbounded_send(multi_flash_run::SessionEvent::Failed(format!(
                        "background task failed: {error}"
                    )));
            }
        },
        |()| Message::MultiFlashSessionDone,
    );

    let consumer = Task::run(receiver, move |event| {
        Message::MultiFlashSessionEvent(session, event)
    });

    Task::batch([producer, consumer])
}

/// Asks what a picked firmware package is for, without unpacking its images.
fn start_multi_flash_identify(path: std::path::PathBuf) -> Task<Message> {
    Task::perform(
        async move {
            let reading = path.clone();
            let identity = tokio::task::spawn_blocking(move || flashfile::identify(&reading))
                .await
                .unwrap_or_else(|error| {
                    Err(flashfile::LoadError::Other(format!(
                        "background task failed: {error}"
                    )))
                });

            (path, identity)
        },
        |(path, identity)| Message::MultiFlashPresetIdentified(path, identity),
    )
}

/// Whether the loaded package is made for another phone than the selected one.
///
/// Both sides have to name a project code: a package without a `vbmeta` image,
/// or a bootloader that reports no `product`, leaves the question open, and an
/// open question is not a warning.
fn project_mismatch(flash: &FirmwareFlashState) -> bool {
    flashfile::project_code_mismatch(
        flash
            .plan
            .as_ref()
            .and_then(|package| package.project_code.as_deref()),
        flash
            .device_vars
            .as_ref()
            .and_then(|variables| variables.product.as_deref()),
    )
}

/// Checks (or unchecks) the package's own `erase` steps that belong to one
/// erase group, so the checklist shows what pressing the button did. Steps of
/// the other group are left alone.
fn check_erase_rows(
    flash: &mut FirmwareFlashState,
    group: flashfile::EraseGroup,
    checked: bool,
) {
    let matched: Vec<usize> = match &flash.plan {
        Some(package) => package
            .steps
            .iter()
            .enumerate()
            .filter(|(_, step)| match step {
                flashfile::FlashOp::Erase { partition } => group
                    .partitions()
                    .contains(&flashfile::normalize_partition(partition).as_str()),
                _ => false,
            })
            .map(|(index, _)| index)
            .collect(),
        None => return,
    };

    for index in matched {
        if let Some(enabled) = flash.enabled.get_mut(index) {
            *enabled = checked;
        }
    }
}

/// Whether the loaded package erases `partition` with a step of its own
/// (whether or not that step is checked).
fn package_erases_partition(flash: &FirmwareFlashState, partition: &str) -> bool {
    flash.plan.as_ref().is_some_and(|package| {
        package.steps.iter().any(|step| match step {
            flashfile::FlashOp::Erase { partition: name } => {
                flashfile::normalize_partition(name) == partition
            }
            _ => false,
        })
    })
}

/// The partitions the active erase groups erase *on top of* the package: the
/// ones it has no `erase` step for, which have to be added as extra commands.
fn extra_erase_partitions(flash: &FirmwareFlashState) -> Vec<String> {
    let mut extra: Vec<String> = Vec::new();

    for group in &flash.erase_groups {
        for partition in group.partitions() {
            if extra.iter().any(|name| name.as_str() == *partition)
                || package_erases_partition(flash, partition)
            {
                continue;
            }

            extra.push((*partition).to_owned());
        }
    }

    extra
}

/// Starts a bootloader device action (read Device ID / unlock): lists the
/// connected fastboot devices in the background. A single supported device
/// runs the action directly; several open the serial picker.
fn start_bootloader_list_devices(
    state: &mut State,
    action: BootloaderDeviceAction,
) -> Task<Message> {
    // Resolve and validate the unlock key up front so we never borrow the
    // bootloader state while asking the bundle for a localized message.
    if action == BootloaderDeviceAction::Unlock {
        let key = state.flash.bootloader.key_input.trim().to_string();
        if key.is_empty() {
            let message = state.l10n.tr("flash-bootloader-key-required");
            state.flash.bootloader.unlock_error = Some(message);
            return Task::none();
        }
        state.flash.bootloader.pending_key = Some(key);
    }

    let bootloader = &mut state.flash.bootloader;
    if bootloader.reading || bootloader.unlocking {
        return Task::none();
    }
    match action {
        BootloaderDeviceAction::ReadDeviceId => {
            bootloader.reading = true;
            bootloader.read_error = None;
            bootloader.read_info = None;
        }
        BootloaderDeviceAction::Unlock => {
            bootloader.unlocking = true;
            bootloader.unlock_error = None;
            bootloader.unlock_notice = None;
            bootloader.unlock_info = None;
        }
    }
    bootloader.picker = None;

    Task::perform(
        async {
            tokio::task::spawn_blocking(fastboot_info::list_devices)
                .await
                .unwrap_or_else(|e| Err(format!("background task failed: {e}")))
        },
        Message::BootloaderDevicesFetched,
    )
}

/// Handles the background device listing: runs the requested action when
/// exactly one supported device is connected, opens the picker for several,
/// or reports that no supported device is present.
fn finish_bootloader_device_list(
    state: &mut State,
    result: Result<fastboot_info::DeviceList, String>,
) -> Task<Message> {
    let (action, unlock_key) = {
        let bootloader = &state.flash.bootloader;
        if bootloader.reading {
            (Some(BootloaderDeviceAction::ReadDeviceId), None)
        } else if bootloader.unlocking {
            (
                Some(BootloaderDeviceAction::Unlock),
                bootloader.pending_key.clone(),
            )
        } else {
            (None, None)
        }
    };

    let Some(action) = action else {
        return Task::none();
    };

    match result {
        Ok(list) if list.devices.len() == 1 => {
            let serial = list.devices[0].serial.clone();
            let bootloader = &mut state.flash.bootloader;
            bootloader.pending_key = None;
            // Keep the busy flag raised so the completion handler accepts the
            // result: finish_bootloader_read / finish_bootloader_unlock only
            // apply the outcome while their flag is still set.
            match action {
                BootloaderDeviceAction::ReadDeviceId => {
                    bootloader.reading = true;
                    start_bootloader_read(serial)
                }
                BootloaderDeviceAction::Unlock => {
                    bootloader.unlocking = true;
                    start_bootloader_unlock(serial, unlock_key.unwrap_or_default())
                }
            }
        }
        Ok(list) if list.devices.len() > 1 => {
            let bootloader = &mut state.flash.bootloader;
            bootloader.reading = false;
            bootloader.unlocking = false;
            bootloader.picker = Some(BootloaderPicker {
                devices: list.devices,
                action,
            });
            Task::none()
        }
        Ok(list) => {
            let error_id = if list.total == 0 {
                "retcn-fill-fastboot-no-device"
            } else {
                "retcn-fill-fastboot-unsupported-device"
            };
            let message = state.l10n.tr(error_id);
            let bootloader = &mut state.flash.bootloader;
            bootloader.reading = false;
            bootloader.unlocking = false;
            bootloader.pending_key = None;
            match action {
                BootloaderDeviceAction::ReadDeviceId => bootloader.read_error = Some(message),
                BootloaderDeviceAction::Unlock => bootloader.unlock_error = Some(message),
            }
            Task::none()
        }
        Err(error) => {
            let bootloader = &mut state.flash.bootloader;
            bootloader.reading = false;
            bootloader.unlocking = false;
            bootloader.pending_key = None;
            match action {
                BootloaderDeviceAction::ReadDeviceId => bootloader.read_error = Some(error),
                BootloaderDeviceAction::Unlock => bootloader.unlock_error = Some(error),
            }
            Task::none()
        }
    }
}

/// Runs the picker's pending action on the device the user selected.
fn run_bootloader_picked_device(state: &mut State, serial: String) -> Task<Message> {
    let (action, unlock_key) = {
        let bootloader = &state.flash.bootloader;
        match bootloader.picker.as_ref().map(|picker| picker.action) {
            Some(BootloaderDeviceAction::ReadDeviceId) => {
                (BootloaderDeviceAction::ReadDeviceId, None)
            }
            Some(BootloaderDeviceAction::Unlock) => {
                (BootloaderDeviceAction::Unlock, bootloader.pending_key.clone())
            }
            None => return Task::none(),
        }
    };

    let bootloader = &mut state.flash.bootloader;
    bootloader.picker = None;
    match action {
        BootloaderDeviceAction::ReadDeviceId => {
            bootloader.reading = true;
            bootloader.read_error = None;
            bootloader.read_info = None;
        }
        BootloaderDeviceAction::Unlock => {
            bootloader.unlocking = true;
            bootloader.unlock_error = None;
            bootloader.unlock_notice = None;
            bootloader.unlock_info = None;
        }
    }

    match action {
        BootloaderDeviceAction::ReadDeviceId => start_bootloader_read(serial),
        BootloaderDeviceAction::Unlock => {
            start_bootloader_unlock(serial, unlock_key.unwrap_or_default())
        }
    }
}

/// Reads the Device ID (`fastboot oem get_unlock_data`) in the background.
fn start_bootloader_read(serial: String) -> Task<Message> {
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || fastboot_info::read_unlock_data(&serial))
                .await
                .unwrap_or_else(|e| Err(format!("background task failed: {e}")))
        },
        Message::BootloaderDeviceIdRead,
    )
}

/// Sends the unlock key (`fastboot oem unlock <key>`) in the background.
fn start_bootloader_unlock(serial: String, key: String) -> Task<Message> {
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || fastboot_info::unlock_bootloader(&serial, &key))
                .await
                .unwrap_or_else(|e| {
                    Err(fastboot_info::UnlockFailure::Other(format!(
                        "background task failed: {e}"
                    )))
                })
        },
        Message::BootloaderUnlockFinished,
    )
}

/// Asks Motorola whether the device qualifies for bootloader unlock in the
/// background (uses the Device ID just read from fastboot).
fn start_unlock_eligibility_check(device_id: String) -> Task<Message> {
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || firmware::check_unlock_eligibility(&device_id))
                .await
                .unwrap_or_else(|e| Err(format!("background task failed: {e}")))
        },
        Message::BootloaderEligibilityChecked,
    )
}

/// Stores a successfully read Device ID (and starts the unlock-eligibility
/// check) or surfaces the read error.
///
/// Only the Manual flow copies the Device ID to the clipboard (so the user
/// can paste it into Motorola's site); the Guided flow consumes the ID
/// internally and never needs the clipboard.
fn finish_bootloader_read(state: &mut State, result: Result<String, String>) -> Task<Message> {
    if !state.flash.bootloader.reading {
        return Task::none();
    }

    let manual = matches!(
        state.flash.bootloader.dialog,
        BootloaderDialog::Manual
    );
    // Only the Manual flow needs the "copied to clipboard" confirmation.
    let copied = if manual {
        Some(state.l10n.tr("flash-bootloader-copied"))
    } else {
        None
    };

    match result {
        Ok(device_id) => {
            let bootloader = &mut state.flash.bootloader;
            bootloader.reading = false;
            bootloader.checking = true;
            bootloader.eligible = None;
            bootloader.read_error = None;
            bootloader.read_info = copied;
            bootloader.device_id = device_id.clone();
            if manual {
                Task::batch([
                    iced::clipboard::write::<Message>(device_id.clone()),
                    start_unlock_eligibility_check(device_id),
                ])
            } else {
                // Guided flow: no clipboard write, no "copied" status line.
                start_unlock_eligibility_check(device_id)
            }
        }
        Err(error) => {
            let bootloader = &mut state.flash.bootloader;
            bootloader.reading = false;
            bootloader.checking = false;
            bootloader.eligible = None;
            bootloader.read_error = Some(error);
            Task::none()
        }
    }
}

/// Handles the result of the `fastboot oem unlock <key>` command.
fn finish_bootloader_unlock(
    state: &mut State,
    result: Result<String, fastboot_info::UnlockFailure>,
) -> Task<Message> {
    if !state.flash.bootloader.unlocking {
        return Task::none();
    }

    match result {
        Ok(_device_text) => {
            let unlocked = state.l10n.tr("flash-bootloader-unlocked");
            let bootloader = &mut state.flash.bootloader;
            bootloader.unlocking = false;
            bootloader.unlock_error = None;
            bootloader.unlock_notice = None;
            bootloader.unlock_info = Some(unlocked);
            bootloader.pending_key = None;
            Task::none()
        }
        Err(fastboot_info::UnlockFailure::OemUnlockingDisabled) => {
            let message = state.l10n.tr("flash-bootloader-oem-unlocking-required");
            let bootloader = &mut state.flash.bootloader;
            bootloader.unlocking = false;
            bootloader.unlock_error = Some(message);
            bootloader.unlock_notice = None;
            bootloader.unlock_info = None;
            bootloader.pending_key = None;
            Task::none()
        }
        Err(fastboot_info::UnlockFailure::Cancelled) => {
            let message = state.l10n.tr("flash-bootloader-unlock-cancelled");
            let bootloader = &mut state.flash.bootloader;
            bootloader.unlocking = false;
            bootloader.unlock_error = None;
            bootloader.unlock_notice = Some(message);
            bootloader.unlock_info = None;
            bootloader.pending_key = None;
            Task::none()
        }
        Err(fastboot_info::UnlockFailure::WrongKey) => {
            let message = state.l10n.tr("flash-bootloader-unlock-wrong-key");
            let bootloader = &mut state.flash.bootloader;
            bootloader.unlocking = false;
            bootloader.unlock_error = Some(message);
            bootloader.unlock_notice = None;
            bootloader.unlock_info = None;
            bootloader.pending_key = None;
            Task::none()
        }
        Err(fastboot_info::UnlockFailure::AlreadyUnlocked) => {
            // The device is already unlocked — not an error, so show it as a
            // success-style info line.
            let message = state.l10n.tr("flash-bootloader-already-unlocked");
            let bootloader = &mut state.flash.bootloader;
            bootloader.unlocking = false;
            bootloader.unlock_error = None;
            bootloader.unlock_notice = None;
            bootloader.unlock_info = Some(message);
            bootloader.pending_key = None;
            Task::none()
        }
        Err(fastboot_info::UnlockFailure::Other(error)) => {
            let bootloader = &mut state.flash.bootloader;
            bootloader.unlocking = false;
            bootloader.unlock_error = Some(error);
            bootloader.unlock_notice = None;
            bootloader.unlock_info = None;
            bootloader.pending_key = None;
            Task::none()
        }
    }
}

/// Starts the tablet identity read by listing the connected fastboot devices.
fn start_tablet_read(state: &mut State) -> Task<Message> {
    let tablet = &mut state.flash.tablet_unlock;
    if tablet.reading || tablet.unlocking {
        return Task::none();
    }
    tablet.reading = true;
    tablet.read_error = None;
    tablet.read_notice = None;
    tablet.picker = None;

    Task::perform(
        async {
            tokio::task::spawn_blocking(fastboot::FastbootDevice::list_devices)
                .await
                .unwrap_or_else(|e| Err(format!("background task failed: {e}")))
        },
        Message::TabletUnlockDevicesFetched,
    )
}

/// Handles the tablet device listing: reads directly from a single device,
/// opens the picker for several, or reports that none is connected.
fn finish_tablet_device_list(
    state: &mut State,
    result: Result<Vec<String>, String>,
) -> Task<Message> {
    if !state.flash.tablet_unlock.reading {
        return Task::none();
    }

    match result {
        Ok(devices) if devices.len() == 1 => start_tablet_read_device(state, devices[0].clone()),
        Ok(devices) if devices.len() > 1 => {
            let tablet = &mut state.flash.tablet_unlock;
            tablet.reading = false;
            tablet.picker = Some(TabletPicker { devices });
            Task::none()
        }
        Ok(_) => {
            let message = state.l10n.tr("retcn-fill-fastboot-no-device");
            let tablet = &mut state.flash.tablet_unlock;
            tablet.reading = false;
            tablet.read_error = Some(message);
            Task::none()
        }
        Err(error) => {
            let tablet = &mut state.flash.tablet_unlock;
            tablet.reading = false;
            tablet.read_error = Some(error);
            Task::none()
        }
    }
}

/// Reads the tablet identity from the chosen device in the background.
fn start_tablet_read_device(state: &mut State, serial: String) -> Task<Message> {
    let tablet = &mut state.flash.tablet_unlock;
    tablet.picker = None;
    tablet.reading = true;
    tablet.read_error = None;
    tablet.read_notice = None;

    let device = serial.clone();
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || tablet_unlock::read_info(&serial))
                .await
                .unwrap_or_else(|e| Err(format!("background task failed: {e}")))
        },
        move |result| Message::TabletUnlockInfoRead(device.clone(), result),
    )
}

/// Stores the identity read from a device, or surfaces the read error.
fn finish_tablet_read(
    state: &mut State,
    device: String,
    result: Result<tablet_unlock::TabletUnlockInfo, String>,
) -> Task<Message> {
    if !state.flash.tablet_unlock.reading {
        return Task::none();
    }

    match result {
        Ok(info) => {
            // A readable Bootloader_SN is exactly the "enhanced" case, which
            // ZUI files under the Legion Y700 5th Gen section.
            let notice = info
                .has_bootloader_sn()
                .then(|| state.l10n.tr("flash-bootloader-tablet-legion"));

            let tablet = &mut state.flash.tablet_unlock;
            tablet.reading = false;
            tablet.read_error = None;
            tablet.read_notice = notice;
            tablet.device = Some(device);
            tablet.bootloader_sn = info.bootloader_sn();
            tablet.bootloader_sn_part1 = info.bootloader_sn_part1;
            tablet.bootloader_sn_part2 = info.bootloader_sn_part2;
            tablet.serial = info.serial;
            tablet.platform = info.platform;
            tablet.read_done = true;
        }
        Err(error) => {
            let tablet = &mut state.flash.tablet_unlock;
            tablet.reading = false;
            tablet.read_error = Some(error);
            tablet.platform = None;
        }
    }

    Task::none()
}

/// Starts the ZUI unlock: download the token for the values read earlier,
/// flash it to the `unlock` partition, then send `oem unlock-go`.
fn start_tablet_unlock(state: &mut State) -> Task<Message> {
    let tablet = &state.flash.tablet_unlock;
    if tablet.reading || tablet.unlocking {
        return Task::none();
    }
    let Some(device) = tablet.device.clone() else {
        return Task::none();
    };
    if tablet.serial.trim().is_empty() {
        return Task::none();
    }
    let info = tablet.info();

    let tablet = &mut state.flash.tablet_unlock;
    tablet.unlocking = true;
    tablet.cancelling = false;
    tablet.unlock_error = None;
    tablet.unlock_info = None;

    let cancel = Arc::new(AtomicBool::new(false));
    tablet.cancel = Some(cancel.clone());

    let token = tablet_unlock::token_path();
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                tablet_unlock::unlock(&device, &info, &token, &cancel)
            })
            .await
            .unwrap_or_else(|e| {
                tablet_unlock::UnlockOutcome::Failed(format!("background task failed: {e}"))
            })
        },
        Message::TabletUnlockFinished,
    )
}

/// "Cancel & Restart": asks the running unlock to stop at its next checkpoint.
///
/// The card stays on the unlocking state until the job acknowledges the
/// request, so no second job can touch the device while the first one is still
/// finishing.
fn cancel_tablet_unlock(state: &mut State) -> Task<Message> {
    let tablet = &mut state.flash.tablet_unlock;
    if !tablet.unlocking || tablet.cancelling {
        return Task::none();
    }

    if let Some(cancel) = &tablet.cancel {
        cancel.store(true, Ordering::Relaxed);
    }
    tablet.cancelling = true;

    Task::none()
}

/// Asks the unlock the manual card still runs to stop (used when the dialog is
/// left); the job clears itself when its outcome arrives.
fn stop_tablet_unlock(tablet: &mut TabletUnlockState) {
    if let Some(cancel) = &tablet.cancel {
        cancel.store(true, Ordering::Relaxed);
    }

    tablet.unlocking = false;
    tablet.cancelling = false;
    tablet.cancel = None;
}

/// Shows the result of the ZUI unlock.
fn finish_tablet_unlock(
    state: &mut State,
    outcome: tablet_unlock::UnlockOutcome,
) -> Task<Message> {
    if !state.flash.tablet_unlock.unlocking {
        return Task::none();
    }

    let l10n = &state.l10n;
    let (info, error) = match outcome {
        tablet_unlock::UnlockOutcome::Unlocked => (
            Some(l10n.tr("flash-bootloader-tablet-confirm-unlock")),
            None,
        ),
        tablet_unlock::UnlockOutcome::NotGenerated => (
            None,
            Some(l10n.tr("flash-bootloader-tablet-not-generated")),
        ),
        tablet_unlock::UnlockOutcome::FlashFailed(message) => (
            None,
            Some(l10n.tr_with_args(
                "flash-bootloader-tablet-unknown-error",
                &[("error", message)],
            )),
        ),
        tablet_unlock::UnlockOutcome::OemUnlockingDisabled => (
            None,
            Some(l10n.tr("flash-bootloader-oem-unlocking-required")),
        ),
        tablet_unlock::UnlockOutcome::Failed(message) => (None, Some(message)),
        // "Cancel & Restart": the card goes back to the idle state it was in
        // before the unlock, keeping the values that were read (they are the
        // ones the user submitted on ZUI's website).
        tablet_unlock::UnlockOutcome::Cancelled => (None, None),
    };

    let tablet = &mut state.flash.tablet_unlock;
    tablet.unlocking = false;
    tablet.cancelling = false;
    tablet.cancel = None;
    tablet.unlock_info = info;
    tablet.unlock_error = error;
    Task::none()
}

/// Lists the connected fastboot devices for the guided tablet flow.
fn start_tablet_guided_listing() -> Task<Message> {
    Task::perform(
        async {
            tokio::task::spawn_blocking(tablet_unlock::list_devices)
                .await
                .unwrap_or_else(|e| Err(format!("background task failed: {e}")))
        },
        Message::TabletGuidedDevicesFetched,
    )
}

/// Stores the guided device listing, or the error the scan hit.
fn finish_tablet_guided_list(
    state: &mut State,
    result: Result<Vec<tablet_unlock::TabletDevice>, String>,
) -> Task<Message> {
    let guided = &mut state.flash.tablet_unlock.guided;
    if !guided.listing {
        return Task::none();
    }

    guided.listing = false;
    match result {
        Ok(devices) => {
            guided.devices = devices;
            guided.list_error = None;
        }
        Err(error) => {
            guided.devices.clear();
            guided.list_error = Some(error);
        }
    }

    Task::none()
}

/// Starts the guided (automatic) unlock on the chosen device.
fn start_tablet_guided_unlock(state: &mut State, serial: String) -> Task<Message> {
    let guided = &mut state.flash.tablet_unlock.guided;
    if guided.unlocking {
        return Task::none();
    }

    guided.job = guided.job.wrapping_add(1);
    let job = guided.job;
    guided.step = TabletGuidedStep::Working;
    guided.unlocking = true;
    guided.cancelling = false;
    guided.platform = None;
    guided.confirming = false;
    guided.result = None;

    let cancel = Arc::new(AtomicBool::new(false));
    guided.cancel = Some(cancel.clone());

    let token = tablet_unlock::token_path();
    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    let emit = move |event| {
        let _ = sender.unbounded_send(event);
    };

    let producer = Task::perform(
        async move {
            match tokio::task::spawn_blocking(move || {
                tablet_unlock::guided_unlock(&serial, &token, &emit, &cancel)
            })
            .await
            {
                Ok(report) => report,
                Err(error) => tablet_unlock::GuidedReport {
                    device: String::new(),
                    info: tablet_unlock::TabletUnlockInfo::default(),
                    outcome: tablet_unlock::GuidedOutcome::Failed(format!(
                        "background task failed: {error}"
                    )),
                },
            }
        },
        move |report| Message::TabletGuidedFinished(job, report),
    );

    let consumer = Task::run(receiver, move |event| Message::TabletGuidedEvent(job, event));

    Task::batch([producer, consumer])
}

/// Records a progress event from the running guided unlock.
fn handle_tablet_guided_event(state: &mut State, job: u64, event: tablet_unlock::GuidedEvent) {
    let guided = &mut state.flash.tablet_unlock.guided;
    if job != guided.job || !guided.unlocking {
        return;
    }

    match event {
        tablet_unlock::GuidedEvent::Platform(platform) => guided.platform = platform,
        tablet_unlock::GuidedEvent::AwaitingConfirm => guided.confirming = true,
    }
}

/// "Cancel & Restart": asks the running job to stop at its next checkpoint.
///
/// The card stays on the working step (showing that it is cancelling) until the
/// job acknowledges the request, so no second job can touch the device while
/// the first one is still finishing.
fn cancel_tablet_guided(state: &mut State) -> Task<Message> {
    let guided = &mut state.flash.tablet_unlock.guided;
    if !guided.unlocking || guided.cancelling {
        return Task::none();
    }

    if let Some(cancel) = &guided.cancel {
        cancel.store(true, Ordering::Relaxed);
    }
    guided.cancelling = true;

    Task::none()
}

/// Asks any job the guided flow still runs to stop (used when the dialog is
/// left); the job clears itself when its report arrives.
fn stop_tablet_guided(guided: &mut TabletGuidedState) {
    if let Some(cancel) = &guided.cancel {
        cancel.store(true, Ordering::Relaxed);
    }
}

/// Shows the guided outcome, or hands the flow over to the manual card.
fn finish_tablet_guided(
    state: &mut State,
    job: u64,
    report: tablet_unlock::GuidedReport,
) -> Task<Message> {
    if job != state.flash.tablet_unlock.guided.job || !state.flash.tablet_unlock.guided.unlocking {
        return Task::none();
    }

    if report.outcome == tablet_unlock::GuidedOutcome::Cancelled {
        // "Cancel & Restart": back to the device list.
        let guided = &mut state.flash.tablet_unlock.guided;
        guided.unlocking = false;
        guided.cancelling = false;
        guided.cancel = None;
        guided.platform = None;
        guided.confirming = false;
        guided.result = None;
        guided.step = TabletGuidedStep::Devices;
        guided.listing = true;
        guided.list_error = None;
        guided.devices.clear();

        return start_tablet_guided_listing();
    }

    if report.outcome == tablet_unlock::GuidedOutcome::ManualRequired {
        // Nothing could be obtained automatically: pre-fill the manual flow
        // with what was read and let the user continue from there.
        let notice = state.l10n.tr("flash-bootloader-tablet-guided-manual");
        let bootloader_sn = report.info.bootloader_sn();

        let tablet = &mut state.flash.tablet_unlock;
        tablet.dialog = TabletUnlockDialog::Manual;
        tablet.guided.unlocking = false;
        tablet.guided.cancelling = false;
        tablet.guided.cancel = None;
        tablet.guided.confirming = false;
        tablet.device = Some(report.device);
        tablet.serial = report.info.serial;
        tablet.bootloader_sn = bootloader_sn;
        tablet.bootloader_sn_part1 = report.info.bootloader_sn_part1;
        tablet.bootloader_sn_part2 = report.info.bootloader_sn_part2;
        tablet.platform = report.info.platform;
        tablet.read_done = true;
        tablet.read_error = None;
        tablet.read_notice = Some(notice);

        return Task::none();
    }

    let guided = &mut state.flash.tablet_unlock.guided;
    guided.unlocking = false;
    guided.cancelling = false;
    guided.cancel = None;
    guided.confirming = false;
    guided.result = Some(report.outcome);
    Task::none()
}

fn view(state: &State) -> Element<'_, Message> {
    let content = match state.mode {
        Mode::Mode1 => lookup::firmware_lookup_view(state),
        // The Multi Flashing page (entered from the Firmware Flash dialog)
        // takes the place of the Mode 2 feature grid.
        Mode::Mode2 if state.flash.multi_flash.open => multi_flash::view(state),
        // The Partition Manager is a workspace of its own as well.
        Mode::Mode2 if qualcomm::is_open(state) => qualcomm::view(state),
        Mode::Mode2 => smartphone::smartphone_flash_view(state),
        Mode::Mode3 => decrypt_mode::view(state),
    };
    let language_options: Vec<Labeled<l10n::Language>> = l10n::Language::ALL
        .iter()
        .map(|&language| Labeled {
            value: language,
            label: state.l10n.tr(language.message_id()),
        })
        .collect();
    let selected_language = Labeled {
        value: state.lang,
        label: state.l10n.tr(state.lang.message_id()),
    };
    let language_dropdown = pick_list(language_options, Some(selected_language), |option| {
        Message::LanguageSelected(option.value)
    })
    .text_shaping(Shaping::Advanced);

    // The tab that was just switched to slides in a little and fades in from
    // the window background colour (see `anim`). A transition that covers the
    // chrome animates the whole window instead, so the content is left alone
    // here and is animated together with that chrome further down.
    //
    // A change of the login page is drawn the same way, one layer in: it is
    // posed inside the tab switch, so both can be running at once. The login
    // page is Mode 1's content, so a login that completes while another
    // feature is on screen (a browser callback arriving late) leaves that
    // feature alone.
    let login_pose = match state.mode {
        Mode::Mode1 => state
            .login_transition
            .as_ref()
            .map(anim::Transition::pose),
        _ => None,
    };

    // A change of the lookup mode — or of the status below the form: asking
    // for a firmware shows a spinner, and its result takes over the form — is
    // a change of the form one layer below the login page: the form is posed
    // where it is drawn (see `lookup`), and only its veil is added here, so
    // the selector itself keeps its place while the form fades in.
    let lookup_pose = match state.mode {
        Mode::Mode1 => state
            .lookup_transition
            .as_ref()
            .map(anim::Transition::pose),
        _ => None,
    };

    let content = match login_pose {
        Some(pose) => anim::animated(content, pose),
        None => content,
    };

    let mut content_layers: Vec<Element<'_, Message>> = match &state.content_transition {
        Some(transition) if !state.content_transition_covers_chrome => {
            let pose = transition.pose();
            vec![
                anim::animated(content, pose),
                anim::veil(anim::window_background(state.dark), pose.veil),
            ]
        }
        _ => vec![content],
    };

    // The login veil goes above the content but below the tab switch's veil
    // and below the language selector pushed next, so the chrome never
    // flickers with the login page.
    if let Some(pose) = login_pose {
        content_layers.push(anim::veil(anim::window_background(state.dark), pose.veil));
    }

    // The lookup form's veil goes in the same place, so a mode (or a status of
    // the request below it) that changes while the login page is settled still
    // fades in from the background.
    if let Some(pose) = lookup_pose {
        content_layers.push(anim::veil(anim::window_background(state.dark), pose.veil));
    }

    // The Multi Flashing page owns the whole window: the language selector
    // and the mode tabs are hidden there, since the page is left through its
    // own Exit button.
    let multi_flashing = state.flash.multi_flash.open;

    // The Partition Manager is a full-window workspace as well, left
    // through its own Exit button.
    let qualcomm_page = qualcomm::is_open(state);

    // The Repack Package dialog is a full-window card as well, so the language
    // selector would show through while that card animates in and out: it is
    // left out while the dialog is open.
    let full_window = multi_flashing
        || qualcomm_page
        || pac_repack::is_open(state)
        || pac_unpack::is_open(state);

    // The language selector sits in the top-right corner; on the Firmware
    // Lookup page a round "About" button is drawn to its left. They stay above
    // the transition's veil, so they do not flicker along with the content.
    if !full_window {
        let corner: Element<'_, Message> = if state.mode == Mode::Mode1 {
            row![about::about_button(), language_dropdown]
                .spacing(8)
                .align_y(Alignment::Center)
                .into()
        } else {
            language_dropdown.into()
        };

        content_layers.push(
            container(corner)
                .padding(8)
                .align_top(Fill)
                .align_right(Fill)
                .into(),
        );
    }

    let content_area = iced::widget::Stack::with_children(content_layers);

    let tab_bar = container(
        row(Mode::ALL.iter().map(|&mode| {
            let tab = button(text(state.l10n.tr(mode.message_id())))
                .width(Fill)
                .on_press(Message::ModeSelected(mode));

            if mode == state.mode {
                tab.style(button::primary)
            } else {
                tab
            }
            .into()
        }))
        .spacing(4),
    )
    .padding(8)
    .width(Fill);

    // The mode tabs are left out of the full-window pages entirely (see
    // `multi_flashing` and `qualcomm_page` above), so no tab is ever
    // clickable while one of them is open.
    let base: Element<'_, Message> = if multi_flashing || qualcomm_page {
        content_area.into()
    } else {
        column![content_area, horizontal_rule(1), tab_bar].into()
    };

    // Entering or leaving the Multi Flashing page is a full-screen change:
    // the language selector and the mode tabs come and go with the content, so
    // the whole window is what slides and fades — rather than the content
    // sliding while the chrome pops in or out.
    let base: Element<'_, Message> = match &state.content_transition {
        Some(transition) if state.content_transition_covers_chrome => {
            let pose = transition.pose();

            iced::widget::Stack::with_children(vec![
                anim::animated(base, pose),
                anim::veil(anim::window_background(state.dark), pose.veil),
            ])
            .into()
        }
        _ => base,
    };

    // The Partition Manager's own prompts are the last thing on screen.
    if let Some(overlay) = qualcomm::close_prompt_overlay(state) {
        return stack![base, animated_overlay(state, overlay)].into();
    }

    // The LUN picker of the Partition Manager's "Dump Entire Storage", drawn
    // over the page (which replaced the tab bar) while it is open.
    if let Some(overlay) = qualcomm::lun_overlay(state) {
        return stack![base, animated_overlay(state, overlay)].into();
    }

    // The Multi Flashing page's modals: its "Preset Manage" dialog, and the
    // prompt that a close request was refused while a run goes on.
    if let Some(overlay) = multi_flash::close_prompt_overlay(state) {
        return stack![base, animated_overlay(state, overlay)].into();
    }

    if let Some(overlay) = multi_flash::preset_options_overlay(state) {
        return stack![base, animated_overlay(state, overlay)].into();
    }

    if let Some(overlay) = multi_flash::preset_overlay(state) {
        return stack![base, animated_overlay(state, overlay)].into();
    }

    // Bootloader Unlock dialog (chooser / manual flow) drawn over the whole
    // window (including the tab bar) when a dialog is open.
    if let Some(overlay) = bootloader::overlay(state) {
        return stack![base, animated_overlay(state, overlay)].into();
    }

    // Tablet Bootloader Unlock dialog (chooser / ZUI manual flow) drawn over
    // the whole window when it is open.
    if let Some(overlay) = tablet_bootloader::overlay(state) {
        return stack![base, animated_overlay(state, overlay)].into();
    }

    // Factory Reset dialog (device selection / requirement checks / confirm)
    // drawn over the whole window when it is open.
    if let Some(overlay) = factory_reset::overlay(state) {
        return stack![base, animated_overlay(state, overlay)].into();
    }

    // Firmware Flash dialog (package + device selection, progress and log)
    // drawn over the whole window while it is open.
    if let Some(overlay) = firmware_flash::overlay(state) {
        return stack![base, animated_overlay(state, overlay)].into();
    }

    // Install Driver dialog (download progress / sudo password) drawn over
    // the whole window while it is open.
    if let Some(overlay) = driver::overlay(state) {
        return stack![base, animated_overlay(state, overlay)].into();
    }

    // Single Partition Flash dialog (device, tool, partition and image
    // selection, the running flash with its log) drawn over the whole window
    // while it is open.
    if let Some(overlay) = partition_flash::overlay(state) {
        return stack![base, animated_overlay(state, overlay)].into();
    }

    // Remove System Bloatware dialog (device selection, app checklist,
    // removal) drawn over the whole window while it is open.
    if let Some(overlay) = bloatware::overlay(state) {
        return stack![base, animated_overlay(state, overlay)].into();
    }

    // Connect ADB Wireless Debugging dialog (QR code, or pairing code) drawn
    // over the whole window while it is open.
    if let Some(overlay) = wireless_dialog::overlay(state) {
        return stack![base, animated_overlay(state, overlay)].into();
    }

    // Fix Captive Portal Server dialog (device selection, the note about the
    // network being reset, the running fix) drawn over the whole window while
    // it is open.
    if let Some(overlay) = captive_portal_dialog::overlay(state) {
        return stack![base, animated_overlay(state, overlay)].into();
    }

    // Unpack Image dialog (image and output selection, contents, progress)
    // drawn over the whole window while it is open.
    if let Some(overlay) = unpack::overlay(state) {
        return stack![base, animated_overlay(state, overlay)].into();
    }

    // Tablet Firmware Decrypt dialog (directory, password, progress) drawn
    // over the whole window while it is open.
    if let Some(overlay) = decrypt::overlay(state) {
        return stack![base, animated_overlay(state, overlay)].into();
    }

    // Unisoc PAC dialog (package and output selection, file list, progress)
    // drawn over the whole window while it is open.
    if let Some(overlay) = pac_unpack::overlay(state) {
        return stack![base, animated_overlay(state, overlay)].into();
    }

    // Repack Package dialog (source selection, record list, progress) drawn
    // over the whole window while it is open.
    if let Some(overlay) = pac_repack::overlay(state) {
        return stack![base, animated_overlay(state, overlay)].into();
    }

    // About dialog (opened from the Firmware Lookup page) drawn over the whole
    // window while it is open.
    if let Some(overlay) = about::overlay(state) {
        return stack![base, animated_overlay(state, overlay)].into();
    }

    // Device picker modal: shown when more than one fastboot device is
    // connected. Clicks on the dimmed backdrop cancel the selection.
    if let Some(overlay) = lookup::device_picker(state) {
        return stack![base, animated_overlay(state, overlay)].into();
    }

    base.into()
}

/// Draws an open dialog: normally in its settled pose behind a dimming scrim,
/// and posed (rising, sliding in, or dropping away) while a transition runs.
///
/// The scrim is part of the modal's settled look, not just of the animation,
/// so a dialog that is not moving still carries it. Closing is animated by
/// holding the close back (see `update`), so while the card is leaving its
/// buttons are blocked and clicks cannot start anything.
fn animated_overlay<'a>(
    state: &State,
    overlay: Element<'a, Message>,
) -> Element<'a, Message> {
    let pose = state
        .overlay_transition
        .as_ref()
        .map_or_else(anim::dialog_rest, |transition| transition.pose());

    let mut children: Vec<Element<'a, Message>> = vec![
        anim::scrim(pose.veil),
        anim::animated(overlay, pose),
    ];

    if state.pending_close.is_some() {
        children.push(
            mouse_area(Space::new(Fill, Fill))
                .on_press(Message::OverlayLeavingPressed)
                .into(),
        );
    }

    iced::widget::Stack::with_children(children).into()
}

/// The firmware lookup UI shown after a successful login.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traditional_chinese_locales_are_detected() {
        assert!(is_traditional_chinese("zh-TW"));
        assert!(is_traditional_chinese("zh-HK"));
        assert!(is_traditional_chinese("zh_MO"));
        assert!(is_traditional_chinese("zh-Hant"));
        assert!(!is_traditional_chinese("zh-CN"));
        assert!(!is_traditional_chinese("zh-Hans"));
        assert!(!is_traditional_chinese("en-US"));
        assert!(!is_traditional_chinese(""));
    }

    #[test]
    fn model_category_is_detected_from_prefix() {
        assert_eq!(
            detect_category("XT2125-4"),
            Some((firmware::Category::Phone, None))
        );
        assert_eq!(
            detect_category("TB350FU"),
            Some((firmware::Category::Tablet, None))
        );
        assert_eq!(
            detect_category("PC-TB300FU"),
            Some((firmware::Category::Tablet, Some("JP")))
        );
        assert_eq!(
            detect_category("CD1901"),
            Some((firmware::Category::Smart, None))
        );
        assert_eq!(
            detect_category("SD650A"),
            Some((firmware::Category::Smart, None))
        );
        assert_eq!(detect_category("  XT2125-4 "), Some((firmware::Category::Phone, None)));
        assert_eq!(detect_category(""), None);
        assert_eq!(detect_category("XYZ123"), None);
    }

    /// Cancelling the directory picker of a firmware that is being added means
    /// the user does not want that firmware after all: the options card is
    /// dropped and the preset list comes back, exactly as it does when the
    /// file pick is cancelled.
    #[test]
    fn a_cancelled_first_directory_pick_drops_the_firmware() {
        let mut state = State::default();
        state.flash.multi_flash.pending = PresetOptions {
            path: Some(std::path::PathBuf::from(r"D:\firmware\arcfox.zip")),
            identity: Some(Ok(flashfile::PackageIdentity::default())),
            ..PresetOptions::default()
        };

        let _ = update(
            &mut state,
            Message::MultiFlashPresetDirectoryPicked(None, DirectoryAsk::New),
        );

        assert!(state.flash.multi_flash.pending.path.is_none());
        assert!(state.flash.multi_flash.pending.identity.is_none());
    }

    /// Changing a directory that is set already is not a change of mind about
    /// the firmware: cancelling that pick leaves the card with what it had.
    #[test]
    fn a_cancelled_directory_change_keeps_the_card() {
        let mut state = State::default();
        let zip = std::path::PathBuf::from(r"D:\firmware\arcfox.zip");
        let directory = std::path::PathBuf::from(r"D:\firmware\arcfox");

        state.flash.multi_flash.pending = PresetOptions {
            path: Some(zip.clone()),
            identity: Some(Ok(flashfile::PackageIdentity::default())),
            extract_to: Some(directory.clone()),
            ..PresetOptions::default()
        };

        let _ = update(
            &mut state,
            Message::MultiFlashPresetDirectoryPicked(None, DirectoryAsk::Change),
        );

        assert_eq!(state.flash.multi_flash.pending.path, Some(zip));
        assert_eq!(state.flash.multi_flash.pending.extract_to, Some(directory));
    }

    /// The bar of a device of a Multi Flashing run counts the procedures that
    /// are done, plus how far the running one got — and never leaves 0 to 1,
    /// whatever the tool reports.
    #[test]
    fn a_device_fraction_counts_the_procedures_that_are_done() {
        assert_eq!(device_fraction(0, 0.0, 4), 0.0);
        assert_eq!(device_fraction(1, 0.5, 4), 0.375);
        assert_eq!(device_fraction(3, 1.0, 4), 1.0);

        // A preset that reported no procedure at all (a package that could not
        // be read): the bar stays empty instead of dividing by zero.
        assert_eq!(device_fraction(0, 0.0, 0), 0.0);

        // A report past the end of the preset does not overfill the bar.
        assert_eq!(device_fraction(9, 0.0, 4), 1.0);
        assert_eq!(device_fraction(2, 3.0, 4), 1.0);
    }

    /// Reports a device the session found, the way `serve` does.
    fn identified(state: &mut State, session: u64, serial: &str, steps: Option<usize>) {
        let _ = update(
            state,
            Message::MultiFlashSessionEvent(
                session,
                multi_flash_run::SessionEvent::Identified {
                    serial: serial.to_owned(),
                    xt_code: Some("XT2451-2".to_owned()),
                    cid: Some("0x0032".to_owned()),
                    steps,
                },
            ),
        );
    }

    /// "Stop" ends the run, not the flashes it started: a device that is being
    /// written to keeps its row (the user watches it finish), the devices the
    /// run is done with go away, and the window cannot be closed until every
    /// one of them reports its end.
    #[test]
    fn stopping_keeps_the_devices_that_are_still_being_flashed() {
        let mut state = State::default();
        state.flash.multi_flash.running = true;
        let session = state.flash.multi_flash.session;

        // A run in progress: closing is refused and the prompt is up.
        let _ = update(
            &mut state,
            Message::WindowCloseRequested(iced::window::Id::unique()),
        );
        assert!(state.flash.multi_flash.close_prompt);

        // Three devices: one that is flashing, one that finished, and one no
        // preset accepts. Only the first is work that is still going on.
        identified(&mut state, session, "ZY00000001", Some(48));
        identified(&mut state, session, "ZY00000002", Some(12));
        identified(&mut state, session, "ZY00000003", None);
        let _ = update(
            &mut state,
            Message::MultiFlashSessionEvent(
                session,
                multi_flash_run::SessionEvent::Finished {
                    serial: "ZY00000002".to_owned(),
                    result: Ok(()),
                },
            ),
        );
        assert_eq!(state.flash.multi_flash.devices.len(), 3);

        let _ = update(&mut state, Message::MultiFlashStop);
        assert!(!state.flash.multi_flash.running);
        assert_eq!(state.flash.multi_flash.flashing.len(), 1);
        assert_eq!(state.flash.multi_flash.devices.len(), 1);
        assert_eq!(state.flash.multi_flash.devices[0].serial, "ZY00000001");

        // The device that is still being written to keeps reporting, and its
        // row is the one the window is waiting for.
        let _ = update(
            &mut state,
            Message::MultiFlashSessionEvent(
                session,
                multi_flash_run::SessionEvent::Step {
                    serial: "ZY00000001".to_owned(),
                    index: 4,
                    total: 48,
                    label: "flash super (super.img)".to_owned(),
                },
            ),
        );
        assert!(matches!(
            state.flash.multi_flash.devices[0].status,
            DeviceStatus::Flashing { index: 4, .. }
        ));

        // A device the session was already serving when "Stop" arrived is
        // shown too: it is being written to, and the window is waiting for it.
        identified(&mut state, session, "ZY00000004", Some(48));
        assert_eq!(state.flash.multi_flash.devices.len(), 2);

        state.flash.multi_flash.close_prompt = false;
        let _ = update(
            &mut state,
            Message::WindowCloseRequested(iced::window::Id::unique()),
        );
        assert!(state.flash.multi_flash.close_prompt);

        // Their flashes end: nothing is being written any more, so the window
        // may be closed again (the rows stay, showing what happened).
        for serial in ["ZY00000001", "ZY00000004"] {
            let _ = update(
                &mut state,
                Message::MultiFlashSessionEvent(
                    session,
                    multi_flash_run::SessionEvent::Finished {
                        serial: serial.to_owned(),
                        result: Ok(()),
                    },
                ),
            );
        }

        assert!(state.flash.multi_flash.flashing.is_empty());
        assert_eq!(state.flash.multi_flash.devices.len(), 2);
        assert!(
            state
                .flash
                .multi_flash
                .devices
                .iter()
                .all(|device| matches!(device.status, DeviceStatus::Finished))
        );

        state.flash.multi_flash.close_prompt = false;
        let _ = update(
            &mut state,
            Message::WindowCloseRequested(iced::window::Id::unique()),
        );
        assert!(!state.flash.multi_flash.close_prompt);
    }

    /// The CID dropdown of the options card puts the code it names into the
    /// field (Add, or Enter, then takes it like a typed one), and the rule
    /// between two groups of it does nothing at all.
    #[test]
    fn picking_a_cid_from_the_dropdown_fills_the_field() {
        let mut state = State::default();
        state.flash.multi_flash.pending.custom = true;

        let retcn = cid::MENU
            .iter()
            .find_map(|item| match item {
                cid::CidMenuItem::Cid { code, .. } if *code == "0x000B" => Some(*code),
                _ => None,
            })
            .expect("the list names RETCN");

        let _ = update(&mut state, Message::MultiFlashCidPresetSelected(Some(retcn)));
        assert_eq!(state.flash.multi_flash.pending.cid_input, "0x000B");

        // A code is put in as the app writes CIDs, so Add takes it unchanged.
        let _ = update(
            &mut state,
            Message::MultiFlashCidPresetSelected(Some("0x0032")),
        );
        assert_eq!(state.flash.multi_flash.pending.cid_input, "0x0032");

        // The rule is a line to read, not a CID.
        let _ = update(&mut state, Message::MultiFlashCidPresetSelected(None));
        assert_eq!(state.flash.multi_flash.pending.cid_input, "0x0032");
        assert!(state.flash.multi_flash.pending.cids.is_empty());
    }

    /// A condition is written against the CID the firmware itself carries: the
    /// package says which carrier it is for, so its list starts with that code
    /// instead of being empty.
    #[test]
    fn a_custom_condition_starts_with_the_firmwares_own_cid() {
        let mut state = State::default();
        state.flash.multi_flash.pending.identity = Some(Ok(flashfile::PackageIdentity {
            cid: Some("0x0032".to_owned()),
            ..flashfile::PackageIdentity::default()
        }));

        let _ = update(&mut state, Message::MultiFlashConditionsCustom);
        assert_eq!(state.flash.multi_flash.pending.cids, ["0x0032"]);

        // Picking the default rule and coming back keeps it: the firmware is
        // still the one that was picked.
        let _ = update(&mut state, Message::MultiFlashConditionsDefault);
        let _ = update(&mut state, Message::MultiFlashConditionsCustom);
        assert_eq!(state.flash.multi_flash.pending.cids, ["0x0032"]);

        // A package that names no CID leaves the list to the user.
        state.flash.multi_flash.pending = PresetOptions {
            identity: Some(Ok(flashfile::PackageIdentity::default())),
            ..PresetOptions::default()
        };

        let _ = update(&mut state, Message::MultiFlashConditionsCustom);
        assert!(state.flash.multi_flash.pending.cids.is_empty());
    }

    /// A line of a flash run goes into that run's log file as well as into the
    /// log on screen: the file is what the user can read (and send on) once the
    /// dialog is gone, and a dialog that is not flashing anything writes none.
    #[test]
    fn a_flash_line_is_written_to_the_runs_log_file() {
        let directory =
            std::env::temp_dir().join(format!("lmnflash-flash-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let path = directory.join("single_flash_ZY12345678_20261012_101112.log");

        let mut flash = FirmwareFlashState {
            log_file: Some(flash_log::FlashLog::create(path.clone())),
            ..FirmwareFlashState::default()
        };

        push_flash_log(&mut flash, "[1/48] flash super (super.img)".to_owned());

        assert_eq!(flash.log, ["[1/48] flash super (super.img)"]);
        let written = std::fs::read_to_string(&path).expect("the run wrote its log");
        assert!(
            written.contains("[1/48] flash super (super.img)"),
            "{written}"
        );

        // Nothing was started, so there is no file to write to.
        let mut idle = FirmwareFlashState::default();
        push_flash_log(&mut idle, "not part of a run".to_owned());
        assert_eq!(idle.log, ["not part of a run"]);
        assert!(idle.log_file.is_none());

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// What a run reports is applied to the table it belongs to: the reports of
    /// a session that was stopped (or of the run before it) are dropped, so a
    /// phone of the last run cannot appear in the new one's table.
    #[test]
    fn a_report_of_another_run_is_dropped() {
        let mut state = State::default();
        let stale = state.flash.multi_flash.session + 7;

        let _ = update(
            &mut state,
            Message::MultiFlashSessionEvent(
                stale,
                multi_flash_run::SessionEvent::Identified {
                    serial: "ZY12345678".to_owned(),
                    xt_code: None,
                    cid: Some("0x0032".to_owned()),
                    steps: None,
                },
            ),
        );

        assert!(state.flash.multi_flash.devices.is_empty());
    }

    /// The "Disconnect" menu offers every wireless device `adb` lists, and last
    /// the entry that drops all of them.
    #[test]
    fn the_disconnect_menu_lists_every_device_and_everything() {
        let mut wireless = WirelessState::default();

        // Nothing connected: dropping everything is still the one thing the
        // menu can offer.
        assert_eq!(
            wireless.disconnect_options(),
            [wireless::DisconnectTarget::Everything]
        );

        wireless.devices.push("192.168.1.5:5555".to_owned());
        wireless.devices.push("192.168.1.9:37000".to_owned());

        assert_eq!(
            wireless.disconnect_options(),
            [
                wireless::DisconnectTarget::Device("192.168.1.5:5555".to_owned()),
                wireless::DisconnectTarget::Device("192.168.1.9:37000".to_owned()),
                wireless::DisconnectTarget::Everything,
            ]
        );
    }

    /// A card that is still moving when the next one takes its place has to be
    /// picked up wherever it is.
    ///
    /// The bloatware dialog changes its card twice in quick succession as soon
    /// as ADB answers (device list → checking → checklist), which used to
    /// restart the entrance from the beginning — it looked like the dialog
    /// animated in twice.
    #[test]
    fn a_card_change_continues_an_animation_that_is_still_running() {
        let mut state = State::default();

        // The dialog eases in …
        start_overlay_transition(&mut state, OVERLAY_NONE, OVERLAY_BLOATWARE);
        assert!(state.overlay_transition.is_some());

        // … and is still on its way when the device list resolves into the
        // next card.
        anim::tick(&mut state.overlay_transition);
        let midway = state
            .overlay_transition
            .as_ref()
            .expect("the entrance is still running")
            .pose();

        start_overlay_transition(&mut state, OVERLAY_BLOATWARE, OVERLAY_BLOATWARE + 1);

        let continued = state
            .overlay_transition
            .as_ref()
            .expect("the new card animates")
            .pose();

        assert_eq!(continued.slide.y, midway.slide.y);
        assert_eq!(continued.slide.x, midway.slide.x);
        assert_eq!(continued.scale, midway.scale);

        // A change with nothing running still slides the new card in.
        state.overlay_transition = None;
        start_overlay_transition(&mut state, OVERLAY_BLOATWARE, OVERLAY_BLOATWARE + 1);
        let fresh = state
            .overlay_transition
            .as_ref()
            .expect("the new card animates")
            .pose();

        assert_eq!(fresh.scale, 1.0);
        assert_ne!(fresh.slide.x, 0.0);
    }

    /// The login flow eases from one step to the next, in the direction the
    /// user is moving through it.
    #[test]
    fn the_login_flow_animates_between_its_steps() {
        let mut state = State::default();
        assert_eq!(login_step(&state), 0);
        assert!(state.login_transition.is_none());

        // Asking to log in moves on to the next step, which slides in from the
        // right: the flow moves forward.
        let _ = update(&mut state, Message::LoginRequested(Click::Left));
        assert!(matches!(state.login, LoginStatus::Fetching { .. }));

        let forward = state
            .login_transition
            .as_ref()
            .expect("the next step animates")
            .pose();
        assert!(forward.slide.x > 0.0, "{}", forward.slide.x);
        assert!(forward.veil > 0.0);

        // A step that resolves before that one settled is picked up mid-slide
        // instead of jumping back to the start of the movement.
        anim::tick(&mut state.login_transition);
        let midway = state
            .login_transition
            .as_ref()
            .expect("the first step is still running")
            .pose();
        let _ = update(
            &mut state,
            Message::LoginUrlFetched(Ok("https://example.test/login".to_owned())),
        );
        assert_ne!(login_step(&state), 1);
        let continued = state
            .login_transition
            .as_ref()
            .expect("the next step animates")
            .pose();
        assert_eq!(continued.slide.x, midway.slide.x);

        // Cancelling goes back to the first step. The slide is settled first,
        // so the direction is the user's and not the leftover movement.
        for _ in 0..20 {
            let _ = update(&mut state, Message::TransitionTick);
        }
        assert!(state.login_transition.is_none());

        let _ = update(&mut state, Message::CancelLogin);
        assert_eq!(login_step(&state), 0);

        let back = state
            .login_transition
            .as_ref()
            .expect("going back animates")
            .pose();
        assert!(back.slide.x < 0.0, "{}", back.slide.x);
    }

    /// The lookup form eases in when the mode above it changes, in the
    /// direction the user is moving through the mode list.
    #[test]
    fn the_lookup_modes_animate_between_each_other() {
        let mut state = State::default();
        assert!(state.lookup_transition.is_none());

        // Moving down the list brings the next form in from the right.
        let _ = update(
            &mut state,
            Message::LookupModeSelected(LookupMode::RetcnSmartphone),
        );
        assert_eq!(state.lookup.mode, LookupMode::RetcnSmartphone);

        let forward = state
            .lookup_transition
            .as_ref()
            .expect("the next form animates")
            .pose();
        assert!(forward.slide.x > 0.0, "{}", forward.slide.x);
        assert!(forward.veil > 0.0);

        // Picking the mode that is already on screen changes nothing.
        for _ in 0..20 {
            let _ = update(&mut state, Message::TransitionTick);
        }
        assert!(state.lookup_transition.is_none());

        let _ = update(
            &mut state,
            Message::LookupModeSelected(LookupMode::RetcnSmartphone),
        );
        assert!(state.lookup_transition.is_none());

        // Going back up the list slides the other way.
        let _ = update(
            &mut state,
            Message::LookupModeSelected(LookupMode::RowSmartphone),
        );
        assert_eq!(state.lookup.mode, LookupMode::RowSmartphone);

        let back = state
            .lookup_transition
            .as_ref()
            .expect("going back animates")
            .pose();
        assert!(back.slide.x < 0.0, "{}", back.slide.x);
    }

    /// A firmware lookup is a step of the same movement: the form eases on to
    /// the spinner while the request runs, and on to its result (or its error)
    /// when the answer arrives.
    #[test]
    fn the_lookup_status_animates_the_form() {
        let mut state = State::default();
        state.login = LoginStatus::LoggedIn {
            token: "token".to_owned(),
            full_name: None,
        };
        // A valid IMEI (right length and check digit), so the request starts
        // instead of reporting the input back.
        state.lookup.imei_input = "490154203237518".to_owned();

        for _ in 0..20 {
            let _ = update(&mut state, Message::TransitionTick);
        }
        assert!(state.lookup_transition.is_none());

        // Asking for the firmware takes the form on to the next step.
        let _ = update(&mut state, Message::LookupRequested);
        assert!(matches!(state.lookup.status, LookupStatus::Fetching));

        let fetching = state
            .lookup_transition
            .as_ref()
            .expect("fetching animates")
            .pose();
        assert!(fetching.slide.x > 0.0, "{}", fetching.slide.x);

        // The answer is the step after it, so the result eases in instead of
        // being swapped in behind the spinner.
        for _ in 0..20 {
            let _ = update(&mut state, Message::TransitionTick);
        }
        assert!(state.lookup_transition.is_none());

        let _ = update(
            &mut state,
            Message::LookupFinished(Err(firmware::FirmwareError::Other(
                "no build for this IMEI".to_owned(),
            ))),
        );
        assert!(matches!(state.lookup.status, LookupStatus::Error(_)));

        let answered = state
            .lookup_transition
            .as_ref()
            .expect("the answer animates")
            .pose();
        assert!(answered.slide.x > 0.0, "{}", answered.slide.x);
        assert!(answered.veil > 0.0);
    }

    /// An image of this test's own that carries a vbmeta struct, and one that
    /// does not.
    fn partition_image(name: &str, avb: bool) -> std::path::PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "lmnflash-partition-flash-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&directory);
        let path = directory.join(name);

        let mut data = vec![0u8; 512];
        if avb {
            data[..4].copy_from_slice(b"AVB0");
        }

        std::fs::write(&path, data).expect("the image");
        path
    }

    /// The tile opens the dialog the flash is set up in.
    #[test]
    fn the_tile_opens_the_partition_flash_dialog() {
        let mut state = State::default();

        let _ = update(
            &mut state,
            Message::SmartphoneFeaturePressed(SmartphoneFeature::PartitionFlash),
        );

        assert!(partition_flash::is_open(&state));
        // The phones on the bus are looked up before the first frame, and the
        // board shows what it found.
        assert!(state.flash.partition.listing);
        assert!(state.flash.partition.devices.is_empty());
    }

    /// The Partition Manager tile takes over the window, and its Exit button
    /// gives the feature grid back.
    #[test]
    fn the_tile_opens_the_partition_manager() {
        let mut state = State::default();

        let _ = update(
            &mut state,
            Message::SmartphoneFeaturePressed(SmartphoneFeature::PartitionManager),
        );
        assert!(qualcomm::is_open(&state));
        // The page hides the tab bar and the language selector, so the whole
        // window travels with it.
        assert!(state.content_transition_covers_chrome);

        let _ = update(&mut state, Message::QualcommFilterChanged("boot".into()));
        assert_eq!(state.flash.qualcomm.filter, "boot");

        let _ = update(&mut state, Message::QualcommExit);
        assert!(!qualcomm::is_open(&state));
        // The search box keeps what was typed, so opening the page again finds
        // the same filter.
        assert_eq!(state.flash.qualcomm.filter, "boot");
    }

    /// A loaded programmer runs inside the phone, so the page is not left while
    /// it does: the phone has to be reset first.
    #[test]
    fn the_page_is_not_left_while_the_programmer_runs() {
        let mut state = State::default();
        state.flash.qualcomm.open = true;
        state.flash.qualcomm.programmer_uploaded = true;

        let _ = update(&mut state, Message::QualcommExit);
        assert!(qualcomm::is_open(&state));

        // A job of its own keeps the page shut as well.
        state.flash.qualcomm.programmer_uploaded = false;
        state.flash.qualcomm.busy = true;
        let _ = update(&mut state, Message::QualcommExit);
        assert!(qualcomm::is_open(&state));

        // Resetting the phone ends the session, and the page can be left.
        state.flash.qualcomm.busy = false;
        let _ = update(&mut state, Message::QualcommExit);
        assert!(!qualcomm::is_open(&state));
    }

    /// Closing the window while the programmer runs asks first: the phone would
    /// otherwise be left in Firehose mode with nothing to talk to it.
    #[test]
    fn closing_the_window_asks_first_while_the_programmer_runs() {
        let mut state = State::default();
        state.flash.qualcomm.open = true;
        state.flash.qualcomm.programmer_uploaded = true;

        let window = iced::window::Id::unique();
        let _ = update(&mut state, Message::WindowCloseRequested(window));

        assert_eq!(state.flash.qualcomm.close_prompt, Some(window));
        assert_eq!(overlay_key(&state), OVERLAY_QUALCOMM_CLOSE_PROMPT);

        // Cancel answers the request with nothing, and the prompt plays out.
        let _ = update(&mut state, Message::QualcommClosePromptCancelled);
        for _ in 0..20 {
            let _ = update(&mut state, Message::TransitionTick);
        }

        assert_eq!(state.flash.qualcomm.close_prompt, None);
        assert!(qualcomm::is_open(&state));

        // Without a session there is nothing to warn about.
        state.flash.qualcomm.programmer_uploaded = false;
        let _ = update(&mut state, Message::WindowCloseRequested(window));
        assert_eq!(state.flash.qualcomm.close_prompt, None);
    }

    /// A port that disappears from the scan cannot stay selected: the page
    /// would claim a phone that is no longer plugged in. Another port the scan
    /// does find takes its place, since the first one is selected by default.
    #[test]
    fn a_disappeared_port_is_dropped_from_the_selection() {
        let mut state = State::default();
        state.flash.qualcomm.port = Some("COM7".to_owned());
        // The programmer lives in the phone, so it left with the port.
        state.flash.qualcomm.programmer_uploaded = true;

        let _ = update(
            &mut state,
            Message::QualcommPortsScanned(vec!["COM7".to_owned(), "COM8".to_owned()]),
        );

        assert_eq!(state.flash.qualcomm.port.as_deref(), Some("COM7"));
        assert_eq!(state.flash.qualcomm.ports, ["COM7", "COM8"]);
        assert!(state.flash.qualcomm.programmer_uploaded);

        let _ = update(
            &mut state,
            Message::QualcommPortsScanned(vec!["COM8".to_owned()]),
        );

        assert_eq!(state.flash.qualcomm.port.as_deref(), Some("COM8"));
        assert_eq!(state.flash.qualcomm.ports, ["COM8"]);
        assert!(!state.flash.qualcomm.programmer_uploaded);

        // With no port left nothing is selected.
        let _ = update(&mut state, Message::QualcommPortsScanned(Vec::new()));
        assert_eq!(state.flash.qualcomm.port, None);
    }

    /// The first port a scan finds is selected on its own, so plugging the
    /// phone in is all it takes; a port that is already selected is left
    /// alone, and so is the table read from it.
    #[test]
    fn the_first_port_a_scan_finds_is_selected() {
        let mut state = State::default();
        let _ = update(
            &mut state,
            Message::QualcommPortsScanned(vec!["COM7".to_owned(), "COM8".to_owned()]),
        );
        assert_eq!(state.flash.qualcomm.port.as_deref(), Some("COM7"));

        // A table read from that port survives a scan that still lists it.
        let _ = update(
            &mut state,
            Message::QualcommFirehoseReady(Ok(firehose::Table {
                storage: StorageType::Ufs,
                partitions: vec![firehose::Partition {
                    name: "boot_a".to_owned(),
                    lun: 0,
                    start_lba: 0,
                    end_lba: 4095,
                    sector_size: 4096,
                }],
            })),
        );
        state.flash.qualcomm.programmer_uploaded = true;
        let _ = update(
            &mut state,
            Message::QualcommPortsScanned(vec!["COM7".to_owned(), "COM8".to_owned()]),
        );
        assert_eq!(state.flash.qualcomm.port.as_deref(), Some("COM7"));
        assert_eq!(state.flash.qualcomm.partitions.len(), 1);
        assert!(state.flash.qualcomm.programmer_uploaded);
    }

    /// A finished injection hands the table to the page; a failed one clears
    /// it, so a stale table never sits next to an error.
    #[test]
    fn a_firehose_injection_fills_the_partition_table() {
        let mut state = State::default();

        let _ = update(
            &mut state,
            Message::QualcommFirehoseReady(Ok(firehose::Table {
                storage: StorageType::Ufs,
                partitions: vec![
                    firehose::Partition {
                        name: "xbl_a".to_owned(),
                        lun: 0,
                        start_lba: 0,
                        end_lba: 1023,
                        sector_size: 4096,
                    },
                    firehose::Partition {
                        name: "super".to_owned(),
                        lun: 2,
                        start_lba: 2048,
                        end_lba: 4095,
                        sector_size: 4096,
                    },
                ],
            })),
        );

        assert_eq!(state.flash.qualcomm.partitions.len(), 2);
        assert_eq!(state.flash.qualcomm.detected, Some(StorageType::Ufs));
        assert_eq!(
            state.flash.qualcomm.status,
            QualcommStatus::Table { count: 2 }
        );
        assert!(!state.flash.qualcomm.busy);

        // A second attempt that fails drops the table the first one read.
        let _ = update(
            &mut state,
            Message::QualcommFirehoseReady(Err("no GUID partition table".to_owned())),
        );

        assert!(state.flash.qualcomm.partitions.is_empty());
        assert_eq!(state.flash.qualcomm.detected, None);
        assert_eq!(
            state.flash.qualcomm.status,
            QualcommStatus::InjectFailed("no GUID partition table".to_owned())
        );
    }

    /// The partition table is a multi-select list, the way a file list is: a
    /// plain click selects one row, Command/Control toggles a row, Shift takes
    /// the range from the row clicked before, and a new table starts empty.
    #[test]
    fn partition_rows_are_selected_the_way_a_file_list_is() {
        let mut state = State::default();

        let table = |name: &str| firehose::Table {
            storage: StorageType::Ufs,
            partitions: (0..5_u64)
                .map(|index| firehose::Partition {
                    name: format!("{name}{index}"),
                    lun: 0,
                    start_lba: index * 4096,
                    end_lba: index * 4096 + 4095,
                    sector_size: 4096,
                })
                .collect(),
        };

        let _ = update(
            &mut state,
            Message::QualcommFirehoseReady(Ok(table("boot"))),
        );
        assert_eq!(
            state.flash.qualcomm.selected,
            [false, false, false, false, false]
        );

        // A plain click is one row, and it replaces whatever was selected.
        let _ = update(&mut state, Message::QualcommPartitionClicked(2));
        assert_eq!(
            state.flash.qualcomm.selected,
            [false, false, true, false, false]
        );
        let _ = update(&mut state, Message::QualcommPartitionClicked(4));
        assert_eq!(
            state.flash.qualcomm.selected,
            [false, false, false, false, true]
        );

        // Command (Control here) toggles a row without touching the others,
        // and makes it the anchor.
        state.flash.qualcomm.modifiers = iced::keyboard::Modifiers::CTRL;
        let _ = update(&mut state, Message::QualcommPartitionClicked(1));
        let _ = update(&mut state, Message::QualcommPartitionClicked(3));
        assert_eq!(
            state.flash.qualcomm.selected,
            [false, true, false, true, true]
        );
        // The same chord on a selected row takes it off again.
        let _ = update(&mut state, Message::QualcommPartitionClicked(3));
        assert_eq!(
            state.flash.qualcomm.selected,
            [false, true, false, false, true]
        );

        // A plain click again selects one row, and makes it the anchor.
        state.flash.qualcomm.modifiers = iced::keyboard::Modifiers::default();
        let _ = update(&mut state, Message::QualcommPartitionClicked(1));
        assert_eq!(
            state.flash.qualcomm.selected,
            [false, true, false, false, false]
        );

        // Shift takes the range between the anchor (1) and the clicked row.
        state.flash.qualcomm.modifiers = iced::keyboard::Modifiers::SHIFT;
        let _ = update(&mut state, Message::QualcommPartitionClicked(4));
        assert_eq!(
            state.flash.qualcomm.selected,
            [false, true, true, true, true]
        );
        // The anchor does not move, so a second Shift-click starts from it
        // again — over the rows the table shows, not over what was selected.
        let _ = update(&mut state, Message::QualcommPartitionClicked(2));
        assert_eq!(
            state.flash.qualcomm.selected,
            [false, true, true, false, false]
        );
        // A range also works upwards.
        let _ = update(&mut state, Message::QualcommPartitionClicked(0));
        assert_eq!(
            state.flash.qualcomm.selected,
            [true, true, false, false, false]
        );

        // Control+A takes every row on screen, and the search box narrows what
        // that means: `boot3` leaves one row.
        state.flash.qualcomm.modifiers = iced::keyboard::Modifiers::default();
        let _ = update(&mut state, Message::QualcommSelectAll);
        assert_eq!(
            state.flash.qualcomm.selected,
            [true, true, true, true, true]
        );
        state.flash.qualcomm.filter = "boot3".to_owned();
        let _ = update(&mut state, Message::QualcommSelectAll);
        assert_eq!(
            state.flash.qualcomm.selected,
            [false, false, false, true, false]
        );
        // A row the filter hides cannot start a range either, so Shift there
        // is just a click on that row.
        state.flash.qualcomm.modifiers = iced::keyboard::Modifiers::SHIFT;
        let _ = update(&mut state, Message::QualcommPartitionClicked(0));
        assert_eq!(
            state.flash.qualcomm.selected,
            [true, false, false, false, false]
        );
        state.flash.qualcomm.filter.clear();
        state.flash.qualcomm.modifiers = iced::keyboard::Modifiers::default();

        // An index the table does not have is ignored.
        let _ = update(&mut state, Message::QualcommPartitionClicked(9));
        assert_eq!(
            state.flash.qualcomm.selected,
            [true, false, false, false, false]
        );

        // A new table is a new list of rows, so no selection carries over.
        let _ = update(
            &mut state,
            Message::QualcommFirehoseReady(Ok(table("system"))),
        );
        assert_eq!(
            state.flash.qualcomm.selected,
            [false, false, false, false, false]
        );
    }

    /// The refresh button clears the table and reads it again — but only while
    /// the programmer runs, since a table cannot be read without one.
    #[test]
    fn a_refresh_clears_the_table_and_reads_it_again() {
        let mut state = State::default();
        let table = firehose::Table {
            storage: StorageType::Ufs,
            partitions: (0..2_u64)
                .map(|index| firehose::Partition {
                    name: format!("boot{index}"),
                    lun: 0,
                    start_lba: index * 4096,
                    end_lba: index * 4096 + 4095,
                    sector_size: 4096,
                })
                .collect(),
        };

        let _ = update(
            &mut state,
            Message::QualcommFirehoseReady(Ok(table.clone())),
        );
        assert_eq!(state.flash.qualcomm.partitions.len(), 2);

        // Without a programmer (or without a port) a refresh can only clear
        // what is on screen.
        state.flash.qualcomm.port = Some("COM7".to_owned());
        let _ = update(&mut state, Message::QualcommRefresh);
        assert!(state.flash.qualcomm.partitions.is_empty());
        assert!(!state.flash.qualcomm.busy);
        assert_eq!(state.flash.qualcomm.status, QualcommStatus::Idle);

        // With one, the page is busy until the read answers.
        let _ = update(
            &mut state,
            Message::QualcommFirehoseReady(Ok(table.clone())),
        );
        state.flash.qualcomm.programmer_uploaded = true;
        let _ = update(&mut state, Message::QualcommRefresh);
        assert!(state.flash.qualcomm.partitions.is_empty());
        assert!(state.flash.qualcomm.busy);
        assert_eq!(state.flash.qualcomm.status, QualcommStatus::Refreshing);

        let _ = update(&mut state, Message::QualcommRefreshed(Ok(table)));
        assert!(!state.flash.qualcomm.busy);
        assert_eq!(state.flash.qualcomm.partitions.len(), 2);
        assert_eq!(
            state.flash.qualcomm.status,
            QualcommStatus::Table { count: 2 }
        );

        // A failed read leaves the page empty and says why.
        state.flash.qualcomm.programmer_uploaded = true;
        let _ = update(&mut state, Message::QualcommRefresh);
        let _ = update(
            &mut state,
            Message::QualcommRefreshed(Err("no GUID partition table".to_owned())),
        );
        assert!(state.flash.qualcomm.partitions.is_empty());
        assert_eq!(
            state.flash.qualcomm.status,
            QualcommStatus::RefreshFailed("no GUID partition table".to_owned())
        );
    }

    /// A reboot ends the session the table was read in, so the rows, the
    /// selection and the programmer go with it.
    #[test]
    fn a_reboot_clears_the_table() {
        let mut state = State::default();
        state.flash.qualcomm.port = Some("COM7".to_owned());
        let _ = update(
            &mut state,
            Message::QualcommFirehoseReady(Ok(firehose::Table {
                storage: StorageType::Ufs,
                partitions: (0..2_u64)
                    .map(|index| firehose::Partition {
                        name: format!("boot{index}"),
                        lun: 0,
                        start_lba: index * 4096,
                        end_lba: index * 4096 + 4095,
                        sector_size: 4096,
                    })
                    .collect(),
            })),
        );
        state.flash.qualcomm.programmer_uploaded = true;
        let _ = update(&mut state, Message::QualcommPartitionClicked(0));
        assert_eq!(state.flash.qualcomm.partitions.len(), 2);

        let _ = update(
            &mut state,
            Message::QualcommRebootModeSelected(RebootMode::Normal),
        );
        let _ = update(&mut state, Message::QualcommResetFinished(Ok(())));

        assert!(state.flash.qualcomm.partitions.is_empty());
        assert!(state.flash.qualcomm.selected.is_empty());
        assert_eq!(state.flash.qualcomm.anchor, None);
        assert_eq!(state.flash.qualcomm.detected, None);
        assert!(!state.flash.qualcomm.programmer_uploaded);
        assert_eq!(state.flash.qualcomm.status, QualcommStatus::Rebooted);
    }

    /// The danger zone's entries are placeholders: picking one changes nothing
    /// at all, the way the page's greyed-out buttons cannot be pressed.
    #[test]
    fn the_danger_zone_entries_do_nothing() {
        let mut state = State::default();
        let _ = update(
            &mut state,
            Message::QualcommFirehoseReady(Ok(firehose::Table {
                storage: StorageType::Ufs,
                partitions: vec![firehose::Partition {
                    name: "boot_a".to_owned(),
                    lun: 0,
                    start_lba: 0,
                    end_lba: 4095,
                    sector_size: 4096,
                }],
            })),
        );
        state.flash.qualcomm.programmer_uploaded = true;
        let _ = update(&mut state, Message::QualcommPartitionClicked(0));

        let before = state.flash.qualcomm.status.clone();
        let _ = update(&mut state, Message::QualcommDangerZoneSelected);

        assert_eq!(state.flash.qualcomm.status, before);
        assert_eq!(state.flash.qualcomm.partitions.len(), 1);
        assert_eq!(state.flash.qualcomm.selected, [true]);
        assert!(!state.flash.qualcomm.busy);
    }

    /// Erasing needs a selection, and the selection is what it reports about.
    #[test]
    fn erase_needs_a_selected_partition() {
        let mut state = State::default();
        state.flash.qualcomm.port = Some("COM7".to_owned());

        let _ = update(&mut state, Message::QualcommErase);
        assert_eq!(
            state.flash.qualcomm.status,
            QualcommStatus::EraseFailed(state.l10n.tr("qc-select-partition"))
        );
        assert!(!state.flash.qualcomm.busy);
    }

    /// The LUN picker lists what the phone reported, starts with every LUN
    /// checked, and hands exactly the checked ones to the dump.
    #[test]
    fn the_lun_picker_follows_what_the_phone_reports() {
        let luns = vec![
            firehose::Lun {
                number: 0,
                sector_size: 4096,
                sectors: 1024,
            },
            firehose::Lun {
                number: 4,
                sector_size: 4096,
                sectors: 2048,
            },
        ];

        let mut state = State::default();
        state.flash.qualcomm.port = Some("COM7".to_owned());
        state.flash.qualcomm.busy = true;
        state.flash.qualcomm.lun_dialog = Some(LunDialog::Reading);

        let _ = update(&mut state, Message::QualcommLunsRead(Ok(luns.clone())));

        assert!(!state.flash.qualcomm.busy);
        match &state.flash.qualcomm.lun_dialog {
            Some(LunDialog::Luns { luns, checked }) => {
                assert_eq!(luns.len(), 2);
                assert_eq!(checked, &[true, true]);
            }
            _ => panic!("the picker lists what the phone reported"),
        }

        // Unchecking the first LUN leaves it out of the dump.
        let _ = update(&mut state, Message::QualcommLunToggled(0, false));
        let _ = update(&mut state, Message::QualcommLunsConfirmed);

        assert_eq!(state.flash.qualcomm.pending_luns, vec![luns[1].clone()]);
        // The card stays up until the save dialog answered, so a cancelled
        // picker can be tried again.
        assert!(state.flash.qualcomm.lun_dialog.is_some());

        // Closing it is played out before it is run, like every other dialog.
        let _ = update(&mut state, Message::QualcommLunDialogClosed);
        assert!(state.pending_close.is_some());

        for _ in 0..20 {
            let _ = update(&mut state, Message::TransitionTick);
        }

        assert!(state.flash.qualcomm.lun_dialog.is_none());
        assert!(state.flash.qualcomm.pending_luns.is_empty());
    }

    /// The LUN picker is a dialog like the app's others: it darkens the page
    /// while it is up and eases in and out.
    #[test]
    fn the_lun_picker_animates_like_the_other_dialogs() {
        let mut state = State::default();

        let _ = update(
            &mut state,
            Message::QualcommLunsRead(Ok(vec![firehose::Lun {
                number: 0,
                sector_size: 4096,
                sectors: 1024,
            }])),
        );

        // It appeared, so a card has to slide in behind the scrim.
        assert!(state.overlay_transition.is_some());
        assert_eq!(overlay_key(&state), OVERLAY_QUALCOMM_LUNS + 1);

        // The card it shows is a step of its own: another failed question
        // slides in from the side rather than replacing it in place.
        let _ = update(&mut state, Message::QualcommLunsRead(Err("NAK".to_owned())));
        assert!(state.overlay_transition.is_some());
        assert_eq!(overlay_key(&state), OVERLAY_QUALCOMM_LUNS + 2);
    }

    /// The picker is left through its own buttons: a click beside the card is
    /// swallowed, so the checked LUNs are not lost to a stray click.
    #[test]
    fn the_lun_picker_backdrop_does_nothing() {
        let mut state = State::default();
        state.flash.qualcomm.lun_dialog = Some(LunDialog::Luns {
            luns: vec![firehose::Lun {
                number: 0,
                sector_size: 4096,
                sectors: 1024,
            }],
            checked: vec![true],
        });

        let _ = update(&mut state, Message::QualcommLunBackdropPressed);

        assert!(state.flash.qualcomm.lun_dialog.is_some());
        // Not even a close to play out: nothing was asked for.
        assert!(state.pending_close.is_none());
        assert!(state.overlay_transition.is_none());
    }

    /// Selecting and deselecting everything, and a dump of nothing.
    #[test]
    fn the_lun_picker_will_not_dump_nothing() {
        let mut state = State::default();
        state.flash.qualcomm.lun_dialog = Some(LunDialog::Luns {
            luns: vec![firehose::Lun {
                number: 0,
                sector_size: 4096,
                sectors: 1024,
            }],
            checked: vec![true],
        });

        let _ = update(&mut state, Message::QualcommLunsAll(false));
        let _ = update(&mut state, Message::QualcommLunsConfirmed);
        assert!(state.flash.qualcomm.pending_luns.is_empty());

        let _ = update(&mut state, Message::QualcommLunsAll(true));
        let _ = update(&mut state, Message::QualcommLunsConfirmed);
        assert_eq!(state.flash.qualcomm.pending_luns.len(), 1);
    }

    /// A layout the phone would not report is shown as the reason, and the
    /// page keeps the table it already has.
    #[test]
    fn a_failed_lun_read_says_why() {
        let mut state = State::default();
        state.flash.qualcomm.busy = true;
        state.flash.qualcomm.lun_dialog = Some(LunDialog::Reading);

        let _ = update(
            &mut state,
            Message::QualcommLunsRead(Err("the phone answered NAK".to_owned())),
        );

        assert!(!state.flash.qualcomm.busy);
        match &state.flash.qualcomm.lun_dialog {
            Some(LunDialog::Failed(error)) => assert!(error.contains("NAK")),
            _ => panic!("the card says why the layout could not be read"),
        }
    }

    /// "Dump Entire Storage" works on a table: without one there is nothing to
    /// dump, and the page does not even start asking.
    #[test]
    fn a_storage_dump_needs_a_table() {
        let mut state = State::default();
        state.flash.qualcomm.port = Some("COM7".to_owned());

        let _ = update(&mut state, Message::QualcommDumpStorage);

        assert!(state.flash.qualcomm.lun_dialog.is_none());
        assert!(state.flash.qualcomm.pending_luns.is_empty());
        assert!(!state.flash.qualcomm.busy);
    }

    /// Every reboot mode starts its own job, and each one reports its outcome.
    #[test]
    fn every_reboot_mode_starts_a_job() {
        for mode in RebootMode::ALL {
            let mut state = State::default();
            state.flash.qualcomm.port = Some("COM7".to_owned());

            let _ = update(
                &mut state,
                Message::QualcommRebootModeSelected(mode),
            );

            assert!(state.flash.qualcomm.busy, "{mode:?} did not start");
            assert_eq!(
                state.flash.qualcomm.status,
                QualcommStatus::Rebooting,
                "{mode:?} did not say it was rebooting"
            );
            assert!(state.flash.qualcomm.progress.is_some(), "{mode:?} has no progress");

            let _ = update(
                &mut state,
                Message::QualcommResetFinished(Err("the phone did not answer".to_owned())),
            );
            assert!(!state.flash.qualcomm.busy);
            assert_eq!(
                state.flash.qualcomm.status,
                QualcommStatus::RebootFailed("the phone did not answer".to_owned())
            );
        }
    }

    /// Without a port there is nothing to talk to, so no mode starts a job.
    #[test]
    fn a_reboot_mode_needs_a_port() {
        let mut state = State::default();

        let _ = update(
            &mut state,
            Message::QualcommRebootModeSelected(RebootMode::OemUnlock),
        );

        assert!(!state.flash.qualcomm.busy);
        assert_eq!(
            state.flash.qualcomm.status,
            QualcommStatus::RebootFailed(state.l10n.tr("qc-select-port"))
        );
    }

    /// The programmer stays in the phone after the upload, so a second inject
    /// would talk to a port that is no longer in Sahara. Selecting a port
    /// starts a new session, and so does a finished reset; Dump and Write need
    /// rows for the same reason.
    #[test]
    fn the_table_actions_follow_the_session_and_the_selection() {
        let mut state = State::default();
        state.flash.qualcomm.port = Some("COM7".to_owned());
        state.flash.qualcomm.programmer_uploaded = true;

        // A newly picked port is a new device to inject into.
        let _ = update(
            &mut state,
            Message::QualcommPortSelected("COM8".to_owned()),
        );
        assert!(!state.flash.qualcomm.programmer_uploaded);

        // The upload phase is what locks the button again.
        state.flash.qualcomm.busy = true;
        let _ = update(&mut state, Message::QualcommFirehoseReading);
        assert!(state.flash.qualcomm.programmer_uploaded);
        state.flash.qualcomm.busy = false;

        // So does a reset, which hands the phone back to Android.
        let _ = update(&mut state, Message::QualcommResetFinished(Ok(())));
        assert!(!state.flash.qualcomm.programmer_uploaded);

        // Nothing is selected, so there is nothing to dump.
        let _ = update(&mut state, Message::QualcommDump);
        assert_eq!(
            state.flash.qualcomm.status,
            QualcommStatus::DumpFailed(state.l10n.tr("qc-select-partition"))
        );

        // Write takes exactly one row; two is a mistake worth reporting.
        let table = firehose::Table {
            storage: StorageType::Ufs,
            partitions: ["boot", "system", "vendor"]
                .iter()
                .enumerate()
                .map(|(index, name)| firehose::Partition {
                    name: (*name).to_owned(),
                    lun: 0,
                    start_lba: index as u64 * 4096,
                    end_lba: index as u64 * 4096 + 4095,
                    sector_size: 4096,
                })
                .collect(),
        };
        let _ = update(&mut state, Message::QualcommFirehoseReady(Ok(table)));
        assert_eq!(state.flash.qualcomm.selected.len(), 3);
        state.flash.qualcomm.modifiers = iced::keyboard::Modifiers::CTRL;
        let _ = update(&mut state, Message::QualcommPartitionClicked(0));
        let _ = update(&mut state, Message::QualcommPartitionClicked(1));
        let _ = update(&mut state, Message::QualcommWrite);
        assert_eq!(
            state.flash.qualcomm.status,
            QualcommStatus::WriteFailed(state.l10n.tr("qc-select-one-partition"))
        );

        // The rows map onto the table by name, in the order they were picked.
        let regions = qualcomm_selected_regions(&state);
        let names: Vec<&str> = regions.iter().map(|region| region.name.as_str()).collect();
        assert_eq!(names, ["boot", "system"]);
        // UFS reports 4096-byte sectors, and a region spans whole sectors.
        assert_eq!(regions[0].sector_size, 4096);
        assert_eq!(regions[0].start_lba, 0);
        assert_eq!(regions[0].end_lba, 4095);
        assert_eq!(regions[1].start_lba, 4096);
    }

    /// A file dropped onto the window is an image for the Single Partition
    /// Flash dialog, and for nothing else.
    #[test]
    fn a_dropped_image_belongs_to_the_partition_flash_dialog() {
        let mut state = State::default();
        let image = partition_image("dragged-vbmeta.img", true);

        // No dialog is open, so there is nothing the file could belong to.
        let _ = update(
            &mut state,
            Message::PartitionFlashImageDropped(image.clone()),
        );
        assert!(state.flash.partition.image.is_none());

        state.flash.partition.open = true;
        let _ = update(
            &mut state,
            Message::PartitionFlashImageDropped(image.clone()),
        );
        assert_eq!(
            state.flash.partition.image.as_deref(),
            Some(image.as_path())
        );
        // The image carries a vbmeta struct, so the two flags apply to it.
        assert!(state.flash.partition.image_avb);

        // A job in flight takes no dropped file: it would change what the
        // running flash is writing.
        let other = partition_image("dragged-boot.img", false);
        state.flash.partition.task = Some(PartitionFlashTask::Flash);
        let _ = update(&mut state, Message::PartitionFlashImageDropped(other));
        assert_eq!(
            state.flash.partition.image.as_deref(),
            Some(image.as_path())
        );
    }

    /// The dropdown beside the partition field fills it, which is what makes
    /// the three usual names one click away.
    #[test]
    fn picking_a_partition_name_fills_the_field() {
        let mut state = State::default();
        state.flash.partition.open = true;

        let _ = update(
            &mut state,
            Message::PartitionFlashPartitionPreset("init_boot"),
        );
        assert_eq!(state.flash.partition.partition, "init_boot");

        // Typing over it keeps working.
        let _ = update(
            &mut state,
            Message::PartitionFlashPartitionChanged("vbmeta".to_owned()),
        );
        assert_eq!(state.flash.partition.partition, "vbmeta");
    }

    /// The two AVB flags are only written into an image that carries a vbmeta
    /// struct: the boxes are drawn disabled for the others, and a click that
    /// arrives anyway must not set the flag behind them.
    #[test]
    fn the_avb_flags_need_an_image_that_can_take_them() {
        let mut state = State::default();
        state.flash.partition.open = true;

        let _ = update(&mut state, Message::PartitionFlashVerityToggled(true));
        let _ = update(
            &mut state,
            Message::PartitionFlashVerificationToggled(true),
        );
        assert!(!state.flash.partition.disable_verity);
        assert!(!state.flash.partition.disable_verification);

        state.flash.partition.image_avb = true;
        let _ = update(&mut state, Message::PartitionFlashVerityToggled(true));
        let _ = update(
            &mut state,
            Message::PartitionFlashVerificationToggled(true),
        );
        assert!(state.flash.partition.disable_verity);
        assert!(state.flash.partition.disable_verification);

        // A new image starts without them, so the boxes never claim a flag
        // that the image in them cannot take.
        set_partition_flash_image(&mut state, partition_image("boot.img", false), false);
        assert!(!state.flash.partition.disable_verity);
        assert!(!state.flash.partition.disable_verification);
        assert!(!state.flash.partition.image_avb);
    }

    /// A phone that was switched to userspace fastboot comes back under the
    /// serial it had before, and is picked up as the selected one — while one
    /// that did not come back leaves nothing selected.
    #[test]
    fn a_switch_to_fastbootd_picks_the_phone_up_again() {
        let mut state = State::default();
        state.flash.partition.open = true;
        state.flash.partition.selected = Some("ZY12345678".to_owned());
        state.flash.partition.task = Some(PartitionFlashTask::SwitchToFastbootd);
        let job = state.flash.partition.job;

        let _ = update(
            &mut state,
            Message::PartitionFlashSwitched(
                job,
                Ok(vec![fastboot_info::DeviceSummary {
                    serial: "ZY12345678".to_owned(),
                    product: "arcfox".to_owned(),
                }]),
            ),
        );

        let flash = &state.flash.partition;
        assert_eq!(flash.switch_result, Some(Ok(())));
        assert_eq!(flash.selected.as_deref(), Some("ZY12345678"));
        // Its variables are read again: the phone is in another mode now, and
        // the card has to say which.
        assert!(flash.reading_device);
        assert!(!partition_flash::busy(&state));

        // A phone that is not on the bus yet is not selected, and the button
        // that started the switch is free again.
        let mut state = State::default();
        state.flash.partition.selected = Some("ZY12345678".to_owned());
        state.flash.partition.task = Some(PartitionFlashTask::SwitchToFastbootd);

        let _ = update(
            &mut state,
            Message::PartitionFlashSwitched(job, Ok(Vec::new())),
        );

        assert!(state.flash.partition.selected.is_none());
        assert!(!partition_flash::busy(&state));

        // A report of a job this dialog no longer runs is dropped.
        let mut state = State::default();
        state.flash.partition.selected = Some("ZY12345678".to_owned());
        state.flash.partition.task = Some(PartitionFlashTask::SwitchToFastbootd);

        let _ = update(
            &mut state,
            Message::PartitionFlashSwitched(job + 1, Err("unplugged".to_owned())),
        );

        assert_eq!(
            state.flash.partition.selected.as_deref(),
            Some("ZY12345678")
        );
        assert!(state.flash.partition.task.is_some());
    }

    /// A dialog is never closed while a job runs: the phone would be left
    /// half-written with nothing to say what happened.
    #[test]
    fn a_running_job_keeps_the_partition_flash_dialog_open() {
        let mut state = State::default();
        state.flash.partition.open = true;
        state.flash.partition.partition = "boot".to_owned();
        state.flash.partition.task = Some(PartitionFlashTask::Flash);

        // The close is played out before it is run, so it is deferred first —
        // and then refused, which eases the dialog back in.
        let _ = update(&mut state, Message::PartitionFlashCancel);
        assert!(state.pending_close.is_some());

        for _ in 0..20 {
            let _ = update(&mut state, Message::TransitionTick);
        }

        assert!(partition_flash::is_open(&state));
        assert!(state.flash.partition.task.is_some());
        assert_eq!(state.flash.partition.partition, "boot");

        // Without a job the close goes through, and the dialog starts over.
        state.flash.partition.task = None;
        let _ = update(&mut state, Message::PartitionFlashCancel);

        for _ in 0..20 {
            let _ = update(&mut state, Message::TransitionTick);
        }

        assert!(!partition_flash::is_open(&state));
        assert!(state.flash.partition.partition.is_empty());
    }

    /// A flash needs a phone, a partition and an image: without any of them
    /// the button is drawn disabled and the handler refuses it.
    #[test]
    fn a_partition_flash_needs_a_phone_a_partition_and_an_image() {
        let mut state = State::default();
        state.flash.partition.open = true;

        assert!(!partition_flash::ready(&state));

        state.flash.partition.selected = Some("ZY12345678".to_owned());
        state.flash.partition.partition = "boot".to_owned();
        assert!(!partition_flash::ready(&state));

        state.flash.partition.image = Some(partition_image("boot.img", true));
        assert!(partition_flash::ready(&state));

        // A blank partition name is nothing to write to.
        state.flash.partition.partition = "   ".to_owned();
        assert!(!partition_flash::ready(&state));

        // Starting without them changes nothing: no job is set up.
        let _ = update(&mut state, Message::PartitionFlashStart);
        assert!(state.flash.partition.task.is_none());
    }
}
