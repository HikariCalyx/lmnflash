//! Detecting the user's default web browser.
//!
//! When the browser sign-in answers HTTP 400 the login page tells the user to
//! clear the `lenovo.com` cookies, and *how* to do that differs per browser.
//! The steps are only written for Chrome, Edge and Firefox; every other (or
//! undetected) browser gets a generic hint.
//!
//! Detection is best effort and never an error: anything unrecognized is
//! [`Browser::Other`]. The answer is cached, because the view calls it while
//! drawing and macOS/Linux detection spawns a helper process.

use std::sync::OnceLock;

/// The browsers the login page knows cookie-clearing steps for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Browser {
    Chrome,
    Edge,
    Firefox,
    /// Any other browser, or one that could not be detected.
    Other,
}

impl Browser {
    /// FTL id of the cookie-clearing steps for this browser.
    pub(crate) fn cookie_hint_id(self) -> &'static str {
        match self {
            Self::Chrome => "login-browser-cookie-chrome",
            Self::Edge => "login-browser-cookie-edge",
            Self::Firefox => "login-browser-cookie-firefox",
            Self::Other => "login-browser-cookie-other",
        }
    }
}

/// The default web browser.
///
/// [`warm`] is called once at startup so the first frame that needs this never
/// blocks on a process spawn.
pub(crate) fn default_browser() -> Browser {
    *DEFAULT.get_or_init(detect)
}

/// Warms the cache at startup (see [`default_browser`]).
pub(crate) fn warm() {
    let _ = default_browser();
}

static DEFAULT: OnceLock<Browser> = OnceLock::new();

fn detect() -> Browser {
    let identifier = if cfg!(target_os = "windows") {
        windows_prog_id()
    } else if cfg!(target_os = "macos") {
        macos_handler()
    } else if cfg!(target_os = "linux") {
        linux_desktop_entry()
    } else {
        None
    };

    identifier.map_or(Browser::Other, |id| classify(&id))
}

/// Maps a platform identifier to a known browser.
///
/// All three platforms name the engine somewhere in the identifier
/// (`ChromeHTML`, `com.google.chrome`, `google-chrome.desktop`), so one
/// case-insensitive search covers them.
fn classify(identifier: &str) -> Browser {
    let identifier = identifier.to_ascii_lowercase();

    if identifier.contains("firefox") {
        // ESR is the same browser on a slower release channel, and registers
        // as `firefox-esr.desktop`.
        Browser::Firefox
    } else if identifier.contains("edge") {
        // `MSEdgeHTM` / `microsoft-edge.desktop`; checked before the Chromium
        // family, whose identifiers never name Edge anyway.
        Browser::Edge
    } else if identifier.contains("chrome") || identifier.contains("chromium") {
        // Chromium is the same engine with the same settings page
        // (`chrome://settings/content/all`), so it shares Chrome's steps. Its
        // Linux ids are `chromium.desktop` and Ubuntu's snap
        // `chromium_chromium.desktop` — neither contains "chrome".
        Browser::Chrome
    } else {
        Browser::Other
    }
}

/// The `ProgId` Windows associates with web links
/// (`HKCU\...\UrlAssociations\<scheme>\UserChoice`), such as `ChromeHTML`,
/// `MSEdgeHTM` or `FirefoxURL-308046B0AF4A39CB`.
///
/// `https` is tried first: that is the association Windows 11 keeps the
/// default browser in.
#[cfg(target_os = "windows")]
fn windows_prog_id() -> Option<String> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    let root = RegKey::predef(HKEY_CURRENT_USER);

    ["https", "http"].into_iter().find_map(|scheme| {
        root.open_subkey(format!(
            r"SOFTWARE\Microsoft\Windows\Shell\Associations\UrlAssociations\{scheme}\UserChoice"
        ))
        .ok()?
        .get_value("ProgId")
        .ok()
    })
}

#[cfg(not(target_os = "windows"))]
fn windows_prog_id() -> Option<String> {
    None
}

/// The handler LaunchServices recorded for web links — a bundle id such as
/// `com.google.chrome`.
///
/// The per-user `com.apple.launchservices.secure` plist is converted to JSON
/// with `plutil` (a system tool): it is a binary plist, and JSON keeps the
/// nesting explicit, so a handler cannot be read out of the wrong entry.
#[cfg(target_os = "macos")]
fn macos_handler() -> Option<String> {
    let home = std::env::var_os("HOME")?;
    let plist = std::path::Path::new(&home).join(
        "Library/Preferences/com.apple.LaunchServices/com.apple.launchservices.secure.plist",
    );

    let output = std::process::Command::new("plutil")
        .args(["-convert", "json", "-o", "-"])
        .arg(&plist)
        .output()
        .ok()?;

    find_web_handler(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(not(target_os = "macos"))]
fn macos_handler() -> Option<String> {
    None
}

/// The `LSHandlerRoleAll` of the entry the LaunchServices plist keeps for web
/// links, read from it as JSON (see [`macos_handler`]).
///
/// The role is only taken from an entry that names `http`/`https` as its
/// scheme, and only from the entry's own top level: every entry also carries a
/// nested `LSHandlerPreferredVersions` whose `LSHandlerRoleAll` is a dash.
///
/// Kept platform-independent so it is unit-tested everywhere.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn find_web_handler(json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let handlers = value.get("LSHandlers")?.as_array()?;

    for handler in handlers {
        let scheme = handler.get("LSHandlerURLScheme").and_then(|id| id.as_str());
        if !matches!(scheme, Some("http") | Some("https")) {
            continue;
        }

        let role = handler.get("LSHandlerRoleAll").and_then(|id| id.as_str());
        if let Some(role) = role.filter(|role| !role.is_empty()) {
            return Some(role.to_owned());
        }
    }

    None
}

/// The `.desktop` file Linux uses for web links, such as
/// `google-chrome.desktop`.
///
/// `xdg-settings` knows the user's actual choice; `xdg-mime` is the fallback
/// when it is not installed.
#[cfg(target_os = "linux")]
fn linux_desktop_entry() -> Option<String> {
    run("xdg-settings", &["get", "default-web-browser"])
        .or_else(|| run("xdg-mime", &["query", "default", "x-scheme-handler/https"]))
}

#[cfg(not(target_os = "linux"))]
fn linux_desktop_entry() -> Option<String> {
    None
}

/// Runs a helper and returns its non-empty, trimmed stdout.
#[cfg(target_os = "linux")]
fn run(program: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();

    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The identifiers every platform can hand us, and the browser each one
    /// means: a Windows `ProgId`, a macOS bundle id, a Linux `.desktop` name —
    /// including a browser's variations, which are the same browser for these
    /// steps.
    #[test]
    fn platform_identifiers_name_their_browser() {
        for chrome in [
            "ChromeHTML",
            "com.google.chrome",
            "google-chrome.desktop",
            // Chromium has Chrome's settings page.
            "chromium.desktop",
            "chromium-browser.desktop",
            "chromium_chromium.desktop",
        ] {
            assert_eq!(classify(chrome), Browser::Chrome, "{chrome}");
        }

        for edge in [
            "MSEdgeHTM",
            "com.microsoft.edgemac",
            "microsoft-edge.desktop",
        ] {
            assert_eq!(classify(edge), Browser::Edge, "{edge}");
        }

        for firefox in [
            "FirefoxURL-308046B0AF4A39CB",
            "org.mozilla.firefox",
            "firefox.desktop",
            // ESR is the same browser on a slower release channel.
            "firefox-esr.desktop",
        ] {
            assert_eq!(classify(firefox), Browser::Firefox, "{firefox}");
        }

        // A Chromium-based browser is only recognized by NAME: Brave and
        // Vivaldi have settings pages of their own, and a Chromodo-style name
        // must not be read as Chrome. Anything unknown stays generic rather
        // than guessing.
        for other in [
            "brave-browser.desktop",
            "vivaldi.desktop",
            "chromodo.desktop",
            "epiphany.desktop",
            "Safari",
            "",
        ] {
            assert_eq!(classify(other), Browser::Other, "{other}");
        }
    }

    /// Each browser gets its own hint, and the id is one the FTL files define.
    #[test]
    fn every_browser_has_its_own_hint() {
        let ids = [
            Browser::Chrome.cookie_hint_id(),
            Browser::Edge.cookie_hint_id(),
            Browser::Firefox.cookie_hint_id(),
            Browser::Other.cookie_hint_id(),
        ];

        for (index, id) in ids.iter().enumerate() {
            assert!(id.starts_with("login-browser-cookie-"), "{id}");
            assert!(!ids[..index].contains(id), "duplicate hint id: {id}");
        }
    }

    /// The macOS plist is read as JSON: the browser is the role of the entry
    /// whose scheme is `http`/`https`, never the nested
    /// `LSHandlerPreferredVersions` role of some other entry.
    #[test]
    fn the_launch_services_web_handler_is_found() {
        let json = r#"{
            "LSHandlers": [
                {
                    "LSHandlerContentType": "public.html",
                    "LSHandlerPreferredVersions": { "LSHandlerRoleAll": "-" },
                    "LSHandlerRoleAll": "com.apple.Safari"
                },
                {
                    "LSHandlerURLScheme": "mailto",
                    "LSHandlerPreferredVersions": { "LSHandlerRoleAll": "-" },
                    "LSHandlerRoleAll": "com.apple.mail"
                },
                {
                    "LSHandlerURLScheme": "http",
                    "LSHandlerPreferredVersions": { "LSHandlerRoleAll": "-" },
                    "LSHandlerRoleAll": "org.mozilla.firefox"
                }
            ]
        }"#;

        assert_eq!(
            find_web_handler(json).as_deref(),
            Some("org.mozilla.firefox")
        );

        // `https` alone is enough, and a plist without a web entry has no
        // browser in it (the OS default applies then).
        assert_eq!(
            find_web_handler(r#"{"LSHandlers":[{"LSHandlerURLScheme":"https","LSHandlerRoleAll":"com.google.chrome"}]}"#)
                .as_deref(),
            Some("com.google.chrome")
        );
        assert_eq!(find_web_handler(r#"{"LSHandlers":[]}"#), None);
        assert_eq!(find_web_handler("not a plist"), None);
    }
}
