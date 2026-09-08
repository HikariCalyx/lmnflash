//! Best-effort telemetry for successful firmware-image lookups.
//!
//! Ported from the design in `reference/telemetry.txt` (originally added to
//! the Android app): when a lookup finds a firmware image, the app POSTs a
//! small JSON event to the HikariCalyx API so firmware builds can be
//! collected. The POST is fire-and-forget — a short timeout, and every
//! failure is swallowed — so telemetry can never change the outcome of a
//! lookup.
//!
//! Two fields describe the environment instead of a device, because this is a
//! desktop app:
//! - `lmnflash_version` is the crate version from `Cargo.toml`.
//! - `fingerprint` is the Rust build target triple (there is no device
//!   fingerprint to read on a desktop app).

use crate::firmware::{CnTabletInfo, FirmwareInfo, Platform, RetcnRequest};
use serde_json::{Value, json};

/// Endpoint receiving successful-lookup telemetry.
pub const TELEMETRY_ENDPOINT: &str = "https://api.hikaricalyx.com/LMN/v1/Telemetry";

/// `lmnflash_version` reported with every event: the crate version from
/// `Cargo.toml`, kept in lockstep with releases.
pub fn lmnflash_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// `fingerprint` reported with every event. A desktop app has no device
/// fingerprint to read, so the Rust build target triple identifies the
/// running platform instead (e.g. `x86_64-pc-windows-msvc`,
/// `aarch64-apple-darwin`). The value is baked in by `build.rs` as
/// `TARGET_TRIPLE`.
pub fn fingerprint() -> &'static str {
    env!("TARGET_TRIPLE")
}

/// Which successful lookup produced the firmware image (maps to the server's
/// `lookupType`).
#[derive(Debug, Clone)]
pub enum Lookup {
    /// ROW smartphone lookup by IMEI.
    RowSmartphone {
        /// The normalized IMEI the lookup was submitted with.
        imei: String,
    },
    /// RETCN (China) smartphone lookup.
    RetcnSmartphone(RetcnRequest),
    /// Tablet lookup by serial number.
    Tablet {
        /// The serial number the lookup was submitted with (the `psn`).
        serial_number: String,
    },
}

/// The firmware image a successful lookup returned.
#[derive(Debug, Clone)]
pub enum Found {
    /// A standard `FirmwareInfo` result (ROW/RETCN phones and the ROW tablet
    /// fallback).
    Standard(FirmwareInfo),
    /// A China-tablet result.
    CnTablet(CnTabletInfo),
}

/// Builds the JSON event sent for a successful lookup.
///
/// The `body` follows the matching schema in `reference/telemetry.txt`; the
/// outer `lmnflash_version` is the crate version and `fingerprint` the Rust
/// build target. Returns `Value::Null` when the halves don't represent a real
/// outcome (e.g. a phone lookup with a CN-tablet result) — the caller skips
/// the request in that case.
pub fn build(lookup: &Lookup, found: &Found) -> Value {
    let lookup_type = match lookup {
        Lookup::RowSmartphone { .. } => "ROWSmartphone",
        Lookup::RetcnSmartphone(_) => "RETCNSmartphone",
        Lookup::Tablet { .. } => "Tablet",
    };

    let body = match (lookup, found) {
        (Lookup::RowSmartphone { imei }, Found::Standard(info)) => json!({
            "imei": imei_number(imei),
            "xtCode": info.model_name,
            "carrier": info.carrier,
            "marketName": info.market_name,
            "packageName": info.file_name,
        }),
        (Lookup::RetcnSmartphone(request), Found::Standard(info)) => json!({
            "imei": imei_number(&request.imei),
            "psn": request.serial_number,
            "xtCode": request.model,
            "carrier": request.carrier,
            // The fingerprint of the serviced phone, as filled on the RETCN
            // form (or pulled from the fastboot device) — the same value the
            // lookup itself used.
            "fingerprint": request.fingerprint,
            "soc": soc_name(request.platform),
            // `fsgVersion` applies to Qualcomm, `simCount` to MediaTek; the
            // other half stays empty/default, mirroring the RETCN form.
            "fsgver": request.fsg_version.clone().unwrap_or_default(),
            "simslot": request.sim_count.unwrap_or(1),
            "marketName": info.market_name,
            "packageName": info.file_name,
        }),
        (Lookup::Tablet { serial_number }, Found::Standard(info)) => json!({
            "psn": serial_number,
            "modelCode": info.model_name,
            "marketName": info.market_name,
            "packageName": info.file_name,
        }),
        (Lookup::Tablet { serial_number }, Found::CnTablet(info)) => json!({
            "psn": serial_number,
            "modelCode": info.product_model,
            "marketName": info.market_name,
            "packageName": info.file_name,
        }),
        // Phones never produce a CN-tablet result and vice versa; when the
        // two halves don't match, report nothing instead of a wrong body.
        _ => return Value::Null,
    };

    json!({
        "lookupType": lookup_type,
        "body": body,
        "lmnflash_version": lmnflash_version(),
        "fingerprint": fingerprint(),
    })
}

/// Sends a telemetry event (blocking). Best-effort: any failure — network,
/// timeout, non-2xx — is returned as `Err` for the caller to log and is never
/// allowed to surface as an app error.
pub fn post(payload: Value) -> Result<(), String> {
    // `build` returns `Value::Null` for combinations we cannot represent;
    // there is nothing useful to send in that case.
    if !payload.is_object() {
        return Ok(());
    }

    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(10))
        .build();

    agent
        .post(TELEMETRY_ENDPOINT)
        .set("Content-Type", "application/json")
        .send_json(payload)
        .map_err(|e| format!("telemetry request failed: {e}"))?;

    Ok(())
}

/// Serializes an IMEI as a JSON number. IMEIs are 14–15 digits, well within a
/// `u64`; an unparseable value (defensive only, the caller validates first)
/// is reported as 0 so the field stays numeric like the reference schemas.
fn imei_number(imei: &str) -> u64 {
    imei.trim().parse().unwrap_or_default()
}

/// Lowercase SoC name used by the server (`qcom`/`mtk`).
fn soc_name(platform: Platform) -> &'static str {
    match platform {
        Platform::Qualcomm => "qcom",
        Platform::MediaTek => "mtk",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_info() -> FirmwareInfo {
        FirmwareInfo {
            market_name: "Motorola Moto G Power".to_string(),
            model_name: "XT2165-4".to_string(),
            sale_model: "XT2165-4".to_string(),
            carrier: "retus".to_string(),
            comments: String::new(),
            publish_date: String::new(),
            rom_match_id: String::new(),
            fingerprint: "motorola/fp/aaaa:15/aaaa-user/release-keys".to_string(),
            rom_id: String::new(),
            rom_uri: "https://example.com/fw/XT2165-4_retus.zip".to_string(),
            tool_uri: String::new(),
            file_name: "XT2165-4_retus.zip".to_string(),
            file_size: String::new(),
            raw_json: "{}".to_string(),
        }
    }

    fn sample_cn_tablet() -> CnTabletInfo {
        CnTabletInfo {
            product_name: "Lenovo Pad Pro 2025".to_string(),
            product_model: "TB371FC".to_string(),
            market_name: "Lenovo Pad Pro 2025".to_string(),
            mtm_compat: "TB371FC".to_string(),
            latest_version: "A11".to_string(),
            id: "123".to_string(),
            download_url: "https://example.com/fw/TB371FC_A11.zip".to_string(),
            publish_date: String::new(),
            file_name: "TB371FC_A11.zip".to_string(),
            file_size: String::new(),
            unzip_password: "FC(fv:SknR".to_string(),
        }
    }

    fn sample_retcn(platform: Platform) -> RetcnRequest {
        RetcnRequest {
            imei: "123456789012347".to_string(),
            serial_number: "ZG22123456".to_string(),
            fingerprint: "motorola/fp/aaaa:15/aaaa-user/release-keys".to_string(),
            model: "XT2165-4".to_string(),
            carrier: "retus".to_string(),
            platform,
            fsg_version: (platform == Platform::Qualcomm)
                .then(|| "AAA_PVT_NADSDS_CUST".to_string()),
            sim_count: (platform == Platform::MediaTek).then_some(2),
        }
    }

    #[test]
    fn row_smartphone_payload_matches_schema() {
        let payload = build(
            &Lookup::RowSmartphone {
                imei: "123456789012347".to_string(),
            },
            &Found::Standard(sample_info()),
        );

        assert_eq!(
            payload,
            json!({
                "lookupType": "ROWSmartphone",
                "body": {
                    "imei": 123456789012347u64,
                    "xtCode": "XT2165-4",
                    "carrier": "retus",
                    "marketName": "Motorola Moto G Power",
                    "packageName": "XT2165-4_retus.zip",
                },
                "lmnflash_version": env!("CARGO_PKG_VERSION"),
                "fingerprint": env!("TARGET_TRIPLE"),
            })
        );
    }

    #[test]
    fn retcn_smartphone_qualcomm_payload_matches_schema() {
        let payload = build(
            &Lookup::RetcnSmartphone(sample_retcn(Platform::Qualcomm)),
            &Found::Standard(sample_info()),
        );

        assert_eq!(
            payload,
            json!({
                "lookupType": "RETCNSmartphone",
                "body": {
                    "imei": 123456789012347u64,
                    "psn": "ZG22123456",
                    "xtCode": "XT2165-4",
                    "carrier": "retus",
                    "fingerprint": "motorola/fp/aaaa:15/aaaa-user/release-keys",
                    "soc": "qcom",
                    "fsgver": "AAA_PVT_NADSDS_CUST",
                    "simslot": 1,
                    "marketName": "Motorola Moto G Power",
                    "packageName": "XT2165-4_retus.zip",
                },
                "lmnflash_version": env!("CARGO_PKG_VERSION"),
                "fingerprint": env!("TARGET_TRIPLE"),
            })
        );
    }

    #[test]
    fn retcn_smartphone_mediatek_uses_simslot() {
        let payload = build(
            &Lookup::RetcnSmartphone(sample_retcn(Platform::MediaTek)),
            &Found::Standard(sample_info()),
        );

        assert_eq!(payload["body"]["soc"], "mtk");
        assert_eq!(payload["body"]["fsgver"], "");
        assert_eq!(payload["body"]["simslot"], 2);
    }

    #[test]
    fn tablet_cn_payload_matches_schema() {
        let payload = build(
            &Lookup::Tablet {
                serial_number: "HA123456".to_string(),
            },
            &Found::CnTablet(sample_cn_tablet()),
        );

        assert_eq!(
            payload,
            json!({
                "lookupType": "Tablet",
                "body": {
                    "psn": "HA123456",
                    "modelCode": "TB371FC",
                    "marketName": "Lenovo Pad Pro 2025",
                    "packageName": "TB371FC_A11.zip",
                },
                "lmnflash_version": env!("CARGO_PKG_VERSION"),
                "fingerprint": env!("TARGET_TRIPLE"),
            })
        );
    }

    #[test]
    fn tablet_row_fallback_payload_uses_model_name() {
        let payload = build(
            &Lookup::Tablet {
                serial_number: "HA123456".to_string(),
            },
            &Found::Standard(sample_info()),
        );

        assert_eq!(payload["lookupType"], "Tablet");
        assert_eq!(payload["body"]["modelCode"], "XT2165-4");
        assert_eq!(payload["body"]["marketName"], "Motorola Moto G Power");
        assert_eq!(payload["body"]["packageName"], "XT2165-4_retus.zip");
    }

    #[test]
    fn mismatched_lookup_and_result_reports_nothing() {
        // A phone lookup never yields a CN-tablet result.
        let payload = build(
            &Lookup::RowSmartphone {
                imei: "123456789012347".to_string(),
            },
            &Found::CnTablet(sample_cn_tablet()),
        );
        assert!(payload.is_null());

        let payload = build(
            &Lookup::RetcnSmartphone(sample_retcn(Platform::Qualcomm)),
            &Found::CnTablet(sample_cn_tablet()),
        );
        assert!(payload.is_null());

        // Posting a non-object payload is a harmless no-op (no network).
        assert_eq!(post(payload), Ok(()));
    }
}
