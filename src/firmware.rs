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
/// Returns which discriminator parameters a model-based lookup needs.
const MODEL_MATCH_ENDPOINT: &str =
    "https://lsa.lenovo.com/Interface/rescueDevice/getRomMatchParams.jhtml";

const CN_MACHINE_ENDPOINT: &str =
    "https://ptstpd.lenovo.com.cn/home/ConfigurationQuery/getMachineSequenceInfo";
const CN_FIRMWARE_ENDPOINT: &str =
    "https://ptstpd.lenovo.com.cn/home/ConfigurationQuery/getPadFlashingMachine";
/// Extraction password for CN tablet firmware packages.
const CN_UNZIP_PASSWORD: &str = "FC(fv:SknR";
/// Derived in `gen.sh` as `<service-root>/Interface/common/rsa.jhtml`
/// (path minus the last two segments).
const RSA_KEY_ENDPOINT: &str = "https://lsa.lenovo.com/Interface/common/rsa.jhtml";

/// The [`FirmwareError::Other`] message used when the server answered
/// successfully but had no resource to return for the IMEI.
const NO_RESOURCE_MESSAGE: &str = "API returned no matching resource";

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

/// Returns the IMEI2 value corresponding to a 15-digit `imei`: its 14-digit
/// body incremented by one, with the Luhn check digit recomputed.
///
/// A device's IMEI2 is its IMEI body + 1 (e.g. the value printed for the
/// second SIM slot), and Lenovo sometimes ties the firmware build to that
/// IMEI instead of the primary one.
fn imei2(imei: &str) -> Option<String> {
    if imei.len() != 15 || imei.bytes().any(|byte| !byte.is_ascii_digit()) {
        return None;
    }

    let body = &imei[..14];
    let value: u64 = body.parse::<u64>().ok()?.checked_add(1)?;
    Some(append_check_digit(&format!("{value:014}")))
}

/// The device codename and Android version encoded in a build fingerprint.
///
/// A Motorola fingerprint looks like
/// `motorola/naples_g_syseq/naples:16/W2WIS36.43-92-4/…`: the second field is
/// the product name and the third is `<device>:<android version>`. The
/// codename is the **product**'s first `_` token (`naples`); the device field
/// can carry a different name (`nevada` builds report `utah`, `paros` builds
/// report `sorap`), so it is not used for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FingerprintParts {
    /// The device codename, e.g. `naples` from `naples_g_syseq`.
    pub codename: String,
    /// The Android version, e.g. `16`.
    pub android_version: String,
}

/// Splits a build fingerprint into its codename and Android version.
///
/// Returns `None` when the fingerprint does not have the
/// `brand/product/device:version/…` shape (a value the device did not report,
/// or one from another vendor).
pub fn parse_fingerprint(fingerprint: &str) -> Option<FingerprintParts> {
    let mut fields = fingerprint.trim().split('/');
    let product = fields.nth(1)?;
    let device_version = fields.next()?;

    let codename = file_codename(product).trim();
    let android_version = device_version.split_once(':')?.1.trim();

    if codename.is_empty() || android_version.is_empty() {
        return None;
    }

    Some(FingerprintParts {
        codename: codename.to_string(),
        android_version: android_version.to_string(),
    })
}

/// The codename at the front of an internal file name or product name
/// (`NAPLES_G_SYS_…` → `naples`, `naples_g_syseq` → `naples`).
fn file_codename(name: &str) -> &str {
    name.split('_').next().unwrap_or_default()
}

/// True for the China channels, which all share CID `0x000B`.
fn is_china_channel(carrier: &str) -> bool {
    ["retcn", "cmcc", "ctcn"]
        .iter()
        .any(|channel| carrier.trim().eq_ignore_ascii_case(channel))
}

/// The channel the mirror spells the file name with.
///
/// Every China edition shares CID `0x000B`, so the mirror publishes a single
/// `RETCN` build for the retail, China Mobile and China Telecom channels: the
/// `cmcc` and `ctcn` codes are folded into `RETCN` here (`retcn` itself is
/// left as it is). Every other channel is upper-cased whatever case the
/// service used — the mirror's `Softbank/` directory holds
/// `XT2507-3_CYBERT_SOFTBANK_…`.
fn lolinet_carrier(carrier: &str) -> String {
    if is_china_channel(carrier) {
        "RETCN".to_string()
    } else {
        carrier.trim().to_uppercase()
    }
}

/// The channel segment of the assumed mirror directory.
///
/// Unlike the file name, the directory keeps a channel the service already
/// capitalized (`Softbank`, `TracFone`) as it is and only upper-cases a plain
/// lower-case code (`retin` → `RETIN`). A China channel becomes `RETCN`.
fn directory_carrier(carrier: &str) -> String {
    if is_china_channel(carrier) {
        return "RETCN".to_string();
    }

    let carrier = carrier.trim();
    if carrier.bytes().any(|byte| byte.is_ascii_uppercase()) {
        carrier.to_string()
    } else {
        carrier.to_uppercase()
    }
}

/// The model code the mirror uses: the XT code (`modelName`), or the sales
/// model when the service did not report one.
fn lolinet_model(info: &FirmwareInfo) -> &str {
    if info.model_name.trim().is_empty() {
        info.sale_model.trim()
    } else {
        info.model_name.trim()
    }
}

/// The product codename of `info` in upper case (e.g. `NAPLES`), from the
/// fingerprint's product field or, failing that, the internal file name.
///
/// The fingerprint's *device* field is deliberately not used: it can carry a
/// different name (`nevada` builds report `utah`).
fn lolinet_codename(info: &FirmwareInfo) -> String {
    match parse_fingerprint(&info.fingerprint) {
        Some(parts) => parts.codename.to_uppercase(),
        None => file_codename(&info.file_name).to_uppercase(),
    }
}

/// The release year assumed from a model code: `20` followed by the first two
/// digits of its number (`XT2611-1` → `2026`). `None` when `model` does not
/// start with an `XT` code.
fn model_year(model: &str) -> Option<String> {
    let model = model.trim();
    let prefix = model.get(..2)?;
    let digits = model.get(2..4)?;

    if !prefix.eq_ignore_ascii_case("XT") || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }

    Some(format!("20{digits}"))
}

/// The name mirrors.lolinet.com uses for the archive behind `info`.
///
/// The service hands out its own internal file name
/// (`NAPLES_G_SYS_W2WIS36.43-92-4_subsidy-DEFAULT_regulatory-DEFAULT_cid50_CFC.xml.zip`)
/// while the mirror lists the same build as
/// `XT2621-6_NAPLES_RETEU_16_W2WIS36.43-92-4_…`: the XT code (`modelName`,
/// **not** the sales part number `saleModel`), the product codename, the
/// channel and the Android version in front of the build id and the
/// regulatory suffix.
///
/// The original name is returned unchanged when it carries no build id, so
/// the report never holds an empty cell for a build that was found.
pub fn lolinet_filename(info: &FirmwareInfo) -> String {
    let model = lolinet_model(info);
    let codename = lolinet_codename(info);
    let android_version = parse_fingerprint(&info.fingerprint)
        .map(|parts| parts.android_version)
        .unwrap_or_default();
    let carrier = lolinet_carrier(&info.carrier);

    let tail = build_tail(&info.file_name);
    if tail.is_empty() {
        return info.file_name.clone();
    }

    let prefix = [
        model,
        codename.as_str(),
        carrier.as_str(),
        android_version.as_str(),
    ];

    let mut name = String::new();
    for part in prefix {
        if !part.is_empty() {
            name.push_str(part);
            name.push('_');
        }
    }
    name.push_str(tail);

    name
}

/// The mirrors.lolinet.com directory the archive is filed under, as far as it
/// can be assumed from `info`.
///
/// The mirror lays its Lenomola firmware out as
/// `<year>/<codename>[_retcn]/official/<channel>`: the year is the first two
/// digits of the model number (`XT2611-1` → `2026`), the codename is the
/// product codename and the trailing channel keeps the service's own
/// capitalisation (`Softbank`, `RETIN`; see [`directory_carrier`]). A China
/// channel gets the `_retcn` codename suffix (`avr` → `avr_retcn`).
///
/// Returns `None` when the model code, the codename or the channel is missing,
/// since the directory cannot be assumed then.
pub fn lolinet_directory(info: &FirmwareInfo) -> Option<String> {
    let year = model_year(lolinet_model(info))?;

    let codename = lolinet_codename(info).to_lowercase();
    let carrier = directory_carrier(&info.carrier);

    if codename.is_empty() || carrier.is_empty() {
        return None;
    }

    let codename_directory = if is_china_channel(&info.carrier) {
        format!("{codename}_retcn")
    } else {
        codename
    };

    Some(format!("{year}/{codename_directory}/official/{carrier}"))
}

/// The build-id-and-suffix tail of an internal file name: everything after
/// the `_SYS_` marker, or from the first `_`-separated token that carries a
/// dot (the build id always does, e.g. `W1VVC36H.7-73-6-3`) when that marker
/// is absent.
///
/// `CYBERT_G_SYS_A171VVH.36-23_subsidy-DEFAULT_…xml.zip`
/// → `A171VVH.36-23_subsidy-DEFAULT_…xml.zip`.
fn build_tail(file_name: &str) -> &str {
    const MARKER: &str = "_SYS_";

    if let Some(index) = file_name.find(MARKER) {
        return &file_name[index + MARKER.len()..];
    }

    // The build id carries a dot while the codename may well carry digits
    // (`AITO25`), so "contains a digit" alone would cut too early.
    first_token_matching(file_name, |token| token.contains('.')).unwrap_or(file_name)
}

/// The `file_name` slice starting at the first `_`-separated token that
/// `matches`, or `None` when no token does.
fn first_token_matching(
    file_name: &str,
    matches: impl Fn(&str) -> bool,
) -> Option<&str> {
    let mut start = 0;
    for token in file_name.split('_') {
        if matches(token) {
            return Some(&file_name[start..]);
        }
        start += token.len() + 1;
    }

    None
}

/// True when a lookup error means "the server has no build for this IMEI"
/// (business code `1000`, or a successful answer with an empty resource list)
/// rather than a transport or authentication failure.
pub fn is_no_resource(error: &FirmwareError) -> bool {
    api_error_code(error) == Some(1000)
        || matches!(error, FirmwareError::Other(message) if message == NO_RESOURCE_MESSAGE)
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

/// Device category used by the model-based lookup (`--category` in
/// `reference/motofw.py`). The server matches tablets/smart devices by
/// country code in addition to model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Category {
    #[default]
    Phone,
    Tablet,
    Smart,
}

impl Category {
    pub const ALL: [Self; 3] = [Self::Phone, Self::Tablet, Self::Smart];

    pub fn message_id(self) -> &'static str {
        match self {
            Self::Phone => "category-phone",
            Self::Tablet => "category-tablet",
            Self::Smart => "category-smart",
        }
    }

    /// The lowercase value sent to the server (`category` parameter).
    pub fn code(self) -> &'static str {
        match self {
            Self::Phone => "phone",
            Self::Tablet => "tablet",
            Self::Smart => "smart",
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
///
/// The by-IMEI endpoint answers with business code `1000` when no build is
/// tied to the given IMEI. Lenovo usually ties the firmware to the device's
/// IMEI2 instead, so on a `1000` this retries ONCE with the IMEI2 value
/// ([`imei2`]: 14-digit body + 1 with the Luhn check digit recomputed).
pub fn fetch_firmware(
    imei: &str,
    token: &str,
    client_uuid: &str,
) -> Result<FirmwareInfo, FirmwareError> {
    let first = lookup_imei_once(imei, token, client_uuid);

    // `1000` = no matching resource for this IMEI. The entered value is
    // usually the IMEI1 while the build is tied to the IMEI2, so try the
    // IMEI2 once (no further retries if that also fails).
    if matches!(&first, Err(error) if api_error_code(error) == Some(1000)) {
        if let Some(alt) = imei2(imei) {
            #[cfg(debug_assertions)]
            eprintln!(
                "[firmware] IMEI {imei} returned code 1000; retrying once with IMEI2 {alt}"
            );

            return lookup_imei_once(&alt, token, client_uuid);
        }
    }

    first
}

/// Runs a single by-IMEI lookup attempt (no IMEI2 retry).
fn lookup_imei_once(
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

/// The parameters the server needs to tell one model's builds apart.
#[derive(Debug, Clone)]
pub struct ModelMatchParams {
    /// Discriminator parameter names required for a model lookup.
    pub params: Vec<String>,
}

/// Looks up which discriminator parameters `getNewResource` needs for a
/// model-based lookup (see `reference/motofw.py find`).
pub fn fetch_model_match_params(
    model: &str,
    token: &str,
    client_uuid: &str,
) -> Result<ModelMatchParams, FirmwareError> {
    let authorization = format!("Bearer {}", token.trim().trim_start_matches("Bearer "));

    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(120))
        .build();

    let request_body = json!({
        "client": { "version": CLIENT_VERSION },
        "dparams": { "modelName": model },
        "language": "en-US",
        "windowsInfo": "Microsoft Windows 11, x64-based PC",
    });

    let api = api_post(
        &agent,
        MODEL_MATCH_ENDPOINT,
        &authorization,
        client_uuid,
        "en-US",
        request_body,
    )?;

    let params = api
        .content
        .get("params")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    Ok(ModelMatchParams { params })
}

/// Looks up the firmware resource for `model` with the collected
/// discriminator parameters (see `reference/motofw.py find`).
///
/// `category` and `country_code` follow `_seed_params`: phones only carry
/// `category`, while tablets/smart devices also send `countryCode`.
pub fn fetch_firmware_by_model(
    model: &str,
    category: Category,
    country_code: &str,
    params: &[(String, String)],
    token: &str,
    client_uuid: &str,
) -> Result<FirmwareInfo, FirmwareError> {
    let authorization = format!("Bearer {}", token.trim().trim_start_matches("Bearer "));

    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(120))
        .build();

    let mut params_object = serde_json::Map::new();
    for (key, value) in params {
        params_object.insert(key.clone(), serde_json::Value::String(value.clone()));
    }
    params_object.insert(
        "category".to_string(),
        serde_json::Value::String(category.code().to_string()),
    );
    if category != Category::Phone && !country_code.is_empty() {
        params_object.insert(
            "countryCode".to_string(),
            serde_json::Value::String(country_code.to_string()),
        );
    }

    let request_body = json!({
        "client": { "version": CLIENT_VERSION },
        "dparams": {
            "modelName": model,
            "code": "0000",
            "params": serde_json::Value::Object(params_object),
            "imei": "",
            "imei2": "",
            "sn": "",
            "matchType": 1,
        },
        "language": "en-US",
        "windowsInfo": "Microsoft Windows 11, x64-based PC",
    });

    post_lookup(
        &agent,
        RETCN_ENDPOINT,
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

/// The authenticated POST response envelope: `content` payload and the raw
/// JSON (preserved for the "copy raw response" button).
struct ApiResponse {
    content: serde_json::Value,
    raw_json: String,
}

/// Shared authenticated POST for the LSA endpoints: builds the device
/// fingerprint, sends the request and checks the business result code
/// (402–409 → [`FirmwareError::AuthExpired`]).
fn api_post(
    agent: &ureq::Agent,
    endpoint: &str,
    authorization: &str,
    client_uuid: &str,
    language: &str,
    request_body: serde_json::Value,
) -> Result<ApiResponse, FirmwareError> {
    let fingerprint = build_fingerprint(agent, authorization, client_uuid, endpoint)
        .map_err(FirmwareError::Other)?;

    let windows_header = base64::engine::general_purpose::STANDARD.encode("Microsoft Windows 11");

    #[cfg(debug_assertions)]
    eprintln!("[firmware] POST {endpoint} body: {request_body}");

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

    #[cfg(debug_assertions)]
    eprintln!("[firmware] POST {endpoint} response: {raw_json}");

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

    Ok(ApiResponse {
        content: response
            .get("content")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        raw_json,
    })
}

/// The LSA business result code behind a non-auth [`FirmwareError`] from
/// [`api_post`], if any. [`api_post`] renders every non-`0000` business code
/// as `API error <code>: <desc>`, which this helper re-parses (used to
/// trigger the IMEI2 retry in [`fetch_firmware`] on code `1000`).
fn api_error_code(error: &FirmwareError) -> Option<u32> {
    let FirmwareError::Other(message) = error else {
        return None;
    };

    message
        .strip_prefix("API error ")
        .and_then(|rest| rest.split(':').next())
        .and_then(|code| code.trim().parse().ok())
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
    let api = api_post(agent, endpoint, authorization, client_uuid, language, request_body)?;

    let item = api
        .content
        .as_array()
        .and_then(|items| items.first())
        .ok_or_else(|| FirmwareError::Other(NO_RESOURCE_MESSAGE.to_string()))?;

    let mut info = parse_firmware_item(item, api.raw_json);

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

/// Motorola's "does this device qualify?" endpoint. The three path segments
/// are taken from the Device ID (see [`unlock_eligibility_url`]).
const VERIFY_PHONE_BASE: &str =
    "https://en-us.support.motorola.com/cc/productRegistration/verifyPhone";

/// Builds the URL used to ask Motorola whether a device qualifies for
/// bootloader unlock.
///
/// The Device ID (from `fastboot oem get_unlock_data`) is split on `#`; the
/// request path uses the 1st, 4th and 3rd chunks, e.g.
/// `…/verifyPhone/<chunk0>/<chunk3>/<chunk2>/`.
pub fn unlock_eligibility_url(device_id: &str) -> Result<String, String> {
    let parts: Vec<&str> = device_id.split('#').collect();
    if parts.len() < 4 {
        return Err(
            "Device ID has fewer than the 4 expected `#`-separated parts".to_string(),
        );
    }

    Ok(format!("{}/{}/{}/{}/", VERIFY_PHONE_BASE, parts[0], parts[3], parts[2]))
}

/// Queries Motorola whether the device qualifies for bootloader unlock.
/// Returns `Ok(true)` when the phone qualifies and `Ok(false)` when it does
/// not.
pub fn check_unlock_eligibility(device_id: &str) -> Result<bool, String> {
    let url = unlock_eligibility_url(device_id)?;
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(30))
        .build();

    let body = agent
        .get(&url)
        .call()
        .map_err(|e| format!("unlock eligibility request failed: {e}"))?
        .into_string()
        .map_err(|e| format!("failed to read unlock eligibility response: {e}"))?;

    let body = body.to_ascii_lowercase();
    if body.contains("not qualified") {
        Ok(false)
    } else if body.contains("qualif") {
        Ok(true)
    } else {
        Err("unexpected unlock eligibility response".to_string())
    }
}

/// Motorola's "request the unlock key" endpoint (also uses the 1st, 4th and
/// 3rd `#`-separated chunks of the Device ID).
const UNLOCK_REQUEST_BASE: &str =
    "https://en-us.support.motorola.com/cc/productRegistration/unlockPhone";

/// The logged-in account/profile page (parsed for the display name and the
/// e-mail).
pub const PORTAL_PROFILE_URL: &str = "https://en-us.support.motorola.com/app/account/profile";

/// Logs the Motorola portal session out. Used before a "Change Account" so the
/// login page actually appears instead of the webview reusing the still-valid
/// session and bouncing straight to the account profile.
const PORTAL_LOGOUT_URL: &str =
    "https://en-us.support.motorola.com/cc/logoutCustom/doLogout";

/// The Motorola portal login entry (redirects to the login page, then back to
/// the bootloader unlock page once authenticated).
pub const PORTAL_LOGIN_URL: &str =
    "https://en-us.support.motorola.com/app/utils/welcome/redirect/standalone%252Fbootloader%252Funlock-your-device-b";

/// The page the portal redirects to after an unlock key was successfully
/// requested.
const UNLOCK_REQUEST_SUCCESS_HINT: &str = "unlock-your-device-c";

/// Builds the URL that requests an unlock key for the device (needs a logged-in
/// portal session cookie).
pub fn unlock_request_url(device_id: &str) -> Result<String, String> {
    let parts: Vec<&str> = device_id.split('#').collect();
    if parts.len() < 4 {
        return Err(
            "Device ID has fewer than the 4 expected `#`-separated parts".to_string(),
        );
    }

    Ok(format!(
        "{}/{}/{}/{}/",
        UNLOCK_REQUEST_BASE, parts[0], parts[3], parts[2]
    ))
}

/// Returns the text between `open` and `close` (first occurrence).
fn html_between<'a>(haystack: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = haystack.find(open)? + open.len();
    let rest = &haystack[start..];
    let end = rest.find(close)?;
    Some(&rest[..end])
}

/// Fetches the logged-in account name and e-mail from the portal profile page
/// using the session cookie captured by the webview login.
pub fn fetch_portal_profile(cookie_header: &str) -> Result<(String, String), String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(30))
        .build();

    let response = agent
        .get(PORTAL_PROFILE_URL)
        .set("Cookie", cookie_header)
        .call()
        .map_err(|e| format!("portal profile request failed: {e}"))?;

    // Only the debug log below needs these; keeping them behind the same cfg
    // avoids "unused variable" warnings in release builds.
    #[cfg(debug_assertions)]
    let (status, final_url) = (response.status(), response.get_url().to_string());

    let body = response
        .into_string()
        .map_err(|e| format!("failed to read portal profile response: {e}"))?;

    #[cfg(debug_assertions)]
    {
        eprintln!(
            "[portal-profile] GET {PORTAL_PROFILE_URL}\n  -> {final_url} (http {status}, {} bytes)",
            body.len()
        );
        let cookie_count = cookie_header
            .split(';')
            .filter(|part| !part.trim().is_empty())
            .count();
        eprintln!("[portal-profile] sent {cookie_count} cookie part(s)");

        let has_name = body.contains("loggedin_div_img_name");
        let has_mail = body.contains("mailto:");
        eprintln!("[portal-profile] name marker={has_name}; mailto marker={has_mail}");
        if !has_name {
            // The page Motorola returned didn't contain the expected marker —
            // show its start so the user can see what it actually is (e.g. a
            // login/error page instead of the profile page).
            let preview: String = body.chars().take(1500).collect();
            eprintln!("[portal-profile] page preview:\n{preview}");
        }
    }

    let name = html_between(&body, "loggedin_div_img_name\">", "</div>")
        .map(|text| text.trim().to_string())
        .ok_or_else(|| "profile page did not expose the logged-in name".to_string())?;

    let email = html_between(&body, "mailto:", "\"")
        .map(|text| text.trim().to_string())
        .ok_or_else(|| "profile page did not expose the logged-in e-mail".to_string())?;

    Ok((name, email))
}

/// Logs the current Motorola portal session out using the session cookie.
///
/// Once the old session is invalid server-side, the next portal login shows
/// its sign-in page again (and lets the user pick a different account)
/// instead of instantly redirecting to the logged-in profile.
pub fn logout_portal(cookie_header: &str) -> Result<(), String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(30))
        .redirects(5)
        .build();

    let response = agent
        .post(PORTAL_LOGOUT_URL)
        .set(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/152.0.0.0 Safari/537.36",
        )
        .set("Accept", "application/json, text/javascript, */*; q=0.01")
        .set("Content-Type", "application/x-www-form-urlencoded; charset=UTF-8")
        .set("Origin", "https://en-us.support.motorola.com")
        .set("Referer", PORTAL_PROFILE_URL)
        .set("X-Requested-With", "XMLHttpRequest")
        .set("Cookie", cookie_header)
        .send_string("currentUrl=%2Fapp%2Faccount%2Fprofile&redirectUrl=%2Fapp%2Fhome")
        .map_err(|e| format!("portal logout failed: {e}"))?;

    let status = response.status();
    if (200..400).contains(&status) {
        Ok(())
    } else {
        Err(format!("portal logout returned http {status}"))
    }
}

/// Requests the unlock key for the device, retrying up to 10 times when the
/// portal answers with its `0020` error page. Succeeds when the portal
/// redirects to the "request done" page.
pub fn request_unlock_key(device_id: &str, cookie_header: &str) -> Result<(), String> {
    let url = unlock_request_url(device_id)?;
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(30))
        .redirects(20)
        .build();

    for _ in 0..10 {
        let response = agent
            .get(&url)
            .set("Cookie", cookie_header)
            .call()
            .map_err(|e| format!("unlock key request failed: {e}"))?;
        let final_url = response.get_url().to_string();
        let body = response
            .into_string()
            .map_err(|e| format!("failed to read unlock key response: {e}"))?;

        let haystack = format!("{final_url} {body}").to_ascii_lowercase();
        if haystack.contains(UNLOCK_REQUEST_SUCCESS_HINT) {
            return Ok(());
        }
        if haystack.contains("error_id/0020") {
            // Retryable error page: try again.
            continue;
        }
        // Any other landing page: treat as retryable too.
    }

    Err("Bootloader Unlock Key cannot be requested".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_unlock_request_url() {
        let device_id = "3A95915042649321#5A5932324B525834524B006D6F746F726F6C0000#8226946C1000ED2C2C9D9A3A981F063D80A30825CA264A20E51C1EDCC25BD0C4#10CAE512002750E10000000000000000";

        assert_eq!(
            unlock_request_url(device_id).unwrap(),
            "https://en-us.support.motorola.com/cc/productRegistration/unlockPhone/3A95915042649321/10CAE512002750E10000000000000000/8226946C1000ED2C2C9D9A3A981F063D80A30825CA264A20E51C1EDCC25BD0C4/"
        );
    }

    #[test]
    fn parses_portal_profile_html() {
        let html = r#"<div class="loggedin_div_img_name">Calyx Hikari</div></div>
            <h2 class="user-data-mail" data-cs-mask>
                <a href="mailto:mega98x@gmail.com">mega98x@gmail.com</a>
            </h2>"#;

        assert_eq!(
            html_between(html, "loggedin_div_img_name\">", "</div>"),
            Some("Calyx Hikari")
        );
        assert_eq!(
            html_between(html, "mailto:", "\""),
            Some("mega98x@gmail.com")
        );
    }


    #[test]
    fn builds_unlock_eligibility_url() {
        let device_id = "3A95915042649321#5A5932324B525834524B006D6F746F726F6C0000#8226946C1000ED2C2C9D9A3A981F063D80A30825CA264A20E51C1EDCC25BD0C4#10CAE512002750E10000000000000000";

        assert_eq!(
            unlock_eligibility_url(device_id).unwrap(),
            "https://en-us.support.motorola.com/cc/productRegistration/verifyPhone/3A95915042649321/10CAE512002750E10000000000000000/8226946C1000ED2C2C9D9A3A981F063D80A30825CA264A20E51C1EDCC25BD0C4/"
        );
    }

    #[test]
    fn rejects_malformed_unlock_eligibility_url() {
        assert!(unlock_eligibility_url("only-two#parts").is_err());
    }

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
    fn imei2_increments_body_and_recomputes_check_digit() {
        // 35807117181431 + 1 → 35807117181432, Luhn check digit 9.
        assert_eq!(
            imei2("358071171814311").as_deref(),
            Some("358071171814329")
        );
        // The IMEI2 result must itself be a valid IMEI.
        assert_eq!(
            validate_imei("358071171814329").as_deref(),
            Ok("358071171814329")
        );
    }

    #[test]
    fn imei2_carries_through_trailing_nines() {
        // …399 + 1 → …400 (carry through the nines), check digit 6.
        assert_eq!(imei2("358071171813990").as_deref(), Some("358071171814006"));
    }

    #[test]
    fn imei2_requires_a_15_digit_numeric_imei() {
        assert_eq!(imei2("1234"), None);
        assert_eq!(imei2("49015420323751"), None); // 14 digits
        assert_eq!(imei2("49015420323751A"), None); // 15 chars, non-digit
    }

    #[test]
    fn api_error_code_reparses_the_business_code() {
        assert_eq!(
            api_error_code(&FirmwareError::Other(
                "API error 1000: no such IMEI".to_string()
            )),
            Some(1000)
        );
        assert_eq!(
            api_error_code(&FirmwareError::Other("API error 400: bad request".to_string())),
            Some(400)
        );
        assert_eq!(api_error_code(&FirmwareError::Other("boom".to_string())), None);
        // Auth-expired errors never carry a parseable business code here.
        assert_eq!(
            api_error_code(&FirmwareError::AuthExpired("API error 403: nope".to_string())),
            None
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
        assert_eq!(
            interface_name(MODEL_MATCH_ENDPOINT),
            "getRomMatchParamsinterface"
        );
    }

    #[test]
    fn category_codes_are_lowercase() {
        assert_eq!(Category::Phone.code(), "phone");
        assert_eq!(Category::Tablet.code(), "tablet");
        assert_eq!(Category::Smart.code(), "smart");
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

    /// Builds a [`FirmwareInfo`] with everything empty but the given fields.
    fn firmware(name: &str, sale_model: &str, carrier: &str, fingerprint: &str) -> FirmwareInfo {
        FirmwareInfo {
            market_name: String::new(),
            model_name: name.to_string(),
            sale_model: sale_model.to_string(),
            carrier: carrier.to_string(),
            comments: String::new(),
            publish_date: String::new(),
            rom_match_id: String::new(),
            fingerprint: fingerprint.to_string(),
            rom_id: String::new(),
            rom_uri: String::new(),
            tool_uri: String::new(),
            file_name: name.to_string(),
            file_size: String::new(),
            raw_json: String::new(),
        }
    }

    #[test]
    fn parses_codename_and_version_from_a_fingerprint() {
        let parts = parse_fingerprint(
            "motorola/cybert/cybert:17/U1TQS34.28-11/3b6f4b:user/release-keys",
        )
        .expect("fingerprint should parse");

        assert_eq!(parts.codename, "cybert");
        assert_eq!(parts.android_version, "17");

        // Values without the `brand/product/device:version` shape are refused
        // instead of yielding a half-filled name.
        assert!(parse_fingerprint("").is_none());
        assert!(parse_fingerprint("cybert").is_none());
        assert!(parse_fingerprint("motorola/cybert").is_none());
        assert!(parse_fingerprint("motorola/cybert/cybert").is_none());
    }

    #[test]
    fn renames_an_api_file_to_the_lolinet_name() {
        let mut info = firmware(
            "XT2507-4",
            "XT2507-4",
            "retin",
            "motorola/cybert/cybert:17/U1TQS34.28-11/3b6f4b:user/release-keys",
        );
        info.file_name =
            "CYBERT_G_SYS_A171VVH.36-23_subsidy-DEFAULT_regulatory-DEFAULT_cid50_CFC.xml.zip"
                .to_string();

        assert_eq!(
            lolinet_filename(&info),
            "XT2507-4_CYBERT_RETIN_17_A171VVH.36-23_subsidy-DEFAULT_regulatory-DEFAULT_cid50_CFC.xml.zip"
        );
    }

    /// Every China edition shares CID `0x000B`, so the mirror publishes one
    /// `RETCN` build: the `cmcc` and `ctcn` channels are folded into `RETCN`
    /// while every other channel is left alone.
    #[test]
    fn folds_the_china_channels_into_retcn() {
        let mut info = firmware(
            "XT2437-4",
            "",
            "cmcc",
            "motorola/paros_cn/sorap:16/W1UQ36H.1-57-9/4beb56-8d0fe:user/release-keys",
        );
        info.file_name =
            "PAROS_CN_W1UQ36H.1-57-9_subsidy-DEFAULT_regulatory-DEFAULT_CFC.xml.zip"
                .to_string();

        assert_eq!(
            lolinet_filename(&info),
            "XT2437-4_PAROS_RETCN_16_W1UQ36H.1-57-9_subsidy-DEFAULT_regulatory-DEFAULT_CFC.xml.zip"
        );

        info.carrier = "ctcn".to_string();
        assert!(lolinet_filename(&info).contains("_PAROS_RETCN_16_"));

        // A non-China channel keeps its own name.
        info.carrier = "retin".to_string();
        assert!(lolinet_filename(&info).contains("_PAROS_RETIN_16_"));
    }

    /// The assumed mirror directory is
    /// `<year>/<codename>[_retcn]/official/<CARRIER>`: the year is the first
    /// two digits of the XT code and a China channel adds the `_retcn`
    /// codename suffix.
    #[test]
    fn assumes_the_mirror_directory() {
        let mut naples = firmware(
            "XT2621-6",
            "PBBS0003SE",
            "reteu",
            "motorola/naples_g_syseq/naples:16/W2WIS36.43-92-4/160cd:user/release-keys",
        );
        naples.file_name =
            "NAPLES_G_SYS_W2WIS36.43-92-4_subsidy-DEFAULT_regulatory-DEFAULT_cid50_CFC.xml.zip"
                .to_string();
        assert_eq!(
            lolinet_directory(&naples).as_deref(),
            Some("2026/naples/official/RETEU")
        );

        // A China channel gets the `_retcn` codename suffix (`XT2611-1` /
        // `avr` → `2026/avr_retcn/official/RETCN`).
        let mut avr = firmware(
            "XT2611-1",
            "",
            "retcn",
            "motorola/avr_retcn/avr_retcn:16/U1UXS36.1-1/abcd:user/release-keys",
        );
        avr.file_name = "AVR_RETCN_U1UXS36.1-1_subsidy-DEFAULT_CFC.xml.zip".to_string();
        assert_eq!(
            lolinet_directory(&avr).as_deref(),
            Some("2026/avr_retcn/official/RETCN")
        );

        // `cmcc` is folded to `RETCN` as well.
        let mut paros = firmware(
            "XT2437-4",
            "",
            "cmcc",
            "motorola/paros_cn/sorap:16/W1UQ36H.1-57-9/4beb56:user/release-keys",
        );
        paros.file_name =
            "PAROS_CN_W1UQ36H.1-57-9_subsidy-DEFAULT_regulatory-DEFAULT_CFC.xml.zip"
                .to_string();
        assert_eq!(
            lolinet_directory(&paros).as_deref(),
            Some("2024/paros_retcn/official/RETCN")
        );

        // A channel the service already capitalized (`Softbank`) is kept as it
        // is in the directory, while the file name stays upper-cased.
        let mut softbank = firmware(
            "XT2507-3",
            "",
            "Softbank",
            "motorola/cybert/cybert:15/V2VVA35.58-46-4/abcd:user/release-keys",
        );
        softbank.file_name =
            "CYBERT_SOFTBANK_V2VVA35.58-46-4_subsidy-DEFAULT_regulatory-DEFAULT_cid50_CFC.xml.zip"
                .to_string();
        assert_eq!(
            lolinet_directory(&softbank).as_deref(),
            Some("2025/cybert/official/Softbank")
        );
        assert_eq!(
            lolinet_filename(&softbank),
            "XT2507-3_CYBERT_SOFTBANK_15_V2VVA35.58-46-4_subsidy-DEFAULT_regulatory-DEFAULT_cid50_CFC.xml.zip"
        );

        // No XT code → no year → no assumed directory.
        let mut unknown = firmware(
            "TB350FU",
            "",
            "reteu",
            "motorola/naples_g_syseq/naples:16/W2WIS36.43-92-4/160cd:user/release-keys",
        );
        unknown.file_name = "NAPLES_G_SYS_W2WIS36.43-92-4_x.zip".to_string();
        assert_eq!(lolinet_directory(&unknown), None);
    }

    /// The XT code (`modelName`) is the model in front of the name — never the
    /// sales part number (`saleModel`), which is what the API returns for e.g.
    /// `PBBS0003SE` and which the mirror does not use.
    #[test]
    fn uses_the_xt_code_not_the_sale_model() {
        let mut info = firmware(
            "XT2621-6",
            "PBBS0003SE",
            "reteu",
            "motorola/naples_g_syseq/naples:16/W2WIS36.43-92-4/160cd-e8cff0:user/release-keys",
        );
        info.file_name =
            "NAPLES_G_SYS_W2WIS36.43-92-4_subsidy-DEFAULT_regulatory-DEFAULT_cid50_CFC.xml.zip"
                .to_string();

        assert_eq!(
            lolinet_filename(&info),
            "XT2621-6_NAPLES_RETEU_16_W2WIS36.43-92-4_subsidy-DEFAULT_regulatory-DEFAULT_cid50_CFC.xml.zip"
        );
    }

    /// The codename comes from the fingerprint's PRODUCT field, not its device
    /// field: a `nevada_g_sysu` build reports the device `utah`, but the
    /// mirror still calls it `NEVADA`.
    #[test]
    fn takes_the_codename_from_the_product_not_the_device() {
        let mut info = firmware(
            "XT2613-1",
            "610214688644",
            "tmo",
            "motorola/nevada_g_sysu/utah:16/W1WNS36.18-114-3-1-2/0022e6-e652657:user/release-keys",
        );
        info.file_name =
            "NEVADA_G_SYS_W1WNS36.18-114-3-1-2_subsidy-TMO_USKU_RSU_regulatory-DEFAULT_cid50_CFC.xml.zip"
                .to_string();

        assert_eq!(
            lolinet_filename(&info),
            "XT2613-1_NEVADA_TMO_16_W1WNS36.18-114-3-1-2_subsidy-TMO_USKU_RSU_regulatory-DEFAULT_cid50_CFC.xml.zip"
        );
    }

    /// Without the `_SYS_` marker the tail starts at the first token carrying
    /// a dot (the build id), so a codename that itself carries digits
    /// (`AITO25`) is not eaten by the `A171VVH.36-23`-style match. A name with
    /// no build id at all is reported unchanged.
    #[test]
    fn falls_back_to_the_first_token_with_a_dot() {
        let mut info = firmware(
            "XT2507-4",
            "XT2507-4",
            "retin",
            "motorola/cybert/cybert:17/U1TQS34.28-11/3b6f4b:user/release-keys",
        );

        info.file_name = "CYBERT_A171VVH.36-23_subsidy-DEFAULT.xml.zip".to_string();
        assert_eq!(
            lolinet_filename(&info),
            "XT2507-4_CYBERT_RETIN_17_A171VVH.36-23_subsidy-DEFAULT.xml.zip"
        );

        // A codename that itself carries digits must survive.
        let mut aito = firmware(
            "XT2553-2",
            "XT2553-2",
            "retcn",
            "motorola/aito25_retcn/aito25_retcn:16/V2VVC35.58-33-5/1234:user/release-keys",
        );
        aito.file_name = "AITO25_RETCN_V2VVC35.58-33-5_subsidy-DEFAULT_CFC.xml.zip".to_string();
        assert_eq!(
            lolinet_filename(&aito),
            "XT2553-2_AITO25_RETCN_16_V2VVC35.58-33-5_subsidy-DEFAULT_CFC.xml.zip"
        );

        // Nothing to rename: the empty name stays empty.
        info.file_name = String::new();
        assert_eq!(lolinet_filename(&info), "");
    }
}
