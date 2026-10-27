//! GeoRAG core library.
//!
//! Front ends (`georag-cli`, `georag-tui`, `georag-daemon`) talk to this crate
//! only. It owns the home folder layout, settings, logging and, from P1 on,
//! models, retrieval, chat, import and update. The engine, extractors and the
//! store are reached through this crate and never directly.
#![forbid(unsafe_code)]

pub mod app;
pub mod chat;
pub mod config;
pub mod conversations;
pub mod devseed;
pub mod doctor;
pub mod engine;
pub mod error;
pub mod home;
pub mod import_v1;
pub mod library;
pub mod logging;
pub mod models;
pub mod pipeline;
pub mod retrieval;
pub mod status;
pub mod tags;
pub mod update;

pub use error::CoreError;
/// Store records front ends display; the store crate itself stays behind core.
pub use georag_store::{
    Chunk, Conversation, FacetCounts, File, FileQuery, FileState, FileTag, Job, Message, Pass,
    Root, Source, Tag,
};
pub use home::Home;

/// Application version, from the workspace manifest.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Build flavour: target triple plus `-cuda` or `-vulkan` where relevant.
pub const FLAVOUR: &str = georag_engine::FLAVOUR;
