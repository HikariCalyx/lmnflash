// Release builds run without a console window; debug builds keep one so
// `eprintln!` diagnostics remain visible with `cargo run`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod about;
mod anim;
mod bloatware;
mod bootloader;
mod browser;
mod bulk;
mod captive_portal;
mod captive_portal_dialog;
mod carrier;
mod compat_iced;
mod config;
mod debloat;
mod decrypt;
mod decrypt_mode;
mod driver;
mod driver_install;
mod factory_reset;
mod fastboot_info;
mod firmware;
mod firmware_flash;
mod flash_engine;
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
mod pac;
mod pac_build;
mod pac_repack;
mod pac_unpack;
mod platform_tools;
mod protocol;
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
    // more than once), so the callback receiver is handed over through a cell.
    let receiver = Arc::new(std::sync::Mutex::new(receiver));

    let application = iced::application(
        move || {
            let mut state = State::initial();

            // Only the primary instance receives forwarded callbacks. The
            // receiver lives as long as the process: a later instance forwards
            // the `softwarefix://` callback here.
            let callbacks = match receiver.lock().expect("boot runs once").take() {
                Some(receiver) => {
                    state.receives_callbacks = true;
                    Task::run(receiver, Message::LoginCallback)
                }
                None => Task::none(),
            };

            // Ask for the OS colour scheme so the Fluent theme matches it from
            // the first frame on.
            let theme = iced::system::theme().map(Message::SystemTheme);

            (state, Task::batch([callbacks, theme]))
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

/// State of the "Multi Flashing" page (Mode 2).
///
/// The page is reached from the Firmware Flash dialog and takes the place of
/// the feature grid; several devices are flashed in one run there.
#[derive(Default)]
struct MultiFlashState {
    /// Whether the page is shown instead of the feature grid.
    open: bool,
    /// The firmware images loaded for the run. The "Preset Manage" dialog
    /// fills them in, and starting a run needs at least one.
    images: Vec<std::path::PathBuf>,
    /// Whether the "Preset Manage" dialog is open.
    preset_dialog: bool,
    /// `true` while a run goes on: the workspace is locked, "Start" reads
    /// "Stop", and the window cannot be closed.
    running: bool,
    /// Whether a close request was refused because a run was going on; the
    /// prompt stays up until it is acknowledged.
    close_prompt: bool,
}

/// State of the "Smart Device Flashing" mode (Mode 2) feature tiles.
#[derive(Default)]
struct SmartphoneFlashState {
    bootloader: BootloaderState,
    tablet_unlock: TabletUnlockState,
    factory_reset: FactoryResetState,
    firmware: FirmwareFlashState,
    driver: DriverState,
    bloatware: BloatwareState,
    wireless: WirelessState,
    captive_portal: CaptivePortalState,
    multi_flash: MultiFlashState,
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
    /// Result of the whole package, once it finished.
    result: Option<Result<(), String>>,
    /// `true` while `oem fb_mode_clear` + `reboot` run on the phone.
    rebooting: bool,
    /// Outcome of that reboot, once it ran.
    reboot_result: Option<Result<(), String>>,
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
            result: None,
            rebooting: false,
            reboot_result: None,
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
    /// How `softwarefix://` links are handled on this machine (Windows only;
    /// `Handler::Unsupported` elsewhere).
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
    /// Firmware Flash: that script was written (or the save dialog was
    /// cancelled, or the write failed).
    FirmwareFlashExported(Result<Option<std::path::PathBuf>, String>),
    /// Firmware Flash: copy the flashing log to the clipboard.
    FirmwareFlashCopyLog,
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
    /// Firmware Flash: leave the dialog for the Multi Flashing page, which
    /// takes the place of the Mode 2 feature grid.
    FirmwareFlashMultiFlashing,
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
    /// Multi Flashing: that picker closed (`None` = cancelled).
    MultiFlashPresetPicked(Option<std::path::PathBuf>),
    /// Multi Flashing: close the "Preset Manage" dialog.
    MultiFlashPresetClose,
    /// Click on the "Preset Manage" dialog's empty space (swallowed, so it
    /// neither dismisses the dialog nor reaches the UI underneath).
    MultiFlashPresetBackdropPressed,
    /// Multi Flashing: start the run of the loaded firmware images.
    MultiFlashStart,
    /// Multi Flashing: stop the running job.
    MultiFlashStop,
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
        // The decrypt dialog only has text while it works, but its spinner
        // ticks along with the others.
        || matches!(state.decrypt.status, DecryptStatus::Working);
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
    // A feature-switch transition animates at full frame rate, but only while
    // one is actually running; iced redraws once per tick.
    let transition = if state.content_transition.is_some()
        || state.overlay_transition.is_some()
        || state.login_transition.is_some()
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

    iced::Subscription::batch([
        ticks,
        countdown,
        transition,
        close_requests,
        system_theme,
    ])
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
/// inside it — including leaving the Firmware Flash dialog for the Multi
/// Flashing page, which closes it. Only these are held back for the exit
/// animation.
fn closes_overlay(message: &Message) -> bool {
    matches!(
        message,
        Message::BootloaderCancel
            | Message::TabletUnlockCancel
            | Message::FactoryResetCancel
            | Message::FirmwareFlashCancel
            | Message::FirmwareFlashMultiFlashing
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
}

/// No dialog is open.
const OVERLAY_NONE: u8 = 0x00;
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
    // The Multi Flashing page's own modals are drawn above everything else:
    // the page can only be reached with every other dialog closed.
    if multi_flash::close_prompt_open(state) {
        return OVERLAY_MULTI_FLASH_CLOSE_PROMPT;
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
        // checklist, the confirmation and the read-info view each have one.
        let flash = &state.flash.firmware;
        let step = if flash.running || flash.result.is_some() {
            3
        } else if flash.editing {
            1
        } else if flash.confirming {
            2
        } else if flash.info_view {
            4
        } else {
            0
        };

        return OVERLAY_FIRMWARE_FLASH + step;
    }

    if driver::is_open(state) {
        return OVERLAY_DRIVER;
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
                            .add_filter("Firmware package", &["zip", "xml"])
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
        Message::FirmwareFlashMultiFlashing => {
            // The workspace is still being built, so it is only reachable from
            // a debug build (the button that leads there is greyed out
            // otherwise, and this keeps a stray event out too).
            if !cfg!(debug_assertions) {
                return Task::none();
            }

            // Never leave the dialog while a job is in flight: the phone must
            // not be left half-flashed.
            let busy = {
                let flash = &state.flash.firmware;
                flash.loading || flash.running || flash.rebooting
            };

            if busy {
                return Task::none();
            }

            // The dialog is discarded exactly as Cancel discards it, so the
            // package it unpacked is cleaned up, and the workspace takes over.
            let job = state.flash.firmware.job + 1;
            state.flash.firmware = FirmwareFlashState {
                job,
                ..FirmwareFlashState::default()
            };
            state.flash.multi_flash.open = true;
            // The workspace slides in the way a tab does. This runs after the
            // dialog finished leaving (the close is held back for its exit
            // animation), so the two movements follow each other.
            state.content_transition = Some(anim::Transition::settle(
                anim::content_start(true),
                anim::content_rest(),
            ));
            // The whole window is animated: the tab bar and the language
            // selector disappear together with the grid instead of popping
            // while the page fades in.
            state.content_transition_covers_chrome = true;

            cleanup_firmware_flash_package()
        }
        Message::MultiFlashExit => {
            // A run has to be stopped first; the button is locked while it
            // goes on, but a stray event must not leave one behind either.
            if state.flash.multi_flash.running {
                return Task::none();
            }

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
        Message::MultiFlashPresetManage => {
            state.flash.multi_flash.preset_dialog = true;
            Task::none()
        }
        Message::MultiFlashPresetBackdropPressed => Task::none(),
        Message::MultiFlashPresetClose => {
            state.flash.multi_flash.preset_dialog = false;
            Task::none()
        }
        Message::MultiFlashPresetAdd => {
            let title = state.l10n.tr("multi-flash-preset-add");

            Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        rfd::FileDialog::new().set_title(&title).pick_file()
                    })
                    .await
                    .unwrap_or(None)
                },
                Message::MultiFlashPresetPicked,
            )
        }
        Message::MultiFlashPresetPicked(path) => {
            if let Some(path) = path {
                let images = &mut state.flash.multi_flash.images;

                if !images.contains(&path) {
                    images.push(path);
                }
            }

            Task::none()
        }
        Message::MultiFlashStart => {
            let multi = &mut state.flash.multi_flash;

            // A run needs something to flash; the button is only drawn live
            // once an image was loaded.
            if !multi.running && !multi.images.is_empty() {
                multi.running = true;
            }

            Task::none()
        }
        Message::MultiFlashStop => {
            state.flash.multi_flash.running = false;
            Task::none()
        }
        Message::WindowCloseRequested(id) => {
            // Closing the window would cut the run short, so it is refused
            // and the user is told to stop the operation first.
            if state.flash.multi_flash.running {
                state.flash.multi_flash.close_prompt = true;
                return Task::none();
            }

            iced::window::close::<Message>(id)
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
                flash_engine::PackageEvent::Log(line) => push_flash_log(&mut flash.log, line),
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
                        &mut flash.log,
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
                    push_flash_log(&mut flash.log, line);
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
        Message::FirmwareFlashExport => start_firmware_flash_export(state),
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
                push_flash_log(&mut flash.log, format!("reboot failed: {error}"));
            }

            flash.reboot_result = Some(result);
            scroll_flash_log_to_end()
        }
        Message::FirmwareFlashCopyLog => {
            let log = &state.flash.firmware.log;

            if log.is_empty() {
                return Task::none();
            }

            iced::clipboard::write::<Message>(log.join("\n"))
        }
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
    flash.log.clear();
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

/// Writes the checked procedures (and the added erases) into a script the
/// user picks the name of: `*.cmd` on Windows, `*.sh` elsewhere.
///
/// The script is written from the same selection `start_firmware_flash` would
/// run, so both describe the same flash. A device is not required — the serial
/// is only written into the script when one is selected.
fn start_firmware_flash_export(state: &mut State) -> Task<Message> {
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
    let serial = flash.selected.clone().unwrap_or_default();
    let shell = flash_script::Shell::for_host();
    let suggested = shell.file_name(package.model.as_deref());
    let extension = shell.extension();

    flash.export_result = None;

    // The save dialog blocks, so it runs on a worker thread; the script is
    // written there too, before the dialog's answer is handed back.
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                let job = flash_script::ScriptJob {
                    package: &package,
                    steps: &steps,
                    extra_erases: &extra_erases,
                    engine: &engine,
                    serial: &serial,
                    shell,
                };
                let script = flash_script::render(&job);

                let Some(path) = rfd::FileDialog::new()
                    .set_file_name(&suggested)
                    .add_filter("Script", &[extension])
                    .save_file()
                else {
                    return Ok(None);
                };

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

/// Appends a line to the flashing log, keeping memory bounded (a factory
/// firmware produces thousands of lines).
fn push_flash_log(log: &mut Vec<String>, line: String) {
    const MAX_LINES: usize = 500;

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

    // The Multi Flashing page owns the whole window: the language selector
    // and the mode tabs are hidden there, since the page is left through its
    // own Exit button.
    let multi_flashing = state.flash.multi_flash.open;

    // The Repack Package dialog is a full-window card as well, so the language
    // selector would show through while that card animates in and out: it is
    // left out while the dialog is open.
    let full_window = multi_flashing || pac_repack::is_open(state) || pac_unpack::is_open(state);

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

    // The mode tabs are left out of the Multi Flashing page entirely (see
    // `multi_flashing` above), so no tab is ever clickable during a run.
    let base: Element<'_, Message> = if multi_flashing {
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

    // The Multi Flashing page's modals: its "Preset Manage" dialog, and the
    // prompt that a close request was refused while a run goes on.
    if let Some(overlay) = multi_flash::close_prompt_overlay(state) {
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
}
