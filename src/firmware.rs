//! Firmware lookup for Lenovo devices, ported from `reference/imeiget.sh`
//! and `reference/gen.sh`.

use base64::Engine as _;
use rsa::pkcs8::DecodePublicKey;
use serde_json::json;
use url::Url;

const CLIENT_VERSION: &str = "7.6.2.10";
const FIRMWARE_ENDPOINT: &str =
    "https://lsa.lenovo.com/Interface/rescueDevice/getNewResourceByImei.jhtml";
const RETCN_ENDPOINT: &str =
    "https://lsa.lenovo.com/Interface/rescueDevice/getNewResource.jhtml";
const TABLET_ROW_ENDPOINT: &str =
    "https://lsa.lenovo.com/Interface/rescueDevice/getNewResourceBySN.jhtml";

const CN_MACHINE_ENDPOINT: &str =
    "https://ptstpd.lenovo.com.cn/home/ConfigurationQuery/getMachineSequenceInfo";
const CN_FIRMWARE_ENDPOINT: &str =
    "https://ptstpd.lenovo.com.cn/home/ConfigurationQuery/getPadFlashingMachine";
/// Extraction password for CN tablet firmware packages.
const CN_UNZIP_PASSWORD: &str = "FC(fv:SknR";
/// Derived in `gen.sh` as `<service-root>/Interface/common/rsa.jhtml`
/// (path minus the last two segments).
const RSA_KEY_ENDPOINT: &str = "https://lsa.lenovo.com/Interface/common/rsa.jhtml";

/// Why an IMEI value was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImeiError {
    NotDigits,
    WrongLength,
    BadChecksum,
}

/// Validates an IMEI and returns the normalized 15-digit string.
///
/// Accepts 15 digits with a valid Luhn checksum, or 14 digits (the check
/// digit is then computed and appended automatically).
pub fn validate_imei(imei: &str) -> Result<String, ImeiError> {
    let digits: String = imei
        .trim()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();

    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ImeiError::NotDigits);
    }

    match digits.len() {
        14 => Ok(append_check_digit(&digits)),
        15 => {
            if !luhn_valid(&digits) {
                return Err(ImeiError::BadChecksum);
            }
            Ok(digits)
        }
        _ => Err(ImeiError::WrongLength),
    }
}

/// True if the Luhn checksum of a 15-digit IMEI is valid.
fn luhn_valid(digits: &str) -> bool {
    luhn_sum(digits, 0) % 10 == 0
}

/// Computes the Luhn check digit and appends it, turning a 14-digit IMEI
/// payload into a 15-digit IMEI.
fn append_check_digit(digits: &str) -> String {
    // In the final 15-digit number the check digit is position 0 from the
    // right, so every existing digit shifts one position (and its doubling
    // pattern flips).
    let payload_sum = luhn_sum(digits, 1);
    let check = (10 - (payload_sum % 10)) % 10;

    format!("{digits}{check}")
}

/// Sums the digits with Luhn doubling. `offset` shifts the positions from the
/// right, accounting for a trailing check digit when computing one.
fn luhn_sum(digits: &str, offset: usize) -> u32 {
    digits
        .bytes()
        .rev()
        .enumerate()
        .map(|(index, byte)| {
            let digit = u32::from(byte - b'0');
            match (index + offset) % 2 {
                1 => {
                    let doubled = digit * 2;
                    if doubled > 9 {
                        doubled - 9
                    } else {
                        doubled
                    }
                }
                _ => digit,
            }
        })
        .sum()
}

/// Firmware information returned for a device.
#[derive(Debug, Clone)]
pub struct FirmwareInfo {
    pub market_name: String,
    pub model_name: String,
    pub sale_model: String,
    pub carrier: String,
    pub comments: String,
    pub publish_date: String,
    pub rom_match_id: String,
    pub fingerprint: String,
    pub rom_id: String,
    pub rom_uri: String,
    pub tool_uri: String,
    /// File name taken from the download URL (percent-decoded).
    pub file_name: String,
    /// Download size as a human-readable string (e.g. "1.5 GB"); empty
    /// when the server did not report one.
    pub file_size: String,
    /// The raw API response, preserved for copying (like `--raw` in imeiget.sh).
    pub raw_json: String,
}

/// Firmware information returned by the CN tablet service.
#[derive(Debug, Clone)]
pub struct CnTabletInfo {
    pub product_name: String,
    pub product_model: String,
    pub market_name: String,
    pub mtm_compat: String,
    pub latest_version: String,
    pub id: String,
    pub download_url: String,
    /// Derived from the `Last-Modified` header of the download URL.
    pub publish_date: String,
    /// File name taken from the download URL (percent-decoded).
    pub file_name: String,
    /// Download size as a human-readable string (e.g. "1.5 GB"); empty
    /// when the server did not report one.
    pub file_size: String,
    /// Extraction (unzip) password for the firmware package.
    pub unzip_password: String,
}

/// An error produced by [`fetch_firmware`].
#[derive(Debug, Clone)]
pub enum FirmwareError {
    /// The Authorization token is invalid or expired (API codes 402–409,
    /// same treatment as in `reference/imeiget.sh`).
    AuthExpired(String),
    /// Any other failure.
    Other(String),
}

/// Supported platform types for the RETCN lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Platform {
    #[default]
    Qualcomm,
    MediaTek,
}

impl Platform {
    pub const ALL: [Self; 2] = [Self::Qualcomm, Self::MediaTek];

    pub fn message_id(self) -> &'static str {
        match self {
            Self::Qualcomm => "platform-qualcomm",
            Self::MediaTek => "platform-mediatek",
        }
    }
}

/// The inputs required for a RETCN (`getNewResource`) firmware lookup.
#[derive(Debug, Clone)]
pub struct RetcnRequest {
    pub imei: String,
    pub serial_number: String,
    pub fingerprint: String,
    pub model: String,
    pub carrier: String,
    pub platform: Platform,
    /// Required for Qualcomm.
    pub fsg_version: Option<String>,
    /// Required for MediaTek (1 = Single, 2 = Dual).
    pub sim_count: Option<u8>,
}

/// Validates a RETCN IMEI: digits only, non-empty (`getfw.sh` applies no
/// Luhn check for this endpoint).
pub fn validate_imei_digits(imei: &str) -> Result<String, ImeiError> {
    let digits: String = imei
        .trim()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();

    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ImeiError::NotDigits);
    }

    Ok(digits)
}

/// Looks up the firmware resource for `imei` (ROW smartphones).
pub fn fetch_firmware(
    imei: &str,
    token: &str,
    client_uuid: &str,
) -> Result<FirmwareInfo, FirmwareError> {
    let authorization = format!("Bearer {}", token.trim().trim_start_matches("Bearer "));

    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(120))
        .build();

    let request_body = json!({
        "client": { "version": CLIENT_VERSION },
        "dparams": { "imei": imei },
        "language": "en-US",
        "windowsInfo": "Microsoft Windows 11, x64-based PC",
    });

    post_lookup(
        &agent,
        FIRMWARE_ENDPOINT,
        &authorization,
        client_uuid,
        "en-US",
        request_body,
    )
}

/// Looks up the firmware resource for a RETCN smartphone (see `getfw.sh`).
pub fn fetch_retcn_firmware(
    request: &RetcnRequest,
    token: &str,
    client_uuid: &str,
) -> Result<FirmwareInfo, FirmwareError> {
    let authorization = format!("Bearer {}", token.trim().trim_start_matches("Bearer "));

    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(120))
        .build();

    let mut params = json!({
        "fingerPrint": request.fingerprint,
        "roCarrier": request.carrier,
        "category": "phone",
    });

    match request.platform {
        Platform::Qualcomm => {
            params["fsgVersion.qcom"] = json!(request.fsg_version.clone().unwrap_or_default());
        }
        Platform::MediaTek => {
            let sim_count = match request.sim_count {
                Some(1) => "Single",
                Some(2) => "Dual",
                _ => "Single",
            };
            params["simCount"] = json!(sim_count);
        }
    }

    let request_body = json!({
        "client": { "version": CLIENT_VERSION },
        "dparams": {
            "modelName": request.model,
            "code": "0000",
            "params": params,
            "imei": request.imei,
            "imei2": "",
            "sn": request.serial_number,
            "channelId": "0x00",
            "matchType": 0,
        },
        "language": "zh-CN",
        "windowsInfo": "Microsoft Windows 11, x64-based PC",
    });

    post_lookup(
        &agent,
        RETCN_ENDPOINT,
        &authorization,
        client_uuid,
        "zh-CN",
        request_body,
    )
}

/// Looks up the firmware resource for a ROW tablet by serial number
/// (see `tablet_snget.sh`).
pub fn fetch_firmware_by_sn(
    serial_number: &str,
    token: &str,
    client_uuid: &str,
) -> Result<FirmwareInfo, FirmwareError> {
    let authorization = format!("Bearer {}", token.trim().trim_start_matches("Bearer "));

    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(120))
        .build();

    let request_body = json!({
        "client": { "version": CLIENT_VERSION },
        "dparams": { "sn": serial_number },
        "language": "en-US",
        "windowsInfo": "Microsoft Windows 11, x64-based PC",
    });

    post_lookup(
        &agent,
        TABLET_ROW_ENDPOINT,
        &authorization,
        client_uuid,
        "en-US",
        request_body,
    )
}

/// Looks up a CN tablet by serial number (no authentication required).
///
/// Returns `Ok(None)` when the CN service has no matching resource, and `Err`
/// for transport/parsing failures (see `lenovo_cn_tablet_check.sh`).
pub fn fetch_cn_tablet(sn: &str) -> Result<Option<CnTabletInfo>, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(120))
        .build();

    // 1. Resolve the MTM from the serial number.
    let encoded_sn: String = url::form_urlencoded::byte_serialize(sn.as_bytes()).collect();
    let machine_url = format!("{CN_MACHINE_ENDPOINT}?MachineNo={encoded_sn}");

    let machine_response: serde_json::Value = agent
        .post(&machine_url)
        .set("Content-Type", "application/json;charset=UTF-8")
        .call()
        .map_err(|e| format!("machine lookup failed: {e}"))?
        .into_json()
        .map_err(|e| format!("invalid machine lookup response: {e}"))?;

    if machine_response
        .get("StatusCode")
        .and_then(serde_json::Value::as_i64)
        != Some(200)
    {
        return Ok(None);
    }

    let mtm = machine_response
        .get("data")
        .and_then(|data| data.get("MTM"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "machine lookup returned no MTM".to_string())?
        .to_string();

    // 2. Query the firmware resource by MTM.
    let firmware_response: serde_json::Value = agent
        .post(CN_FIRMWARE_ENDPOINT)
        .set("Content-Type", "application/json;charset=UTF-8")
        .send_json(json!({ "mtm": mtm }))
        .map_err(|e| format!("firmware lookup failed: {e}"))?
        .into_json()
        .map_err(|e| format!("invalid firmware lookup response: {e}"))?;

    if firmware_response
        .get("code")
        .and_then(serde_json::Value::as_i64)
        != Some(200)
    {
        return Ok(None);
    }

    let item = firmware_response
        .get("data")
        .and_then(serde_json::Value::as_array)
        .and_then(|items| items.first())
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    if item.is_null() {
        return Ok(None);
    }

    let string = |key: &str| {
        item.get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string()
    };

    let mtm_compat = string("mtm")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(", ");

    let download_url = string("download_url");
    let (publish_date, file_size) = if download_url.is_empty() {
        (String::new(), String::new())
    } else {
        match fetch_download_headers(&agent, &download_url) {
            Ok(headers) => (
                headers.last_modified,
                headers.content_length.map(human_size).unwrap_or_default(),
            ),
            Err(_) => (String::new(), String::new()),
        }
    };

    Ok(Some(CnTabletInfo {
        product_name: string("product_name"),
        product_model: string("product_model"),
        market_name: string("market_name"),
        mtm_compat,
        latest_version: string("latest_version"),
        id: string("id"),
        download_url: download_url.clone(),
        publish_date,
        file_name: filename_from_url(&download_url),
        file_size,
        unzip_password: CN_UNZIP_PASSWORD.to_string(),
    }))
}

/// Shared request + parsing logic for both lookup endpoints.
fn post_lookup(
    agent: &ureq::Agent,
    endpoint: &str,
    authorization: &str,
    client_uuid: &str,
    language: &str,
    request_body: serde_json::Value,
) -> Result<FirmwareInfo, FirmwareError> {
    let fingerprint = build_fingerprint(agent, authorization, client_uuid, endpoint)
        .map_err(FirmwareError::Other)?;

    let windows_header = base64::engine::general_purpose::STANDARD.encode("Microsoft Windows 11");

    let raw_json = agent
        .post(endpoint)
        .set(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 6.3; WOW64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/51.0.2704.79 Safari/537.36",
        )
        .set("Connection", "Close")
        .set("Content-Type", "application/json")
        .set("Request-Tag", "lmsa")
        .set("Authorization", authorization)
        .set("X-Device-Fingerprint", &fingerprint)
        .set("clientUUID", client_uuid)
        .set("clientVersion", CLIENT_VERSION)
        .set("windowsInfo", &windows_header)
        .set("language", language)
        .set("Cache-Control", "no-store,no-cache")
        .set("Pragma", "no-cache")
        .send_json(request_body)
        .map_err(|e| FirmwareError::Other(format!("firmware request failed: {e}")))?
        .into_string()
        .map_err(|e| FirmwareError::Other(format!("failed to read firmware response: {e}")))?;

    let response: serde_json::Value = serde_json::from_str(&raw_json)
        .map_err(|e| FirmwareError::Other(format!("invalid firmware response: {e}")))?;

    let code = response.get("code").and_then(serde_json::Value::as_str);
    if code != Some("0000") {
        let message = format!(
            "API error {}: {}",
            code.unwrap_or("unknown"),
            response
                .get("desc")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("request failed")
        );

        // 402–409 signal an invalid or expired token (see the scripts).
        return Err(match code.and_then(|c| c.parse::<u32>().ok()) {
            Some(402..=409) => FirmwareError::AuthExpired(message),
            _ => FirmwareError::Other(message),
        });
    }

    let item = response
        .get("content")
        .and_then(serde_json::Value::as_array)
        .and_then(|items| items.first())
        .ok_or_else(|| {
            FirmwareError::Other("API returned no matching resource".to_string())
        })?;

    let mut info = parse_firmware_item(item, raw_json);

    // Fall back to the download URL's Last-Modified header when the API
    // did not provide a publish date; also read the download size.
    if !info.rom_uri.is_empty() {
        if let Ok(headers) = fetch_download_headers(agent, &info.rom_uri) {
            if info.publish_date.is_empty() {
                info.publish_date = headers.last_modified;
            }

            info.file_size = headers
                .content_length
                .map(human_size)
                .unwrap_or_default();
        }
    }

    Ok(info)
}

/// Headers of interest read from a download URL.
struct DownloadHeaders {
    last_modified: String,
    content_length: Option<u64>,
}

/// Reads the `Last-Modified` and `Content-Length` headers of a download URL
/// (HEAD first, falling back to a Range GET since some CDNs reject HEAD).
fn fetch_download_headers(
    agent: &ureq::Agent,
    url: &str,
) -> Result<DownloadHeaders, String> {
    let (is_head, response) = match agent.head(url).call() {
        Ok(response) => (true, response),
        Err(_) => (
            false,
            agent
                .get(url)
                .set("Range", "bytes=0-0")
                .call()
                .map_err(|e| format!("download URL request failed: {e}"))?,
        ),
    };

    let last_modified = response
        .header("Last-Modified")
        .unwrap_or_default()
        .to_string();

    let content_length = if is_head {
        response
            .header("Content-Length")
            .and_then(|value| value.parse::<u64>().ok())
    } else {
        // A Range GET reports the full size in `Content-Range`
        // (e.g. `bytes 0-0/123456`); `Content-Length` only covers the range.
        response
            .header("Content-Range")
            .and_then(|value| value.rsplit('/').next())
            .and_then(|total| total.parse::<u64>().ok())
            .or_else(|| {
                response
                    .header("Content-Length")
                    .and_then(|value| value.parse::<u64>().ok())
            })
    };

    Ok(DownloadHeaders {
        last_modified,
        content_length,
    })
}

/// Extracts the file name from a download URL (percent-decoded).
fn filename_from_url(url: &str) -> String {
    let path = url.split('?').next().unwrap_or(url);
    let name = path.rsplit('/').next().unwrap_or_default();

    percent_decode(name)
}

/// Decodes percent escapes (`%20` etc.) in a URL component.
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
            {
                output.push(high * 16 + low);
                index += 3;
                continue;
            }
        }

        output.push(bytes[index]);
        index += 1;
    }

    String::from_utf8_lossy(&output).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Formats a byte count for display (e.g. `1.5 GB`).
fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];

    let mut value = bytes as f64;
    let mut unit = 0;

    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        return format!("{bytes} B");
    }

    let mut text = format!("{value:.2}");
    if text.contains('.') {
        text = text.trim_end_matches('0').trim_end_matches('.').to_string();
    }

    format!("{text} {}", UNITS[unit])
}

fn parse_firmware_item(item: &serde_json::Value, raw_json: String) -> FirmwareInfo {
    let rom = item.get("romResource").unwrap_or(&serde_json::Value::Null);
    let tool = item.get("toolResource").unwrap_or(&serde_json::Value::Null);

    let string = |value: &serde_json::Value, key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };

    let rom_uri = string(rom, "uri");

    FirmwareInfo {
        market_name: string(item, "marketName"),
        model_name: string(item, "modelName"),
        sale_model: string(item, "saleModel"),
        carrier: string(item, "carrier"),
        comments: string(item, "comments"),
        publish_date: string(item, "publishDate"),
        rom_match_id: string(item, "romMatchId"),
        fingerprint: string(item, "fingerPrint"),
        rom_id: string(rom, "id"),
        rom_uri: rom_uri.clone(),
        tool_uri: string(tool, "uri"),
        file_name: filename_from_url(&rom_uri),
        file_size: String::new(),
        raw_json,
    }
}

/// Builds the `X-Device-Fingerprint` header value: RSA PKCS#1 v1.5 encryption
/// of `<timestamp_ms>|<authorization>|<interface>` (see `reference/gen.sh`).
fn build_fingerprint(
    agent: &ureq::Agent,
    authorization: &str,
    client_uuid: &str,
    endpoint: &str,
) -> Result<String, String> {
    // RSA public key from <service-root>/common/rsa.jhtml.
    let key_response: serde_json::Value = agent
        .post(RSA_KEY_ENDPOINT)
        .set("Cache-Control", "no-cache")
        .set("Request-Tag", "lmsa")
        .set("Authorization", authorization)
        .set("clientUUID", client_uuid)
        .call()
        .map_err(|e| format!("RSA key request failed: {e}"))?
        .into_json()
        .map_err(|e| format!("invalid RSA key response: {e}"))?;

    let key_text = key_response
        .get("desc")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "RSA response has no desc field".to_string())?
        .trim();

    let public_key = if key_text.contains("-----BEGIN") {
        rsa::RsaPublicKey::from_public_key_pem(key_text)
            .map_err(|e| format!("invalid RSA public key: {e}"))?
    } else {
        let der = base64::engine::general_purpose::STANDARD
            .decode(key_text)
            .map_err(|e| format!("invalid RSA key encoding: {e}"))?;

        rsa::RsaPublicKey::from_public_key_der(&der)
            .map_err(|e| format!("invalid RSA public key: {e}"))?
    };

    let timestamp_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("system clock error: {e}"))?
        .as_millis();

    let plaintext = format!("{timestamp_ms}|{authorization}|{}", interface_name(endpoint));

    let encrypted = public_key
        .encrypt(
            &mut rsa::rand_core::OsRng,
            rsa::Pkcs1v15Encrypt,
            plaintext.as_bytes(),
        )
        .map_err(|e| format!("fingerprint encryption failed: {e}"))?;

    Ok(base64::engine::general_purpose::STANDARD.encode(encrypted))
}

/// Derives the interface name used in the fingerprint plaintext:
/// `getNewResourceByImei.jhtml` → `getNewResourceByImeiinterface`.
fn interface_name(url: &str) -> String {
    let parsed = Url::parse(url).expect("endpoint is a valid URL");
    let segments: Vec<&str> = parsed.path_segments().map(|s| s.collect()).unwrap_or_default();
    let last = segments.last().copied().unwrap_or_default();

    match last.rfind('.') {
        Some(dot) if dot > 0 => format!("{}interface", &last[..dot]),
        _ => format!("{last}interface"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_imei() {
        assert_eq!(
            validate_imei("490154203237518").as_deref(),
            Ok("490154203237518")
        );
        assert_eq!(
            validate_imei(" 490154 203237518 ").as_deref(),
            Ok("490154203237518")
        );
    }

    #[test]
    fn appends_check_digit_to_14_digits() {
        assert_eq!(
            validate_imei("49015420323751").as_deref(),
            Ok("490154203237518")
        );
    }

    #[test]
    fn rejects_wrong_length() {
        assert_eq!(validate_imei("49015420323"), Err(ImeiError::WrongLength));
        assert_eq!(
            validate_imei("4901542032375"),
            Err(ImeiError::WrongLength)
        );
        assert_eq!(
            validate_imei("4901542032375180"),
            Err(ImeiError::WrongLength)
        );
    }

    #[test]
    fn rejects_non_digits() {
        assert_eq!(validate_imei("49015420323751A"), Err(ImeiError::NotDigits));
        assert_eq!(validate_imei(""), Err(ImeiError::NotDigits));
    }

    #[test]
    fn rejects_bad_checksum() {
        assert_eq!(
            validate_imei("490154203237519"),
            Err(ImeiError::BadChecksum)
        );
    }

    #[test]
    fn interface_name_strips_jhtml() {
        assert_eq!(
            interface_name(FIRMWARE_ENDPOINT),
            "getNewResourceByImeiinterface"
        );
        assert_eq!(interface_name(RETCN_ENDPOINT), "getNewResourceinterface");
        assert_eq!(
            interface_name(TABLET_ROW_ENDPOINT),
            "getNewResourceBySNinterface"
        );
    }

    #[test]
    fn human_sizes_are_readable() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(1024), "1 KB");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(3_221_225_472), "3 GB");
        assert_eq!(human_size(1_099_511_627_776), "1 TB");
    }

    #[test]
    fn extracts_file_name_from_url() {
        assert_eq!(
            filename_from_url(
                "https://cdn.example.com/roms/SAVANNAH_RETAIL_10_ROM.zip?token=abc"
            ),
            "SAVANNAH_RETAIL_10_ROM.zip"
        );
        assert_eq!(
            filename_from_url("https://cdn.example.com/roms/My%20ROM%2B1.zip"),
            "My ROM+1.zip"
        );
        assert_eq!(filename_from_url(""), "");
    }
}
