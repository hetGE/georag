//! `config/settings.toml`: typed settings with the documented defaults.
//!
//! `DEFAULT_SETTINGS_TOML` is the file `georag init` writes, comments
//! included. A test keeps it in step with `Settings::default()`.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};

/// The default settings file, written verbatim by `georag init`. Lives in
/// `defaults/settings.toml` so release bundling can ship it as
/// `config/settings.toml.default` without compiling this crate.
pub const DEFAULT_SETTINGS_TOML: &str = include_str!("../defaults/settings.toml");

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Settings {
    pub app: AppSettings,
    pub engine: EngineSettings,
    pub chat: ChatSettings,
    pub auto: AutoSettings,
    pub index: IndexSettings,
    pub update: UpdateSettings,
    pub tui: TuiSettings,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AppSettings {
    pub version_checked_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EngineSettings {
    pub backend: String,
    pub gpu_layers: i32,
    pub kv_cache_type: String,
}

impl Default for EngineSettings {
    fn default() -> Self {
        Self {
            backend: "auto".into(),
            gpu_layers: -1,
            kv_cache_type: "q8_0".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChatSettings {
    pub mode: String,
    pub manual_tier: String,
    pub history_turns: u32,
    pub thinking: bool,
}

impl Default for ChatSettings {
    fn default() -> Self {
        Self {
            mode: "auto".into(),
            manual_tier: "optimal".into(),
            history_turns: 4,
            thinking: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoSettings {
    pub min_relevance: f32,
    pub max_passes: u32,
    pub min_candidates: u32,
    pub synthesis: bool,
    pub candidate_k: u32,
}

impl Default for AutoSettings {
    fn default() -> Self {
        Self {
            min_relevance: 0.65,
            max_passes: 6,
            min_candidates: 16,
            synthesis: true,
            candidate_k: 400,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IndexSettings {
    pub chunk_tokens: u32,
    pub chunk_overlap: u32,
    pub extract_parallelism: u32,
    pub embed_batch: u32,
    pub watch_debounce_ms: u64,
}

impl Default for IndexSettings {
    fn default() -> Self {
        Self {
            chunk_tokens: 1000,
            chunk_overlap: 200,
            extract_parallelism: 0,
            embed_batch: 64,
            watch_debounce_ms: 2000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateSettings {
    pub auto_check: bool,
    pub channel: String,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self {
            auto_check: true,
            channel: "stable".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TuiSettings {
    pub color: String,
    pub mouse: bool,
}

impl Default for TuiSettings {
    fn default() -> Self {
        Self {
            color: "auto".into(),
            mouse: true,
        }
    }
}

impl Settings {
    /// Load settings from `path`. A missing file yields the defaults; a
    /// present but invalid file is an error that names the file.
    pub fn load(path: &Path) -> Result<Settings> {
        if !path.is_file() {
            return Ok(Settings::default());
        }
        let text = fs::read_to_string(path).map_err(|source| CoreError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        toml::from_str(&text).map_err(|source| CoreError::SettingsParse {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Write settings to `path`. Comments from the default template are not
    /// preserved; `config set` (P1) will edit the document in place instead.
    pub fn save(&self, path: &Path) -> Result<()> {
        let text = toml::to_string_pretty(self)?;
        fs::write(path, text).map_err(|source| CoreError::Io {
            path: path.to_path_buf(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_template_matches_default_struct() {
        let parsed: Settings = toml::from_str(DEFAULT_SETTINGS_TOML).unwrap();
        assert_eq!(parsed, Settings::default());
    }

    #[test]
    fn partial_file_fills_in_defaults() {
        let parsed: Settings = toml::from_str("[engine]\nbackend = \"cpu\"\n").unwrap();
        assert_eq!(parsed.engine.backend, "cpu");
        assert_eq!(parsed.engine.gpu_layers, -1);
        assert_eq!(parsed.auto, AutoSettings::default());
    }

    #[test]
    fn round_trips_through_save_format() {
        let text = toml::to_string_pretty(&Settings::default()).unwrap();
        let back: Settings = toml::from_str(&text).unwrap();
        assert_eq!(back, Settings::default());
    }
}
