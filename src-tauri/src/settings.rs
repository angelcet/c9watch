use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// User-controlled application settings, persisted to
/// `~/.claude/c9watch/settings.json`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Whether the embedded web/WebSocket server accepts connections from the
    /// local network.
    ///
    /// Defaults to `false` (secure by default): only loopback clients (the
    /// desktop app itself and same-machine browsers) can connect. When `true`,
    /// clients on the LAN may connect — a valid auth token is still required.
    pub remote_access: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            remote_access: false,
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        let path = Self::get_path();
        if let Ok(content) = fs::read_to_string(path) {
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            Self::default()
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::get_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let content = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(path, content).map_err(|e| e.to_string())
    }

    fn get_path() -> PathBuf {
        let home = dirs::home_dir().expect("Failed to get home directory");
        home.join(".claude").join("c9watch").join("settings.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_remote_access_disabled() {
        assert!(!Settings::default().remote_access);
    }

    #[test]
    fn deserializes_empty_object_to_default() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert!(!s.remote_access);
    }

    #[test]
    fn roundtrips_remote_access_flag() {
        let s = Settings {
            remote_access: true,
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert!(back.remote_access);
    }
}
