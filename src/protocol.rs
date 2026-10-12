//! `softwarefix://` URL protocol handler registration.
//!
//! The official Motorola/Lenovo "Software Fix" registers the `softwarefix://`
//! scheme so the browser can hand its OAuth callback back to it. LMN Flash
//! uses the same callback (see [`crate::login::extract_login`]), so it can
//! take the scheme over — but only when the user asks for it: at startup the
//! scheme is registered **only when nothing else handles it**, and the Manual
//! Login page offers a toggle otherwise.
//!
//! * **Windows** — a per-user registry class
//!   (`HKCU\Software\Classes\softwarefix`), so no administrator rights are
//!   needed and the original program's machine-wide registration is never
//!   touched. The per-user command that a takeover shadows is remembered (see
//!   [`crate::config`]) so switching back can restore it; a machine-wide
//!   handler needs no backup, because deleting our key reveals it again.
//! * **Linux** — a desktop entry under `~/.local/share/applications` plus
//!   `xdg-mime default … x-scheme-handler/softwarefix`. The entry runs the
//!   executable with the URL as an argument, which
//!   [`callback_arg`] picks up.
//! * **macOS** — a LaunchServices registration
//!   (`LSSetDefaultHandlerForURLScheme`) for the scheme the bundle declares in
//!   its `Info.plist` (`CFBundleURLTypes`, see `macos/Info.plist.mako`), so the
//!   application has to run from its bundle. The URL then arrives as a
//!   `kAEGetURL` Apple Event sent to the **running** application — iced exposes
//!   no API for it, so the event is handled here (see
//!   [`listen_for_callbacks`]) and no second process is started. The
//!   registration names the bundle identifier rather than a path, so a moved
//!   application stays registered; LaunchServices has no way to forget a
//!   handler though, which is what [`restore`] has to work around.

use iced::futures::channel::mpsc::UnboundedReceiver;

/// The URL scheme handled by the official Software Fix program.
const SCHEME: &str = "softwarefix";

/// Which program currently handles `softwarefix://` links.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Handler {
    /// The platform has no runtime URL protocol handling at all (neither
    /// Windows, Linux nor macOS).
    Unsupported,
    /// Nothing is registered for the scheme.
    None,
    /// LMN Flash itself handles the scheme.
    Ours,
    /// Another program handles the scheme; carries its command line (Windows),
    /// desktop entry id (Linux) or bundle identifier (macOS).
    Other(String),
}

impl Handler {
    /// The program a "foreign" handler names, for display.
    pub fn program(&self) -> Option<String> {
        match self {
            Handler::Other(command) => Some(command_program(command)),
            _ => None,
        }
    }
}

/// Registers LMN Flash when nothing else handles the scheme, reporting the
/// resulting state (and the failure, if registration did not work).
///
/// Called once at startup. An existing handler is deliberately left alone.
pub fn ensure_registered() -> (Handler, Option<String>) {
    let existing = current();

    if existing != Handler::None {
        return (existing, None);
    }

    match register() {
        Ok(()) => (current(), None),
        Err(error) => (Handler::None, Some(error)),
    }
}

/// The `softwarefix://` callback URL this process was launched with, if any.
///
/// Windows passes the clicked URL as a command-line argument; both the bare
/// form (`"C:\…\lmnflash.exe" "softwarefix://callback?…"`) and an explicit
/// `--softwarefix <url>` switch are accepted.
pub fn callback_arg() -> Option<String> {
    let prefix = format!("{SCHEME}://");
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        if arg.to_ascii_lowercase().starts_with(&prefix) {
            return Some(arg);
        }
        if arg == "--softwarefix" {
            return args.next();
        }
    }

    None
}

/// Extracts the executable path from a protocol command line.
///
/// The command is typically `"C:\path\app.exe" "%1"`; the first quoted token
/// (or, for an unquoted path, the first whitespace-separated one) is the
/// program.
pub fn command_program(command: &str) -> String {
    let command = command.trim();

    if let Some(rest) = command.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            return rest[..end].to_string();
        }
    }

    command
        .split_whitespace()
        .next()
        .unwrap_or(command)
        .to_string()
}

/// The file name of the Linux desktop entry that registers the scheme.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const DESKTOP_ID: &str = "lmnflash.desktop";

/// The `~/.local/share/applications` desktop entry that registers LMN Flash
/// as the `softwarefix://` handler.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn desktop_entry_path() -> std::path::PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.data_dir().join("applications"))
        .unwrap_or_default()
        .join(DESKTOP_ID)
}

/// Quotes a path for a desktop-entry `Exec` value (the spec requires
/// escaping `\`, `"`, backtick and `$` inside the double quotes).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn exec_path(path: &std::path::Path) -> String {
    let mut quoted = String::from("\"");

    for character in path.to_string_lossy().chars() {
        if matches!(character, '\\' | '"' | '`' | '$') {
            quoted.push('\\');
        }
        quoted.push(character);
    }

    quoted.push('"');
    quoted
}

/// The `Exec=` value of a desktop entry, if it has one.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn entry_exec(contents: &str) -> Option<&str> {
    contents.lines().find_map(|line| line.strip_prefix("Exec="))
}

/// The desktop entry that declares LMN Flash as the `softwarefix://` handler.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn desktop_entry(exe: &std::path::Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=LMN Flash\n\
         Comment=Firmware lookup and flashing utility\n\
         Exec={} %u\n\
         Terminal=false\n\
         NoDisplay=true\n\
         MimeType=x-scheme-handler/{SCHEME};\n\
         Categories=Utility;\n",
        exec_path(exe)
    )
}

#[cfg(windows)]
mod platform {
    use super::{Handler, SCHEME, command_program};
    use winreg::RegKey;
    use winreg::enums::{HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};

    /// True when a command line names this very executable.
    fn is_ours(command: &str) -> bool {
        let program = command_program(command);

        let Ok(exe) = std::env::current_exe() else {
            return false;
        };

        program.eq_ignore_ascii_case(&exe.to_string_lossy())
    }

    /// True when a command line names this executable *or* a previous copy of
    /// it that has since been moved away (so the registration is ours, just
    /// outdated).
    pub(super) fn is_ours_or_moved(command: &str) -> bool {
        if is_ours(command) {
            return true;
        }

        let program = command_program(command);
        let registered = std::path::Path::new(&program);

        let Ok(exe) = std::env::current_exe() else {
            return false;
        };

        let (Some(registered_name), Some(exe_name)) = (registered.file_name(), exe.file_name())
        else {
            return false;
        };

        // Same program name, but the registered path is gone: the application
        // was moved (or deleted) after it registered itself.
        registered_name.eq_ignore_ascii_case(exe_name) && !registered.exists()
    }

    const CLASSES: &str = r"Software\Classes";
    const COMMAND_SUBKEY: &str = r"shell\open\command";
    const URL_PROTOCOL_VALUE: &str = "URL Protocol";

    /// Path of the class below `HKEY_CURRENT_USER`.
    fn class_path() -> String {
        format!(r"{CLASSES}\{SCHEME}")
    }

    /// Path of the handler command below `HKEY_CURRENT_USER`.
    pub(super) fn command_path() -> String {
        format!(r"{CLASSES}\{SCHEME}\{COMMAND_SUBKEY}")
    }

    /// Path of the handler command below `HKEY_CLASSES_ROOT`.
    ///
    /// `HKEY_CLASSES_ROOT` *is* the merged `Software\Classes`, so this must
    /// NOT repeat the `Software\Classes` prefix — doing so looks up
    /// `HKCR\Software\Classes\…`, which never exists.
    pub(super) fn scheme_command_path() -> String {
        format!(r"{SCHEME}\{COMMAND_SUBKEY}")
    }

    /// The effective command line (the merged `HKEY_CLASSES_ROOT` view).
    fn registered_command() -> Option<String> {
        RegKey::predef(HKEY_CLASSES_ROOT)
            .open_subkey_with_flags(scheme_command_path(), KEY_READ)
            .and_then(|key| key.get_value::<String, _>(""))
            .ok()
    }

    /// The command line registered per-user (HKCU), if any.
    fn user_command() -> Option<String> {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(command_path(), KEY_READ)
            .and_then(|key| key.get_value::<String, _>(""))
            .ok()
    }

    pub(super) fn current() -> Handler {
        let Some(command) = registered_command() else {
            return Handler::None;
        };

        if is_ours(&command) {
            Handler::Ours
        } else if is_ours_or_moved(&command) {
            // Our own registration, left behind by an executable that has been
            // moved: report it as unregistered so it is rewritten in place.
            Handler::None
        } else {
            Handler::Other(command)
        }
    }

    pub(super) fn register() -> Result<(), String> {
        let exe = std::env::current_exe()
            .map_err(|e| format!("could not locate the executable: {e}"))?;
        let command = format!("\"{}\" \"%1\"", exe.to_string_lossy());

        // Remember a per-user handler we are about to shadow so switching back
        // can restore it. Recorded once: re-registering (e.g. after the
        // executable moved) must not overwrite the backup with our own command.
        if crate::config::load_protocol_backup().is_none() {
            let previous = user_command().filter(|command| !is_ours_or_moved(command));
            crate::config::save_protocol_backup(previous.as_deref())?;
        }

        let description = "URL:SoftwareFix Protocol".to_string();
        let protocol_flag = String::new();

        let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey_with_flags(class_path(), KEY_WRITE)
            .map_err(|e| format!("could not open the registry key: {e}"))?;

        key.set_value("", &description)
            .and_then(|_| key.set_value(URL_PROTOCOL_VALUE, &protocol_flag))
            .map_err(|e| format!("could not write the protocol description: {e}"))?;

        let (command_key, _) = key
            .create_subkey_with_flags(COMMAND_SUBKEY, KEY_WRITE)
            .map_err(|e| format!("could not open the command key: {e}"))?;

        command_key
            .set_value("", &command)
            .map_err(|e| format!("could not write the handler command: {e}"))
    }

    pub(super) fn restore() -> Result<(), String> {
        let current = RegKey::predef(HKEY_CURRENT_USER);

        match crate::config::load_protocol_backup() {
            // A per-user handler was replaced: put its command back.
            Some(command) => {
                let key = current
                    .open_subkey_with_flags(command_path(), KEY_WRITE)
                    .map_err(|e| format!("could not open the registry key: {e}"))?;

                key.set_value("", &command)
                    .map_err(|e| format!("could not restore the original handler: {e}"))?;
            }
            // Only a machine-wide handler (if any) was shadowed: deleting our
            // key reveals it again.
            None => match current.delete_subkey_all(class_path()) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!("could not remove the registry key: {error}"));
                }
            },
        }

        crate::config::clear_protocol_backup()
    }
}

/// Linux registration through a per-user desktop entry and `xdg-mime`.
///
/// The entry runs the executable with the clicked URL as an argument
/// (`Exec=… %u`), which [`callback_arg`] picks up — unlike macOS, no event
/// plumbing is needed.
#[cfg(target_os = "linux")]
mod platform {
    use super::{DESKTOP_ID, Handler, SCHEME, desktop_entry, desktop_entry_path, entry_exec, exec_path};
    use std::process::Command;

    /// The MIME type that names a URL scheme handler.
    fn mime_type() -> String {
        format!("x-scheme-handler/{SCHEME}")
    }

    /// The desktop entry id the scheme currently points at, if any.
    fn query_default() -> Option<String> {
        let output = Command::new("xdg-mime")
            .arg("query")
            .arg("default")
            .arg(mime_type())
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let id = String::from_utf8_lossy(&output.stdout).trim().to_string();

        (!id.is_empty()).then_some(id)
    }

    /// Points the scheme at a desktop entry.
    fn set_default(id: &str) -> Result<(), String> {
        let output = Command::new("xdg-mime")
            .arg("default")
            .arg(id)
            .arg(mime_type())
            .output()
            .map_err(|e| format!("could not run xdg-mime: {e}"))?;

        if output.status.success() {
            Ok(())
        } else {
            Err(format!("xdg-mime default exited with {}", output.status))
        }
    }

    /// Rebuilds the desktop entry cache (best effort: a missing
    /// `update-desktop-database` must not fail registration).
    fn update_database() {
        if let Some(directory) = desktop_entry_path().parent() {
            let _ = Command::new("update-desktop-database").arg(directory).output();
        }
    }

    /// True when our desktop entry still runs this executable.
    fn our_desktop_entry_is_current() -> bool {
        let Ok(contents) = std::fs::read_to_string(desktop_entry_path()) else {
            return false;
        };

        let Ok(exe) = std::env::current_exe() else {
            return false;
        };

        entry_exec(&contents)
            .is_some_and(|value| value.trim_start().starts_with(&exec_path(&exe)))
    }

    pub(super) fn current() -> Handler {
        match query_default() {
            Some(id) if id == DESKTOP_ID => {
                // A stale association (the desktop entry was deleted, or it
                // still runs a previous location of the executable) is not a
                // working handler: report it as unregistered so that it is
                // rewritten in place.
                if our_desktop_entry_is_current() {
                    Handler::Ours
                } else {
                    Handler::None
                }
            }
            Some(id) => Handler::Other(id),
            None => Handler::None,
        }
    }

    pub(super) fn register() -> Result<(), String> {
        let exe = std::env::current_exe()
            .map_err(|e| format!("could not locate the executable: {e}"))?;
        let path = desktop_entry_path();

        // Remember a handler we are about to shadow so switching back can
        // restore it. Recorded once, like the Windows registration.
        if crate::config::load_protocol_backup().is_none() {
            let previous = query_default().filter(|id| id != DESKTOP_ID);
            crate::config::save_protocol_backup(previous.as_deref())?;
        }

        let directory = path
            .parent()
            .ok_or_else(|| "could not locate the applications directory".to_string())?;

        std::fs::create_dir_all(directory)
            .map_err(|e| format!("could not create {}: {e}", directory.display()))?;
        std::fs::write(&path, desktop_entry(&exe))
            .map_err(|e| format!("could not write {}: {e}", path.display()))?;

        update_database();
        set_default(DESKTOP_ID)
    }

    pub(super) fn restore() -> Result<(), String> {
        if let Some(previous) = crate::config::load_protocol_backup() {
            set_default(&previous)?;
        }

        let _ = std::fs::remove_file(desktop_entry_path());
        update_database();

        crate::config::clear_protocol_backup()
    }
}

/// macOS registration through LaunchServices and the Apple Event that carries
/// the clicked URL.
///
/// Declaring the scheme in the bundle's `Info.plist` (`CFBundleURLTypes`, see
/// `macos/Info.plist.mako`) is what makes LaunchServices offer LMN Flash for
/// `softwarefix://` links at all; pointing the scheme *at* the bundle is the
/// runtime call in [`register`]. The URL then arrives as an Apple Event
/// (`kInternetEventClass` / `kAEGetURL`) sent to the running application, which
/// [`listen_for_callbacks`] receives — no second process is involved.
#[cfg(target_os = "macos")]
mod platform {
    use super::{Handler, SCHEME};
    use std::ffi::{c_char, c_void};
    use std::sync::Mutex;

    use iced::futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, NSObject, NSObjectProtocol};
    use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};

    /// `kCFStringEncodingUTF8`.
    const UTF8: u32 = 0x0800_0100;

    /// `kInternetEventClass` and `kAEGetURL`: the class and the id of the
    /// Apple Event macOS sends when a URL scheme is opened. Both are the
    /// four-character code `'GURL'`.
    const AE_GET_URL: u32 = u32::from_be_bytes(*b"GURL");

    /// `keyDirectObject`: the Apple Event parameter carrying the URL.
    const KEY_DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

    type CFStringRef = *const c_void;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFBundleGetMainBundle() -> *const c_void;
        /// Borrowed: the bundle keeps its identifier alive.
        fn CFBundleGetIdentifier(bundle: *const c_void) -> CFStringRef;
        fn CFStringCreateWithCString(
            allocator: *const c_void,
            value: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFStringGetLength(string: CFStringRef) -> isize;
        fn CFStringGetMaximumSizeForEncoding(length: isize, encoding: u32) -> isize;
        fn CFStringGetCString(
            string: CFStringRef,
            buffer: *mut c_char,
            size: isize,
            encoding: u32,
        ) -> u8;
        fn CFRelease(value: *const c_void);
    }

    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        /// Owned: the caller has to release the returned string.
        fn LSCopyDefaultHandlerForURLScheme(scheme: CFStringRef) -> CFStringRef;
        fn LSSetDefaultHandlerForURLScheme(scheme: CFStringRef, handler: CFStringRef) -> i32;
    }

    /// A `CFString` this process created, released when it is dropped.
    struct OwnedString(CFStringRef);

    impl OwnedString {
        /// Wraps a Rust string in a `CFString`.
        fn new(value: &str) -> Option<Self> {
            let value = std::ffi::CString::new(value).ok()?;
            let string =
                unsafe { CFStringCreateWithCString(std::ptr::null(), value.as_ptr(), UTF8) };

            (!string.is_null()).then_some(Self(string))
        }
    }

    impl Drop for OwnedString {
        fn drop(&mut self) {
            unsafe { CFRelease(self.0) };
        }
    }

    /// Copies a `CFString` (borrowed or owned) into a Rust string.
    fn string_from_cf(string: CFStringRef) -> Option<String> {
        if string.is_null() {
            return None;
        }

        let length = unsafe { CFStringGetLength(string) };
        // The conversion can need more bytes than there are characters, so ask
        // for the worst case (plus the terminator both calls use).
        let size = unsafe { CFStringGetMaximumSizeForEncoding(length, UTF8) } + 1;
        let mut buffer = vec![0u8; usize::try_from(size).ok()?];

        if unsafe { CFStringGetCString(string, buffer.as_mut_ptr().cast(), size, UTF8) } == 0 {
            return None;
        }

        let end = buffer
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(buffer.len());

        String::from_utf8(buffer[..end].to_vec()).ok()
    }

    /// The bundle identifier this application is registered under, if it runs
    /// from a bundle.
    fn our_handler() -> Option<String> {
        let bundle = unsafe { CFBundleGetMainBundle() };

        if bundle.is_null() {
            return None;
        }

        string_from_cf(unsafe { CFBundleGetIdentifier(bundle) })
    }

    /// The bundle identifier LaunchServices opens `softwarefix://` links with.
    fn default_handler() -> Option<String> {
        let scheme = OwnedString::new(SCHEME)?;
        let handler = unsafe { LSCopyDefaultHandlerForURLScheme(scheme.0) };

        if handler.is_null() {
            return None;
        }

        // Returned owned, so it is released once the copy has been made.
        let handler = OwnedString(handler);

        string_from_cf(handler.0)
    }

    /// Points the scheme at a bundle identifier.
    fn set_handler(identifier: &str) -> Result<(), String> {
        let scheme = OwnedString::new(SCHEME)
            .ok_or_else(|| "could not create the scheme string".to_string())?;
        let handler = OwnedString::new(identifier)
            .ok_or_else(|| "could not create the handler string".to_string())?;
        let status = unsafe { LSSetDefaultHandlerForURLScheme(scheme.0, handler.0) };

        if status == 0 {
            Ok(())
        } else {
            Err(format!("LaunchServices error {status}"))
        }
    }

    pub(super) fn current() -> Handler {
        match default_handler() {
            // The registration names a bundle identifier rather than a path, so
            // an application that was moved is still recognized as ours.
            Some(identifier) if Some(&identifier) == our_handler().as_ref() => Handler::Ours,
            Some(identifier) => Handler::Other(identifier),
            None => Handler::None,
        }
    }

    pub(super) fn register() -> Result<(), String> {
        let identifier = our_handler()
            .ok_or_else(|| "no bundle identifier: run LMNFlash.app".to_string())?;

        // Remember the handler we are about to replace. Recorded once, like the
        // other registrations, so re-registering cannot overwrite the backup
        // with our own identifier.
        if crate::config::load_protocol_backup().is_none() {
            let previous = default_handler().filter(|handler| handler != &identifier);
            crate::config::save_protocol_backup(previous.as_deref())?;
        }

        set_handler(&identifier)
    }

    pub(super) fn restore() -> Result<(), String> {
        match crate::config::load_protocol_backup() {
            Some(previous) => set_handler(&previous)?,
            // LaunchServices can only point a scheme at another handler, never
            // back at nothing: with no handler remembered there is nothing to
            // restore, and our own registration has to stay.
            None if current() == Handler::Ours => {
                return Err("macOS cannot unregister a URL handler; pick another program \
                            for softwarefix:// links in System Settings instead"
                    .to_string());
            }
            None => {}
        }

        crate::config::clear_protocol_backup()
    }

    /// Where the Apple Event handler pushes the URLs it receives.
    ///
    /// Filled in by [`listen_for_callbacks`] before the event loop starts; a
    /// callback arriving before that has nowhere to go and is dropped.
    static CALLBACKS: Mutex<Option<UnboundedSender<String>>> = Mutex::new(None);

    // The `NSAppleEventManager` target for `kAEGetURL`, the event macOS sends
    // when a `softwarefix://` link is opened: whatever the browser (or Finder)
    // hands to the scheme ends up in `handleGetURLEvent:`.
    //
    // The descriptors are only talked to through `msg_send!` because objc2
    // binds `NSAppleEventManager` and its descriptor type behind the
    // `objc2-core-services` feature, which would pull in a second C-API crate
    // for these three calls.
    define_class!(
        // SAFETY: `NSObject` has no subclassing requirements, and the class
        // declares no instance variables of its own.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "LmnFlashUrlHandler"]
        struct UrlHandler;

        // SAFETY: `NSObjectProtocol` has no requirements.
        unsafe impl NSObjectProtocol for UrlHandler {}

        impl UrlHandler {
            // The selector registered below, called on the main thread.
            #[unsafe(method(handleGetURLEvent:withReplyEvent:))]
            fn handle_get_url_event(&self, event: &AnyObject, _reply: &AnyObject) {
                // SAFETY: the Apple Event Manager always passes the event as an
                // `NSAppleEventDescriptor`, and both calls are documented to
                // return nil (checked here) when there is nothing to return.
                let descriptor: *mut AnyObject =
                    unsafe { msg_send![event, paramDescriptorForKeyword: KEY_DIRECT_OBJECT] };

                if descriptor.is_null() {
                    return;
                }

                let Some(url) = (unsafe { url_from_descriptor(&*descriptor) }) else {
                    return;
                };

                let Ok(mut callbacks) = CALLBACKS.lock() else {
                    return;
                };

                let Some(sender) = callbacks.as_mut() else {
                    return;
                };

                let _ = sender.unbounded_send(url);
            }
        }
    );

    /// The URL an `NSAppleEventDescriptor` carries.
    ///
    /// # Safety
    ///
    /// `descriptor` must point to a live `NSAppleEventDescriptor`.
    unsafe fn url_from_descriptor(descriptor: &AnyObject) -> Option<String> {
        // Both objects are autoreleased, so they outlive this call; the copy
        // into a `String` has to happen before the event is disposed of, which
        // is why nothing borrowed from them is returned.
        let string: *mut AnyObject = unsafe { msg_send![descriptor, stringValue] };

        if string.is_null() {
            return None;
        }

        let utf8: *const c_char = unsafe { msg_send![&*string, UTF8String] };

        if utf8.is_null() {
            return None;
        }

        Some(
            unsafe { std::ffi::CStr::from_ptr(utf8) }
                .to_string_lossy()
                .into_owned(),
        )
    }

    impl UrlHandler {
        /// Creates the handler, or `None` when `init` did not return an object.
        fn new(mtm: MainThreadMarker) -> Option<Retained<Self>> {
            let this = Self::alloc(mtm).set_ivars(());

            // SAFETY: `NSObject`'s `init` on a freshly allocated instance of a
            // subclass that has no instance variables.
            unsafe { msg_send![super(this), init] }
        }
    }

    /// Installs the `kAEGetURL` handler and returns the receiver the URLs it
    /// brings arrive on, or `None` when the events cannot be received at all.
    ///
    /// The handler goes to `NSAppleEventManager` rather than into an
    /// `NSApplicationDelegate` of our own: winit (and with it iced) owns that
    /// delegate, so replacing it would break the application.
    pub(super) fn listen_for_callbacks() -> Option<UnboundedReceiver<String>> {
        // Apple Events are delivered on the main thread, which is where the
        // application starts up and where this is called from.
        let Some(mtm) = MainThreadMarker::new() else {
            eprintln!("[protocol] not on the main thread: softwarefix:// callbacks are dropped");

            return None;
        };

        let Some(handler) = UrlHandler::new(mtm) else {
            eprintln!("[protocol] could not create the URL event handler");

            return None;
        };

        // SAFETY: `NSAppleEventManager` is a Foundation class that every GUI
        // application has, the selector above is implemented, and the handler
        // object is leaked below so that it outlives the registration.
        unsafe {
            let Some(class) = AnyClass::get(c"NSAppleEventManager") else {
                eprintln!("[protocol] NSAppleEventManager is missing");

                return None;
            };

            let manager: *mut AnyObject = msg_send![class, sharedAppleEventManager];

            if manager.is_null() {
                eprintln!("[protocol] there is no shared Apple Event manager");

                return None;
            }

            let _: () = msg_send![
                &*manager,
                setEventHandler: &*handler,
                andSelector: sel!(handleGetURLEvent:withReplyEvent:),
                forEventClass: AE_GET_URL,
                andEventID: AE_GET_URL
            ];
        }

        let (sender, receiver) = iced::futures::channel::mpsc::unbounded();

        match CALLBACKS.lock() {
            Ok(mut callbacks) => *callbacks = Some(sender),
            Err(_) => return None,
        }

        // The Apple Event Manager is documented not to retain the handler, and
        // the object is main-thread only (so it cannot live in a static): leak
        // it instead of registering a second handler later.
        std::mem::forget(handler);

        Some(receiver)
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod platform {
    use super::Handler;

    pub(super) fn current() -> Handler {
        Handler::Unsupported
    }

    pub(super) fn register() -> Result<(), String> {
        Err("softwarefix:// links cannot be registered on this platform".to_string())
    }

    pub(super) fn restore() -> Result<(), String> {
        Err("softwarefix:// links cannot be registered on this platform".to_string())
    }
}

/// Which program currently handles `softwarefix://` links.
pub fn current() -> Handler {
    platform::current()
}

/// Makes LMN Flash the handler for `softwarefix://` links.
pub fn register() -> Result<(), String> {
    platform::register()
}

/// Restores the handler that was in place before LMN Flash took over (or
/// removes LMN Flash's registration when there was none).
///
/// macOS cannot remove a registration, only point it somewhere else, so there
/// the second case fails instead (see [`platform`]).
pub fn restore() -> Result<(), String> {
    platform::restore()
}

/// Receives the `softwarefix://` callbacks that are delivered to **this**
/// process, for the platforms where the operating system does not start a
/// second copy of the application for them.
///
/// macOS delivers the clicked URL as an Apple Event to the running application
/// (see [`platform`]), so the receiver returned here yields exactly the URLs
/// that [`crate::instance`] forwards on Windows and Linux. `None` means the
/// callbacks cannot arrive at all, which is the case everywhere else — and on
/// macOS on the platforms and in the situations where the event cannot be
/// handled.
pub fn listen_for_callbacks() -> Option<UnboundedReceiver<String>> {
    #[cfg(target_os = "macos")]
    {
        platform::listen_for_callbacks()
    }

    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `HKEY_CLASSES_ROOT` is the merged `Software\Classes`, so the command
    /// lookup must NOT repeat the `Software\Classes` prefix.
    #[cfg(windows)]
    #[test]
    fn the_class_root_lookup_is_scheme_relative() {
        assert_eq!(
            platform::scheme_command_path(),
            r"softwarefix\shell\open\command"
        );
        assert_eq!(
            platform::command_path(),
            r"Software\Classes\softwarefix\shell\open\command"
        );
    }

    /// A per-user class really is visible through `HKEY_CLASSES_ROOT` when it
    /// is read with the scheme-relative path (the precedence the whole toggle
    /// relies on). Uses a throwaway class name, removed again here.
    #[cfg(windows)]
    #[test]
    fn a_per_user_class_is_visible_through_the_class_root() {
        use winreg::RegKey;
        use winreg::enums::{HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};

        let name = format!("lmnflash-test-{}", std::process::id());
        let user = RegKey::predef(HKEY_CURRENT_USER);
        let command = r#""C:\throwaway\lmnflash.exe" "%1""#.to_string();

        {
            let (key, _) = user
                .create_subkey_with_flags(
                    format!(r"Software\Classes\{name}\shell\open\command"),
                    KEY_WRITE,
                )
                .expect("could not create the throwaway class");

            key.set_value("", &command)
                .expect("could not write the throwaway command");
        }

        let read = RegKey::predef(HKEY_CLASSES_ROOT)
            .open_subkey_with_flags(format!(r"{name}\shell\open\command"), KEY_READ)
            .and_then(|key| key.get_value::<String, _>(""))
            .ok();

        let _ = user.delete_subkey_all(format!(r"Software\Classes\{name}"));

        assert_eq!(read.as_deref(), Some(command.as_str()));
    }

    #[test]
    fn extracts_the_program_from_a_command_line() {
        assert_eq!(
            command_program(r#""C:\Program Files\Motorola\Software Fix\SoftwareFix.exe" "%1""#),
            r"C:\Program Files\Motorola\Software Fix\SoftwareFix.exe"
        );
        assert_eq!(
            command_program(r"C:\tools\app.exe %1"),
            r"C:\tools\app.exe"
        );
        assert_eq!(command_program(""), "");
    }

    #[test]
    fn handler_reports_the_foreign_program() {
        let handler = Handler::Other(r#""C:\Tools\SoftwareFix.exe" "%1""#.to_string());

        assert_eq!(handler.program().as_deref(), Some(r"C:\Tools\SoftwareFix.exe"));
        assert_eq!(Handler::Ours.program(), None);
    }

    #[test]
    fn builds_a_desktop_entry_for_the_scheme() {
        let entry = desktop_entry(std::path::Path::new("/home/u/LMN Flash/lmnflash"));

        assert!(entry.starts_with("[Desktop Entry]\n"));
        assert!(entry.contains("MimeType=x-scheme-handler/softwarefix;"));
        assert!(entry.contains("Exec=\"/home/u/LMN Flash/lmnflash\" %u"));
    }

    #[test]
    fn escapes_the_desktop_entry_program_path() {
        assert_eq!(
            exec_path(std::path::Path::new(r#"/opt/a$b/c\d/e"f"#)),
            r#""/opt/a\$b/c\\d/e\"f""#
        );
        assert_eq!(
            exec_path(std::path::Path::new("/plain/lmnflash")),
            r#""/plain/lmnflash""#
        );
    }

    #[test]
    fn reads_the_desktop_entry_program() {
        let entry = desktop_entry(std::path::Path::new("/opt/LMN Flash/lmnflash"));
        let exec = entry_exec(&entry).expect("no Exec line");

        assert!(exec.trim_start().starts_with(&exec_path(std::path::Path::new(
            "/opt/LMN Flash/lmnflash"
        ))));
        assert!(!exec.trim_start().starts_with(&exec_path(std::path::Path::new(
            "/moved/lmnflash"
        ))));
        assert_eq!(entry_exec("[Desktop Entry]\nType=Application\n"), None);
    }

    /// A registration that names a moved copy of the executable is still ours,
    /// and must not be mistaken for another program's handler.
    #[cfg(windows)]
    #[test]
    fn a_moved_registration_is_still_ours() {
        let exe = std::env::current_exe().expect("no current executable");
        let name = exe.file_name().expect("no file name");

        assert!(platform::is_ours_or_moved(&format!(
            "\"{}\" \"%1\"",
            exe.display()
        )));

        let moved = std::path::Path::new(r"C:\lmnflash-was-here").join(name);
        assert!(platform::is_ours_or_moved(&format!(
            "\"{}\" \"%1\"",
            moved.display()
        )));

        // Another program, and an existing file with our name elsewhere, are
        // not ours.
        assert!(!platform::is_ours_or_moved(
            r#""C:\Program Files\Software Fix\Software Fix.exe" "%1""#
        ));

        let directory = std::env::temp_dir().join("lmnflash-move-test");
        std::fs::create_dir_all(&directory).expect("could not create the temporary directory");

        let elsewhere = directory.join(name);
        std::fs::write(&elsewhere, b"").expect("could not create the temporary file");

        assert!(!platform::is_ours_or_moved(&format!(
            "\"{}\" \"%1\"",
            elsewhere.display()
        )));
    }
}
