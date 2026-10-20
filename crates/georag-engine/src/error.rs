use crate::backend::Backend;

/// Errors raised by the engine crate.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error(
        "backend {requested} was requested but this {flavour} build found no {requested} device: set engine.backend = \"auto\", or install the {requested} flavour of GeoRAG"
    )]
    BackendUnavailable {
        requested: Backend,
        flavour: &'static str,
    },

    #[error("unknown backend name {0:?}: use auto, metal, cuda, vulkan or cpu")]
    InvalidBackend(String),

    #[error(
        "llama.cpp reported no compute devices at all, not even the CPU: the engine libraries may be missing or broken"
    )]
    NoDevices,

    #[error("llama.cpp failed to initialise ({0})")]
    Init(String),

    #[error("could not read the GGUF header of {path} ({reason})")]
    Gguf {
        path: std::path::PathBuf,
        reason: String,
    },

    #[error("could not load {path} ({reason}): check the file with `georag models verify`")]
    Load {
        path: std::path::PathBuf,
        reason: String,
    },

    #[error(
        "could not create a context of {n_ctx} tokens ({reason}): lower engine.max_ctx or pick a smaller model"
    )]
    Context { n_ctx: u32, reason: String },

    #[error("no {0} model is loaded: pull one with `georag models pull` and mark it in use")]
    NotLoaded(&'static str),

    #[error(
        "the prompt is {tokens} tokens but the context holds {n_ctx}: shorten the history or raise the context"
    )]
    PromptTooLong { tokens: usize, n_ctx: u32 },

    #[error("generation failed ({0})")]
    Decode(String),

    #[error("{0} is not available yet")]
    NotSupported(&'static str),

    #[error("the engine thread has stopped")]
    Stopped,
}
