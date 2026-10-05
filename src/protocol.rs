//! `softwarefix://` URL protocol handler registration.
//!
//! The official Motorola/Lenovo "Software Fix" registers the `softwarefix://`
//! scheme so the browser can hand its OAuth callback back to it. LMN Flash
//! uses the same callback (see [`crate::login::extract_login`]), so it can
//! take the scheme over — but on Windows only when the user asks for it: at
//! startup the scheme is registered **only when nothing else handles it**, and
//! the Manual Login page offers a toggle otherwise.
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
//! * **macOS** — not registered. LaunchServices would open the application,
//!   but the URL arrives as an `kAEGetURL` Apple Event that iced exposes no
//!   API for (its `Event` enum has no URL variant), so the callback would be
//!   silently dropped; the manual paste flow is the fallback there.

/// The URL scheme handled by the official Software Fix program.
const SCHEME: &str = "softwarefix";

/// Which program currently handles `softwarefix://` links.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Handler {
    /// The platform has no runtime URL protocol handling (macOS).
    Unsupported,
    /// Nothing is registered for the scheme.
    None,
    /// LMN Flash itself handles the scheme.
    Ours,
    /// Another program handles the scheme; carries its command line (Windows)
    /// or desktop entry id (Linux).
    Other(String),
}

impl Handler {
    /// The program a "foreign" handler command line names, for display.
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

#[cfg(not(any(windows, target_os = "linux")))]
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
pub fn restore() -> Result<(), String> {
    platform::restore()
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
