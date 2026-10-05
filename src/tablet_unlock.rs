//! Lenovo tablet bootloader unlock (Mode 2, Smartphone Flash).
//!
//! Lenovo tablets are not Motorola devices and have no CID, so the unlock
//! works differently from the phone flow: the user reads the serial number and
//! the `Bootloader_SN` from fastboot, submits them on Lenovo's ZUI unlock
//! website, and the site then makes a per-device `sn.img` token available.
//! [`unlock`] downloads that token, flashes it to the `unlock` partition and
//! finishes with `oem unlock-go`.
//!
//! Everything here talks to hardware or the network and is meant to run on a
//! background thread; the dialog that presents it lives in
//! `crate::tablet_bootloader`.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// Lenovo's ZUI bootloader-unlock website (Chinese only), where the serial
/// number and `Bootloader_SN` are submitted.
pub const UNLOCK_SITE_URL: &str = "https://www.zui.com/iunlock";

/// Token endpoint for tablets that report a `Bootloader_SN`.
const ENHANCED_BOOT_URL: &str = "http://cdn.zui.lenovomm.com/developer/enhancedboot";

/// Token endpoint for tablets without a `Bootloader_SN`.
const TABLET_BOOT_URL: &str = "http://cdn.zui.lenovomm.com/developer/tabletboot";

/// The token endpoint is served over plain HTTP and has no TLS, so the URL
/// must never be upgraded to `https`.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);

/// The identity a ZUI unlock request needs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TabletUnlockInfo {
    /// Serial number, left-padded to eight characters.
    pub serial: String,
    /// `Bootloader_SN_Part1`, empty when the bootloader does not report it.
    pub bootloader_sn_part1: String,
    /// `Bootloader_SN_Part2`, empty when the bootloader does not report it.
    pub bootloader_sn_part2: String,
}

impl TabletUnlockInfo {
    /// The `Bootloader_SN` shown to the user: both parts concatenated.
    pub fn bootloader_sn(&self) -> String {
        format!(
            "{}{}",
            self.bootloader_sn_part1.trim(),
            self.bootloader_sn_part2.trim()
        )
    }

    /// Whether a `Bootloader_SN` was read, in which case the enhanced token
    /// endpoint is used.
    pub fn has_bootloader_sn(&self) -> bool {
        !self.bootloader_sn().is_empty()
    }
}

/// Left-pads a serial number to at least eight characters with `0`
/// (`123456` → `00123456`), the form the ZUI endpoints expect.
pub fn pad_serial(serial: &str) -> String {
    let serial = serial.trim();

    if serial.len() >= 8 {
        serial.to_owned()
    } else {
        format!("{serial:0>8}")
    }
}

/// Reads the serial number and `Bootloader_SN` from one device.
///
/// Every variable is read best effort: a tablet that does not report the
/// `Bootloader_SN` (or a part of it) still yields the serial number.
pub fn read_info(device_serial: &str) -> Result<TabletUnlockInfo, String> {
    let device = fastboot::FastbootDevice::connect(device_serial)?;

    let read = |name: &str| -> String {
        crate::fastboot_info::getvar_value(&device, name)
            .map(|value| value.trim().to_owned())
            .unwrap_or_default()
    };

    // The USB serial the device was found with stands in when the bootloader
    // does not answer `getvar serialno`.
    let serial = read("serialno");
    let serial = if serial.is_empty() {
        device_serial.to_owned()
    } else {
        serial
    };

    Ok(TabletUnlockInfo {
        serial: pad_serial(&serial),
        bootloader_sn_part1: read("Bootloader_SN_Part1"),
        bootloader_sn_part2: read("Bootloader_SN_Part2"),
    })
}

/// The URL the device's unlock token is downloaded from.
///
/// A device that reports a `Bootloader_SN` uses the enhanced endpoint, where
/// the two parts are concatenated (the `+` ZUI documentation writes between
/// them is only a notation for the join and is not part of the URI);
/// otherwise the (padded) serial number is used.
pub fn token_url(info: &TabletUnlockInfo) -> String {
    if info.has_bootloader_sn() {
        format!(
            "{ENHANCED_BOOT_URL}/{}{}/sn.img",
            info.bootloader_sn_part1.trim(),
            info.bootloader_sn_part2.trim()
        )
    } else {
        format!("{TABLET_BOOT_URL}/{}/sn.img", info.serial.trim())
    }
}

/// Where the downloaded token is written.
pub fn token_path() -> PathBuf {
    std::env::temp_dir()
        .join("lmnflash-tablet-unlock")
        .join("sn.img")
}

/// Downloads `url` to `path`. Any failure means the token is not available —
/// it has not been generated yet, or the server cannot be reached.
fn download_token(url: &str, path: &Path) -> Result<(), String> {
    let response = ureq::AgentBuilder::new()
        .timeout(DOWNLOAD_TIMEOUT)
        .build()
        .get(url)
        .call()
        .map_err(|error| error.to_string())?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    let mut file = std::fs::File::create(path).map_err(|error| error.to_string())?;
    std::io::copy(&mut response.into_reader(), &mut file).map_err(|error| error.to_string())?;

    Ok(())
}

/// What happened during a ZUI unlock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnlockOutcome {
    /// The token was flashed and `oem unlock-go` succeeded; the tablet shows
    /// its own confirmation prompt.
    Unlocked,
    /// The token has not been generated (or could not be downloaded).
    NotGenerated,
    /// Flashing the token failed; carries the bootloader's own message.
    FlashFailed(String),
    /// `oem unlock-go` was refused (OEM unlocking must be enabled first).
    OemUnlockingDisabled,
    /// The device could not be reached.
    Failed(String),
}

/// Runs the whole unlock: download the token for `info`, flash it to the
/// `unlock` partition, then send `oem unlock-go`.
pub fn unlock(device_serial: &str, info: &TabletUnlockInfo, token: &Path) -> UnlockOutcome {
    let url = token_url(info);

    if let Err(error) = download_token(&url, token) {
        #[cfg(debug_assertions)]
        eprintln!("[tablet] token download failed ({url}): {error}");
        #[cfg(not(debug_assertions))]
        let _ = &error;

        return UnlockOutcome::NotGenerated;
    }

    let device = match fastboot::FastbootDevice::connect(device_serial) {
        Ok(device) => device,
        Err(error) => return UnlockOutcome::Failed(error),
    };

    if let Err(error) = device.flash_file("unlock", token, None) {
        return UnlockOutcome::FlashFailed(raw_failure(&error));
    }

    match device.oem("unlock-go") {
        Ok(()) => UnlockOutcome::Unlocked,
        Err(_) => UnlockOutcome::OemUnlockingDisabled,
    }
}

/// The bootloader's own FAIL text, without the description the fastboot crate
/// wraps it in (e.g. `Flash failed: <payload>`).
fn raw_failure(message: &str) -> String {
    const WRAPPERS: &[&str] = &["Flash failed: ", "Download rejected: "];

    for wrapper in WRAPPERS {
        if let Some(rest) = message.strip_prefix(wrapper) {
            return rest.to_owned();
        }
    }

    message.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_serials_are_left_padded() {
        assert_eq!(pad_serial("123456"), "00123456");
        assert_eq!(pad_serial("12345678"), "12345678");
        assert_eq!(pad_serial("123456789"), "123456789");
        assert_eq!(pad_serial("  42  "), "00000042");
    }

    #[test]
    fn enhanced_endpoint_uses_the_bootloader_sn() {
        let info = TabletUnlockInfo {
            serial: "00123456".to_owned(),
            bootloader_sn_part1: "ABCD".to_owned(),
            bootloader_sn_part2: "EFGH".to_owned(),
        };

        assert_eq!(
            token_url(&info),
            "http://cdn.zui.lenovomm.com/developer/enhancedboot/ABCDEFGH/sn.img"
        );
    }

    #[test]
    fn plain_endpoint_uses_the_serial() {
        let info = TabletUnlockInfo {
            serial: "00123456".to_owned(),
            ..TabletUnlockInfo::default()
        };

        assert_eq!(
            token_url(&info),
            "http://cdn.zui.lenovomm.com/developer/tabletboot/00123456/sn.img"
        );
    }

    #[test]
    fn the_token_url_is_never_https() {
        // The endpoint has no TLS, so an accidental upgrade would break it.
        assert!(token_url(&TabletUnlockInfo::default()).starts_with("http://"));
    }

    #[test]
    fn the_bootloader_sn_is_the_two_parts_concatenated() {
        let info = TabletUnlockInfo {
            serial: "x".to_owned(),
            bootloader_sn_part1: "AAA".to_owned(),
            bootloader_sn_part2: "BBB".to_owned(),
        };

        assert_eq!(info.bootloader_sn(), "AAABBB");
        assert!(info.has_bootloader_sn());
        assert!(!TabletUnlockInfo::default().has_bootloader_sn());
    }

    #[test]
    fn wrapper_prefixes_are_stripped_from_failures() {
        assert_eq!(
            raw_failure("Flash failed: Partition not found"),
            "Partition not found"
        );
        assert_eq!(raw_failure("W: timeout"), "W: timeout");
    }
}
