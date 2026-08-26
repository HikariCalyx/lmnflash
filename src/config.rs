//! Persistence of login credentials, mirroring `reference/login.sh`'s
//! `config.conf` plus a timestamp so cached sessions are reused for a
//! limited time only.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use directories::BaseDirs;
use serde::{Deserialize, Serialize};

/// Cached credentials are reused only within this window.
const VALIDITY_SECS: u64 = 3 * 60 * 60;

/// The name of the credentials cache file.
const CONFIG_FILE: &str = "config.conf";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Credentials {
    authorization: String,
    client_uuid: String,
    saved_at: u64,
}

/// Loads cached credentials if they exist and are younger than 3 hours.
///
/// Returns the bare token and its paired client UUID.
pub fn load_credentials() -> Option<(String, String)> {
    let raw = std::fs::read_to_string(config_path()).ok()?;

    let credentials: Credentials = serde_json::from_str(&raw).ok()?;

    if credentials.authorization.trim().is_empty() || credentials.client_uuid.trim().is_empty() {
        return None;
    }

    let now = unix_secs();
    if now.saturating_sub(credentials.saved_at) >= VALIDITY_SECS {
        return None;
    }

    let token = credentials
        .authorization
        .trim()
        .strip_prefix("Bearer ")
        .map(str::to_owned)
        .unwrap_or_else(|| credentials.authorization.trim().to_owned());

    Some((token, credentials.client_uuid))
}

/// Saves the credentials atomically in the platform config directory.
pub fn save_credentials(token: &str, client_uuid: &str) -> Result<(), String> {
    let credentials = Credentials {
        authorization: format!("Bearer {}", token.trim().trim_start_matches("Bearer ")),
        client_uuid: client_uuid.trim().to_string(),
        saved_at: unix_secs(),
    };

    let json =
        serde_json::to_string(&credentials).map_err(|e| format!("failed to serialize: {e}"))?;

    let dir = config_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("failed to create config directory: {e}"))?;

    let path = dir.join(CONFIG_FILE);
    let tmp = dir.join("config.conf.tmp");

    std::fs::write(&tmp, json).map_err(|e| format!("failed to write credentials: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("failed to save credentials: {e}"))?;

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
