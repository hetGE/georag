//! Walk roots, upsert what is there, hash what changed, queue extraction.

use std::path::Path;

use georag_store::{FileState, Root, ScanRecord, Store};
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
    let mut out = Vec::new();
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
        if !georag_extract::handles(&ext) {
            continue;
        }
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let rel = entry
            .path()
            .strip_prefix(root_path)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .replace('\\', "/");
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
    Ok(out)
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
    let seen = walk(root_path)?;
    let (changed, missing) = store.files_apply_scan(root.id, &seen)?;
    let mut queued = 0usize;
    let mut unchanged = 0usize;
    for id in changed {
        let Some(file) = store.file_get(id)? else {
            continue;
        };
        let abs = root_path.join(&file.rel_path);
        let hash = match blake3_file(&abs) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!("hash {}: {e}", abs.display());
                continue;
            }
        };
        let previous = store.file_hash(id)?;
        if previous.as_deref() == Some(hash.as_str()) && file.state == FileState::Stale {
            // Same content, only metadata moved: back to indexed.
            store.file_set_state(id, FileState::Indexed, None)?;
            unchanged += 1;
            continue;
        }
        store.file_set_hash(id, &hash)?;
        let next = match file.state {
            FileState::Imported => FileState::Imported,
            _ => FileState::New,
        };
        store.file_set_state(id, next, None)?;
        store.job_enqueue("extract", Some(id), None)?;
        queued += 1;
    }
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
