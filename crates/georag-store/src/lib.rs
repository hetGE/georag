//! SQLite store: metadata, vectors (sqlite-vec), full text (FTS5), chats, jobs.
//!
//! One writer connection behind a mutex, a small pool of read connections,
//! WAL mode, 5 s busy timeout. Core uses only the typed API here; no SQL
//! leaves this crate.
//!
//! `unsafe` appears once, to register the sqlite-vec extension (ADR 0002).
#![deny(unsafe_code)]

pub mod chunks;
pub mod conversations;
pub mod error;
pub mod files;
pub mod meta;
pub mod search;
pub mod store;
pub mod time;

pub use chunks::{Chunk, NewChunk};
pub use conversations::{Conversation, Message, NewMessage, NewPass, Pass, Source};
pub use error::StoreError;
pub use files::{File, FileState, Root};
pub use search::{Hit, SearchFilter};
pub use store::Store;

pub type Result<T> = std::result::Result<T, StoreError>;
