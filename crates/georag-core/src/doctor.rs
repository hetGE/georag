//! `georag doctor`: flavour, backend and why, device memory, and (once models
//! exist in P1) the context ladder, a generation timing and the cosine score
//! distribution for a sample question.

use std::fmt;
use std::path::PathBuf;

use georag_engine::{Backend, BackendInfo, BackendPreference};
use serde::Serialize;

use crate::config::Settings;
use crate::error::Result;
use crate::home::Home;

#[derive(Debug, Clone, Default)]
pub struct DoctorOptions {
    /// Overrides `settings.engine.backend`.
    pub backend: Option<String>,
    /// CI mode: force the CPU backend, skip anything that needs a model.
    pub ci: bool,
    /// Sample question for the cosine score distribution (section 7.5).
    pub scores: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub version: &'static str,
    pub flavour: &'static str,
    pub home: PathBuf,
    pub home_initialised: bool,
    pub home_writable: bool,
    pub settings_backend: String,
    pub backend: BackendInfo,
    pub chat_model: Option<String>,
    pub embed_model: Option<String>,
    pub context_ladder: Option<Vec<u32>>,
    pub n_ctx: Option<u32>,
    pub generation_tokens_per_second: Option<f32>,
    pub embed_dim: Option<usize>,
    pub embed_millis: Option<u128>,
    pub score_deciles: Option<Vec<f32>>,
    /// Things the user should know, in display order.
    pub notes: Vec<String>,
}

pub fn run(home: &Home, settings: &Settings, opts: &DoctorOptions) -> Result<DoctorReport> {
    let mut notes = Vec::new();

    let home_initialised = home.is_initialised();
    let home_writable = home.check_writable().is_ok();
    if !home_initialised {
        notes.push("home folder is not initialised: run `georag init`".to_string());
    }
    if !home_writable {
        notes.push("home folder is read only: indexing and updates will be refused".to_string());
    }

    let requested = if opts.ci {
        "cpu".to_string()
    } else {
        opts.backend
            .clone()
            .unwrap_or_else(|| settings.engine.backend.clone())
    };
    let pref: BackendPreference = requested.parse()?;
    let backend = georag_engine::backend::select_backend(pref)?;
    if let Some(w) = &backend.warning {
        notes.push(w.clone());
    }
    if opts.ci {
        notes.push("CI mode: CPU backend forced".to_string());
    }

    let registry = crate::models::Registry::load(home)?;
    let chat_model = registry
        .in_use(crate::models::ModelRole::Chat)
        .map(|m| m.id.clone());
    let embed_model = registry
        .in_use(crate::models::ModelRole::Embed)
        .map(|m| m.id.clone());
    let mut context_ladder = None;
    let mut n_ctx = None;
    let mut generation_tokens_per_second = None;
    let mut embed_dim = None;
    let mut embed_millis = None;
    let mut score_deciles = None;

    if chat_model.is_none() {
        notes.push("no chat model in use: context ladder and generation timing skipped (`georag models pull qwen3.5-9b-q4_k_m`)".to_string());
    }
    if embed_model.is_none() {
        notes.push("no embedding model in use: embedding check and scores skipped (`georag models pull nomic-embed-text-v1.5`)".to_string());
    }

    if !opts.ci && (chat_model.is_some() || embed_model.is_some()) {
        let mut probe_settings = settings.clone();
        probe_settings.engine.backend = requested.clone();
        match crate::engine::start(home, &probe_settings, &registry, None) {
            Ok(resident) => {
                if chat_model.is_some() {
                    context_ladder = Some(resident.plan.ladder.clone());
                    n_ctx = Some(resident.plan.n_ctx);
                }
                if chat_model.is_some() {
                    match time_generation(resident.engine.as_ref()) {
                        Ok(tps) => generation_tokens_per_second = Some(tps),
                        Err(e) => notes.push(format!("generation timing failed: {e}")),
                    }
                }
                if embed_model.is_some() {
                    let started = std::time::Instant::now();
                    match resident.engine.embed(
                        &[
                            "search_query: bearing capacity of a bored pile in stiff clay"
                                .to_string(),
                        ],
                        georag_engine::Lane::Interactive,
                    ) {
                        Ok(v) => {
                            embed_millis = Some(started.elapsed().as_millis());
                            embed_dim = v.first().map(Vec::len);
                        }
                        Err(e) => notes.push(format!("embedding check failed: {e}")),
                    }
                    if let Some(question) = &opts.scores {
                        match score_distribution(
                            home,
                            &registry,
                            resident.engine.as_ref(),
                            question,
                        ) {
                            Ok(Some(d)) => score_deciles = Some(d),
                            Ok(None) => notes.push(
                                "score distribution skipped: the index has no chunks yet"
                                    .to_string(),
                            ),
                            Err(e) => notes.push(format!("score distribution failed: {e}")),
                        }
                    }
                }
            }
            Err(e) => notes.push(format!("models could not be loaded: {e}")),
        }
    } else if opts.ci {
        notes.push("CI mode: models not loaded".to_string());
    }

    Ok(DoctorReport {
        version: crate::VERSION,
        flavour: crate::FLAVOUR,
        home: home.root().to_path_buf(),
        home_initialised,
        home_writable,
        settings_backend: settings.engine.backend.clone(),
        backend,
        chat_model,
        embed_model,
        context_ladder,
        n_ctx,
        generation_tokens_per_second,
        embed_dim,
        embed_millis,
        score_deciles,
        notes,
    })
}

/// Generate 50 tokens from a short prompt and report tokens per second.
fn time_generation(engine: &dyn georag_engine::InferenceEngine) -> Result<f32> {
    use georag_engine::{CancelToken, ChatMessage, GenRequest, Lane};
    let prompt = engine.chat_prompt(
        &[ChatMessage::new(
            "user",
            "Count from one to fifty in words, separated by commas.",
        )],
        false,
    )?;
    let result = engine.generate(
        GenRequest {
            prompt,
            max_tokens: 50,
            temperature: 0.0,
            seed: 1,
        },
        Lane::Interactive,
        Box::new(|_| {}),
        CancelToken::new(),
    )?;
    Ok(result.tokens_per_second)
}

/// Cosine deciles of the top 200 candidates for a question (section 7.5).
fn score_distribution(
    home: &Home,
    registry: &crate::models::Registry,
    engine: &dyn georag_engine::InferenceEngine,
    question: &str,
) -> Result<Option<Vec<f32>>> {
    let db = home.db_path();
    if !db.is_file() {
        return Ok(None);
    }
    let store = georag_store::Store::open(&db)?;
    if store.embed_info()?.is_none() || store.chunk_count()? == 0 {
        return Ok(None);
    }
    let embed_id = registry
        .in_use(crate::models::ModelRole::Embed)
        .map(|m| m.id.clone())
        .unwrap_or_default();
    let (_, query_prefix) =
        crate::models::catalog::embed_prefixes_for(&embed_id).unwrap_or(("", ""));
    let q = engine.embed(
        &[format!("{query_prefix}{question}")],
        georag_engine::Lane::Interactive,
    )?;
    let hits = store.knn(&q[0], &georag_store::SearchFilter::default(), 200)?;
    if hits.is_empty() {
        return Ok(None);
    }
    let mut scores: Vec<f32> = hits.iter().map(|h| h.score).collect();
    scores.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let deciles: Vec<f32> = (0..=10)
        .map(|i| {
            let idx = ((scores.len() - 1) * i) / 10;
            scores[idx]
        })
        .collect();
    Ok(Some(deciles))
}

/// Bytes as GiB with one decimal, for people.
pub fn fmt_gib(bytes: u64) -> String {
    format!("{:.1} GiB", bytes as f64 / (1u64 << 30) as f64)
}

impl fmt::Display for DoctorReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "georag doctor")?;
        writeln!(f, "  version    {}", self.version)?;
        writeln!(f, "  flavour    {}", self.flavour)?;
        let home_state = match (self.home_initialised, self.home_writable) {
            (true, true) => "initialised, writable",
            (true, false) => "initialised, read only",
            (false, true) => "not initialised, writable",
            (false, false) => "not initialised, read only",
        };
        writeln!(f, "  home       {}  ({home_state})", self.home.display())?;
        writeln!(
            f,
            "  settings   engine.backend = \"{}\"",
            self.settings_backend
        )?;
        writeln!(f)?;

        let b = &self.backend;
        writeln!(f, "backend")?;
        writeln!(f, "  chosen     {}: {}", b.backend, b.reason)?;
        writeln!(
            f,
            "  device     {} ({})",
            b.device.description, b.device.name
        )?;
        let memory_note = match b.backend {
            Backend::Metal => " (recommended working set)",
            Backend::Cuda => " (cudaMemGetInfo)",
            Backend::Vulkan => " (device heap budget)",
            Backend::Cpu => " (system memory)",
        };
        writeln!(
            f,
            "  memory     {} total, {} free{memory_note}",
            fmt_gib(b.device.memory_total),
            fmt_gib(b.device.memory_free)
        )?;
        writeln!(f, "  devices    {}", b.all_devices.len())?;
        for d in &b.all_devices {
            writeln!(
                f,
                "             {:<10} {:<24} {:<14} {} total, {} free",
                d.name,
                d.description,
                d.kind.to_string(),
                fmt_gib(d.memory_total),
                fmt_gib(d.memory_free)
            )?;
        }
        writeln!(f)?;

        writeln!(f, "models")?;
        writeln!(
            f,
            "  chat       {}",
            self.chat_model.as_deref().unwrap_or("none in use")
        )?;
        writeln!(
            f,
            "  embed      {}",
            self.embed_model.as_deref().unwrap_or("none in use")
        )?;
        match &self.context_ladder {
            Some(ladder) if !ladder.is_empty() => {
                let rungs: Vec<String> = ladder.iter().map(u32::to_string).collect();
                writeln!(
                    f,
                    "  ladder     {} (using {})",
                    rungs.join(", "),
                    self.n_ctx.unwrap_or(0)
                )?;
            }
            Some(_) => writeln!(f, "  ladder     nothing fits")?,
            None => writeln!(f, "  ladder     skipped")?,
        }
        match self.generation_tokens_per_second {
            Some(tps) => writeln!(f, "  timing     {tps:.1} tokens/s over 50 tokens")?,
            None => writeln!(f, "  timing     skipped")?,
        }
        match (self.embed_dim, self.embed_millis) {
            (Some(dim), Some(ms)) => {
                writeln!(f, "  embedding  {dim} dimensions, {ms} ms for one query")?
            }
            _ => writeln!(f, "  embedding  skipped")?,
        }
        match &self.score_deciles {
            Some(deciles) => {
                let cols: Vec<String> = deciles.iter().map(|s| format!("{s:.2}")).collect();
                writeln!(f, "  scores     top 200 cosine deciles: {}", cols.join(" "))?;
            }
            None => writeln!(f, "  scores     skipped")?,
        }

        if !self.notes.is_empty() {
            writeln!(f)?;
            writeln!(f, "notes")?;
            for n in &self.notes {
                writeln!(f, "  {n}")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gib_formatting() {
        assert_eq!(fmt_gib(0), "0.0 GiB");
        assert_eq!(fmt_gib(16 << 30), "16.0 GiB");
        assert_eq!(fmt_gib((16u64 << 30) + (1 << 29)), "16.5 GiB");
    }
}
