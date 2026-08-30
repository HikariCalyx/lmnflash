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

#[cfg(test)]
mod tests {
    use super::*;

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
