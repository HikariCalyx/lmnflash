// Release builds run without a console window; debug builds keep one so
// `eprintln!` diagnostics remain visible with `cargo run`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod about;
mod anim;
mod bootloader;
mod bulk;
mod carrier;
mod config;
mod decrypt;
mod driver;
mod driver_install;
mod factory_reset;
mod fastboot_info;
mod firmware;
mod firmware_flash;
mod flash_engine;
mod flash_script;
mod flashfile;
mod guided;
mod instance;
mod l10n;
mod login;
mod lookup;
mod platform_tools;
mod protocol;
mod smartphone;
mod tablet_bootloader;
mod tablet_unlock;
mod telemetry;
mod webview;

use iced::widget::{
    button, column, container, horizontal_rule, mouse_area, pick_list, row, stack, text_editor,
    Space,
};
use iced::widget::text::Shaping;
use iced::{Alignment, Element, Fill, Font, Size, Task};

use smartphone::SmartphoneFeature;

/// A `text` label rendered with advanced text shaping.
///
/// Advanced shaping makes missing glyphs (e.g. CJK on a system whose UI
/// language is not Chinese) fall back through the installed system fonts,
/// so text renders correctly regardless of the OS language.
fn text<'a>(
    fragment: impl iced::widget::text::IntoFragment<'a>,
) -> iced::widget::Text<'a> {
    iced::widget::text(fragment).shaping(Shaping::Advanced)
}

pub fn main() -> iced::Result {
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

    let icon = iced::window::icon::from_file_data(include_bytes!("icon.ico"), None).ok();

    iced::application(|state: &State| state.l10n.tr("app-title"), update, view)
        .default_font(system_ui_font())
        .window(iced::window::Settings {
            icon,
            ..iced::window::Settings::default()
        })
        .window_size(Size::new(720.0, 720.0))
        .centered()
        .subscription(subscription)
        .run_with(move || {
            let mut state = State::initial();
            // Only the primary instance receives forwarded callbacks.
            state.receives_callbacks = receiver.is_some();

            // Live as long as the process: a later instance forwards the
            // `softwarefix://` callback here.
            let callbacks = match receiver {
                Some(receiver) => Task::run(receiver, Message::LoginCallback),
                None => Task::none(),
            };

            (state, callbacks)
        })
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
    directory: Option<std::path::PathBuf>,
    custom_password: bool,
    password: String,
    status: DecryptStatus,
}

/// State of the "Smartphone Flash" mode (Mode 2) feature tiles.
#[derive(Default)]
struct SmartphoneFlashState {
    bootloader: BootloaderState,
    tablet_unlock: TabletUnlockState,
    factory_reset: FactoryResetState,
    firmware: FirmwareFlashState,
    driver: DriverState,
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
    /// Guided-vs-Manual chooser (the guided wizard is not implemented for
    /// tablets, so its button is disabled).
    Choosing,
    /// The manual (ZUI) unlock flow.
    Manual,
}

/// Fastboot devices offered by the tablet flow when several are connected.
#[derive(Debug, Clone)]
struct TabletPicker {
    /// USB serials, in discovery order.
    devices: Vec<String>,
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
    unlock_error: Option<String>,
    unlock_info: Option<String>,
}

impl TabletUnlockState {
    /// The identity the token URL is built from.
    fn info(&self) -> tablet_unlock::TabletUnlockInfo {
        tablet_unlock::TabletUnlockInfo {
            serial: self.serial.clone(),
            bootloader_sn_part1: self.bootloader_sn_part1.clone(),
            bootloader_sn_part2: self.bootloader_sn_part2.clone(),
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
    error: Option<String>,
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
    flash: SmartphoneFlashState,
    /// Whether the About dialog (the round "i" button next to the language
    /// selector on the Firmware Lookup page) is open.
    about_open: bool,
    /// Monotonic UI animation tick, advanced by a time subscription while a
    /// busy indicator (spinner) is visible.
    anim_tick: u64,
    /// In-flight fade + slide of the tab content, started when the user
    /// switches to another feature (Mode 1/2/3).
    content_transition: Option<anim::Transition>,
    /// In-flight fade + slide of the modal dialog (see `overlay_key`).
    overlay_transition: Option<anim::Transition>,
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
            flash: SmartphoneFlashState::default(),
            about_open: false,
            anim_tick: 0,
            content_transition: None,
            overlay_transition: None,
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
    /// Tablet Bootloader Unlock: the unlock job finished.
    TabletUnlockFinished(tablet_unlock::UnlockOutcome),
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
    let animating = guided.logging_in
        || guided.logging_out
        || guided.fetching_account
        || guided.requesting
        || reset.listing
        || reset.checking
        || reset.resetting
        || tablet.reading
        || tablet.unlocking
        || firmware.listing
        || firmware.loading
        || firmware.running
        || firmware.rebooting
        || firmware.reading_info
        || firmware.reading_device
        || firmware.installing_tools
        || firmware.terminal_busy
        || firmware.reading_version
        || driver.busy;
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
        || state.pending_close.is_some()
    {
        iced::time::every(anim::FRAME).map(|_| Message::TransitionTick)
    } else {
        iced::Subscription::none()
    };

    iced::Subscription::batch([ticks, countdown, transition])
}

fn update(state: &mut State, message: Message) -> Task<Message> {
    // A dialog that is animating out has already queued the close that removes
    // it, and its buttons are blocked: a second close request would only
    // restart the animation.
    if state.pending_close.is_some() && closes_overlay(&message) {
        return Task::none();
    }

    let before = overlay_key(state);

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

    state.overlay_transition = match (before, after) {
        (_, OVERLAY_NONE) => None,
        (OVERLAY_NONE, _) => Some(anim::Transition::settle(
            anim::dialog_enter(),
            anim::dialog_rest(),
        )),
        // A card stacked on top of this one is dismissed: the card below was
        // never gone, so it is revealed rather than played in again.
        _ if is_stacked(before) && !is_stacked(after) => None,
        _ => Some(anim::Transition::settle(
            anim::dialog_switch(switch_forward(before, after)),
            anim::dialog_rest(),
        )),
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
            | Message::DriverCancel
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
/// The tablet unlock chooser and the manual (ZUI) flow, then its device
/// picker stacked on top.
const OVERLAY_TABLET_CHOOSER: u8 = 0x70;
const OVERLAY_TABLET_MANUAL: u8 = 0x71;
const OVERLAY_TABLET_PICKER: u8 = 0x72;
const OVERLAY_FIRMWARE_FLASH: u8 = 0x80;
/// The About dialog opened from the Firmware Lookup page.
const OVERLAY_ABOUT: u8 = 0x90;
const OVERLAY_DRIVER: u8 = 0xA0;
const OVERLAY_RETCN_PICKER: u8 = 0xC0;

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

/// The index of a tab, used to tell which way the user is moving through the
/// feature tabs (and therefore which way the content should slide in).
fn mode_index(mode: Mode) -> usize {
    Mode::ALL.iter().position(|&tab| tab == mode).unwrap_or(0)
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

            state.mode = mode;
            Task::none()
        }
        Message::AnimTick => {
            state.anim_tick = state.anim_tick.wrapping_add(1);
            Task::none()
        }
        Message::TransitionTick => {
            anim::tick(&mut state.content_transition);
            anim::tick(&mut state.overlay_transition);

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
        },
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
            state.flash.tablet_unlock = TabletUnlockState {
                dialog: TabletUnlockDialog::Choosing,
                ..TabletUnlockState::default()
            };
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
            Task::none()
        }
        Message::TabletUnlockCancel => {
            let tablet = &mut state.flash.tablet_unlock;
            tablet.dialog = TabletUnlockDialog::Closed;
            tablet.picker = None;
            tablet.reading = false;
            tablet.unlocking = false;
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
        Message::TabletUnlockFinished(outcome) => finish_tablet_unlock(state, outcome),
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

                        state.login = LoginStatus::WebViewOpen { url: url.clone() };
                        let title = state.l10n.tr("login-dialog-title");

                        Task::perform(
                            async move {
                                tokio::task::spawn_blocking(move || {
                                    webview::show_login_dialog(&title, &url)
                                })
                                .await
                                .unwrap_or_else(|e| Err(format!("background task failed: {e}")))
                            },
                            Message::WebViewFinished,
                        )
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
                let _ = failure_sender.unbounded_send(flash_engine::PackageEvent::Failed(format!(
                    "background task failed: {error}"
                )));
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
    iced::widget::scrollable::snap_to(
        iced::widget::scrollable::Id::new(firmware_flash::LOG_SCROLL_ID),
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
            tablet.read_done = true;
        }
        Err(error) => {
            let tablet = &mut state.flash.tablet_unlock;
            tablet.reading = false;
            tablet.read_error = Some(error);
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
    tablet.unlock_error = None;
    tablet.unlock_info = None;

    let token = tablet_unlock::token_path();
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || tablet_unlock::unlock(&device, &info, &token))
                .await
                .unwrap_or_else(|e| {
                    tablet_unlock::UnlockOutcome::Failed(format!("background task failed: {e}"))
                })
        },
        Message::TabletUnlockFinished,
    )
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
    };

    let tablet = &mut state.flash.tablet_unlock;
    tablet.unlocking = false;
    tablet.unlock_info = info;
    tablet.unlock_error = error;
    Task::none()
}

fn view(state: &State) -> Element<'_, Message> {
    let content = match state.mode {
        Mode::Mode1 => lookup::firmware_lookup_view(state),
        Mode::Mode2 => smartphone::smartphone_flash_view(state),
        Mode::Mode3 => decrypt::view(state),
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
    // the window background colour (see `anim`).
    let mut content_layers: Vec<Element<'_, Message>> = match &state.content_transition {
        Some(transition) => {
            let pose = transition.pose();
            vec![
                anim::animated(content, pose),
                anim::veil(anim::window_background(), pose.veil),
            ]
        }
        None => vec![content],
    };

    // The language selector sits in the top-right corner; on the Firmware
    // Lookup page a round "About" button is drawn to its left. They stay above
    // the transition's veil, so they do not flicker along with the content.
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

    let base = column![content_area, horizontal_rule(1), tab_bar];

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

}
