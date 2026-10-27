//! Ingestion (handoff section 6): scan, extract, tag, chunk, embed, index.
//! Every stage is a job row per file; the supervisor runs stages with
//! bounded parallelism and reports progress as events.

pub mod chunker;
pub mod scan;
pub mod stages;
pub mod supervisor;
pub mod watch;

pub use scan::{ScanReport, scan_all, scan_root};
pub use supervisor::{IndexOptions, PipelineEvent, Report, run};
