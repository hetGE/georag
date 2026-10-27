//! Library roots: folders the user keeps documents in.

use std::path::{Path, PathBuf};

use georag_store::{Root, Store};

use crate::error::{CoreError, Result};

/// Add a root. The name defaults to the folder name. The v1 `_0RAG` folder
/// inside a root is never scanned.
pub fn add(store: &Store, path: &Path, name: Option<&str>) -> Result<Root> {
    let path = path.canonicalize().map_err(|source| CoreError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if !path.is_dir() {
        return Err(CoreError::Invalid(format!(
            "{} is not a folder",
            path.display()
        )));
    }
    let name = name.map(str::to_string).unwrap_or_else(|| {
        path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "library".into())
    });
    if store.roots()?.iter().any(|r| r.name == name) {
        return Err(CoreError::Invalid(format!(
            "a root named {name:?} exists: pick another with --name"
        )));
    }
    if let Some(existing) = store.roots()?.iter().find(|r| Path::new(&r.path) == path) {
        return Err(CoreError::Invalid(format!(
            "{} is already the root {:?}",
            path.display(),
            existing.name
        )));
    }
    Ok(store.root_add(&name, &path)?)
}

pub fn list(store: &Store) -> Result<Vec<Root>> {
    Ok(store.roots()?)
}

/// Re-point a root when a drive letter or mount changed; files keep their
/// relative paths.
pub fn move_root(store: &Store, name: &str, new_path: &Path) -> Result<()> {
    let new_path = new_path.canonicalize().map_err(|source| CoreError::Io {
        path: new_path.to_path_buf(),
        source,
    })?;
    if !store.root_move(name, &new_path)? {
        return Err(CoreError::Invalid(format!("no root named {name:?}")));
    }
    Ok(())
}

/// Remove a root and every file, chunk and vector under it.
pub fn remove(store: &Store, name: &str) -> Result<()> {
    if !store.root_remove(name)? {
        return Err(CoreError::Invalid(format!("no root named {name:?}")));
    }
    Ok(())
}

/// Delete records of files that are missing on disk.
pub fn prune(store: &Store) -> Result<usize> {
    Ok(store.files_prune_skipped()?)
}

pub fn absolute_path(root: &Root, rel_path: &str) -> PathBuf {
    Path::new(&root.path).join(rel_path)
}
