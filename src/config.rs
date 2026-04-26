/// Persistent configuration loaded from / saved to `config.json`.
use std::path::PathBuf;
use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::protocol::HWMode;
use crate::modes::ModeConfig;

// ── Top-level config ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub device:   DeviceConfig,
    pub defaults: DefaultsConfig,
    pub scenes:   std::collections::HashMap<String, SceneConfig>,
    pub schedule: ScheduleConfig,
}

impl Default for Config {
    fn default() -> Self {
        let mut scenes = std::collections::HashMap::new();
        scenes.insert("movie".into(),   SceneConfig { brightness: Some(25), color: Some([255, 130, 50]), mode: Some("static".into()), hw_mode: None, speed: None });
        scenes.insert("party".into(),   SceneConfig { brightness: None, color: None, mode: Some("hw".into()), hw_mode: Some(HWMode::Strobe7Color), speed: Some(75) });
        scenes.insert("romance".into(), SceneConfig { brightness: Some(50), color: Some([200, 20, 80]), mode: Some("pulse".into()), hw_mode: None, speed: None });
        scenes.insert("relax".into(),   SceneConfig { brightness: Some(55), color: None, mode: Some("hw".into()), hw_mode: Some(HWMode::FadePurple), speed: Some(25) });
        scenes.insert("focus".into(),   SceneConfig { brightness: Some(100), color: Some([210, 230, 255]), mode: Some("static".into()), hw_mode: None, speed: None });
        scenes.insert("gaming".into(),  SceneConfig { brightness: None, color: None, mode: Some("rainbow".into()), hw_mode: None, speed: None });
        scenes.insert("chill".into(),   SceneConfig { brightness: Some(60), color: Some([30, 80, 200]), mode: Some("pulse".into()), hw_mode: None, speed: None });

        Self {
            device:   DeviceConfig::default(),
            defaults: DefaultsConfig::default(),
            scenes,
            schedule: ScheduleConfig::default(),
        }
    }
}

// ── Device section ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DeviceConfig {
    /// Device MAC address. Leave empty to auto-discover.
    pub mac:                   String,
    pub auto_discover:         bool,
    pub scan_timeout_secs:     f32,
    pub connection_timeout_secs: f32,
    pub reconnect_attempts:    u32,
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self {
            mac:                     String::new(),
            auto_discover:           true,
            scan_timeout_secs:       6.0,
            connection_timeout_secs: 10.0,
            reconnect_attempts:      3,
        }
    }
}

// ── Defaults section ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DefaultsConfig {
    pub brightness: u8,
    pub speed:      u8,
    pub color:      [u8; 3],
    pub mode:       String,
}

impl Default for DefaultsConfig {
    fn default() -> Self {
        Self {
            brightness: 80,
            speed:      50,
            color:      [255, 100, 30],
            mode:       "pulse".into(),
        }
    }
}

// ── Scene preset ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneConfig {
    pub brightness: Option<u8>,
    pub color:      Option<[u8; 3]>,
    pub mode:       Option<String>,
    pub hw_mode:    Option<HWMode>,
    pub speed:      Option<u8>,
}

// ── Schedule ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ScheduleConfig {
    pub enabled: bool,
    pub entries: Vec<ScheduleEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleEntry {
    /// "HH:MM" in 24-hour format
    pub time:   String,
    /// "on" | "off" | "scene:<name>" | "mode:<name>" | "brightness:<0-100>"
    pub action: String,
}

// ── I/O helpers ───────────────────────────────────────────────────────────────

impl Config {
    /// Default config path: next to the running binary.
    pub fn default_path() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("config.json")))
            .unwrap_or_else(|| PathBuf::from("config.json"))
    }

    pub fn load(path: &PathBuf) -> Result<Self> {
        if path.exists() {
            let text = std::fs::read_to_string(path)?;
            Ok(serde_json::from_str(&text)?)
        } else {
            Ok(Self::default())
        }
    }

    pub fn save(&self, path: &PathBuf) -> Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(path, text)?;
        Ok(())
    }

    pub fn save_default(path: &PathBuf) -> Result<()> {
        Self::default().save(path)
    }
}
