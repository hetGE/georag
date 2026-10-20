//! GeoRAG core library.
//!
//! Front ends (`georag-cli`, `georag-tui`, `georag-daemon`) talk to this crate
//! only. It owns the home folder layout, settings, logging and, from P1 on,
//! models, retrieval, chat, import and update. The engine, extractors and the
//! store are reached through this crate and never directly.

pub mod config;
pub mod doctor;
pub mod error;
pub mod home;
pub mod logging;

pub use error::CoreError;
pub use home::Home;

/// Application version, from the workspace manifest.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Build flavour: target triple plus `-cuda` or `-vulkan` where relevant.
pub const FLAVOUR: &str = georag_engine::FLAVOUR;
