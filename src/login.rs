//! Lenovo login URL generation and token extraction, ported from
//! `reference/login.sh`.

use serde_json::json;
use url::Url;

use base64::Engine as _;

const BASE_URL: &str = "https://lsa.lenovo.com";
const CLIENT_VERSION: &str = "7.6.2.10";

/// Lenovo ID language for the given OS locale string.
///
/// The service accepts locale codes like `zh_CN` / `zh_TW` / `en_US`;
/// Chinese maps to Traditional for Taiwan/Hong Kong/Macau and Simplified
/// otherwise, matching the app's UI languages.
fn language_code_for_locale(locale: &str) -> &'static str {
    let normalized = locale.replace('_', "-").to_ascii_lowercase();

    let Some(rest) = normalized.strip_prefix("zh") else {
        return "en_US";
    };

    if rest.contains("hant")
        || rest.starts_with("-tw")
        || rest.starts_with("-hk")
        || rest.starts_with("-mo")
    {
        "zh_TW"
    } else {
        "zh_CN"
    }
}

/// Lenovo ID language based on the OS language.
fn language_code() -> &'static str {
    language_code_for_locale(&sys_locale::get_locale().unwrap_or_default())
}

/// Requests the login URL from the Lenovo service (see `reference/login.sh`).
pub fn fetch_login_url(client_uuid: &str) -> Result<String, String> {
    let language = language_code();
    let language_header = language.replace('_', "-");
    let windows_header = base64::engine::general_purpose::STANDARD.encode("Windows 10");

    let request_body = json!({
        "client": { "version": CLIENT_VERSION },
        "dparams": { "key": "TIP_URL" },
        "language": language_header,
        "windowsInfo": "Windows 10, x64-based PC",
    });

    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(30))
        .build();

    let response: serde_json::Value = agent
        .post(&format!("{BASE_URL}/Interface/dictionary/getApiInfo.jhtml"))
        .set("Content-Type", "application/json")
        .set("Cache-Control", "no-cache")
        .set("Request-Tag", "lmsa")
        .set("clientVersion", CLIENT_VERSION)
        .set("language", &language_header)
        .set("windowsInfo", &windows_header)
        .set("clientUUID", client_uuid)
        .send_json(request_body)
        .map_err(|e| format!("login URL request failed: {e}"))?
        .into_json()
        .map_err(|e| format!("invalid login URL response: {e}"))?;

    if response.get("code").and_then(serde_json::Value::as_str) != Some("0000") {
        return Err(format!(
            "TIP_URL request failed: {}",
            response
                .get("desc")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown error")
        ));
    }

    let login_url = response
        .get("content")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "TIP_URL response has no login URL".to_string())?;

    add_login_params(login_url)
}

/// Appends the language and login prompt parameters and validates the `state`.
fn add_login_params(login_url: &str) -> Result<String, String> {
    let mut url = Url::parse(login_url).map_err(|e| format!("login URL is invalid: {e}"))?;

    {
        let mut query = url.query_pairs_mut();
        query.append_pair("lenovoid.lang", language_code());
        query.append_pair("prompt", "login");
    }

    let has_state = url
        .query_pairs()
        .any(|(key, value)| key == "state" && !value.is_empty());
    if !has_state {
        return Err("generated login URL has no valid state".to_string());
    }

    Ok(url.to_string())
}

/// The result of a successful login: the Authorization token and, if present,
/// the account display name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginInfo {
    pub token: String,
    pub full_name: Option<String>,
}

/// Extracts the Authorization token and account name from a pasted callback.
///
/// Accepts the raw `SoftwareFix://callback?Authorization=…&fullName=…` URL or
/// a JSON envelope containing it (as produced by some login pages).
pub fn extract_login(input: &str) -> Result<LoginInfo, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("callback URL is empty".to_string());
    }

    let callback = if trimmed.starts_with('{') {
        let value: serde_json::Value = serde_json::from_str(trimmed)
            .map_err(|e| format!("invalid JSON callback: {e}"))?;

        ["content", "msg", "desc"]
            .iter()
            .filter_map(|key| value.get(*key).and_then(serde_json::Value::as_str))
            .find(|s| s.to_ascii_lowercase().starts_with("softwarefix://"))
            .map(str::to_owned)
            .ok_or_else(|| "JSON contains no SoftwareFix callback URL".to_string())?
    } else {
        trimmed.to_owned()
    };

    let url = Url::parse(&callback).map_err(|e| format!("invalid callback URL: {e}"))?;

    let scheme = url.scheme().to_ascii_lowercase();
    if matches!(scheme.as_str(), "http" | "https") {
        return Err(
            "browser success-page URLs cannot be reused; paste the SoftwareFix://callback URL"
                .to_string(),
        );
    }
    if scheme != "softwarefix"
        || url.host_str().map(str::to_ascii_lowercase).as_deref() != Some("callback")
    {
        return Err("invalid SoftwareFix callback".to_string());
    }

    let mut errors = Vec::new();
    let mut tokens = Vec::new();
    let mut full_names = Vec::new();
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "error" => errors.push(value.into_owned()),
            "Authorization" => tokens.push(value.into_owned()),
            "fullName" => full_names.push(value.into_owned()),
            _ => {}
        }
    }

    if let Some(error) = errors.into_iter().find(|e| !e.is_empty()) {
        return Err(format!("login failed: {error}"));
    }
    if tokens.len() != 1 || tokens[0].is_empty() {
        return Err("SoftwareFix callback has no single Authorization value".to_string());
    }

    let mut token = tokens.into_iter().next().expect("token checked above");
    if token.len() >= 7 && token[..7].eq_ignore_ascii_case("bearer ") {
        token = token[7..].to_owned();
    }

    let full_name = full_names.into_iter().find(|name| !name.is_empty());

    Ok(LoginInfo { token, full_name })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_callback_url() {
        let info = extract_login(
            "SoftwareFix://callback?Authorization=Bearer%20abc123&fullName=Zhang%20San",
        )
        .expect("callback should parse");

        assert_eq!(info.token, "abc123");
        assert_eq!(info.full_name.as_deref(), Some("Zhang San"));
    }

    #[test]
    fn parses_json_envelope() {
        let info = extract_login(
            r#"{"code":"0000","desc":"SoftwareFix://callback?Authorization=abc123&fullName=Test"}"#,
        )
        .expect("json envelope should parse");

        assert_eq!(info.token, "abc123");
        assert_eq!(info.full_name.as_deref(), Some("Test"));
    }

    #[test]
    fn rejects_https_url() {
        assert!(extract_login("https://example.com/success").is_err());
    }

    #[test]
    fn allows_missing_full_name() {
        let info = extract_login("SoftwareFix://callback?Authorization=abc123").unwrap();

        assert_eq!(info.token, "abc123");
        assert_eq!(info.full_name, None);
    }

    #[test]
    fn language_code_follows_os_locale() {
        assert_eq!(language_code_for_locale("zh-CN"), "zh_CN");
        assert_eq!(language_code_for_locale("zh_TW"), "zh_TW");
        assert_eq!(language_code_for_locale("zh-HK"), "zh_TW");
        assert_eq!(language_code_for_locale("zh-Hant"), "zh_TW");
        assert_eq!(language_code_for_locale("en-US"), "en_US");
        assert_eq!(language_code_for_locale("ja-JP"), "en_US");
        assert_eq!(language_code_for_locale(""), "en_US");
    }
}
