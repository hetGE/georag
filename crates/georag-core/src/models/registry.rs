//! `config/models.toml`: every model GeoRAG knows about, managed (downloaded
//! into `models/`) or linked (anywhere on disk), with the role it serves.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::home::Home;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelRole {
    Chat,
    Embed,
    Vision,
}

impl ModelRole {
    pub fn as_str(self) -> &'static str {
        match self {
            ModelRole::Chat => "chat",
            ModelRole::Embed => "embed",
            ModelRole::Vision => "vision",
        }
    }
}

impl std::str::FromStr for ModelRole {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "chat" => Ok(ModelRole::Chat),
            "embed" | "embedding" => Ok(ModelRole::Embed),
            "vision" | "mmproj" => Ok(ModelRole::Vision),
            other => Err(CoreError::Invalid(format!(
                "unknown model role {other:?}: use chat, embed or vision"
            ))),
        }
    }
}

impl std::fmt::Display for ModelRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelKind {
    Managed,
    Linked,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelEntry {
    pub id: String,
    pub role: ModelRole,
    pub kind: ModelKind,
    /// Relative to the home folder for managed models, absolute for linked.
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n_ctx_train: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dim: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pairs_with: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(default)]
    pub in_use: bool,
}

impl ModelEntry {
    pub fn resolve_path(&self, home: &Home) -> PathBuf {
        let p = Path::new(&self.path);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            home.root().join(p)
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default, rename = "model")]
    pub models: Vec<ModelEntry>,
}

impl Registry {
    pub fn load(home: &Home) -> Result<Registry> {
        let path = home.models_toml_path();
        if !path.is_file() {
            return Ok(Registry::default());
        }
        let text = fs::read_to_string(&path).map_err(|source| CoreError::Io {
            path: path.clone(),
            source,
        })?;
        toml::from_str(&text).map_err(|source| CoreError::SettingsParse { path, source })
    }

    pub fn save(&self, home: &Home) -> Result<()> {
        let path = home.models_toml_path();
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|source| CoreError::Io {
                path: dir.to_path_buf(),
                source,
            })?;
        }
        let text = toml::to_string_pretty(self)?;
        fs::write(&path, text).map_err(|source| CoreError::Io { path, source })
    }

    pub fn get(&self, id: &str) -> Option<&ModelEntry> {
        self.models.iter().find(|m| m.id == id)
    }

    pub fn in_use(&self, role: ModelRole) -> Option<&ModelEntry> {
        self.models.iter().find(|m| m.role == role && m.in_use)
    }

    /// Add or replace an entry by id.
    pub fn upsert(&mut self, entry: ModelEntry) {
        match self.models.iter_mut().find(|m| m.id == entry.id) {
            Some(existing) => *existing = entry,
            None => self.models.push(entry),
        }
    }

    /// Mark `id` as the model in use for its role; others of that role are
    /// cleared. Returns the entry.
    pub fn set_in_use(&mut self, id: &str) -> Result<&ModelEntry> {
        let role = self.get(id).map(|m| m.role).ok_or_else(|| {
            CoreError::Invalid(format!("no model with id {id:?}: see `georag models list`"))
        })?;
        for m in &mut self.models {
            if m.role == role {
                m.in_use = m.id == id;
            }
        }
        Ok(self.get(id).expect("just set"))
    }

    pub fn remove(&mut self, id: &str) -> Option<ModelEntry> {
        let pos = self.models.iter().position(|m| m.id == id)?;
        Some(self.models.remove(pos))
    }

    /// If no model of `role` is in use, pick the first one of that role.
    pub fn ensure_in_use(&mut self, role: ModelRole) {
        if self.in_use(role).is_none()
            && let Some(m) = self.models.iter_mut().find(|m| m.role == role)
        {
            m.in_use = true;
        }
    }
}

/// A stable id from a file name: lowercase, `.gguf` stripped, separators to `-`.
pub fn id_from_filename(file: &str) -> String {
    let stem = file.strip_suffix(".gguf").unwrap_or(file);
    let mut out = String::new();
    for c in stem.chars() {
        if c.is_ascii_alphanumeric() || c == '.' {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, role: ModelRole) -> ModelEntry {
        ModelEntry {
            id: id.into(),
            role,
            kind: ModelKind::Managed,
            path: format!("models/x/{id}.gguf"),
            sha256: None,
            n_ctx_train: None,
            size_bytes: None,
            dim: None,
            pairs_with: None,
            repo: None,
            in_use: false,
        }
    }

    #[test]
    fn in_use_is_exclusive_per_role() {
        let mut r = Registry::default();
        r.upsert(entry("a", ModelRole::Chat));
        r.upsert(entry("b", ModelRole::Chat));
        r.upsert(entry("e", ModelRole::Embed));
        r.set_in_use("a").unwrap();
        r.set_in_use("e").unwrap();
        r.set_in_use("b").unwrap();
        assert_eq!(r.in_use(ModelRole::Chat).unwrap().id, "b");
        assert_eq!(r.in_use(ModelRole::Embed).unwrap().id, "e");
        assert!(!r.get("a").unwrap().in_use);
        assert!(r.set_in_use("zzz").is_err());
    }

    #[test]
    fn toml_round_trip_matches_handoff_shape() {
        let text = r#"
[[model]]
id = "nomic-embed-text-v1.5"
role = "embed"
kind = "managed"
path = "models/nomic-ai/nomic-embed-text-v1.5-GGUF/nomic-embed-text-v1.5.Q8_0.gguf"
dim = 768
in_use = true

[[model]]
id = "qwen3-27b-q4_k_m"
role = "chat"
kind = "linked"
path = "/Volumes/LLMs/Qwen3-27B-Q4_K_M.gguf"
"#;
        let r: Registry = toml::from_str(text).unwrap();
        assert_eq!(r.models.len(), 2);
        assert_eq!(r.in_use(ModelRole::Embed).unwrap().dim, Some(768));
        assert_eq!(r.models[1].kind, ModelKind::Linked);
        let back: Registry = toml::from_str(&toml::to_string_pretty(&r).unwrap()).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn ids_from_filenames() {
        assert_eq!(
            id_from_filename("Qwen3.5-9B-Q4_K_M.gguf"),
            "qwen3.5-9b-q4-k-m"
        );
        assert_eq!(
            id_from_filename("mmproj-Qwen3.5-9B-BF16.gguf"),
            "mmproj-qwen3.5-9b-bf16"
        );
    }
}
