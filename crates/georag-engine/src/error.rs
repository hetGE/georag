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
}
