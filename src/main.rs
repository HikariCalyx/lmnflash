// Release builds run without a console window; debug builds keep one so
// `eprintln!` diagnostics remain visible with `cargo run`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bootloader;
mod carrier;
mod config;
mod decrypt;
mod factory_reset;
mod fastboot_info;
mod firmware;
mod firmware_flash;
mod flash_engine;
mod flash_script;
mod flashfile;
mod guided;
mod l10n;
mod login;
mod telemetry;
mod webview;

use iced::widget::{
    button, checkbox, column, container, horizontal_rule, mouse_area, pick_list,
    row, scrollable, stack, text_input, Space,
};
use iced::widget::text::Shaping;
use iced::{Alignment, Element, Fill, Font, Size, Task};

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
        .run_with(|| (State::initial(), Task::none()))
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

/// A feature tile offered by the smartphone-firmware-flash mode (Mode 2).
///
/// Each variant maps to a localized title and action button; new flashing
/// features are added here as they are implemented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SmartphoneFeature {
    BootloaderUnlock,
    FactoryReset,
    FirmwareFlash,
}

impl SmartphoneFeature {
    const ALL: [Self; 3] = [
        Self::BootloaderUnlock,
        Self::FactoryReset,
        Self::FirmwareFlash,
    ];

    fn title_id(self) -> &'static str {
        match self {
            Self::BootloaderUnlock => "flash-bootloader-title",
            Self::FactoryReset => "flash-factory-reset-title",
            Self::FirmwareFlash => "flash-firmware-title",
        }
    }

    /// The action buttons of the tile, paired with the FTL id of their label
    /// and the message they send.
    ///
    /// `None` marks an action that is not implemented yet: its button is
    /// rendered disabled (a button without `on_press`), so the tile already
    /// shows what is coming without pretending to work.
    fn actions(self) -> Vec<(&'static str, Option<Message>)> {
        let pressed = || Some(Message::SmartphoneFeaturePressed(self));

        match self {
            Self::BootloaderUnlock => vec![("flash-bootloader-button", pressed())],
            Self::FactoryReset => vec![("flash-factory-reset-button", pressed())],
            // Firmware flashing is offered per device type; tablet firmware
            // flashing is not implemented yet.
            Self::FirmwareFlash => vec![
                ("flash-firmware-smartphone-button", pressed()),
                ("flash-firmware-tablet-button", None),
            ],
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
}

impl LookupMode {
    const ALL: [Self; 4] = [
        Self::RowSmartphone,
        Self::RetcnSmartphone,
        Self::Tablet,
        Self::ByModel,
    ];

    fn message_id(self) -> &'static str {
        match self {
            Self::RowSmartphone => "lookup-mode-row",
            Self::RetcnSmartphone => "lookup-mode-retcn",
            Self::Tablet => "lookup-mode-tablet",
            Self::ByModel => "lookup-mode-by-model",
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
    factory_reset: FactoryResetState,
    firmware: FirmwareFlashState,
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
    lookup: LookupState,
    decrypt: DecryptState,
    flash: SmartphoneFlashState,
    /// Monotonic UI animation tick, advanced by a time subscription while a
    /// busy indicator (spinner) is visible.
    anim_tick: u64,
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
            lookup: LookupState::default(),
            decrypt: DecryptState::default(),
            flash: SmartphoneFlashState::default(),
            anim_tick: 0,
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
    LanguageSelected(l10n::Language),
    LoginRequested(Click),
    LoginUrlFetched(Result<String, String>),
    WebViewFinished(Result<String, String>),
    ManualInputChanged(String),
    SubmitManual,
    OpenBrowser,
    CopyUrl,
    CancelLogin,
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
    /// The best-effort telemetry POST finished (only logged; telemetry must
    /// never affect the app).
    TelemetrySent(Result<(), String>),
    /// UI animation tick, emitted while a busy indicator is shown (drives the
    /// spinners without blocking the UI thread).
    AnimTick,
}

/// Drives the small UI spinner animations while a busy status is shown.
/// Returns an idle stream when nothing needs to animate, so the UI only ticks
/// (and redraws) while a spinner is actually visible.
fn subscription(state: &State) -> iced::Subscription<Message> {
    let guided = &state.flash.bootloader.guided;
    let reset = &state.flash.factory_reset;
    let firmware = &state.flash.firmware;
    let animating = guided.logging_in
        || guided.logging_out
        || guided.fetching_account
        || guided.requesting
        || reset.listing
        || reset.checking
        || reset.resetting
        || firmware.listing
        || firmware.loading
        || firmware.running
        || firmware.rebooting
        || firmware.reading_info
        || firmware.reading_device;
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

    iced::Subscription::batch([ticks, countdown])
}

fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::ModeSelected(mode) => {
            state.mode = mode;
            Task::none()
        }
        Message::AnimTick => {
            state.anim_tick = state.anim_tick.wrapping_add(1);
            Task::none()
        }
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
            state.flash.firmware.engine = engine;
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

                package
                    .steps
                    .iter()
                    .enumerate()
                    .filter(|(_, step)| step.part() == Some(part))
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
                LoginStatus::Manual { url, .. } => url.clone(),
                _ => return Task::none(),
            };
            open_browser(&url)
        }
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
        Message::BrowserOpened => Task::none(),
    }
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
/// to the application and lists the connected devices.
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
        engine,
        listing: true,
        ..FirmwareFlashState::default()
    };

    Task::batch([
        start_firmware_flash_list_devices(),
        // A previous dialog may have left a multi-gigabyte package behind.
        cleanup_firmware_flash_package(),
    ])
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

fn view(state: &State) -> Element<'_, Message> {
    let content = match state.mode {
        Mode::Mode1 => firmware_lookup_view(state),
        Mode::Mode2 => smartphone_flash_view(state),
        Mode::Mode3 => decrypt_view(state),
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

    // Language selector overlay in the top-right corner.
    let content_area = stack![
        content,
        container(language_dropdown)
            .padding(8)
            .align_top(Fill)
            .align_right(Fill),
    ];

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
        return stack![base, overlay].into();
    }

    // Factory Reset dialog (device selection / requirement checks / confirm)
    // drawn over the whole window when it is open.
    if let Some(overlay) = factory_reset::overlay(state) {
        return stack![base, overlay].into();
    }

    // Firmware Flash dialog (package + device selection, progress and log)
    // drawn over the whole window while it is open.
    if let Some(overlay) = firmware_flash::overlay(state) {
        return stack![base, overlay].into();
    }

    // Device picker modal: shown when more than one fastboot device is
    // connected. Clicks on the dimmed backdrop cancel the selection.
    if let DevicePicker::Open(devices) = &state.lookup.retcn.device_picker {
        let device_buttons = iced::widget::Column::with_children(
            devices.iter().map(|device| {
                button(text(device.label()))
                    .width(Fill)
                    .on_press(Message::FastbootDeviceSelected(device.serial.clone()))
                    .into()
            }),
        )
        .spacing(8);

        let card = container(
            column![
                text(state.l10n.tr("retcn-pick-device-title")).size(18.0),
                device_buttons,
                button(text(state.l10n.tr("login-cancel")))
                    .on_press(Message::FastbootDevicePickerCancelled),
            ]
            .spacing(12)
            .align_x(Alignment::Center),
        )
        .padding(16)
        .width(340)
        .style(container::rounded_box);

        let backdrop = mouse_area(Space::new(Fill, Fill))
            .on_press(Message::FastbootDevicePickerCancelled);

        return stack![
            base,
            backdrop,
            container(card)
                .width(Fill)
                .height(Fill)
                .center_x(Fill)
                .center_y(Fill),
        ]
        .into();
    }

    base.into()
}

/// The smartphone-firmware-flash UI (Mode 2): a grid of feature tiles, each
/// carrying an action button. Features are added to the grid incrementally.
fn smartphone_flash_view(state: &State) -> Element<'_, Message> {
    let tiles: Vec<Element<'_, Message>> = SmartphoneFeature::ALL
        .iter()
        .map(|&feature| smartphone_feature_tile(state, feature))
        .collect();

    let grid = container(
        iced::widget::Row::with_children(tiles)
            .spacing(12)
            .align_y(Alignment::Start)
            .wrap(),
    )
    .width(Fill)
    .padding(8);

    container(scrollable(grid).width(Fill).height(Fill))
        .width(Fill)
        .height(Fill)
        .padding(16)
        .into()
}

/// A single feature tile in the smartphone-flash grid.
fn smartphone_feature_tile(state: &State, feature: SmartphoneFeature) -> Element<'_, Message> {
    let l10n = &state.l10n;

    let actions: Vec<Element<'_, Message>> = feature
        .actions()
        .into_iter()
        .map(|(label_id, message)| -> Element<'_, Message> {
            let action = button(text(l10n.tr(label_id))).width(Fill);

            match message {
                Some(message) => action.on_press(message).into(),
                // Not available yet: iced greys out a button without `on_press`
                // and ignores clicks on it.
                None => action.into(),
            }
        })
        .collect();

    container(
        column![
            text(l10n.tr(feature.title_id())).size(16.0),
            iced::widget::Row::with_children(actions).spacing(8),
        ]
        .spacing(12)
        .align_x(Alignment::Start),
    )
    .width(320)
    .padding(16)
    .style(container::rounded_box)
    .into()
}

/// The firmware-decrypt UI (Mode 3): pick a directory, optionally provide a
/// custom password, then decrypt every `*.x`/`*.t` file found in it.
fn decrypt_view(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;
    let decrypt_state = &state.decrypt;

    let busy = matches!(
        decrypt_state.status,
        DecryptStatus::PickingDir | DecryptStatus::Working
    );

    let pick_button = if busy {
        button(text(l10n.tr("decrypt-select-dir")))
    } else {
        button(text(l10n.tr("decrypt-select-dir"))).on_press(Message::DecryptPickDirRequested)
    };

    let path_text = match &decrypt_state.directory {
        Some(path) => path.display().to_string(),
        None => l10n.tr("decrypt-no-dir"),
    };

    let mut content = column![
        text(l10n.tr("decrypt-description")).size(14.0),
        row![
            pick_button,
            container(
                text(path_text)
                    .size(13.0)
                    .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
                    .width(420),
            )
            .padding(8)
            .style(container::rounded_box),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        checkbox(
            l10n.tr("decrypt-custom-password"),
            decrypt_state.custom_password,
        )
        .text_shaping(Shaping::Advanced)
        .on_toggle(Message::DecryptCustomPasswordToggled),
    ]
    .spacing(8)
    .align_x(Alignment::Center);

    if decrypt_state.custom_password {
        content = content.push(
            row![
                text(l10n.tr("decrypt-password-label")).size(14.0),
                text_input("", &decrypt_state.password)
                    .secure(true)
                    .on_input(Message::DecryptPasswordChanged)
                    .on_submit(Message::DecryptRequested)
                    .width(240),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );
    }

    let decrypt_button = if busy || decrypt_state.directory.is_none() {
        button(text(l10n.tr("decrypt-button")))
    } else {
        button(text(l10n.tr("decrypt-button"))).on_press(Message::DecryptRequested)
    };
    content = content.push(decrypt_button);

    match &decrypt_state.status {
        DecryptStatus::Idle | DecryptStatus::PickingDir => {}
        DecryptStatus::Working => {
            content = content.push(text(l10n.tr("decrypt-working")).size(14.0));
        }
        DecryptStatus::Error(error) => {
            content = content.push(
                text(error.clone())
                    .size(14.0)
                    .style(iced::widget::text::danger),
            );
        }
        DecryptStatus::Done(summary) => {
            if summary.total == 0 {
                content = content.push(text(l10n.tr("decrypt-no-files")).size(14.0));
            } else {
                let message = l10n.tr_with_args(
                    "decrypt-done",
                    &[
                        ("ok", summary.succeeded.to_string()),
                        ("fail", summary.failed.len().to_string()),
                    ],
                );

                let style = if summary.failed.is_empty() {
                    iced::widget::text::success
                } else {
                    iced::widget::text::danger
                };
                content = content.push(text(message).size(14.0).style(style));

                if !summary.failed.is_empty() {
                    content = content.push(text(l10n.tr("decrypt-failed-files")).size(13.0));

                    let rows = iced::widget::Column::with_children(
                        summary.failed.iter().map(|(path, error)| {
                            text(format!("{}: {}", path.display(), error))
                                .size(12.0)
                                .into()
                        }),
                    )
                    .spacing(4)
                    .align_x(Alignment::Start);

                    content = content.push(scrollable(rows).height(120).width(520));
                }
            }
        }
    }

    container(content)
        .width(Fill)
        .height(Fill)
        .center_x(Fill)
        .center_y(Fill)
        .into()
}

fn firmware_lookup_view(state: &State) -> Element<'_, Message> {
    let l10n = &state.l10n;

    let inner: Element<'_, Message> = match &state.login {
        LoginStatus::LoggedOut => column![
            text(l10n.tr("login-prompt")).size(24.0),
            mouse_area(
                button(text(l10n.tr("login-button")).size(20.0))
                    .padding([12, 24])
                    .on_press(Message::LoginRequested(Click::Left)),
            )
            .on_right_press(Message::LoginRequested(Click::Right)),
            text(l10n.tr("login-button-hint")).size(14.0),
        ]
        .spacing(16)
        .align_x(Alignment::Center)
        .into(),
        LoginStatus::Fetching { .. } => {
            text(l10n.tr("login-fetching")).size(20.0).into()
        }
        LoginStatus::WebViewOpen { .. } => column![
            text(l10n.tr("login-webview-open")).size(20.0),
            button(text(l10n.tr("login-cancel"))).on_press(Message::CancelLogin),
        ]
        .spacing(12)
        .align_x(Alignment::Center)
        .into(),
        LoginStatus::Manual { url, input, notice, .. } => {
            let placeholder = l10n.tr("login-manual-placeholder");

            let url_box = container(
                text(url.clone())
                    .size(12.0)
                    .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
                    .width(460),
            )
            .padding(8)
            .style(container::rounded_box);

            let mut content = column![
                text(l10n.tr("login-manual-prompt")).size(20.0),
                text(l10n.tr("login-url-label")).size(14.0),
                url_box,
                row![
                    button(text(l10n.tr("login-copy-url"))).on_press(Message::CopyUrl),
                    button(text(l10n.tr("login-open-browser"))).on_press(Message::OpenBrowser),
                ]
                .spacing(8),
                text_input(&placeholder, input)
                    .on_input(Message::ManualInputChanged)
                    .on_submit(Message::SubmitManual)
                    .padding(10)
                    .width(480),
                row![
                    button(text(l10n.tr("login-submit"))).on_press(Message::SubmitManual),
                    button(text(l10n.tr("login-cancel"))).on_press(Message::CancelLogin),
                ]
                .spacing(8),
            ]
            .spacing(12)
            .align_x(Alignment::Center);

            if let Some(reason) = notice {
                content = content
                    .push(
                        text(l10n.tr("login-webview-fallback"))
                            .size(14.0)
                            .style(iced::widget::text::danger),
                    )
                    .push(text(reason.clone()).size(12.0));
            }

            content.into()
        }
        LoginStatus::Error(error) => {
            let message = l10n.tr_with_args("login-error", &[("error", error.clone())]);

            column![
                text(message).size(18.0).style(iced::widget::text::danger),
                button(text(l10n.tr("login-back"))).on_press(Message::CancelLogin),
            ]
            .spacing(12)
            .align_x(Alignment::Center)
            .into()
        }
        LoginStatus::LoggedIn { token, full_name } => {
            let mut content = column![lookup_view(state)]
                .spacing(12)
                .align_x(Alignment::Center);

            if cfg!(debug_assertions) {
                let debug = container(
                    column![
                        text(l10n.tr("debug-info")).size(16.0),
                        text(format!(
                            "{}: {}",
                            l10n.tr("debug-account"),
                            full_name.as_deref().unwrap_or("—")
                        )),
                        text(format!("{}: Bearer {}", l10n.tr("debug-token"), token)),
                        text(format!("{}: {}", l10n.tr("debug-uuid"), state.client_uuid)),
                    ]
                    .spacing(4)
                    .align_x(Alignment::Start),
                )
                .padding(12)
                .style(container::rounded_box);

                content = content.push(debug);
            }

            content.into()
        }
    };

    container(inner)
        .width(Fill)
        .height(Fill)
        .center_x(Fill)
        .center_y(Fill)
        .into()
}

/// The firmware lookup UI shown after a successful login.
fn lookup_view<'a>(state: &'a State) -> Element<'a, Message> {
    let l10n = &state.l10n;

    let options: Vec<Labeled<LookupMode>> = LookupMode::ALL
        .iter()
        .map(|&mode| Labeled {
            value: mode,
            label: l10n.tr(mode.message_id()),
        })
        .collect();

    let selected = Labeled {
        value: state.lookup.mode,
        label: l10n.tr(state.lookup.mode.message_id()),
    };

    let dropdown = pick_list(options, Some(selected), |option| {
        Message::LookupModeSelected(option.value)
    })
    .text_shaping(Shaping::Advanced);

    let mode_content: Element<'_, Message> = match state.lookup.mode {
        LookupMode::RowSmartphone => {
            let placeholder = l10n.tr("lookup-imei-placeholder");
            let fetching = matches!(state.lookup.status, LookupStatus::Fetching);

            let lookup_button = if fetching {
                button(text(l10n.tr("lookup-button")))
            } else {
                button(text(l10n.tr("lookup-button"))).on_press(Message::LookupRequested)
            };

            let content = column![
                row![
                    text(l10n.tr("lookup-imei-label")).size(16.0),
                    text_input(&placeholder, &state.lookup.imei_input)
                        .on_input(Message::ImeiInputChanged)
                        .on_submit(Message::LookupRequested)
                        .width(220),
                    lookup_button,
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            ]
            .spacing(8)
            .align_x(Alignment::Center);

            push_lookup_status(state, l10n, content).into()
        }
        LookupMode::RetcnSmartphone => {
            let placeholder = l10n.tr("lookup-imei-placeholder");
            let fetching = matches!(state.lookup.status, LookupStatus::Fetching);

            let lookup_button = if fetching {
                button(text(l10n.tr("lookup-button")))
            } else {
                button(text(l10n.tr("lookup-button"))).on_press(Message::RetcnLookupRequested)
            };

            let fastboot_fetching = matches!(
                state.lookup.retcn.fastboot_status,
                Some(FastbootStatus::Fetching)
            );
            let fill_button = if fastboot_fetching {
                button(text(l10n.tr("retcn-fill-fastboot-fetching")))
            } else {
                button(text(l10n.tr("retcn-fill-fastboot")))
                    .on_press(Message::FastbootFillRequested)
            };

            let platform_options: Vec<Labeled<firmware::Platform>> = firmware::Platform::ALL
                .iter()
                .map(|&platform| Labeled {
                    value: platform,
                    label: l10n.tr(platform.message_id()),
                })
                .collect();
            let selected_platform = Labeled {
                value: state.lookup.retcn.platform,
                label: l10n.tr(state.lookup.retcn.platform.message_id()),
            };
            let platform_dropdown: iced::widget::PickList<
                '_,
                Labeled<firmware::Platform>,
                Vec<Labeled<firmware::Platform>>,
                Labeled<firmware::Platform>,
                Message,
            > = pick_list(platform_options, Some(selected_platform), |option| {
                Message::RetcnPlatformSelected(option.value)
            })
            .text_shaping(Shaping::Advanced);

            let sim_options: Vec<Labeled<SimCount>> = SimCount::ALL
                .iter()
                .map(|&sim_count| Labeled {
                    value: sim_count,
                    label: l10n.tr(sim_count.message_id()),
                })
                .collect();
            let selected_sim = Labeled {
                value: state.lookup.retcn.sim_count,
                label: l10n.tr(state.lookup.retcn.sim_count.message_id()),
            };
            let sim_dropdown: iced::widget::PickList<
                '_,
                Labeled<SimCount>,
                Vec<Labeled<SimCount>>,
                Labeled<SimCount>,
                Message,
            > = pick_list(sim_options, Some(selected_sim), |option| {
                Message::RetcnSimCountSelected(option.value)
            })
            .text_shaping(Shaping::Advanced);

            let platform_extra: Element<'_, Message> = match state.lookup.retcn.platform {
                firmware::Platform::Qualcomm => row![
                    text(l10n.tr("retcn-fsg-label")).size(14.0),
                    text_input("", &state.lookup.retcn.fsg_version)
                        .on_input(|value| {
                            Message::RetcnFieldChanged(RetcnField::FsgVersion, value)
                        })
                        .width(240),
                ]
                .spacing(8)
                .align_y(Alignment::Center)
                .into(),
                firmware::Platform::MediaTek => row![
                    text(l10n.tr("retcn-sim-label")).size(14.0),
                    sim_dropdown,
                ]
                .spacing(8)
                .align_y(Alignment::Center)
                .into(),
            };

            let content = column![
                row![
                    text(l10n.tr("lookup-imei-label")).size(14.0),
                    text_input(&placeholder, &state.lookup.imei_input)
                        .on_input(Message::ImeiInputChanged)
                        .width(160),
                    text(l10n.tr("retcn-sn-label")).size(14.0),
                    text_input("", &state.lookup.retcn.serial_number)
                        .on_input(|value| {
                            Message::RetcnFieldChanged(RetcnField::SerialNumber, value)
                        })
                        .width(160),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                row![
                    text(l10n.tr("retcn-model-label")).size(14.0),
                    text_input("", &state.lookup.retcn.model)
                        .on_input(|value| Message::RetcnFieldChanged(RetcnField::Model, value))
                        .width(120),
                    text(l10n.tr("retcn-carrier-label")).size(14.0),
                    text_input("", &state.lookup.retcn.carrier)
                        .on_input(|value| Message::RetcnFieldChanged(RetcnField::Carrier, value))
                        .width(120),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                row![
                    text(l10n.tr("retcn-fingerprint-label")).size(14.0),
                    text_input("", &state.lookup.retcn.fingerprint)
                        .on_input(|value| {
                            Message::RetcnFieldChanged(RetcnField::Fingerprint, value)
                        })
                        .width(300),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                row![
                    text(l10n.tr("retcn-platform-label")).size(14.0),
                    platform_dropdown,
                    platform_extra,
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                row![fill_button, lookup_button]
                    .spacing(8)
                    .align_y(Alignment::Center),
            ]
            .spacing(8)
            .align_x(Alignment::Center);

            let mut content = content;

            match &state.lookup.retcn.fastboot_status {
                Some(FastbootStatus::Filled(serial)) => {
                    content = content.push(
                        text(l10n.tr_with_args(
                            "retcn-fill-fastboot-filled",
                            &[("serial", serial.clone())],
                        ))
                        .size(13.0)
                        .style(iced::widget::text::success),
                    );
                }
                Some(FastbootStatus::Error(error)) => {
                    content = content.push(
                        text(error.clone()).size(13.0).style(iced::widget::text::danger),
                    );
                }
                _ => {}
            }

            push_lookup_status(state, l10n, content).into()
        }
        LookupMode::Tablet => {
            let fetching = matches!(state.lookup.status, LookupStatus::Fetching);

            let lookup_button = if fetching {
                button(text(l10n.tr("lookup-button")))
            } else {
                button(text(l10n.tr("lookup-button"))).on_press(Message::TabletLookupRequested)
            };

            let content = column![
                row![
                    text(l10n.tr("tablet-sn-label")).size(16.0),
                    text_input("", &state.lookup.tablet.serial_number)
                        .on_input(Message::TabletSnChanged)
                        .on_submit(Message::TabletLookupRequested)
                        .width(220),
                    lookup_button,
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            ]
            .spacing(8)
            .align_x(Alignment::Center);

            push_lookup_status(state, l10n, content).into()
        }
        LookupMode::ByModel => {
            let fetching = matches!(state.lookup.status, LookupStatus::Fetching);

            let lookup_button = if fetching {
                button(text(l10n.tr("lookup-button")))
            } else {
                button(text(l10n.tr("lookup-button"))).on_press(Message::ModelLookupRequested)
            };

            let category_options: Vec<Labeled<firmware::Category>> = firmware::Category::ALL
                .iter()
                .map(|&category| Labeled {
                    value: category,
                    label: l10n.tr(category.message_id()),
                })
                .collect();
            let selected_category = Labeled {
                value: state.lookup.by_model.category,
                label: l10n.tr(state.lookup.by_model.category.message_id()),
            };
            let category_picker: iced::widget::PickList<
                '_,
                Labeled<firmware::Category>,
                Vec<Labeled<firmware::Category>>,
                Labeled<firmware::Category>,
                Message,
            > = pick_list(category_options, Some(selected_category), |option| {
                Message::ModelCategorySelected(option.value)
            })
            .text_shaping(Shaping::Advanced);

            let mut content = column![
                row![
                    text(l10n.tr("fw-model-name")).size(16.0),
                    text_input("", &state.lookup.by_model.model)
                        .on_input(Message::ModelInputChanged)
                        .on_submit(Message::ModelLookupRequested)
                        .width(220),
                    lookup_button,
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                row![
                    text(l10n.tr("by-model-category-label")).size(14.0),
                    category_picker,
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            ]
            .spacing(8)
            .align_x(Alignment::Center);

            // Country code only matters for tablets/smart devices
            // (phones omit it, matching `_seed_params` in motofw.py).
            if state.lookup.by_model.category != firmware::Category::Phone {
                content = content.push(
                    row![
                        text(l10n.tr("by-model-country-label")).size(14.0),
                        text_input("", &state.lookup.by_model.country_code)
                            .on_input(Message::ModelCountryChanged)
                            .width(100),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                );
            }

            // Discriminator fields the server needs for this model (shown
            // after the first request resolves `getRomMatchParams`).
            let params_ready =
                state.lookup.by_model.loaded_model == state.lookup.by_model.model.trim();
            if params_ready {
                for (index, key) in state.lookup.by_model.required.iter().enumerate() {
                    let label_id = match key.as_str() {
                        "fingerPrint" => Some("retcn-fingerprint-label"),
                        "roCarrier" => Some("retcn-carrier-label"),
                        "fsgVersion.qcom" => Some("retcn-fsg-label"),
                        "simCount" => Some("retcn-sim-label"),
                        _ => None,
                    };
                    let label = match label_id {
                        Some(id) => l10n.tr(id),
                        None => key.clone(),
                    };
                    let value = state
                        .lookup
                        .by_model
                        .values
                        .get(index)
                        .cloned()
                        .unwrap_or_default();

                    content = content.push(
                        row![
                            text(label).size(14.0),
                            text_input("", &value)
                                .on_input(move |input| Message::ModelParamChanged(index, input))
                                .width(220),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    );
                }
            }

            push_lookup_status(state, l10n, content).into()
        }
    };

    column![dropdown, mode_content]
        .spacing(12)
        .align_x(Alignment::Center)
        .into()
}

/// Appends the lookup status (progress, error, or result) to a column.
fn push_lookup_status<'a>(
    state: &'a State,
    l10n: &'a l10n::Bundle,
    mut content: iced::widget::Column<'a, Message>,
) -> iced::widget::Column<'a, Message> {
    match &state.lookup.status {
        LookupStatus::Idle => {}
        LookupStatus::Fetching => {
            content = content.push(text(l10n.tr("lookup-fetching")).size(14.0));
        }
        LookupStatus::Error(error) => {
            content = content.push(text(error.clone()).size(14.0).style(iced::widget::text::danger));
        }
        LookupStatus::Done(result) => {
            let view = match result {
                LookupResult::Standard(info) => firmware_info_view(l10n, info),
                LookupResult::CnTablet(info) => cn_tablet_info_view(l10n, info),
            };
            content = content.push(view);
        }
    }

    content
}

/// Displays a CN tablet lookup result, including the extraction password.
fn cn_tablet_info_view<'a>(
    l10n: &'a l10n::Bundle,
    info: &firmware::CnTabletInfo,
) -> Element<'a, Message> {
    let fields: [(&str, &str); 9] = [
        ("cn-product-name", &info.product_name),
        ("cn-product-model", &info.product_model),
        ("cn-market-name", &info.market_name),
        ("cn-mtm-compat", &info.mtm_compat),
        ("cn-latest-version", &info.latest_version),
        ("cn-id", &info.id),
        ("fw-publish-date", &info.publish_date),
        ("fw-file-name", &info.file_name),
        ("fw-file-size", &info.file_size),
    ];

    let copy_uri_button = if info.download_url.is_empty() {
        button(text(l10n.tr("fw-copy-uri")))
    } else {
        button(text(l10n.tr("fw-copy-uri")))
            .on_press(Message::CopyDownloadUri(info.download_url.clone()))
    };

    let copy_password_button = button(text(l10n.tr("tablet-copy-password")))
        .on_press(Message::CopyCnPassword(info.unzip_password.clone()));

    let buttons = row![copy_uri_button, copy_password_button].spacing(8);

    container(
        column!(
            column(fields.iter().map(|(id, value)| {
                let label = l10n.tr(id);
                let value = if value.is_empty() { "—" } else { value };

                text(format!("{label}: {value}")).size(13.0).into()
            }))
            .spacing(4)
            .align_x(Alignment::Start),
            buttons,
        )
        .spacing(8)
        .align_x(Alignment::Start),
    )
    .padding(12)
    .width(520)
    .style(container::rounded_box)
    .into()
}

/// Displays the fields of a firmware lookup result.
fn firmware_info_view<'a>(
    l10n: &'a l10n::Bundle,
    info: &firmware::FirmwareInfo,
) -> Element<'a, Message> {
    let fields: [(&str, &str); 11] = [
        ("fw-market-name", &info.market_name),
        ("fw-model-name", &info.model_name),
        ("fw-sale-model", &info.sale_model),
        ("fw-carrier", &info.carrier),
        ("fw-publish-date", &info.publish_date),
        ("fw-file-name", &info.file_name),
        ("fw-file-size", &info.file_size),
        ("fw-rom-id", &info.rom_id),
        ("fw-rom-match-id", &info.rom_match_id),
        ("fw-fingerprint", &info.fingerprint),
        ("fw-comments", &info.comments),
    ];

    let copy_uri_button = if info.rom_uri.is_empty() {
        button(text(l10n.tr("fw-copy-uri")))
    } else {
        button(text(l10n.tr("fw-copy-uri")))
            .on_press(Message::CopyDownloadUri(info.rom_uri.clone()))
    };

    let copy_tool_button = if info.tool_uri.is_empty() {
        button(text(l10n.tr("fw-copy-tool")))
    } else {
        button(text(l10n.tr("fw-copy-tool")))
            .on_press(Message::CopyToolUri(info.tool_uri.clone()))
    };

    let copy_raw_button = button(text(l10n.tr("fw-copy-raw")))
        .on_press(Message::CopyRawJson(info.raw_json.clone()));

    let buttons = row![copy_uri_button, copy_tool_button, copy_raw_button].spacing(8);

    container(
        column!(
            column(fields.iter().map(|(id, value)| {
                let label = l10n.tr(id);
                let value = if value.is_empty() { "—" } else { value };

                text(format!("{label}: {value}")).size(13.0).into()
            }))
            .spacing(4)
            .align_x(Alignment::Start),
            buttons,
        )
        .spacing(8)
        .align_x(Alignment::Start),
    )
    .padding(12)
    .width(520)
    .style(container::rounded_box)
    .into()
}

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
