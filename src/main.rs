// Release builds run without a console window; debug builds keep one so
// `eprintln!` diagnostics remain visible with `cargo run`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod fastboot_info;
mod firmware;
mod l10n;
mod login;
mod webview;

use iced::widget::{
    button, column, container, horizontal_rule, mouse_area, pick_list, row,
    stack, text_input, Space,
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
}

impl LookupMode {
    const ALL: [Self; 3] = [Self::RowSmartphone, Self::RetcnSmartphone, Self::Tablet];

    fn message_id(self) -> &'static str {
        match self {
            Self::RowSmartphone => "lookup-mode-row",
            Self::RetcnSmartphone => "lookup-mode-retcn",
            Self::Tablet => "lookup-mode-tablet",
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
    Open(Vec<String>),
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

    l10n::Language::EnUs
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
    CopyCnPassword(String),
    CopyDownloadUri(String),
    CopyToolUri(String),
    CopyRawJson(String),
}

fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::ModeSelected(mode) => {
            state.mode = mode;
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
        Message::CancelLogin => {
            state.login = LoginStatus::LoggedOut;
            state.lookup.retry_after_login = false;
            Task::none()
        }
        Message::LookupModeSelected(mode) => {
            state.lookup.mode = mode;
            state.lookup.status = LookupStatus::Idle;
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
            Ok(list) if list.supported.len() == 1 => {
                state.lookup.retcn.device_picker = DevicePicker::Closed;
                let serial = list.supported[0].clone();
                start_fastboot_fill(serial)
            }
            Ok(list) if list.supported.len() > 1 => {
                state.lookup.retcn.fastboot_status = None;
                state.lookup.retcn.device_picker = DevicePicker::Open(list.supported);
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
                Ok(result) => state.lookup.status = LookupStatus::Done(result),
                Err(error) => state.lookup.status = LookupStatus::Error(error),
            }

            Task::none()
        }
        Message::CopyCnPassword(password) => iced::clipboard::write::<Message>(password),
        Message::LookupRequested => request_lookup(state),
        Message::LookupFinished(result) => {
            if !matches!(state.lookup.status, LookupStatus::Fetching) {
                return Task::none();
            }

            match result {
                Ok(info) => {
                    state.lookup.status = LookupStatus::Done(LookupResult::Standard(info));
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

fn view(state: &State) -> Element<'_, Message> {
    let content = match state.mode {
        Mode::Mode1 => firmware_lookup_view(state),
        _ => placeholder_view(state),
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

    // Device picker modal: shown when more than one fastboot device is
    // connected. Clicks on the dimmed backdrop cancel the selection.
    if let DevicePicker::Open(devices) = &state.lookup.retcn.device_picker {
        let device_buttons = iced::widget::Column::with_children(
            devices.iter().map(|serial| {
                button(text(serial.clone()))
                    .width(Fill)
                    .on_press(Message::FastbootDeviceSelected(serial.clone()))
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

fn placeholder_view(state: &State) -> Element<'_, Message> {
    container(
        column![
            text(state.l10n.tr("app-title")).size(52.0),
            text(state.l10n.tr(state.mode.message_id())).size(20.0),
        ]
        .spacing(8)
        .align_x(Alignment::Center),
    )
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
}
