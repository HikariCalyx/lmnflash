//! The "Fix Captive Portal Server" feature (Mode 2): pointing a phone's
//! captive-portal check and NTP server at servers that answer.
//!
//! A phone reports *"Connected, no Internet"* when it cannot reach the server
//! it uses to check that the connection works. On some networks that server is
//! unreachable while the Internet itself is fine, so the check (and the clock
//! synchronization) is pointed somewhere reachable instead. Which setting
//! spells that out depends on the Android version:
//!
//! ```text
//! # Android 7.1 and newer (API 25+)
//! settings put global captive_portal_http_url  http://connect.rom.miui.com/generate_204
//! settings put global captive_portal_https_url https://connect.rom.miui.com/generate_204
//!
//! # Android 6.0 – 7.0 (API 23/24): one server, no URLs
//! settings put global captive_portal_server connect.rom.miui.com
//!
//! # Android 5.0 – 5.1 (API 21/22): detection is turned off instead
//! settings put global captive_portal_mode 0
//!
//! # every supported version
//! settings put global ntp_server time.asia.apple.com
//! ```
//!
//! Android 5.0 is the oldest version with any of these settings. On Android 10
//! (API 29) and newer the connection is also cycled afterwards — airplane mode
//! on, a short wait, off again — because the connectivity service only
//! re-reads its settings when the network comes up; on Android 9 and older the
//! user has to do that by hand ([`needs_manual_airplane_mode`] says which it
//! is).
//!
//! The whole sequence runs on the *device*, detached from the `adb shell` that
//! started it: the airplane-mode cycle drops the very connection the command
//! came in on, so a command that stayed attached to it would be killed halfway
//! through. The computer therefore starts the detached job and then drops its
//! own wireless connection immediately instead of waiting for it to time out.
//!
//! The dialog only *presents* this: the flow state and its update handlers
//! live in `crate` (`State::flash.captive_portal`), and the card in
//! [`crate::captive_portal_dialog`].

use crate::debloat;
use crate::platform_tools;
use crate::wireless::{self, DisconnectTarget};

/// The oldest Android the fix supports: 5.0 (API 21), which is where captive
/// portal settings exist for the first time.
pub(crate) const MIN_SDK: u32 = 21;

/// Android 6.0 (API 23), the first version with the single
/// `captive_portal_server` setting; Android 5.x only has detection on or off.
const LEGACY_SERVER_SDK: u32 = 23;

/// Android 7.1 (API 25), where the single server setting was replaced by the
/// `captive_portal_http_url`/`_https_url` pair.
const URL_SETTINGS_SDK: u32 = 25;

/// The API level the airplane-mode cycle was added for: Android 10. The
/// connectivity service caches its settings, so they are only re-read when the
/// network is brought up again — which is something the user has to do by hand
/// on everything older.
pub(crate) const AIRPLANE_MODE_SDK: u32 = 29;

/// How long the device stays in airplane mode before it is switched back off.
const AIRPLANE_MODE_SECONDS: u32 = 5;

/// Which captive-portal setting a device understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Settings {
    /// Android 5.0 – 5.1: there is no server to point anywhere, so the check
    /// is turned off (`captive_portal_mode 0`) and the phone stops asking.
    Disabled,
    /// Android 6.0 – 7.0: one check server, named by `captive_portal_server`
    /// (a bare host, without scheme or path).
    LegacyServer,
    /// Android 7.1 and newer: the HTTP and HTTPS check URLs.
    Urls,
}

impl Settings {
    /// The setting `sdk` understands, or `None` for an Android older than
    /// [`MIN_SDK`].
    pub(crate) fn for_sdk(sdk: u32) -> Option<Self> {
        if sdk < MIN_SDK {
            return None;
        }

        if sdk < LEGACY_SERVER_SDK {
            return Some(Self::Disabled);
        }

        if sdk < URL_SETTINGS_SDK {
            return Some(Self::LegacyServer);
        }

        Some(Self::Urls)
    }
}

/// Whether the user has to cycle airplane mode by hand for the new settings to
/// take effect.
///
/// Android 10 and newer do it themselves (see [`device_command`]); on Android
/// 9 and older nothing brings the network back up for them, so the dialog says
/// so.
pub(crate) fn needs_manual_airplane_mode(sdk: u32) -> bool {
    sdk < AIRPLANE_MODE_SDK
}

/// Why the fix could not be applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FixError {
    /// The device runs an Android older than [`MIN_SDK`], which has none of
    /// the captive-portal settings.
    Unsupported,
    /// Anything else, with what the tools said.
    Failed(String),
}

/// What a completed fix did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FixReport {
    /// The Android API level the device reported.
    pub sdk: u32,
    /// The wireless device whose connection was dropped after the job was
    /// started, when the device was reached over the network.
    pub disconnected: Option<String>,
}

impl FixReport {
    /// Whether the dialog has to tell the user to cycle airplane mode by hand.
    pub(crate) fn needs_manual_airplane_mode(&self) -> bool {
        needs_manual_airplane_mode(self.sdk)
    }
}

/// The Android API level `serial` runs.
pub(crate) fn android_sdk(serial: &str) -> Result<u32, FixError> {
    let output = platform_tools::run_adb(&[
        "-s",
        serial,
        "shell",
        "getprop",
        "ro.build.version.sdk",
    ])
    .map_err(FixError::Failed)?;

    let value = output.trim();

    value.parse().map_err(|_| {
        FixError::Failed(format!(
            "the device did not report its Android version (`{value}`)"
        ))
    })
}

/// The command the *device* runs, detached from the `adb shell` that starts
/// it.
///
/// It is one line for the device's `sh`: a `nohup` subshell with every step in
/// it, backgrounded and disowned so it survives the shell (and the USB/Wi-Fi
/// connection) going away, with all three of its streams redirected so the
/// `adb shell` returns as soon as the job is started.
///
/// The steps are separated by `;` and carry no quotes of their own, so the
/// whole thing can be given to `adb shell` as a single argument.
///
/// `sdk` decides how the check is pointed at the server (see [`Settings`]) and
/// whether the airplane-mode cycle is added.
pub(crate) fn device_command(sdk: u32) -> String {
    let mut steps: Vec<String> = Vec::new();

    match Settings::for_sdk(sdk) {
        // Without the check URL settings there is no server to point anywhere,
        // so the check is turned off instead of being misdirected.
        Some(Settings::Disabled) => {
            steps.push("settings put global captive_portal_mode 0".to_owned());
        }
        Some(Settings::LegacyServer) => {
            steps.push(
                "settings put global captive_portal_server connect.rom.miui.com".to_owned(),
            );
        }
        _ => {
            steps.push(
                "settings put global captive_portal_http_url http://connect.rom.miui.com/generate_204"
                    .to_owned(),
            );
            steps.push(
                "settings put global captive_portal_https_url https://connect.rom.miui.com/generate_204"
                    .to_owned(),
            );
        }
    }

    // The clock is not part of the check, so it is pointed at a reachable
    // server on every version.
    steps.push("settings put global ntp_server time.asia.apple.com".to_owned());

    if sdk >= AIRPLANE_MODE_SDK {
        // The cycle is what makes the phone pick the new settings up: the
        // connectivity service reads them when the network comes back. On
        // everything older the user does it by hand (see
        // [`needs_manual_airplane_mode`]).
        steps.push("cmd connectivity airplane-mode enable".to_owned());
        steps.push(format!("sleep {AIRPLANE_MODE_SECONDS}"));
        steps.push("cmd connectivity airplane-mode disable".to_owned());
    }

    let body = steps.join("; ");

    // `disown` is a shell builtin that not every device's `sh` has; a shell
    // without it must not turn a started job into a reported failure.
    format!(
        "nohup sh -c '{body}' > /dev/null 2>&1 < /dev/null & disown 2>/dev/null || true"
    )
}

/// Applies the fix to `serial`.
///
/// The steps run detached on the device, so this returns while they are still
/// going on. When the device is reached over the network its connection is
/// dropped right after the job is started: the airplane-mode cycle is about to
/// take the network away anyway, and a connection that waits for that would
/// only time out.
pub(crate) fn fix(serial: &str) -> Result<FixReport, FixError> {
    debloat::ensure_adb().map_err(FixError::Failed)?;

    let sdk = android_sdk(serial)?;

    if Settings::for_sdk(sdk).is_none() {
        return Err(FixError::Unsupported);
    }

    platform_tools::run_adb(&["-s", serial, "shell", &device_command(sdk)])
        .map_err(FixError::Failed)?;

    let disconnected = wireless::is_wireless(serial)
        .then(|| wireless::disconnect(&DisconnectTarget::Device(serial.to_owned())))
        .and_then(Result::ok)
        .map(|()| serial.to_owned());

    Ok(FixReport { sdk, disconnected })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_are_written_on_android_7_1() {
        let command = device_command(25);

        assert!(
            command.contains(
                "settings put global captive_portal_http_url http://connect.rom.miui.com/generate_204"
            ),
            "{command}"
        );
        assert!(
            command.contains(
                "settings put global captive_portal_https_url https://connect.rom.miui.com/generate_204"
            ),
            "{command}"
        );
        assert!(
            command.contains("settings put global ntp_server time.asia.apple.com"),
            "{command}"
        );

        // The older spellings are gone from Android 7.1 on.
        assert!(!command.contains("captive_portal_server"), "{command}");
        assert!(!command.contains("captive_portal_mode"), "{command}");

        // The airplane-mode cycle belongs to Android 10 and newer.
        assert!(!command.contains("airplane-mode"), "{command}");
    }

    #[test]
    fn android_6_and_7_use_the_single_server_setting() {
        for sdk in [23, 24] {
            let command = device_command(sdk);

            assert!(
                command.contains(
                    "settings put global captive_portal_server connect.rom.miui.com"
                ),
                "{command}"
            );
            assert!(
                command.contains("settings put global ntp_server time.asia.apple.com"),
                "{command}"
            );

            // The URL settings and the off switch do not exist there.
            assert!(!command.contains("captive_portal_http_url"), "{command}");
            assert!(!command.contains("captive_portal_mode"), "{command}");
        }
    }

    #[test]
    fn android_5_turns_the_check_off() {
        for sdk in [21, 22] {
            let command = device_command(sdk);

            assert!(
                command.contains("settings put global captive_portal_mode 0"),
                "{command}"
            );
            assert!(
                command.contains("settings put global ntp_server time.asia.apple.com"),
                "{command}"
            );

            // There is no server to point anywhere on this version.
            assert!(!command.contains("captive_portal_server"), "{command}");
            assert!(!command.contains("captive_portal_http_url"), "{command}");
        }
    }

    #[test]
    fn older_than_android_5_is_not_supported() {
        assert_eq!(Settings::for_sdk(20), None);
        assert_eq!(Settings::for_sdk(19), None);

        // And every supported version has a setting of its own.
        assert_eq!(Settings::for_sdk(21), Some(Settings::Disabled));
        assert_eq!(Settings::for_sdk(22), Some(Settings::Disabled));
        assert_eq!(Settings::for_sdk(23), Some(Settings::LegacyServer));
        assert_eq!(Settings::for_sdk(24), Some(Settings::LegacyServer));
        assert_eq!(Settings::for_sdk(25), Some(Settings::Urls));
        assert_eq!(Settings::for_sdk(34), Some(Settings::Urls));
    }

    #[test]
    fn android_10_cycles_airplane_mode() {
        let command = device_command(29);

        let enable = command
            .find("cmd connectivity airplane-mode enable")
            .expect("airplane mode is enabled");
        let wait = command.find("sleep 5").expect("the wait is there");
        let disable = command
            .find("cmd connectivity airplane-mode disable")
            .expect("airplane mode is disabled again");

        // In that order, and after the settings.
        assert!(enable < wait && wait < disable, "{command}");
        assert!(
            command
                .find("settings put global ntp_server")
                .expect("the settings come first")
                < enable,
            "{command}"
        );
    }

    #[test]
    fn older_devices_need_airplane_mode_by_hand() {
        // Android 9 is the last version that does not cycle it itself.
        for sdk in [21, 24, 25, 28] {
            assert!(needs_manual_airplane_mode(sdk), "{sdk}");
            assert!(!device_command(sdk).contains("airplane-mode"), "{sdk}");
        }

        for sdk in [29, 34] {
            assert!(!needs_manual_airplane_mode(sdk), "{sdk}");
            assert!(device_command(sdk).contains("airplane-mode"), "{sdk}");
        }
    }

    #[test]
    fn the_job_is_detached_from_the_shell_that_starts_it() {
        let command = device_command(34);

        // Backgrounded, so `adb shell` returns at once; the connection it came
        // in on is about to go away.
        assert!(command.starts_with("nohup sh -c '"), "{command}");
        assert!(command.contains("' > /dev/null 2>&1 < /dev/null &"), "{command}");
        assert!(command.trim_end().ends_with("disown 2>/dev/null || true"), "{command}");

        // One argument, so the steps have to be joined for the device's `sh`;
        // the only quotes are the pair around the subshell.
        assert!(!command.contains('\n'), "{command}");
        assert_eq!(command.matches('\'').count(), 2, "{command}");
    }
}
