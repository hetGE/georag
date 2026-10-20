//! The "at a glance" snapshot shared by `georag status --json`, the Home
//! screen and, later, `/v1/status` on the daemon.

use std::path::PathBuf;

use georag_engine::{BackendInfo, BackendPreference};
use serde::Serialize;

use crate::config::Settings;
use crate::home::Home;

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct FileCounts {
    pub new: u64,
    pub indexed: u64,
    pub failed: u64,
    pub stale: u64,
    pub skipped: u64,
}

impl FileCounts {
    pub fn total(&self) -> u64 {
        self.new + self.indexed + self.failed + self.stale + self.skipped
    }
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct ModelsInUse {
    pub chat: Option<String>,
    pub embed: Option<String>,
    pub vision: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RootSummary {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Snapshot {
    pub version: &'static str,
    pub flavour: &'static str,
    pub home: PathBuf,
    pub home_initialised: bool,
    pub backend: Option<BackendInfo>,
    /// Set when the engine probe failed; the UI shows it instead of memory.
    pub backend_error: Option<String>,
    pub roots: Vec<RootSummary>,
    pub files: FileCounts,
    pub models: ModelsInUse,
    pub daemon_running: bool,
    pub jobs_queued: u64,
    pub jobs_running: u64,
}

impl Snapshot {
    /// Take the snapshot. Never fails: a probe failure is recorded in
    /// `backend_error` so the UI can still open.
    pub fn take(home: &Home, settings: &Settings) -> Snapshot {
        let (backend, backend_error) = match settings.engine.backend.parse::<BackendPreference>() {
            Ok(pref) => match georag_engine::backend::select_backend(pref) {
                Ok(info) => (Some(info), None),
                Err(e) => (None, Some(e.to_string())),
            },
            Err(e) => (None, Some(e.to_string())),
        };
        Snapshot {
            version: crate::VERSION,
            flavour: crate::FLAVOUR,
            home: home.root().to_path_buf(),
            home_initialised: home.is_initialised(),
            backend,
            backend_error,
            // Library roots, file counts, models and jobs arrive with P1 and P2.
            roots: Vec::new(),
            files: FileCounts::default(),
            models: ModelsInUse::default(),
            daemon_running: false,
            jobs_queued: 0,
            jobs_running: 0,
        }
    }

    /// A snapshot with nothing probed, for tests and offline rendering.
    pub fn empty(home: PathBuf) -> Snapshot {
        Snapshot {
            version: crate::VERSION,
            flavour: crate::FLAVOUR,
            home,
            home_initialised: false,
            backend: None,
            backend_error: None,
            roots: Vec::new(),
            files: FileCounts::default(),
            models: ModelsInUse::default(),
            daemon_running: false,
            jobs_queued: 0,
            jobs_running: 0,
        }
    }

    /// First run: no library root, or no in-use chat or embedding model.
    pub fn is_first_run(&self) -> bool {
        self.roots.is_empty() || self.models.chat.is_none() || self.models.embed.is_none()
    }

    pub fn models_ready(&self) -> bool {
        self.models.chat.is_some() && self.models.embed.is_some()
    }
}

impl std::fmt::Display for Snapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use crate::doctor::fmt_gib;
        writeln!(f, "georag {} ({})", self.version, self.flavour)?;
        writeln!(
            f,
            "  home       {}{}",
            self.home.display(),
            if self.home_initialised {
                ""
            } else {
                "  (not initialised)"
            }
        )?;
        match (&self.backend, &self.backend_error) {
            (Some(b), _) => writeln!(
                f,
                "  engine     {} on {}, {} free of {}",
                b.backend,
                b.device.description,
                fmt_gib(b.device.memory_free),
                fmt_gib(b.device.memory_total)
            )?,
            (None, Some(e)) => writeln!(f, "  engine     unavailable: {e}")?,
            (None, None) => writeln!(f, "  engine     not probed")?,
        }
        writeln!(
            f,
            "  models     chat {}, embed {}",
            opt(&self.models.chat),
            opt(&self.models.embed)
        )?;
        writeln!(f, "  roots      {}", self.roots.len())?;
        writeln!(
            f,
            "  files      {} total: {} indexed, {} new, {} failed, {} stale, {} skipped",
            self.files.total(),
            self.files.indexed,
            self.files.new,
            self.files.failed,
            self.files.stale,
            self.files.skipped
        )?;
        writeln!(
            f,
            "  daemon     {}",
            if self.daemon_running {
                "running"
            } else {
                "not running"
            }
        )?;
        writeln!(
            f,
            "  jobs       {} queued, {} running",
            self.jobs_queued, self.jobs_running
        )
    }
}

fn opt(s: &Option<String>) -> &str {
    s.as_deref().unwrap_or("none")
}
