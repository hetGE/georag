//! The per-file stages: extract (and chunk), tag, embed (and index).

use std::path::Path;

use georag_engine::{InferenceEngine, Lane};
use georag_extract::{ExtractCtx, ExtractError};
use georag_store::{FileState, Store};

use crate::config::Settings;
use crate::error::Result;
use crate::home::Home;
use crate::pipeline::chunker::chunk_text;
use crate::tags::{LlmCandidate, TagFile, tag_batch_by_llm};

/// Below this many characters per page a PDF is treated as scanned.
pub const MIN_CHARS_PER_PAGE: f64 = 20.0;

/// What happened to one file in a stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageOutcome {
    Done,
    Failed(String),
    Skipped(String),
}

/// Extract text, chunk it with stored token counts, store chunks without
/// vectors. Files with no text go to `failed` with the reason `no_text`.
pub fn extract_file(
    home: &Home,
    settings: &Settings,
    store: &Store,
    engine: &dyn InferenceEngine,
    file_id: i64,
) -> Result<StageOutcome> {
    let Some(file) = store.file_get(file_id)? else {
        return Ok(StageOutcome::Skipped("file row vanished".into()));
    };
    let roots = store.roots()?;
    let Some(root) = roots.iter().find(|r| r.id == file.root_id) else {
        return Ok(StageOutcome::Skipped("root vanished".into()));
    };
    let path = Path::new(&root.path).join(&file.rel_path);
    if !path.is_file() {
        store.file_set_state(file_id, FileState::Skipped, Some("missing on disk"))?;
        return Ok(StageOutcome::Skipped("missing on disk".into()));
    }
    let ctx = ExtractCtx {
        engine_dir: home.engine_dir(),
        min_chars_per_page: MIN_CHARS_PER_PAGE,
    };
    let extracted = match georag_extract::extract(&path, &ctx) {
        Ok(e) => e,
        Err(ExtractError::NoText { chars_per_page, .. }) => {
            store.file_set_state(file_id, FileState::Failed, Some("no_text"))?;
            return Ok(StageOutcome::Failed(format!(
                "no text layer ({chars_per_page:.0} characters per page): OCR needed"
            )));
        }
        Err(e) => {
            store.file_set_state(file_id, FileState::Failed, Some(&e.to_string()))?;
            return Ok(StageOutcome::Failed(e.to_string()));
        }
    };
    let chunks = chunk_text(
        engine,
        &extracted.text,
        settings.index.chunk_tokens as usize,
        settings.index.chunk_overlap as usize,
    )?;
    if chunks.is_empty() {
        store.file_set_state(file_id, FileState::Failed, Some("no_text"))?;
        return Ok(StageOutcome::Failed("no text found".into()));
    }
    store.chunks_replace_for_file(file_id, &chunks, None)?;
    // chunks_replace_for_file sets `extracted`; imported files skip tagging.
    if file.state == FileState::Imported || store.file_has_tags(file_id)? {
        store.file_set_state(file_id, FileState::Tagged, None)?;
        store.job_enqueue("embed", Some(file_id), None)?;
    } else {
        store.job_enqueue("tag", Some(file_id), None)?;
    }
    Ok(StageOutcome::Done)
}

/// Rule pass for one file; returns true when a rule matched.
pub fn tag_file_by_rules(store: &Store, defs: &TagFile, file_id: i64) -> Result<bool> {
    let Some(file) = store.file_get(file_id)? else {
        return Ok(false);
    };
    let matches = defs.match_rules(&file.rel_path);
    for (name, confidence) in &matches {
        store.file_tag_add(file_id, name, "rule", Some(*confidence), false)?;
    }
    Ok(!matches.is_empty())
}

/// Tag stage over a batch of files: rules first, then one LLM call for the
/// rest. Every file then moves to `tagged` and queues `embed`.
pub fn tag_batch(
    store: &Store,
    engine: &dyn InferenceEngine,
    defs: &TagFile,
    file_ids: &[i64],
    use_llm: bool,
) -> Result<Vec<(i64, StageOutcome)>> {
    let mut outcomes = Vec::new();
    let mut unmatched = Vec::new();
    for &id in file_ids {
        if tag_file_by_rules(store, defs, id)? {
            outcomes.push((id, StageOutcome::Done));
        } else {
            unmatched.push(id);
        }
    }
    if use_llm && !unmatched.is_empty() {
        let mut candidates = Vec::new();
        for &id in &unmatched {
            let Some(file) = store.file_get(id)? else {
                continue;
            };
            let preview = file.preview.clone().unwrap_or_default();
            candidates.push(LlmCandidate {
                file_id: id,
                filename: file.filename.clone(),
                preview,
            });
        }
        match tag_batch_by_llm(engine, defs, &candidates) {
            Ok(results) => {
                for (id, tags) in results {
                    for (name, confidence) in tags {
                        store.file_tag_add(id, &name, "llm", Some(confidence), false)?;
                    }
                    outcomes.push((id, StageOutcome::Done));
                }
            }
            Err(e) => {
                tracing::warn!("llm tagging failed: {e}");
                for &id in &unmatched {
                    outcomes.push((id, StageOutcome::Done));
                }
            }
        }
    } else {
        for &id in &unmatched {
            outcomes.push((id, StageOutcome::Done));
        }
    }
    for &id in file_ids {
        store.file_set_state(id, FileState::Tagged, None)?;
        store.job_enqueue("embed", Some(id), None)?;
    }
    Ok(outcomes)
}

/// Embed a file's chunks in batches on the bulk lane and store the vectors.
pub fn embed_file(
    settings: &Settings,
    store: &Store,
    engine: &dyn InferenceEngine,
    doc_prefix: &str,
    file_id: i64,
) -> Result<StageOutcome> {
    let chunks = store.chunks_for_file(file_id)?;
    if chunks.is_empty() {
        store.file_set_state(file_id, FileState::Failed, Some("no chunks to embed"))?;
        return Ok(StageOutcome::Failed("no chunks".into()));
    }
    let batch = (settings.index.embed_batch as usize).max(1);
    let mut vectors: Vec<(i64, Vec<f32>)> = Vec::with_capacity(chunks.len());
    for group in chunks.chunks(batch) {
        let texts: Vec<String> = group
            .iter()
            .map(|c| format!("{doc_prefix}{}", c.text))
            .collect();
        let embedded = engine.embed(&texts, Lane::Bulk)?;
        for (c, v) in group.iter().zip(embedded) {
            vectors.push((c.id, v));
        }
    }
    store.chunks_set_vectors(file_id, &vectors)?;
    Ok(StageOutcome::Done)
}
