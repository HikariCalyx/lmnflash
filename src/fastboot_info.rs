//! Reads device information from a connected fastboot device (USB), used to
//! pre-fill the RETCN lookup form.

use crate::firmware::Platform;
use std::collections::HashMap;

/// Values read from `fastboot getvar all` that map onto the RETCN form.
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub serial_number: String,
    pub imei: String,
    pub model: String,
    pub carrier: String,
    pub fingerprint: String,
    pub fsg_version: String,
    /// Chipset platform, when it can be inferred from the device variables.
    pub platform: Option<Platform>,
    /// SIM slot count (1 or 2), when the device reports it via the
    /// `oem hw dualsim` command.
    pub sim_count: Option<u8>,
}

/// Fastboot devices found on the USB bus.
#[derive(Debug, Clone)]
pub struct DeviceList {
    /// Serial numbers of supported (Motorola) devices.
    pub supported: Vec<String>,
    /// Total number of fastboot devices found, including unsupported ones.
    pub total: usize,
}

/// Lists the serial numbers of all connected Motorola fastboot devices
/// (other vendors are filtered out).
pub fn list_devices() -> Result<DeviceList, String> {
    let serials = fastboot::FastbootDevice::list_devices()?;
    let total = serials.len();

    let mut supported = Vec::new();
    for serial in serials {
        if is_supported_device(&serial) {
            supported.push(serial);
        }
    }

    Ok(DeviceList { supported, total })
}

/// True if the device with the given serial looks like a Motorola device.
fn is_supported_device(serial: &str) -> bool {
    let Ok(device) = fastboot::FastbootDevice::connect(serial) else {
        return false;
    };
    let Ok(lines) = device.getvar_all() else {
        return false;
    };

    is_motorola(&parse_getvar_all(&lines))
}

/// True if the variables identify a Motorola (Lenovo) device.
///
/// Motorola bootloaders always report a `cid` variable (e.g. `0x0032`),
/// which other vendors' fastboot implementations do not provide.
fn is_motorola(variables: &HashMap<String, String>) -> bool {
    variables
        .get("cid")
        .is_some_and(|value| !value.trim().is_empty())
}

/// Connects to the fastboot device with the given serial and reads its
/// variables, returning the values mapped onto the RETCN form.
pub fn read_device_info(serial: &str) -> Result<DeviceInfo, String> {
    let device = fastboot::FastbootDevice::connect(serial)?;
    let variables = parse_getvar_all(&device.getvar_all()?);

    if !is_motorola(&variables) {
        return Err("device is not supported (not a Motorola device)".to_string());
    }

    let get = |candidates: &[&str]| {
        candidates
            .iter()
            .find_map(|key| variables.get(*key).cloned())
            .unwrap_or_default()
    };

    let fsg_version = get(&[
        "fsg-id",
        "fsg-version",
        "fsgid",
        "fsgversion",
        "fsg-version.qcom",
        "fsgVersion.qcom",
    ]);

    // Fall back to any variable whose name mentions "fsg".
    let fsg_version = if fsg_version.is_empty() {
        variables
            .iter()
            .find(|(key, _)| key.to_ascii_lowercase().contains("fsg"))
            .map(|(_, value)| value.clone())
            .unwrap_or_default()
    } else {
        fsg_version
    };

    // Otherwise derive it from `version-baseband`: the value carries the
    // modem build first, then the FSG version
    // (e.g. `M8635_… ARCFOX_PVT1_NADSDS_CUST`).
    let fsg_version = if fsg_version.is_empty() {
        fsg_from_baseband(&get(&["version-baseband"]))
    } else {
        fsg_version
    };

    if fsg_version.is_empty() && cfg!(debug_assertions) {
        eprintln!("[fastboot] no fsg key found; variables: {variables:?}");
    }

    let serial_number = get(&["serialno"]);
    let serial_number = if serial_number.is_empty() {
        serial.to_string()
    } else {
        serial_number
    };

    let platform = detect_platform(&variables);

    // Query the SIM slot count directly via the OEM command
    // `fastboot oem hw dualsim`, which reports `dualsim: true` on dual-SIM
    // hardware (works regardless of chipset). Bootloaders that don't support
    // the command leave the count unknown.
    let sim_count = device
        .oem_info("hw dualsim")
        .ok()
        .and_then(|lines| parse_dualsim(&lines));

    Ok(DeviceInfo {
        serial_number,
        imei: get(&["imei"]),
        model: get(&["sku", "product"]),
        carrier: get(&["ro.carrier", "carrier"]),
        fingerprint: get(&["ro.build.fingerprint", "fingerprint", "ro.fingerprint"]),
        fsg_version,
        platform,
        sim_count,
    })
}

/// Detects the chipset platform from the device variables.
fn detect_platform(variables: &HashMap<String, String>) -> Option<Platform> {
    let qualcomm = variables
        .keys()
        .any(|key| key.starts_with("ro.build.version.qcom"))
        || variables.get("cpu").is_some_and(|cpu| {
            let cpu = cpu.to_ascii_lowercase();

            ["sm_", "msm", "apq", "sdm", "qcm", "sc7", "qsc", "snapdragon"]
                .iter()
                .any(|needle| cpu.starts_with(needle) || cpu.contains(needle))
        });

    let mediatek = variables
        .keys()
        .any(|key| key.to_ascii_lowercase().contains("mediatek"))
        || variables.get("cpu").is_some_and(|cpu| {
            let cpu = cpu.to_ascii_lowercase();

            cpu.contains("mediatek")
                || cpu.contains("dimensity")
                || cpu.contains("mtk")
                || (cpu.starts_with("mt")
                    && cpu.chars().nth(2).is_some_and(|c| c.is_ascii_digit()))
        });

    match (qualcomm, mediatek) {
        (true, false) => Some(Platform::Qualcomm),
        (false, true) => Some(Platform::MediaTek),
        _ => None,
    }
}

/// Parses `getvar all` output lines like `(bootloader) key: value`.
///
/// Long values are split across USB packets and reported as `key[0]`,
/// `key[1]`, … — those parts are stored as a single value joined in
/// index order.
fn parse_getvar_all(lines: &[String]) -> HashMap<String, String> {
    let mut parts: HashMap<String, Vec<(usize, String)>> = HashMap::new();

    for line in lines {
        let line = line.strip_prefix("(bootloader)").unwrap_or(line).trim();

        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().to_string();

        if let Some((base, index)) = split_indexed_key(key) {
            parts.entry(base).or_default().push((index, value));
        } else {
            parts
                .entry(key.to_string())
                .or_default()
                .push((0, value));
        }
    }

    parts
        .into_iter()
        .map(|(key, mut values)| {
            values.sort_by_key(|(index, _)| *index);
            let joined = values
                .into_iter()
                .map(|(_, value)| value)
                .collect::<Vec<_>>()
                .join("");

            (key, joined)
        })
        .collect()
}

/// Splits an indexed key like `ro.build.fingerprint[2]` into its base name
/// and index.
fn split_indexed_key(key: &str) -> Option<(String, usize)> {
    let open = key.rfind('[')?;
    if !key.ends_with(']') {
        return None;
    }

    let index: usize = key[open + 1..key.len() - 1].parse().ok()?;

    Some((key[..open].to_string(), index))
}

/// Derives the FSG version from the joined `version-baseband` value:
/// everything after the first whitespace-separated token
/// (e.g. `M8635_… ARCFOX_PVT1_NADSDS_CUST` → `ARCFOX_PVT1_NADSDS_CUST`).
fn fsg_from_baseband(baseband: &str) -> String {
    baseband
        .split_once(' ')
        .map(|(_, rest)| rest.trim().to_string())
        .unwrap_or_default()
}

/// Parses the SIM slot count from the `oem hw dualsim` response lines
/// (e.g. `dualsim: true` → Dual, `dualsim: false` → Single). Also accepts
/// numeric values (`1`, `2`, `0x2`, …) for bootloaders that report a count.
fn parse_dualsim(lines: &[String]) -> Option<u8> {
    let text = lines.join("\n").to_ascii_lowercase();

    if text.contains("true") {
        return Some(2);
    }
    if text.contains("false") {
        return Some(1);
    }

    lines
        .iter()
        .flat_map(|line| line.split([':', '=', ' ']))
        .find_map(|token| {
            let token = token.trim();
            let value = if let Some(hex) = token.strip_prefix("0x") {
                u8::from_str_radix(hex, 16).ok()?
            } else {
                token.parse::<u8>().ok()?
            };

            matches!(value, 1 | 2).then_some(value)
        })
}

/// Motorola's bootloader-unlock request page, where the Device ID is
/// submitted to receive the unique unlock key.
pub const UNLOCK_PAGE_URL: &str =
    "https://en-us.support.motorola.com/app/standalone/bootloader/unlock-your-device-b";

/// Parses the INFO payloads of `fastboot oem get_unlock_data` into the
/// Device ID.
///
/// Bootloaders reply in one of two shapes:
/// - modern ones return a single line prefixed `Unlock data:` that already
///   holds the whole ID (`Unlock data:3A95…&#…`); the label is stripped;
/// - older ones return the raw chunks, one per line (`0A4004…`, …), which
///   are concatenated.
///
/// A "Failed to get unlock data." notice is surfaced as an error instead of
/// being treated as part of the ID.
fn parse_unlock_data(lines: &[String]) -> Result<String, String> {
    let mut data = String::new();

    for line in lines {
        let text = line.trim();

        if text.to_ascii_lowercase().contains("failed to get unlock data") {
            return Err(
                "Failed to get unlock data: the device did not return its Device ID.".to_string(),
            );
        }

        // Strip an optional case-insensitive "Unlock data:" label.
        let lower = text.to_ascii_lowercase();
        let body = match lower.find("unlock data:") {
            Some(start) => &text[start + "unlock data:".len()..],
            None => text,
        };
        data.push_str(body);
    }

    if data.is_empty() {
        return Err("No unlock data returned by the device.".to_string());
    }

    Ok(data)
}

/// Runs `fastboot oem get_unlock_data` and returns the Device ID.
pub fn read_unlock_data(serial: &str) -> Result<String, String> {
    let device = fastboot::FastbootDevice::connect(serial)?;
    let lines = device.oem_info("get_unlock_data")?;
    parse_unlock_data(&lines)
}

/// Why sending the unlock key to the bootloader failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnlockFailure {
    /// The bootloader refuses because "OEM unlocking" is disabled in Android
    /// Developer Options.
    OemUnlockingDisabled,
    /// The user declined the on-device unlock confirmation (or it timed out);
    /// Motorola answers with an empty `FAILED (remote: '')` and no INFO.
    Cancelled,
    /// The unlock key was rejected (INFO "Code validation failure").
    WrongKey,
    /// Any other failure, carrying the bootloader's raw message.
    Other(String),
}

/// Sends the Motorola unlock command `fastboot oem unlock <key>` and returns
/// any INFO/OKAY text the bootloader reported.
///
/// The reply is classified so the UI can guide the user: a developer-option
/// notice (`Check 'OEM unlocking'…` / `Check 'Allow OEM Unlock'…`) becomes
/// [`UnlockFailure::OemUnlockingDisabled`], `Code validation failure` becomes
/// [`UnlockFailure::WrongKey`], an empty failure becomes
/// [`UnlockFailure::Cancelled`], and everything else is
/// [`UnlockFailure::Other`] with the raw message.
///
/// Note that MediaTek bootloaders answer `OKAY` even when they only remind
/// the user to enable the developer option, or when the key is wrong
/// (`Code validation failure.`); those OKAY-but-not-unlocked replies are
/// detected too. An *empty* OKAY means the user declined / timed out on
/// MediaTek, but is a normal success on Qualcomm — so the device platform is
/// detected first (reusing `detect_platform`) to disambiguate.
pub fn unlock_bootloader(serial: &str, key: &str) -> Result<String, UnlockFailure> {
    let device = fastboot::FastbootDevice::connect(serial).map_err(UnlockFailure::Other)?;

    // Best-effort platform detection: MediaTek answers a bare OKAY when the
    // user declines the unlock prompt (or times out), whereas Qualcomm FAILs
    // in that situation — so an empty OKAY means different things per vendor.
    let platform = device
        .getvar_all()
        .ok()
        .map(|lines| parse_getvar_all(&lines))
        .and_then(|variables| detect_platform(&variables));

    let command = format!("unlock {}", key.trim());
    match device.oem_info_with_fail_details(&command) {
        Ok(lines) => unlock_reply(lines.join("\n"), platform),
        Err(message) => Err(classify_unlock_failure(&message)),
    }
}

/// True when a reply mentions the Developer Options toggle that must be
/// enabled first. Qualcomm word it as "Check 'OEM unlocking'…", MediaTek as
/// "Check 'Allow OEM Unlock'…".
fn mentions_developer_toggle(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("oem unlocking") || lower.contains("allow oem unlock")
}

/// Turns a successful (OKAY) unlock response into a result. An OKAY does NOT
/// always mean the bootloader was unlocked:
///
/// - MediaTek reminds the user to enable the developer option
///   (`Check 'Allow OEM Unlock'…`) or rejects a wrong key
///   (`Code validation failure.`) while still answering OKAY;
/// - if the user declines the on-device prompt (or it times out), MediaTek
///   answers a bare OKAY with **no** message — and no unlock happens
///   (Qualcomm FAILs instead, so an empty OKAY there is a success);
/// - a confirmed unlock is the one that carries an extra message (e.g.
///   `Bootloader unlock success`) before the OKAY.
fn unlock_reply(
    text: String,
    platform: Option<crate::firmware::Platform>,
) -> Result<String, UnlockFailure> {
    if mentions_developer_toggle(&text) {
        Err(UnlockFailure::OemUnlockingDisabled)
    } else if text.to_ascii_lowercase().contains("code validation failure") {
        Err(UnlockFailure::WrongKey)
    } else if text.trim().is_empty() {
        // Empty OKAY = declined / timeout on MediaTek; a genuine success on
        // Qualcomm (or when the platform could not be detected).
        match platform {
            Some(crate::firmware::Platform::MediaTek) => Err(UnlockFailure::Cancelled),
            _ => Ok(text),
        }
    } else {
        // Non-empty OKAY without a blocker notice = the unlock happened.
        Ok(text)
    }
}

/// Maps a bootloader failure message to an [`UnlockFailure`] kind. Motorola
/// bootloaders preface the FAILED packet with INFO lines such as
/// `Check 'OEM unlocking' in Android Settings > Developer` or
/// `Code validation failure`; declining the on-device prompt (or a timeout)
/// yields an empty FAILED with no INFO lines.
fn classify_unlock_failure(message: &str) -> UnlockFailure {
    let lower = message.to_ascii_lowercase();

    if mentions_developer_toggle(message) {
        UnlockFailure::OemUnlockingDisabled
    } else if lower.contains("code validation failure") {
        UnlockFailure::WrongKey
    } else if message.trim().is_empty() {
        UnlockFailure::Cancelled
    } else {
        UnlockFailure::Other(message.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_oem_unlock_disabled_failure() {
        let message =
            "Check 'OEM unlocking' in Android Settings > Developer | Options";
        assert_eq!(
            classify_unlock_failure(message),
            UnlockFailure::OemUnlockingDisabled
        );
    }

    #[test]
    fn classifies_mtk_allow_oem_unlock_disabled() {
        let message =
            "Check 'Allow OEM Unlock' in Android Settings > Developer | Options";
        assert_eq!(
            classify_unlock_failure(message),
            UnlockFailure::OemUnlockingDisabled
        );
        // MediaTek answers OKAY while only reminding the user: the success
        // reply path must still be treated as blocked, not as unlocked.
        assert_eq!(
            unlock_reply(message.to_string(), Some(crate::firmware::Platform::MediaTek)),
            Err(UnlockFailure::OemUnlockingDisabled)
        );
    }

    #[test]
    fn confirmed_unlock_reply_is_success() {
        // A confirmed unlock carries an extra message before the OKAY.
        assert_eq!(
            unlock_reply(
                "Bootloader unlock success".to_string(),
                Some(crate::firmware::Platform::MediaTek)
            ),
            Ok("Bootloader unlock success".to_string())
        );
    }

    #[test]
    fn empty_okay_depends_on_platform() {
        // MediaTek answers a bare OKAY (no message) when the user declines
        // the prompt or it times out — that is NOT an unlock.
        assert_eq!(
            unlock_reply(String::new(), Some(crate::firmware::Platform::MediaTek)),
            Err(UnlockFailure::Cancelled)
        );
        // Qualcomm FAILs on decline/timeout instead, so an empty OKAY there
        // is a genuine success.
        assert_eq!(
            unlock_reply(String::new(), Some(crate::firmware::Platform::Qualcomm)),
            Ok(String::new())
        );
        // Unknown platform falls back to treating an empty OKAY as success.
        assert_eq!(unlock_reply(String::new(), None), Ok(String::new()));
    }

    #[test]
    fn mtk_wrong_key_okay_reply_is_rejected() {
        assert_eq!(
            unlock_reply(
                "Code validation failure.".to_string(),
                Some(crate::firmware::Platform::MediaTek)
            ),
            Err(UnlockFailure::WrongKey)
        );
    }

    #[test]
    fn classifies_wrong_key_failure() {
        let message = "Code validation failure";
        assert_eq!(classify_unlock_failure(message), UnlockFailure::WrongKey);
    }

    #[test]
    fn classifies_cancelled_failure_when_empty() {
        assert_eq!(classify_unlock_failure(""), UnlockFailure::Cancelled);
    }

    #[test]
    fn classifies_other_unlock_failure() {
        let message = "FAILED (remote: 'bad key')";
        assert_eq!(
            classify_unlock_failure(message),
            UnlockFailure::Other(message.to_string())
        );
    }

    #[test]
    fn strips_unlock_data_label_from_single_line() {
        let line = "Unlock data:3A95915042649321#5A5932324B525834524B006D6F746F726F6C0000#8226946C1000ED2C2C9D9A3A981F063D80A30825CA264A20E51C1EDCC25BD0C4#10CAE512002750E10000000000000000";
        let parsed = parse_unlock_data(&[line.to_string()]).unwrap();

        assert_eq!(parsed, line.strip_prefix("Unlock data:").unwrap());
        assert!(!parsed.contains("Unlock data:"));
    }

    #[test]
    fn concatenates_unprefixed_unlock_data_chunks() {
        let chunks = [
            "0A40040192024205#4C4D3556313230",
            "30373731363031303332323239#BD00",
            "8A672BA4746C2CE02328A2AC0C39F95",
            "1A3E5#1F53280002000000000000000",
            "0000000",
        ];
        let lines: Vec<String> = chunks.iter().map(|s| s.to_string()).collect();

        assert_eq!(parse_unlock_data(&lines).unwrap(), chunks.concat());
    }

    #[test]
    fn rejects_failed_unlock_data_notice() {
        let lines = ["Failed to get unlock data.".to_string()];
        let error = parse_unlock_data(&lines).unwrap_err();

        assert!(error.contains("Failed to get unlock data"));
    }

    #[test]
    fn rejects_empty_unlock_data() {
        let lines = ["".to_string()];
        assert!(parse_unlock_data(&lines).is_err());
    }

    #[test]
    fn parses_getvar_all_lines() {
        let variables = parse_getvar_all(&[
            "(bootloader) version: 0.5".to_string(),
            "(bootloader) serialno: ZY22ABC".to_string(),
            "(bootloader) sku: XT2201-2".to_string(),
            "(bootloader) ro.build.fingerprint[0]: motorola/x/x:11/A/x:user".to_string(),
            "(bootloader) ro.build.fingerprint[1]: /release-keys".to_string(),
        ]);

        assert_eq!(variables.get("version").map(String::as_str), Some("0.5"));
        assert_eq!(
            variables.get("serialno").map(String::as_str),
            Some("ZY22ABC")
        );
        assert_eq!(variables.get("sku").map(String::as_str), Some("XT2201-2"));
        assert_eq!(
            variables.get("ro.build.fingerprint").map(String::as_str),
            Some("motorola/x/x:11/A/x:user/release-keys")
        );
    }

    #[test]
    fn joins_out_of_order_parts_by_index() {
        let variables = parse_getvar_all(&[
            "(bootloader) version-baseband[1]: T1_NADSDS_CUST".to_string(),
            "(bootloader) version-baseband[0]: M8635_DE50 ARCFOX_PV".to_string(),
        ]);

        assert_eq!(
            variables.get("version-baseband").map(String::as_str),
            Some("M8635_DE50 ARCFOX_PVT1_NADSDS_CUST")
        );
    }

    #[test]
    fn derives_fsg_version_from_baseband() {
        assert_eq!(
            fsg_from_baseband("M8635_DE50_31.2496.01.60.98R ARCFOX_PVT1_NADSDS_CUST"),
            "ARCFOX_PVT1_NADSDS_CUST"
        );
        assert_eq!(fsg_from_baseband(""), "");
    }

    #[test]
    fn detects_qualcomm_platform() {
        let variables = parse_getvar_all(&["(bootloader) cpu: SM_PALAWAN 1.0".to_string()]);

        assert_eq!(detect_platform(&variables), Some(Platform::Qualcomm));
    }

    #[test]
    fn detects_mediatek_platform() {
        let variables = parse_getvar_all(&["(bootloader) cpu: MT6833".to_string()]);

        assert_eq!(detect_platform(&variables), Some(Platform::MediaTek));
    }

    #[test]
    fn leaves_platform_unknown_without_hints() {
        let variables = parse_getvar_all(&["(bootloader) cpu: UNKNOWN 1.0".to_string()]);

        assert_eq!(detect_platform(&variables), None);
    }

    #[test]
    fn parses_dualsim_from_oem_output() {
        // Dual-SIM device, exactly as `fastboot oem hw dualsim` reports it.
        assert_eq!(
            parse_dualsim(&["dualsim: true".to_string()]),
            Some(2)
        );
        // Single-SIM device.
        assert_eq!(
            parse_dualsim(&["dualsim: false".to_string()]),
            Some(1)
        );
        // Case-insensitive.
        assert_eq!(parse_dualsim(&["DualSIM: TRUE".to_string()]), Some(2));
        // Numeric fallback for bootloaders that report a count.
        assert_eq!(parse_dualsim(&["1".to_string()]), Some(1));
        assert_eq!(parse_dualsim(&["2".to_string()]), Some(2));
        assert_eq!(parse_dualsim(&["dualsim: 0x2".to_string()]), Some(2));
        // Unrecognized output leaves the count unknown.
        assert_eq!(parse_dualsim(&["dualsim: 0".to_string()]), None);
        assert_eq!(parse_dualsim(&["unknown".to_string()]), None);
        assert_eq!(parse_dualsim(&[]), None);
    }

    #[test]
    fn identifies_motorola_devices() {
        let moto = parse_getvar_all(&[
            "(bootloader) cid: 0x000B".to_string(),
            "(bootloader) version-bootloader[0]: MBM-3.1-cybert-bc5896-".to_string(),
        ]);

        assert!(is_motorola(&moto));
    }

    #[test]
    fn rejects_non_motorola_devices() {
        let non_moto = parse_getvar_all(&[
            "(bootloader) version-bootloader:".to_string(),
            "(bootloader) variant:SM_ UFS".to_string(),
        ]);

        assert!(!is_motorola(&non_moto));
    }
}
