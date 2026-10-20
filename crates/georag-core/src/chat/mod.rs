//! Chat: single pass, manual tiers and auto depth (handoff section 7).

pub mod budget;
pub mod planner;
pub mod prompts;
pub mod runner;
pub mod tiers;

use serde::{Deserialize, Serialize};

pub use planner::{Pass, Plan, PlanSummary, plan_auto};
pub use runner::{ChatContext, ChatRequest, Controls, RunOutcome, run};
pub use tiers::Tier;

/// Live controls while a multi-pass run is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunControl {
    SkipPass,
    AcceptPass,
    StopAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PassStatus {
    Done,
    Skipped,
    Stopped,
    Accepted,
}

impl PassStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            PassStatus::Done => "done",
            PassStatus::Skipped => "skipped",
            PassStatus::Stopped => "stopped",
            PassStatus::Accepted => "accepted",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Auto,
    Manual,
}

impl std::str::FromStr for Mode {
    type Err = crate::CoreError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Mode::Auto),
            "manual" => Ok(Mode::Manual),
            other => Err(crate::CoreError::Invalid(format!(
                "unknown chat mode {other:?}: use auto or manual"
            ))),
        }
    }
}

/// Events streamed to the CLI, the TUI and (P3) the daemon.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ChatEvent {
    Plan {
        summary: PlanSummary,
    },
    PassStart {
        k: u16,
        sources_from: u32,
        sources_to: u32,
    },
    Token {
        k: Option<u16>,
        text: String,
    },
    PassEnd {
        k: u16,
        status: PassStatus,
        cut_off: bool,
    },
    FinalStart,
    Done {
        message_id: i64,
        sources: Vec<georag_store::Source>,
        cut_off: bool,
    },
    Error {
        message: String,
    },
}
