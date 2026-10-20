//! SQLite store: metadata, vectors (sqlite-vec), full text (FTS5), chats, jobs.
//!
//! P1 adds conversations and models; P2 adds files, chunks and search. The
//! schema lives in numbered migrations applied in order.

/// Errors from the store.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("the store is not available yet ({0})")]
    NotReady(&'static str),
}
