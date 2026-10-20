//! Models: the registry in `config/models.toml`, the compiled-in catalogue,
//! Hugging Face lookups and resumable pulls.

pub mod catalog;
pub mod hf;
pub mod ops;
pub mod pull;
pub mod registry;

pub use catalog::{CatalogEntry, catalog};
pub use pull::{PullManager, PullState, PullStatus};
pub use registry::{ModelEntry, ModelKind, ModelRole, Registry};
