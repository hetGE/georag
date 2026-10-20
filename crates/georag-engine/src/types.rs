//! The `InferenceEngine` trait and its request and result types (handoff
//! section 2.3). `LlamaCppEngine` implements it.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;

use crate::backend::BackendInfo;
use crate::error::EngineError;
use crate::gguf::GgufInfo;

/// Which resident model a request addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Chat,
    Embed,
}

/// Priority lanes on the engine inbox (section 5.3). Lower is served first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Lane {
    /// Chat generation, question embedding, pass synthesis.
    Interactive = 0,
    /// Image captions and the LLM tag fallback.
    Vision = 1,
    /// Embedding batches during indexing.
    Bulk = 2,
}

/// Models to make resident. Idempotent: loading the same files again only
/// resizes the context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadSpec {
    pub chat: Option<PathBuf>,
    pub mmproj: Option<PathBuf>,
    pub embed: Option<PathBuf>,
    pub n_ctx: u32,
    pub gpu_layers: i32,
    pub kv_cache_type: String,
}

/// Result of `fits`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum FitVerdict {
    Fits { needed_bytes: u64, free_bytes: u64 },
    DoesNotFit { needed_bytes: u64, free_bytes: u64 },
}

pub type Token = i32;

/// One chat turn for the model's template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn new(role: &str, content: impl Into<String>) -> Self {
        Self {
            role: role.to_string(),
            content: content.into(),
        }
    }
}

/// One generation request over an already templated prompt.
#[derive(Debug, Clone)]
pub struct GenRequest {
    pub prompt: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub seed: u32,
}

/// Why generation stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
pub struct GenResult {
    pub text: String,
    pub prompt_tokens: u32,
    pub tokens_generated: u32,
    pub finish_reason: FinishReason,
    pub tokens_per_second: f32,
}

/// Receives streamed token text as it is produced.
pub type TokenSink = Box<dyn FnMut(&str) + Send>;

/// Cooperative cancellation, checked between tokens.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Snapshot for the status bar and the Status screen.
#[derive(Debug, Clone, Default, Serialize)]
pub struct EngineStats {
    pub chat_model: Option<String>,
    pub embed_model: Option<String>,
    pub n_ctx: u32,
    pub n_ctx_train: u32,
    /// Rungs of the context ladder that fit, ascending.
    pub ladder: Vec<u32>,
    pub memory_used_bytes: u64,
    pub memory_total_bytes: u64,
    pub tokens_per_second: f32,
    pub loaded: bool,
    pub resident_since: Option<String>,
}

/// The engine every front end talks to through core.
pub trait InferenceEngine: Send + Sync {
    fn backend(&self) -> BackendInfo;
    fn load(&self, spec: LoadSpec) -> Result<(), EngineError>;
    fn resize_ctx(&self, n_ctx: u32) -> Result<(), EngineError>;
    fn fits(&self, model: &GgufInfo, n_ctx: u32) -> FitVerdict;
    /// Largest ladder rung that fits the loaded (or given) chat model.
    fn max_ctx_that_fits(&self) -> Option<u32>;
    fn tokenize(&self, role: Role, text: &str) -> Result<Vec<Token>, EngineError>;
    /// Render messages with the chat model's template, ready for `generate`.
    fn chat_prompt(&self, messages: &[ChatMessage], thinking: bool) -> Result<String, EngineError>;
    fn embed(&self, texts: &[String], prio: Lane) -> Result<Vec<Vec<f32>>, EngineError>;
    fn generate(
        &self,
        req: GenRequest,
        prio: Lane,
        sink: TokenSink,
        cancel: CancelToken,
    ) -> Result<GenResult, EngineError>;
    fn caption(&self, image: &[u8], prompt: &str, prio: Lane) -> Result<String, EngineError>;
    fn stats(&self) -> EngineStats;
}
