//! Topic tags (handoff 6.4): definitions and heuristic patterns from
//! `config/tags.toml`, seeded into the store; rule matching on folder and
//! file names; the LLM fallback for files no rule matched.

use std::fs;
use std::path::Path;

use georag_engine::{CancelToken, ChatMessage, GenRequest, InferenceEngine, Lane};
use georag_store::Store;
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::home::Home;

pub const DEFAULT_TAGS_TOML: &str = include_str!("../defaults/tags.toml");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagDef {
    pub name: String,
    pub display_name: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub patterns: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagFile {
    #[serde(default, rename = "tag")]
    pub tags: Vec<TagDef>,
}

impl TagFile {
    pub fn defaults() -> TagFile {
        toml::from_str(DEFAULT_TAGS_TOML).expect("the built-in tags file parses")
    }

    /// Load `config/tags.toml`, writing the defaults first if it is absent.
    pub fn load_or_create(home: &Home) -> Result<TagFile> {
        let path = home.tags_toml_path();
        if !path.is_file() {
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir).map_err(|source| CoreError::Io {
                    path: dir.to_path_buf(),
                    source,
                })?;
            }
            fs::write(&path, DEFAULT_TAGS_TOML).map_err(|source| CoreError::Io {
                path: path.clone(),
                source,
            })?;
        }
        let text = fs::read_to_string(&path).map_err(|source| CoreError::Io {
            path: path.clone(),
            source,
        })?;
        toml::from_str(&text).map_err(|source| CoreError::SettingsParse { path, source })
    }

    pub fn save(&self, home: &Home) -> Result<()> {
        let path = home.tags_toml_path();
        let text = toml::to_string_pretty(self)?;
        fs::write(&path, text).map_err(|source| CoreError::Io { path, source })
    }

    pub fn get(&self, name: &str) -> Option<&TagDef> {
        self.tags.iter().find(|t| t.name == name)
    }

    pub fn upsert(&mut self, def: TagDef) {
        match self.tags.iter_mut().find(|t| t.name == def.name) {
            Some(t) => *t = def,
            None => self.tags.push(def),
        }
    }

    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.tags.len();
        self.tags.retain(|t| t.name != name);
        self.tags.len() != before
    }

    /// Make the store's tags table match this file (names, display names,
    /// descriptions, colours). Tags only in the store are kept.
    pub fn seed(&self, store: &Store) -> Result<()> {
        for t in &self.tags {
            store.tag_upsert(
                &t.name,
                &t.display_name,
                t.description.as_deref(),
                t.color.as_deref(),
            )?;
        }
        Ok(())
    }

    /// Rule pass: patterns against the folder path and the file name with
    /// separators replaced by spaces, case-insensitively. Confidence 0.7.
    pub fn match_rules(&self, rel_path: &str) -> Vec<(String, f64)> {
        let path = Path::new(rel_path);
        let folder = path
            .parent()
            .map(|p| p.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let name = stem.replace(['_', '-', '.'], " ");
        let combined = format!("{folder} {name}");
        let mut out = Vec::new();
        for t in &self.tags {
            if t.patterns
                .iter()
                .any(|p| !p.is_empty() && combined.contains(&p.to_lowercase()))
            {
                out.push((t.name.clone(), 0.7));
            }
        }
        out
    }
}

/// One file for the LLM fallback: name and the first characters of text.
#[derive(Debug, Clone)]
pub struct LlmCandidate {
    pub file_id: i64,
    pub filename: String,
    pub preview: String,
}

/// Tag names with confidences for one file.
pub type TagAssignment = Vec<(String, f64)>;

pub const LLM_BATCH: usize = 10;
pub const LLM_PREVIEW_CHARS: usize = 1500;

/// Classify up to `LLM_BATCH` files in one call on the vision and tagging
/// lane. Returns (file_id, tags) with confidence 0.8 for valid tag names.
pub fn tag_batch_by_llm(
    engine: &dyn InferenceEngine,
    defs: &TagFile,
    files: &[LlmCandidate],
) -> Result<Vec<(i64, TagAssignment)>> {
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let categories: Vec<String> = defs
        .tags
        .iter()
        .map(|t| {
            format!(
                "- {}: {}",
                t.name,
                t.description.as_deref().unwrap_or(&t.display_name)
            )
        })
        .collect();
    let mut entries = String::new();
    for (i, f) in files.iter().enumerate() {
        let preview: String = f.preview.chars().take(LLM_PREVIEW_CHARS).collect();
        entries.push_str(&format!(
            "FILE {}: {}\n{}\n---\n",
            i + 1,
            f.filename,
            preview
        ));
    }
    let user = format!(
        "Classify each of these geotechnical engineering documents into one or more categories.\n\nAvailable categories:\n{}\n\n{}\nReply with ONLY a JSON object mapping file numbers to arrays of category names, for example {{\"1\": [\"piling\"], \"2\": []}}. Use an empty array when none apply clearly.",
        categories.join("\n"),
        entries
    );
    let prompt = engine.chat_prompt(
        &[
            ChatMessage::new(
                "system",
                "You are a geotechnical engineering document classifier. Reply only with JSON.",
            ),
            ChatMessage::new("user", user),
        ],
        false,
    )?;
    let result = engine.generate(
        GenRequest {
            prompt,
            max_tokens: 400,
            temperature: 0.0,
            seed: 7,
        },
        Lane::Vision,
        Box::new(|_| {}),
        CancelToken::new(),
    )?;
    let text = crate::chat::prompts::strip_think(&result.text);
    let json = extract_json_object(&text);
    let parsed: serde_json::Value = serde_json::from_str(json).unwrap_or(serde_json::Value::Null);
    let mut out = Vec::new();
    for (i, f) in files.iter().enumerate() {
        let key = (i + 1).to_string();
        let names: Vec<String> = parsed
            .get(&key)
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let valid: TagAssignment = names
            .into_iter()
            .filter(|n| defs.get(n).is_some())
            .map(|n| (n, 0.8))
            .collect();
        out.push((f.file_id, valid));
    }
    Ok(out)
}

/// The first `{ ... }` block in a reply, with code fences removed.
fn extract_json_object(text: &str) -> &str {
    match (text.find('{'), text.rfind('}')) {
        (Some(start), Some(end)) if start < end => &text[start..=end],
        _ => "{}",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_parse_with_all_34_tags() {
        let defs = TagFile::defaults();
        assert_eq!(defs.tags.len(), 34);
        assert_eq!(defs.get("piling").unwrap().display_name, "Piling");
        assert!(
            defs.get("piling")
                .unwrap()
                .patterns
                .contains(&"bored pile".to_string())
        );
    }

    #[test]
    fn rules_match_folder_and_file_name() {
        let defs = TagFile::defaults();
        let tags: Vec<String> = defs
            .match_rules("Projects/Harbour/Bored_Pile-Capacity.pdf")
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert!(tags.contains(&"piling".to_string()));
        let tags: Vec<String> = defs
            .match_rules("Lab/triaxial results 2024.xlsx")
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(tags, vec!["lab_testing".to_string()]);
        assert!(defs.match_rules("misc/notes.txt").is_empty());
    }

    #[test]
    fn json_block_is_found_inside_fences() {
        assert_eq!(
            extract_json_object("```json\n{\"1\": [\"piling\"]}\n```"),
            "{\"1\": [\"piling\"]}"
        );
        assert_eq!(extract_json_object("no json"), "{}");
    }
}
