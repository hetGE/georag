//! The `InferenceEngine` trait and its request and result types (handoff
//! section 2.3). `LlamaCppEngine` implements it in P1; P0 fixes the shape so
//! core can be written against it.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::backend::BackendInfo;
use crate::error::EngineError;

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

/// Models to make resident. Idempotent: loading the same spec twice is a no-op.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadSpec {
    pub chat: Option<PathBuf>,
    pub mmproj: Option<PathBuf>,
    pub embed: Option<PathBuf>,
    pub n_ctx: u32,
    pub gpu_layers: i32,
    pub kv_cache_type: String,
}

/// What the GGUF header says about a model, enough for the memory fit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GgufInfo {
    pub architecture: String,
    pub n_ctx_train: u32,
    pub n_layer: u32,
    pub n_head_kv: u32,
    pub head_dim: u32,
    pub weight_bytes: u64,
    pub embed_dim: Option<u32>,
}

/// Result of `fits`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FitVerdict {
    Fits { needed_bytes: u64, free_bytes: u64 },
    DoesNotFit { needed_bytes: u64, free_bytes: u64 },
}

pub type Token = i32;

/// One generation request.
#[derive(Debug, Clone)]
pub struct GenRequest {
    pub prompt_tokens: Vec<Token>,
    pub max_tokens: u32,
    pub temperature: f32,
}

/// Why generation stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishReason {
    Stop,
    Length,
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct GenResult {
    pub text: String,
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
#[derive(Debug, Clone, Default)]
pub struct EngineStats {
    pub chat_model: Option<String>,
    pub embed_model: Option<String>,
    pub n_ctx: u32,
    pub memory_used_bytes: u64,
    pub tokens_per_second: f32,
}

/// The engine every front end talks to through core.
pub trait InferenceEngine: Send + Sync {
    fn backend(&self) -> BackendInfo;
    fn load(&self, spec: LoadSpec) -> Result<(), EngineError>;
    fn resize_ctx(&self, n_ctx: u32) -> Result<(), EngineError>;
    fn fits(&self, model: &GgufInfo, n_ctx: u32) -> FitVerdict;
    fn tokenize(&self, role: Role, text: &str) -> Result<Vec<Token>, EngineError>;
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
