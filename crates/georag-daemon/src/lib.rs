//! Local daemon: axum on 127.0.0.1, SSE events, job supervisor, discovery file.
//!
//! P3 fills this crate.

/// Errors from the daemon.
#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error("the daemon is not built yet (phase P3)")]
    NotBuilt,
}
