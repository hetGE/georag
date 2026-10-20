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
    pub context_ladder: Option<Vec<u32>>,
    pub generation_tokens_per_second: Option<f32>,
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

    // Models arrive in P1. Until then the model-dependent sections are
    // reported as skipped rather than guessed.
    let chat_model = None;
    notes.push("no chat model in use: context ladder and generation timing skipped (models arrive with `georag models pull`)".to_string());
    if opts.scores.is_some() {
        notes.push("no embedding model in use: score distribution skipped".to_string());
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
        context_ladder: None,
        generation_tokens_per_second: None,
        score_deciles: None,
        notes,
    })
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
        match &self.context_ladder {
            Some(ladder) => {
                let rungs: Vec<String> = ladder.iter().map(u32::to_string).collect();
                writeln!(f, "  ladder     {}", rungs.join(", "))?;
            }
            None => writeln!(f, "  ladder     skipped")?,
        }
        match self.generation_tokens_per_second {
            Some(tps) => writeln!(f, "  timing     {tps:.1} tokens/s over 50 tokens")?,
            None => writeln!(f, "  timing     skipped")?,
        }
        match &self.score_deciles {
            Some(deciles) => {
                let cols: Vec<String> = deciles.iter().map(|s| format!("{s:.2}")).collect();
                writeln!(f, "  scores     {}", cols.join(" "))?;
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
