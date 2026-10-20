//! GeoRAG inference engine.
//!
//! Wraps llama.cpp (through `llama-cpp-2`) behind the `InferenceEngine`
//! trait. This crate and `georag-extract` are the only places FFI is allowed;
//! in P0 the crate itself contains no `unsafe` because the device probe goes
//! through the safe wrapper.

#[cfg(all(feature = "cuda", feature = "vulkan"))]
compile_error!("enable only one of the `cuda` and `vulkan` features: a bundle has one flavour");

pub mod backend;
pub mod error;
pub mod fit;
pub mod gguf;
pub mod llama;
pub mod types;

pub use backend::{Backend, BackendInfo, BackendPreference, DeviceInfo, DeviceKind};
pub use error::EngineError;
pub use gguf::GgufInfo;
pub use llama::LlamaCppEngine;
pub use types::{
    CancelToken, ChatMessage, EngineStats, FinishReason, FitVerdict, GenRequest, GenResult,
    InferenceEngine, Lane, LoadSpec, Role, TokenSink,
};

/// Build flavour: target triple plus `-cuda` or `-vulkan` where relevant.
pub const FLAVOUR: &str = env!("GEORAG_FLAVOUR");
