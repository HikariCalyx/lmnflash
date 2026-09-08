//! Embedded WebView login dialog (wry + winit), running in a child process.
//!
//! winit 0.30 allows only ONE event loop per process, and iced already owns
//! it. The dialog therefore runs as a separate invocation of our own binary
//! (`--login-dialog` mode), which also isolates the WebView (WebView2 on
//! Windows, WKWebView on macOS, WebKitGTK on Linux) from the main process.
//! The parent passes the dialog title and login URL over stdin and
//! reads the captured `SoftwareFix://callback` URL back from stdout.

use std::process::{Command, Stdio};
use std::sync::mpsc;

/// Command-line switch that marks this process as the login dialog.
const DIALOG_MODE_ARG: &str = "--login-dialog";

/// Command-line switch that marks this process as the Motorola portal-login
/// dialog (captures the session cookie after login).
const PORTAL_MODE_ARG: &str = "--portal-login";

/// The (decoded) page the Motorola portal redirects to once the user has
/// signed in. This must be the real `/app/standalone/...` path with actual
/// slashes — the login *entry* URL only contains the double-encoded
/// `standalone%252Fbootloader%252Funlock-your-device-b`, which must NOT be
/// treated as a successful login (that would close the dialog before the user
/// ever signs in).
const PORTAL_LOGIN_SUCCESS: &str = "/app/standalone/bootloader/unlock-your-device-b";

/// The logged-in account profile. When a valid session already exists (kept in
/// the webview's persistent profile from a previous login) the portal skips the
/// login page and redirects straight here instead of to the unlock page — this
/// is still a valid, ready-to-use session.
const PORTAL_LOGGED_IN_PROFILE: &str = "/app/account/profile";

/// True once the portal session is ready to use: the webview has either landed
/// on the post-login unlock page or (already logged in) on the account profile.
/// Both are decoded `/app/...` pages — the guest/entry redirects use the
/// double-encoded `welcome/redirect/...%252F...` form and never match here.
fn portal_login_complete(uri: &str) -> bool {
    let uri = uri.to_ascii_lowercase();
    uri.contains(PORTAL_LOGIN_SUCCESS) || uri.contains(PORTAL_LOGGED_IN_PROFILE)
}

/// The session captured by a successful Motorola portal login.
#[derive(Debug, Clone)]
pub struct PortalLogin {
    /// Cookie header (`name=value; name2=value2`) for the portal session.
    pub cookie_header: String,
    /// The URL the portal landed on after login.
    pub url: String,
}

/// True when the built-in webview can be used on this machine.
pub fn webview_available() -> bool {
    #[cfg(target_os = "windows")]
    {
        webview2_runtime_installed()
    }
    #[cfg(target_os = "macos")]
    {
        true
    }
    #[cfg(target_os = "linux")]
    {
        true
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        false
    }
}

/// Whether the WebView2 runtime is installed (Windows).
#[cfg(target_os = "windows")]
fn webview2_runtime_installed() -> bool {
    use std::process::Command;

    let key = "HKLM\\SOFTWARE\\WOW6432Node\\Microsoft\\EdgeUpdate\\Clients\\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";
    let output = Command::new("reg")
        .args(["query", key, "/v", "pv"])
        .output();
    matches!(output, Ok(out) if out.status.success())
}

/// If this process was started as the login dialog, runs it and returns the
/// process exit code. Returns `None` in normal app mode.
pub fn run_dialog_process_if_requested() -> Option<i32> {
    let mut args = std::env::args();
    let _exe = args.next();
    match args.next().as_deref() {
        Some(DIALOG_MODE_ARG) => Some(run_login_child()),
        Some(PORTAL_MODE_ARG) => Some(run_portal_child()),
        _ => None,
    }
}

/// Child-process entry for the regular (Lenovo OAuth) login dialog.
fn run_login_child() -> i32 {
    use std::io::BufRead;

    let mut lines = std::io::stdin().lock().lines();
    let title = lines.next().transpose().ok().flatten().unwrap_or_default();
    let login_url = lines.next().transpose().ok().flatten().unwrap_or_default();

    match run_dialog(&title, &login_url) {
        Ok(callback) => {
            println!("{callback}");
            0
        }
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

/// Child-process entry for the Motorola portal login: prints the captured
/// session (cookies + landing URL) as a single JSON line on stdout.
fn run_portal_child() -> i32 {
    use std::io::BufRead;

    let mut lines = std::io::stdin().lock().lines();
    let title = lines.next().transpose().ok().flatten().unwrap_or_default();
    let login_url = lines.next().transpose().ok().flatten().unwrap_or_default();

    match run_portal_login(&title, &login_url) {
        Ok(login) => {
            let json = serde_json::json!({
                "cookie_header": login.cookie_header,
                "url": login.url,
            });
            println!("{json}");
            0
        }
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

/// Shows the Motorola portal login in the embedded webview and returns the
/// captured session (cookies + landing URL).
pub fn show_portal_login(title: &str, login_url: &str) -> Result<PortalLogin, String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("failed to locate executable: {e}"))?;

    let mut child = Command::new(exe)
        .arg(PORTAL_MODE_ARG)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to start portal login dialog: {e}"))?;

    {
        use std::io::Write;

        let stdin = child.stdin.as_mut().expect("dialog stdin is piped");
        writeln!(stdin, "{title}")
            .and_then(|_| writeln!(stdin, "{login_url}"))
            .map_err(|e| format!("failed to send login data to dialog: {e}"))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("failed to wait for portal login dialog: {e}"))?;

    // The child's own diagnostics (e.g. the `[portal-nav]` page list) are
    // piped to stderr; relay them to our console so they can be inspected.
    #[cfg(debug_assertions)]
    {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        if !stderr.is_empty() {
            eprintln!("[portal-child]\n{stderr}");
        }
    }

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let value: serde_json::Value = serde_json::from_str(stdout.trim())
            .map_err(|e| format!("invalid portal login result: {e}"))?;

        let cookie_header = value
            .get("cookie_header")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        let url = value
            .get("url")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();

        if cookie_header.is_empty() {
            return Err("portal login did not expose a session cookie".to_string());
        }

        Ok(PortalLogin { cookie_header, url })
    } else {
        let error = String::from_utf8_lossy(&output.stderr);
        let error = error.trim();
        if error.is_empty() {
            Err("portal login was closed before completing".to_string())
        } else {
            Err(error.to_string())
        }
    }
}

/// Shows the login dialog and blocks until the login callback URL is captured.
pub fn show_login_dialog(title: &str, login_url: &str) -> Result<String, String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("failed to locate executable: {e}"))?;

    let mut child = Command::new(exe)
        .arg(DIALOG_MODE_ARG)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to start login dialog: {e}"))?;

    {
        use std::io::Write;

        let stdin = child.stdin.as_mut().expect("dialog stdin is piped");
        writeln!(stdin, "{title}")
            .and_then(|_| writeln!(stdin, "{login_url}"))
            .map_err(|e| format!("failed to send login data to dialog: {e}"))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("failed to wait for login dialog: {e}"))?;

    if output.status.success() {
        let callback = String::from_utf8_lossy(&output.stdout);
        let callback = callback.trim();
        if callback.is_empty() {
            return Err("login dialog closed without a login callback".to_string());
        }
        Ok(callback.to_string())
    } else {
        let error = String::from_utf8_lossy(&output.stderr);
        let error = error.trim();
        if error.is_empty() {
            Err("login dialog closed before login completed".to_string())
        } else {
            Err(error.to_string())
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn run_dialog(_title: &str, _login_url: &str) -> Result<String, String> {
    Err("the embedded webview is only supported on Windows, macOS and Linux".to_string())
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
enum DialogUserEvent {
    Callback(String),
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
struct DialogApp {
    title: String,
    login_url: String,
    proxy: winit::event_loop::EventLoopProxy<DialogUserEvent>,
    callback_tx: mpsc::Sender<String>,
    dialog_error_tx: mpsc::Sender<String>,
    dialog: Option<(winit::window::Window, wry::WebView)>,
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
impl winit::application::ApplicationHandler<DialogUserEvent> for DialogApp {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        use winit::dpi::{LogicalPosition, LogicalSize};
        use winit::window::{UserAttentionType, WindowAttributes, WindowLevel};
        use wry::WebViewBuilder;

        const WIDTH: f64 = 480.0;
        const HEIGHT: f64 = 640.0;

        if self.dialog.is_some() {
            return;
        }

        event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);

        // Center the dialog on the primary monitor
        // (winit has no `WindowPosition::Centered`).
        let position = event_loop.primary_monitor().map(|monitor| {
            let scale = monitor.scale_factor();
            let size = monitor.size().to_logical::<f64>(scale);
            let origin = monitor.position();

            LogicalPosition::new(
                origin.x as f64 + (size.width - WIDTH) / 2.0,
                origin.y as f64 + (size.height - HEIGHT) / 2.0,
            )
        });

        let mut attributes = WindowAttributes::default()
            .with_title(&self.title)
            .with_inner_size(LogicalSize::new(WIDTH, HEIGHT))
            .with_visible(true)
            .with_window_level(WindowLevel::AlwaysOnTop);
        if let Some(position) = position {
            attributes = attributes.with_position(position);
        }

        // The navigation handler owns its own sender, so clone the proxy.
        let proxy = self.proxy.clone();

        let result = event_loop
            .create_window(attributes)
            .map_err(|e| format!("failed to create dialog window: {e}"))
            .and_then(|window| {
                let webview = WebViewBuilder::new()
                    .with_url(&self.login_url)
                    .with_navigation_handler(move |uri: String| {
                        if uri.to_ascii_lowercase().starts_with("softwarefix://") {
                            // The OAuth flow finished: capture the callback and
                            // cancel the navigation to the custom scheme.
                            let _ = proxy.send_event(DialogUserEvent::Callback(uri));
                            false
                        } else {
                            true
                        }
                    })
                    .build(&window)
                    .map_err(|e| format!("webview unavailable: {e}"))?;

                Ok((window, webview))
            });

        match result {
            Ok((window, webview)) => {
                eprintln!("[webview] dialog window and webview created");
                window.focus_window();
                window.request_user_attention(Some(UserAttentionType::Informational));
                self.dialog = Some((window, webview));
            }
            Err(error) => {
                eprintln!("[webview] dialog creation failed: {error}");
                let _ = self.dialog_error_tx.send(error);
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        if let winit::event::WindowEvent::CloseRequested = event {
            event_loop.exit();
        }
    }

    fn user_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        event: DialogUserEvent,
    ) {
        match event {
            DialogUserEvent::Callback(callback) => {
                let _ = self.callback_tx.send(callback);
                event_loop.exit();
            }
        }
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn run_dialog(title: &str, login_url: &str) -> Result<String, String> {
    use winit::event_loop::EventLoop;

    eprintln!("[webview] creating event loop…");

    let mut event_loop_builder = EventLoop::<DialogUserEvent>::with_user_event();

    #[cfg(target_os = "windows")]
    {
        use winit::platform::windows::EventLoopBuilderExtWindows;
        event_loop_builder.with_any_thread(true);
    }

    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};

        // `Accessory` keeps this dialog child process out of the Dock and
        // menu bar, but its window still accepts clicks and keyboard input.
        event_loop_builder.with_activation_policy(ActivationPolicy::Accessory);
    }

    let event_loop = event_loop_builder
        .build()
        .map_err(|e| format!("failed to create event loop: {e}"))?;

    eprintln!("[webview] event loop created, entering run…");

    let (callback_tx, callback_rx) = mpsc::channel::<String>();
    let (dialog_error_tx, dialog_error_rx) = mpsc::channel::<String>();

    let mut app = DialogApp {
        title: title.to_owned(),
        login_url: login_url.to_owned(),
        proxy: event_loop.create_proxy(),
        callback_tx,
        dialog_error_tx,
        dialog: None,
    };

    event_loop
        .run_app(&mut app)
        .map_err(|e| format!("login dialog event loop failed: {e}"))?;

    match callback_rx.try_recv() {
        Ok(callback) => {
            eprintln!("[webview] callback captured");
            Ok(callback)
        }
        Err(_) => Err(dialog_error_rx.try_recv().unwrap_or_else(|_| {
            "login dialog was closed before login completed".to_string()
        })),
    }
}

/// Linux dialog: WebKitGTK inside a plain GTK window.
///
/// wry cannot embed into a winit window on Linux (X11-only, and GTK's main
/// loop must run alongside), so the dialog child process runs its own GTK
/// main loop instead. This works on both X11 and Wayland.
#[cfg(target_os = "linux")]
fn run_dialog(title: &str, login_url: &str) -> Result<String, String> {
    use gtk::prelude::*;
    use wry::{WebViewBuilder, WebViewBuilderExtUnix};

    gtk::init().map_err(|e| format!("failed to initialize GTK: {e}"))?;

    const WIDTH: i32 = 480;
    const HEIGHT: i32 = 640;

    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title(title);
    window.set_default_size(WIDTH, HEIGHT);
    window.set_position(gtk::WindowPosition::Center);
    window.set_keep_above(true);

    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
    window.add(&vbox);

    let (callback_tx, callback_rx) = mpsc::channel::<String>();

    // Closing the dialog window quits the GTK main loop without a callback.
    window.connect_destroy(|_| gtk::main_quit());

    let _webview = WebViewBuilder::new()
        .with_url(login_url)
        .with_navigation_handler(move |uri: String| {
            if uri.to_ascii_lowercase().starts_with("softwarefix://") {
                // The OAuth flow finished: capture the callback, cancel the
                // navigation to the custom scheme and close the dialog.
                let _ = callback_tx.send(uri);
                gtk::main_quit();
                false
            } else {
                true
            }
        })
        .build_gtk(&vbox)
        .map_err(|e| format!("webview unavailable: {e}"))?;

    eprintln!("[webview] dialog window and webview created");

    window.show_all();
    gtk::main();

    match callback_rx.try_recv() {
        Ok(callback) => {
            eprintln!("[webview] callback captured");
            Ok(callback)
        }
        Err(_) => Err("login dialog was closed before login completed".to_string()),
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn run_portal_login(_title: &str, _login_url: &str) -> Result<PortalLogin, String> {
    Err("the embedded webview is only supported on Windows, macOS and Linux".to_string())
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
enum PortalUserEvent {
    Landed(String),
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
struct PortalApp {
    title: String,
    login_url: String,
    proxy: winit::event_loop::EventLoopProxy<PortalUserEvent>,
    tx: mpsc::Sender<PortalLogin>,
    error_tx: mpsc::Sender<String>,
    dialog: Option<(winit::window::Window, wry::WebView)>,
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
impl winit::application::ApplicationHandler<PortalUserEvent> for PortalApp {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        use winit::dpi::{LogicalPosition, LogicalSize};
        use winit::window::{UserAttentionType, WindowAttributes, WindowLevel};
        use wry::WebViewBuilder;

        const WIDTH: f64 = 480.0;
        const HEIGHT: f64 = 720.0;

        if self.dialog.is_some() {
            return;
        }

        event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);

        let position = event_loop.primary_monitor().map(|monitor| {
            let scale = monitor.scale_factor();
            let size = monitor.size().to_logical::<f64>(scale);
            let origin = monitor.position();

            LogicalPosition::new(
                origin.x as f64 + (size.width - WIDTH) / 2.0,
                origin.y as f64 + (size.height - HEIGHT) / 2.0,
            )
        });

        let mut attributes = WindowAttributes::default()
            .with_title(&self.title)
            .with_inner_size(LogicalSize::new(WIDTH, HEIGHT))
            .with_visible(true)
            .with_window_level(WindowLevel::AlwaysOnTop);
        if let Some(position) = position {
            attributes = attributes.with_position(position);
        }

        let proxy = self.proxy.clone();

        let result = event_loop
            .create_window(attributes)
            .map_err(|e| format!("failed to create portal window: {e}"))
            .and_then(|window| {
                let webview = WebViewBuilder::new()
                    .with_url(&self.login_url)
                    .with_navigation_handler(move |uri: String| {
                        #[cfg(debug_assertions)]
                        eprintln!("[portal-nav] {uri}");
                        // Capture the session once the portal is ready: a fresh
                        // login lands on the unlock page, while an existing
                        // session skips straight to the account profile.
                        if portal_login_complete(&uri) {
                            let _ = proxy.send_event(PortalUserEvent::Landed(uri));
                            false
                        } else {
                            true
                        }
                    })
                    .build(&window)
                    .map_err(|e| format!("webview unavailable: {e}"))?;

                Ok((window, webview))
            });

        match result {
            Ok((window, webview)) => {
                eprintln!("[webview] portal window and webview created");
                window.focus_window();
                window.request_user_attention(Some(UserAttentionType::Informational));
                self.dialog = Some((window, webview));
            }
            Err(error) => {
                eprintln!("[webview] portal creation failed: {error}");
                let _ = self.error_tx.send(error);
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        if let winit::event::WindowEvent::CloseRequested = event {
            event_loop.exit();
        }
    }

    fn user_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        event: PortalUserEvent,
    ) {
        let PortalUserEvent::Landed(url) = event;
        // Grab the session cookies from the webview so the parent can use
        // them for the profile fetch and the unlock-key request.
        let cookies = self
            .dialog
            .as_ref()
            .and_then(|(_, webview)| webview.cookies().ok())
            .unwrap_or_default();
        let cookie_header = cookies
            .iter()
            .map(|cookie| format!("{}={}", cookie.name(), cookie.value()))
            .collect::<Vec<_>>()
            .join("; ");

        let _ = self.tx.send(PortalLogin {
            cookie_header,
            url,
        });
        event_loop.exit();
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn run_portal_login(title: &str, login_url: &str) -> Result<PortalLogin, String> {
    use winit::event_loop::EventLoop;

    eprintln!("[webview] creating portal event loop…");

    let mut event_loop_builder = EventLoop::<PortalUserEvent>::with_user_event();

    #[cfg(target_os = "windows")]
    {
        use winit::platform::windows::EventLoopBuilderExtWindows;
        event_loop_builder.with_any_thread(true);
    }

    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        event_loop_builder.with_activation_policy(ActivationPolicy::Accessory);
    }

    let event_loop = event_loop_builder
        .build()
        .map_err(|e| format!("failed to create event loop: {e}"))?;

    let (tx, rx) = mpsc::channel::<PortalLogin>();
    let (error_tx, error_rx) = mpsc::channel::<String>();

    let mut app = PortalApp {
        title: title.to_owned(),
        login_url: login_url.to_owned(),
        proxy: event_loop.create_proxy(),
        tx,
        error_tx,
        dialog: None,
    };

    event_loop
        .run_app(&mut app)
        .map_err(|e| format!("portal login event loop failed: {e}"))?;

    match rx.try_recv() {
        Ok(login) => {
            eprintln!("[webview] portal login session captured");
            Ok(login)
        }
        Err(_) => Err(error_rx.try_recv().unwrap_or_else(|_| {
            "portal login was closed before completing".to_string()
        })),
    }
}

/// Linux portal login (WebKitGTK). Kept in sync with the GTK login dialog.
#[cfg(target_os = "linux")]
fn run_portal_login(title: &str, login_url: &str) -> Result<PortalLogin, String> {
    use gtk::prelude::*;
    use wry::{WebViewBuilder, WebViewBuilderExtUnix};

    gtk::init().map_err(|e| format!("failed to initialize GTK: {e}"))?;

    const WIDTH: i32 = 480;
    const HEIGHT: i32 = 720;

    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title(title);
    window.set_default_size(WIDTH, HEIGHT);
    window.set_position(gtk::WindowPosition::Center);
    window.set_keep_above(true);

    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
    window.add(&vbox);

    let (tx, rx) = mpsc::channel::<PortalLogin>();

    window.connect_destroy(|_| gtk::main_quit());

    let webview = WebViewBuilder::new()
        .with_url(login_url)
        .with_navigation_handler(move |uri: String| {
            #[cfg(debug_assertions)]
            eprintln!("[portal-nav] {uri}");
            if portal_login_complete(&uri) {
                let _ = tx.send(PortalLogin {
                    // WebKitGTK may not expose cookies via wry on Linux; when
                    // empty the parent reports a clear error.
                    cookie_header: String::new(),
                    url: uri,
                });
                gtk::main_quit();
                false
            } else {
                true
            }
        })
        .build_gtk(&vbox)
        .map_err(|e| format!("webview unavailable: {e}"))?;

    let _webview = webview;
    eprintln!("[webview] portal window and webview created");

    window.show_all();
    gtk::main();

    rx.try_recv()
        .map_err(|_| "portal login was closed before completing".to_string())
}

#[cfg(test)]
mod tests {
    use super::portal_login_complete;

    /// The double-encoded *entry* and *guest-session* redirects must never be
    /// mistaken for a completed login (they would close the dialog before the
    /// user signs in).
    #[test]
    fn entry_and_guest_redirects_are_not_completed_logins() {
        assert!(!portal_login_complete(
            "https://en-us.support.motorola.com/app/utils/welcome/redirect/standalone%252Fbootloader%252Funlock-your-device-b"
        ));
        assert!(!portal_login_complete(
            "https://en-us.support.motorola.com/app/utils/welcome/redirect/account%252Fprofile/session/L3RpbWUvMTc4ODg0OTczOS8"
        ));
        assert!(!portal_login_complete("https://en-us.support.motorola.com/app/account/login"));
    }

    /// The post-login unlock page and the already-logged-in account profile are
    /// both valid, ready-to-use sessions.
    #[test]
    fn unlock_and_profile_landings_are_completed_logins() {
        assert!(portal_login_complete(
            "https://en-us.support.motorola.com/app/standalone/bootloader/unlock-your-device-b"
        ));
        assert!(portal_login_complete(
            "https://en-us.support.motorola.com/app/standalone/bootloader/unlock-your-device-b?lang=en"
        ));
        assert!(portal_login_complete(
            "https://en-us.support.motorola.com/app/account/profile"
        ));
    }
}
