//! Walk roots, upsert what is there, hash what changed, queue extraction.

use std::path::Path;

use georag_store::{Root, ScanRecord, Store};
use serde::Serialize;
use walkdir::WalkDir;

use crate::error::{CoreError, Result};

/// Folders never entered.
pub const SKIP_DIRS: &[&str] = &[
    "_0RAG",
    ".git",
    "__pycache__",
    "node_modules",
    "venv",
    ".venv",
    "target",
];
/// File names never indexed.
pub const JUNK_FILES: &[&str] = &[
    ".DS_Store",
    "Thumbs.db",
    "desktop.ini",
    "ehthumbs.db",
    "ehthumbs_vista.db",
];

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct ScanReport {
    pub root: String,
    pub seen: usize,
    pub queued: usize,
    pub unchanged: usize,
    pub missing: usize,
}

fn wanted(entry: &walkdir::DirEntry) -> bool {
    let name = entry.file_name().to_string_lossy();
    if name.starts_with('.') || name.starts_with("._") {
        return false;
    }
    if entry.file_type().is_dir() {
        return !SKIP_DIRS.contains(&name.as_ref());
    }
    true
}

/// List supported files under a root as scan records.
pub fn walk(root_path: &Path) -> Result<Vec<ScanRecord>> {
    Ok(walk_all(root_path)?.0)
}

/// Supported files as scan records, plus the relative paths of files that
/// are present but of a type georag does not read.
pub fn walk_all(root_path: &Path) -> Result<(Vec<ScanRecord>, Vec<String>)> {
    let mut out = Vec::new();
    let mut unsupported = Vec::new();
    for entry in WalkDir::new(root_path)
        .follow_links(false)
        .into_iter()
        .filter_entry(wanted)
    {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("scan: {e}");
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy();
        if JUNK_FILES.contains(&name.as_ref()) {
            continue;
        }
        let ext = entry
            .path()
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let rel = entry
            .path()
            .strip_prefix(root_path)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .replace('\\', "/");
        if !georag_extract::handles(&ext) {
            unsupported.push(rel);
            continue;
        }
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        out.push(ScanRecord {
            rel_path: rel,
            size_bytes: meta.len() as i64,
            mtime,
        });
    }
    out.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok((out, unsupported))
}

pub fn blake3_file(path: &Path) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    let mut file = std::fs::File::open(path).map_err(|source| CoreError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    std::io::copy(&mut file, &mut hasher).map_err(|source| CoreError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(hasher.finalize().to_hex().to_string())
}

/// Files hashed between two writes to the store.
const HASH_BATCH: usize = 256;

/// Hashing is disk bound: a few threads saturate an SSD without starving
/// the rest of the machine.
fn hash_pool() -> rayon::ThreadPool {
    rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .thread_name(|i| format!("georag-hash-{i}"))
        .build()
        .expect("hash pool")
}

/// Scan one root: upsert, hash changed files, queue `extract` for files
/// whose content changed or was never processed.
pub fn scan_root(store: &Store, root: &Root) -> Result<ScanReport> {
    let root_path = Path::new(&root.path);
    if !root_path.is_dir() {
        return Err(CoreError::Invalid(format!(
            "root {:?} is not reachable at {}: re-point it with `georag library move`",
            root.name, root.path
        )));
    }
    let started = std::time::Instant::now();
    let (seen, unsupported) = walk_all(root_path)?;
    let (changed, missing) = store.files_apply_scan(root.id, &seen, &unsupported)?;
    tracing::info!(
        "scan {}: {} files seen, {} to hash, {} missing, walk took {:.1}s",
        root.name,
        seen.len(),
        changed.len(),
        missing,
        started.elapsed().as_secs_f64()
    );
    let mut queued = 0usize;
    let mut unchanged = 0usize;
    let mut hashed = 0usize;
    for ids in changed.chunks(HASH_BATCH) {
        let hashes: Vec<(i64, String)> = hash_pool().install(|| {
            use rayon::prelude::*;
            ids.par_iter()
                .filter_map(|&id| {
                    let file = store.file_get(id).ok().flatten()?;
                    let abs = root_path.join(&file.rel_path);
                    match blake3_file(&abs) {
                        Ok(h) => Some((id, h)),
                        Err(e) => {
                            tracing::warn!("hash {}: {e}", abs.display());
                            None
                        }
                    }
                })
                .collect()
        });
        let (q, u) = store.files_finish_scan(&hashes)?;
        queued += q;
        unchanged += u;
        hashed += hashes.len();
        if changed.len() > HASH_BATCH {
            tracing::info!(
                "scan {}: hashed {} of {} files, {:.1}s so far",
                root.name,
                hashed,
                changed.len(),
                started.elapsed().as_secs_f64()
            );
        }
    }
    tracing::info!(
        "scan {}: {} queued for extraction, {} unchanged, took {:.1}s",
        root.name,
        queued,
        unchanged,
        started.elapsed().as_secs_f64()
    );
    Ok(ScanReport {
        root: root.name.clone(),
        seen: seen.len(),
        queued,
        unchanged,
        missing,
    })
}

pub fn scan_all(store: &Store) -> Result<Vec<ScanReport>> {
    let mut reports = Vec::new();
    for root in store.roots()? {
        match scan_root(store, &root) {
            Ok(r) => reports.push(r),
            Err(e) => {
                tracing::warn!("{e}");
                reports.push(ScanReport {
                    root: root.name.clone(),
                    ..Default::default()
                });
            }
        }
    }
    Ok(reports)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_skips_hidden_junk_and_unsupported() {
        let dir = std::env::temp_dir().join(format!("georag-walk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("piles/_0RAG")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join("piles/report.pdf"), b"x").unwrap();
        std::fs::write(dir.join("piles/_0RAG/metadata.db"), b"x").unwrap();
        std::fs::write(dir.join("piles/.DS_Store"), b"x").unwrap();
        std::fs::write(dir.join("piles/._report.pdf"), b"x").unwrap();
        std::fs::write(dir.join("notes.TXT"), b"x").unwrap();
        std::fs::write(dir.join("archive.zip"), b"x").unwrap();
        std::fs::write(dir.join(".git/config"), b"x").unwrap();
        let seen = walk(&dir).unwrap();
        let paths: Vec<&str> = seen.iter().map(|s| s.rel_path.as_str()).collect();
        assert_eq!(paths, vec!["notes.TXT", "piles/report.pdf"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
