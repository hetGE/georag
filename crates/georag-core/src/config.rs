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
    /// Cap on the context ladder; 0 means the ladder alone decides.
    pub max_ctx: u32,
}

impl Default for EngineSettings {
    fn default() -> Self {
        Self {
            backend: "auto".into(),
            gpu_layers: -1,
            kv_cache_type: "q8_0".into(),
            max_ctx: 0,
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
    /// Every key as `section.key` with its current value, in file order.
    pub fn entries(&self) -> Vec<(String, String)> {
        let doc: toml::Value = toml::Value::try_from(self).expect("settings serialise");
        let mut out = Vec::new();
        if let toml::Value::Table(sections) = doc {
            for (section, value) in sections {
                if let toml::Value::Table(keys) = value {
                    for (key, v) in keys {
                        let text = match v {
                            toml::Value::String(s) => s,
                            other => other.to_string(),
                        };
                        out.push((format!("{section}.{key}"), text));
                    }
                }
            }
        }
        out
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.entries()
            .into_iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    /// Set `section.key` from text, keeping the field's type. Unknown keys
    /// and unparsable values are errors with the expected type named.
    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        let mut doc: toml::Value = toml::Value::try_from(&*self).expect("settings serialise");
        let (section, name) = key.split_once('.').ok_or_else(|| {
            CoreError::Invalid(format!(
                "key {key:?} must be section.key, for example chat.history_turns"
            ))
        })?;
        let current = doc
            .get(section)
            .and_then(|s| s.get(name))
            .cloned()
            .ok_or_else(|| {
                CoreError::Invalid(format!("unknown setting {key:?}: see `georag config get`"))
            })?;
        let new_value = match current {
            toml::Value::String(_) => toml::Value::String(value.to_string()),
            toml::Value::Integer(_) => toml::Value::Integer(
                value
                    .trim()
                    .parse()
                    .map_err(|_| CoreError::Invalid(format!("{key} expects a whole number")))?,
            ),
            toml::Value::Float(_) => toml::Value::Float(
                value
                    .trim()
                    .parse()
                    .map_err(|_| CoreError::Invalid(format!("{key} expects a number")))?,
            ),
            toml::Value::Boolean(_) => match value.trim().to_ascii_lowercase().as_str() {
                "true" | "yes" | "on" | "1" => toml::Value::Boolean(true),
                "false" | "no" | "off" | "0" => toml::Value::Boolean(false),
                _ => return Err(CoreError::Invalid(format!("{key} expects true or false"))),
            },
            _ => return Err(CoreError::Invalid(format!("{key} cannot be set from text"))),
        };
        doc[section][name] = new_value;
        let updated: Settings = doc
            .try_into()
            .map_err(|e: toml::de::Error| CoreError::Invalid(format!("{key}: {e}")))?;
        validate(&updated)?;
        *self = updated;
        Ok(())
    }

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

/// Cross-field checks with user-facing messages.
pub fn validate(s: &Settings) -> Result<()> {
    if !matches!(
        s.engine.backend.as_str(),
        "auto" | "metal" | "cuda" | "vulkan" | "cpu"
    ) {
        return Err(CoreError::Invalid(
            "engine.backend must be auto, metal, cuda, vulkan or cpu".into(),
        ));
    }
    if !matches!(s.chat.mode.as_str(), "auto" | "manual") {
        return Err(CoreError::Invalid(
            "chat.mode must be auto or manual".into(),
        ));
    }
    if !matches!(
        s.chat.manual_tier.as_str(),
        "quick" | "optimal" | "deep" | "deeper" | "ludicrous"
    ) {
        return Err(CoreError::Invalid(
            "chat.manual_tier must be quick, optimal, deep, deeper or ludicrous".into(),
        ));
    }
    if !(0.0..=1.0).contains(&s.auto.min_relevance) {
        return Err(CoreError::Invalid(
            "auto.min_relevance must be between 0 and 1".into(),
        ));
    }
    if s.index.chunk_overlap >= s.index.chunk_tokens {
        return Err(CoreError::Invalid(
            "index.chunk_overlap must be smaller than index.chunk_tokens".into(),
        ));
    }
    if !matches!(s.update.channel.as_str(), "stable" | "nightly") {
        return Err(CoreError::Invalid(
            "update.channel must be stable or nightly".into(),
        ));
    }
    if !matches!(s.tui.color.as_str(), "auto" | "truecolor" | "256" | "16") {
        return Err(CoreError::Invalid(
            "tui.color must be auto, truecolor, 256 or 16".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip_through_get_and_set() {
        let mut s = Settings::default();
        assert_eq!(s.get("chat.history_turns").as_deref(), Some("4"));
        s.set("chat.history_turns", "6").unwrap();
        assert_eq!(s.chat.history_turns, 6);
        s.set("auto.min_relevance", "0.7").unwrap();
        assert!((s.auto.min_relevance - 0.7).abs() < 1e-6);
        s.set("update.auto_check", "off").unwrap();
        assert!(!s.update.auto_check);
        assert!(s.set("chat.history_turns", "many").is_err());
        assert!(s.set("engine.backend", "opencl").is_err());
        assert!(s.set("nope.key", "1").is_err());
        assert!(s.entries().iter().any(|(k, _)| k == "index.embed_batch"));
    }

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
