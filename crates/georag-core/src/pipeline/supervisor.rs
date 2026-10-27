//! Runs queued jobs stage by stage: extract on the CPU pool, tag in
//! batches on the vision lane, embed one file at a time on the bulk lane.

use std::sync::{Arc, Mutex};

use georag_store::{FileState, Store};
use serde::Serialize;

use crate::app::App;
use crate::error::Result;
use crate::pipeline::stages::{self, StageOutcome};
use crate::tags::TagFile;

#[derive(Debug, Clone, Default)]
pub struct IndexOptions {
    /// Only files carrying one of these tags.
    pub tags: Vec<String>,
    /// Re-extract files that are already indexed.
    pub reprocess: bool,
    /// Only these file ids.
    pub ids: Vec<i64>,
    /// Skip the LLM fallback in tagging.
    pub no_llm: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PipelineEvent {
    pub stage: &'static str,
    pub file_id: i64,
    pub rel_path: String,
    pub outcome: String,
    pub done: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Report {
    pub extracted: usize,
    pub tagged: usize,
    pub embedded: usize,
    pub failed: usize,
    pub skipped: usize,
}

fn rel_path(store: &Store, id: i64) -> String {
    store
        .file_get(id)
        .ok()
        .flatten()
        .map(|f| f.rel_path)
        .unwrap_or_else(|| format!("#{id}"))
}

/// Queue extraction for files selected by the options.
pub fn queue(app: &App, opts: &IndexOptions) -> Result<usize> {
    let store = &app.store;
    let ids: Vec<i64> = if !opts.ids.is_empty() {
        opts.ids.clone()
    } else if opts.reprocess {
        let q = georag_store::FileQuery {
            tag: opts.tags.first().cloned(),
            ..Default::default()
        };
        store
            .files_query(&q)?
            .into_iter()
            .filter(|f| f.state != FileState::Skipped)
            .map(|f| f.id)
            .collect()
    } else {
        let mut ids = store.files_in_state(
            &[FileState::New, FileState::Stale, FileState::Imported],
            usize::MAX,
        )?;
        // Extracted but never embedded (for example after a model change).
        ids.extend(store.files_in_state(&[FileState::Extracted, FileState::Tagged], usize::MAX)?);
        ids.extend(store.files_without_vectors()?);
        ids.sort_unstable();
        ids.dedup();
        ids
    };
    let mut n = 0;
    for id in ids {
        let Some(file) = store.file_get(id)? else {
            continue;
        };
        if !opts.tags.is_empty() {
            let tags = store.file_tags(id)?;
            if !tags.iter().any(|t| opts.tags.contains(&t.tag)) {
                continue;
            }
        }
        match file.state {
            FileState::Extracted => store.job_enqueue("tag", Some(id), None)?,
            FileState::Tagged => store.job_enqueue("embed", Some(id), None)?,
            FileState::Indexed if !opts.reprocess => store.job_enqueue("embed", Some(id), None)?,
            _ => store.job_enqueue("extract", Some(id), None)?,
        };
        n += 1;
    }
    Ok(n)
}

/// Drain the queues. `on_event` is called after every file; return false
/// from `keep_going` to stop between files.
pub fn run(
    app: &App,
    opts: &IndexOptions,
    on_event: &mut dyn FnMut(PipelineEvent),
    keep_going: &dyn Fn() -> bool,
) -> Result<Report> {
    let store = app.store.clone();
    store.jobs_requeue_running()?;
    let defs = TagFile::load_or_create(&app.home)?;
    defs.seed(&store)?;
    let report = Arc::new(Mutex::new(Report::default()));
    let mut done = 0usize;

    // Extract: CPU pool, bounded by settings.index.extract_parallelism.
    let parallelism = match app.settings.index.extract_parallelism {
        0 => std::thread::available_parallelism()
            .map(|n| (n.get() / 2).max(1))
            .unwrap_or(2),
        n => n as usize,
    };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(parallelism)
        .thread_name(|i| format!("georag-extract-{i}"))
        .build()
        .map_err(|e| crate::CoreError::Invalid(e.to_string()))?;
    loop {
        if !keep_going() {
            break;
        }
        let mut batch = Vec::new();
        for _ in 0..parallelism {
            match store.job_claim("extract")? {
                Some(j) => batch.push(j),
                None => break,
            }
        }
        if batch.is_empty() {
            break;
        }
        let total_queued = batch.len()
            + store
                .jobs(&["queued"], usize::MAX)?
                .iter()
                .filter(|j| j.kind == "extract")
                .count();
        let results: Vec<(i64, Option<i64>, Result<StageOutcome>)> = pool.install(|| {
            use rayon::prelude::*;
            batch
                .par_iter()
                .map(|job| {
                    let file_id = job.file_id.unwrap_or(0);
                    let r = stages::extract_file(
                        &app.home,
                        &app.settings,
                        &store,
                        app.engine.as_ref(),
                        file_id,
                    );
                    (job.id, job.file_id, r)
                })
                .collect()
        });
        for (job_id, file_id, r) in results {
            let file_id = file_id.unwrap_or(0);
            let outcome = match r {
                Ok(StageOutcome::Done) => {
                    report.lock().unwrap().extracted += 1;
                    store.job_finish(job_id, "done", None)?;
                    "extracted".to_string()
                }
                Ok(StageOutcome::Failed(e)) => {
                    report.lock().unwrap().failed += 1;
                    store.job_finish(job_id, "failed", Some(&e))?;
                    format!("failed: {e}")
                }
                Ok(StageOutcome::Skipped(e)) => {
                    report.lock().unwrap().skipped += 1;
                    store.job_finish(job_id, "done", Some(&e))?;
                    format!("skipped: {e}")
                }
                Err(e) => {
                    report.lock().unwrap().failed += 1;
                    store.job_finish(job_id, "failed", Some(&e.to_string()))?;
                    store.file_set_state(file_id, FileState::Failed, Some(&e.to_string()))?;
                    format!("failed: {e}")
                }
            };
            done += 1;
            on_event(PipelineEvent {
                stage: "extract",
                file_id,
                rel_path: rel_path(&store, file_id),
                outcome,
                done,
                total: done + total_queued.saturating_sub(batch.len()),
            });
        }
    }

    // Tag: batches of up to 10 through one LLM call each.
    loop {
        if !keep_going() {
            break;
        }
        let mut jobs = Vec::new();
        for _ in 0..crate::tags::LLM_BATCH {
            match store.job_claim("tag")? {
                Some(j) => jobs.push(j),
                None => break,
            }
        }
        if jobs.is_empty() {
            break;
        }
        let ids: Vec<i64> = jobs.iter().filter_map(|j| j.file_id).collect();
        match stages::tag_batch(&store, app.engine.as_ref(), &defs, &ids, !opts.no_llm) {
            Ok(outcomes) => {
                for j in &jobs {
                    store.job_finish(j.id, "done", None)?;
                }
                for (id, _) in outcomes {
                    report.lock().unwrap().tagged += 1;
                    done += 1;
                    let tags: Vec<String> =
                        store.file_tags(id)?.into_iter().map(|t| t.tag).collect();
                    on_event(PipelineEvent {
                        stage: "tag",
                        file_id: id,
                        rel_path: rel_path(&store, id),
                        outcome: if tags.is_empty() {
                            "no tags".into()
                        } else {
                            tags.join(", ")
                        },
                        done,
                        total: done,
                    });
                }
            }
            Err(e) => {
                for j in &jobs {
                    store.job_finish(j.id, "failed", Some(&e.to_string()))?;
                }
                report.lock().unwrap().failed += jobs.len();
            }
        }
    }

    // Embed: one file at a time on the bulk lane.
    while keep_going() {
        let Some(job) = store.job_claim("embed")? else {
            break;
        };
        let file_id = job.file_id.unwrap_or(0);
        let outcome = match stages::embed_file(
            &app.settings,
            &store,
            app.engine.as_ref(),
            &app.doc_prefix,
            file_id,
        ) {
            Ok(StageOutcome::Done) => {
                report.lock().unwrap().embedded += 1;
                store.job_finish(job.id, "done", None)?;
                "indexed".to_string()
            }
            Ok(StageOutcome::Failed(e)) | Ok(StageOutcome::Skipped(e)) => {
                report.lock().unwrap().failed += 1;
                store.job_finish(job.id, "failed", Some(&e))?;
                format!("failed: {e}")
            }
            Err(e) => {
                report.lock().unwrap().failed += 1;
                store.job_finish(job.id, "failed", Some(&e.to_string()))?;
                store.file_set_state(file_id, FileState::Failed, Some(&e.to_string()))?;
                format!("failed: {e}")
            }
        };
        done += 1;
        let remaining = store
            .jobs(&["queued"], usize::MAX)?
            .iter()
            .filter(|j| j.kind == "embed")
            .count();
        on_event(PipelineEvent {
            stage: "embed",
            file_id,
            rel_path: rel_path(&store, file_id),
            outcome,
            done,
            total: done + remaining,
        });
    }
    store.jobs_clear_finished()?;
    let r = report.lock().unwrap().clone();
    Ok(r)
}
