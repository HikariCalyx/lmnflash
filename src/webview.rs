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

/// If this process was started as the login dialog, runs it and returns the
/// process exit code. Returns `None` in normal app mode.
pub fn run_dialog_process_if_requested() -> Option<i32> {
    let mut args = std::env::args();
    let _exe = args.next();
    if args.next().as_deref() != Some(DIALOG_MODE_ARG) {
        return None;
    }

    // `title` and `login_url` arrive as two lines on stdin, which avoids any
    // command-line quoting issues.
    use std::io::BufRead;

    let mut lines = std::io::stdin().lock().lines();
    let title = lines.next().transpose().ok().flatten().unwrap_or_default();
    let login_url = lines.next().transpose().ok().flatten().unwrap_or_default();

    let code = match run_dialog(&title, &login_url) {
        Ok(callback) => {
            println!("{callback}");
            0
        }
        Err(error) => {
            eprintln!("{error}");
            1
        }
    };

    Some(code)
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
    use wry::WebViewBuilderExtUnix;

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
