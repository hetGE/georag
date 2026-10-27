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
pub mod jobs;
pub mod meta;
pub mod search;
pub mod store;
pub mod tags;
pub mod time;
pub mod v1;

pub use chunks::{Chunk, NewChunk};
pub use conversations::{Conversation, Message, NewMessage, NewPass, Pass, Source};
pub use error::StoreError;
pub use files::{FacetCounts, File, FileQuery, FileState, Root, ScanRecord};
pub use jobs::Job;
pub use search::{Hit, SearchFilter};
pub use store::Store;
pub use tags::{FileTag, Tag};

pub type Result<T> = std::result::Result<T, StoreError>;
