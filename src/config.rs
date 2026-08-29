//! Persistence of login credentials and UI language, mirroring
//! `reference/login.sh`'s `config.conf` plus a timestamp so cached sessions
//! are reused for a limited time only.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use directories::BaseDirs;
use serde::{Deserialize, Serialize};

/// Cached credentials are reused only within this window.
const VALIDITY_SECS: u64 = 3 * 60 * 60;

/// The name of the config file.
const CONFIG_FILE: &str = "config.conf";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Config {
    #[serde(default)]
    authorization: String,
    #[serde(default)]
    client_uuid: String,
    #[serde(default)]
    saved_at: u64,
    /// Previously selected UI language, stored as a locale code
    /// (`en-US` / `zh-Hans`).
    #[serde(default)]
    language: Option<String>,
}

/// Loads cached credentials if they exist and are younger than 3 hours.
///
/// Returns the bare token and its paired client UUID.
pub fn load_credentials() -> Option<(String, String)> {
    let config = load_config();

    if config.authorization.trim().is_empty() || config.client_uuid.trim().is_empty() {
        return None;
    }

    let now = unix_secs();
    if now.saturating_sub(config.saved_at) >= VALIDITY_SECS {
        return None;
    }

    let token = config
        .authorization
        .trim()
        .strip_prefix("Bearer ")
        .map(str::to_owned)
        .unwrap_or_else(|| config.authorization.trim().to_owned());

    Some((token, config.client_uuid))
}

/// Saves the credentials atomically in the platform config directory.
pub fn save_credentials(token: &str, client_uuid: &str) -> Result<(), String> {
    let mut config = load_config();

    config.authorization = format!("Bearer {}", token.trim().trim_start_matches("Bearer "));
    config.client_uuid = client_uuid.trim().to_string();
    config.saved_at = unix_secs();

    save_config(&config)
}

/// Loads the previously selected UI language, if any.
pub fn load_language() -> Option<crate::l10n::Language> {
    load_config()
        .language
        .as_deref()
        .and_then(crate::l10n::Language::from_code)
}

/// Stores the selected UI language, preserving any cached credentials.
pub fn save_language(language: crate::l10n::Language) -> Result<(), String> {
    let mut config = load_config();

    config.language = Some(language.code().to_string());

    save_config(&config)
}

/// Reads the config file, falling back to an empty config when missing or
/// malformed (fields are all optional so older files stay readable).
fn load_config() -> Config {
    std::fs::read_to_string(config_path())
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_config(config: &Config) -> Result<(), String> {
    let json =
        serde_json::to_string(config).map_err(|e| format!("failed to serialize: {e}"))?;

    let dir = config_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("failed to create config directory: {e}"))?;

    let path = dir.join(CONFIG_FILE);
    let tmp = dir.join("config.conf.tmp");

    std::fs::write(&tmp, json).map_err(|e| format!("failed to write config: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("failed to save config: {e}"))?;

    Ok(())
}

/// The directory holding `config.conf`:
///
/// - Windows: `%AppData%\lmnflash`
/// - Linux: `~/.config/lmnflash`
/// - macOS: `~/Library/Application Support/com.hikaricalyx.lmnflash`
fn config_dir() -> PathBuf {
    let base = BaseDirs::new()
        .map(|dirs| dirs.config_dir().to_path_buf())
        .unwrap_or_default();

    let sub_dir = if cfg!(target_os = "macos") {
        "com.hikaricalyx.lmnflash"
    } else {
        "lmnflash"
    };

    base.join(sub_dir)
}

fn config_path() -> PathBuf {
    config_dir().join(CONFIG_FILE)
}

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
