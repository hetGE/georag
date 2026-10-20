//! Library roots and files. P1 needs only enough to seed chunks; P2 adds
//! scanning, state queries and facets.

use std::path::Path;

use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use crate::Result;
use crate::store::Store;
use crate::time::now_rfc3339;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Root {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub added_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FileState {
    New,
    Imported,
    Extracted,
    Tagged,
    Indexed,
    Failed,
    Stale,
    Skipped,
}

impl FileState {
    pub fn as_str(self) -> &'static str {
        match self {
            FileState::New => "new",
            FileState::Imported => "imported",
            FileState::Extracted => "extracted",
            FileState::Tagged => "tagged",
            FileState::Indexed => "indexed",
            FileState::Failed => "failed",
            FileState::Stale => "stale",
            FileState::Skipped => "skipped",
        }
    }

    pub fn parse(s: &str) -> Option<FileState> {
        Some(match s {
            "new" => FileState::New,
            "imported" => FileState::Imported,
            "extracted" => FileState::Extracted,
            "tagged" => FileState::Tagged,
            "indexed" => FileState::Indexed,
            "failed" => FileState::Failed,
            "stale" => FileState::Stale,
            "skipped" => FileState::Skipped,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct File {
    pub id: i64,
    pub root_id: i64,
    pub rel_path: String,
    pub filename: String,
    pub ext: Option<String>,
    pub size_bytes: Option<i64>,
    pub state: FileState,
    pub chunk_count: i64,
    pub token_count: i64,
    pub preview: Option<String>,
}

impl Store {
    pub fn root_add(&self, name: &str, path: &Path) -> Result<Root> {
        let added_at = now_rfc3339();
        let conn = self.writer();
        conn.execute(
            "INSERT INTO library_roots(name, path, added_at) VALUES (?1, ?2, ?3)",
            params![name, path.to_string_lossy(), added_at],
        )?;
        let id = conn.last_insert_rowid();
        Ok(Root {
            id,
            name: name.to_string(),
            path: path.to_string_lossy().to_string(),
            added_at,
        })
    }

    pub fn roots(&self) -> Result<Vec<Root>> {
        self.read(|c| {
            let mut st =
                c.prepare("SELECT id, name, path, added_at FROM library_roots ORDER BY id")?;
            let rows = st.query_map([], |r| {
                Ok(Root {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    path: r.get(2)?,
                    added_at: r.get(3)?,
                })
            })?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// Insert or update a file row by (root, rel_path); returns its id.
    pub fn file_upsert(
        &self,
        root_id: i64,
        rel_path: &str,
        size_bytes: Option<i64>,
        state: FileState,
    ) -> Result<i64> {
        let filename = Path::new(rel_path)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        let ext = Path::new(rel_path)
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase());
        let conn = self.writer();
        conn.execute(
            "INSERT INTO files(root_id, rel_path, filename, ext, size_bytes, state)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(root_id, rel_path) DO UPDATE SET filename = excluded.filename, ext = excluded.ext,
               size_bytes = excluded.size_bytes, state = excluded.state",
            params![root_id, rel_path, filename, ext, size_bytes, state.as_str()],
        )?;
        let id: i64 = conn.query_row(
            "SELECT id FROM files WHERE root_id = ?1 AND rel_path = ?2",
            params![root_id, rel_path],
            |r| r.get(0),
        )?;
        Ok(id)
    }

    pub fn file_set_state(&self, id: i64, state: FileState, error: Option<&str>) -> Result<()> {
        self.writer().execute(
            "UPDATE files SET state = ?2, error = ?3 WHERE id = ?1",
            params![id, state.as_str(), error],
        )?;
        Ok(())
    }

    pub fn file_get(&self, id: i64) -> Result<Option<File>> {
        self.read(|c| {
            Ok(c.query_row(
                "SELECT id, root_id, rel_path, filename, ext, size_bytes, state, chunk_count, token_count, preview FROM files WHERE id = ?1",
                [id],
                |r| {
                    Ok(File {
                        id: r.get(0)?,
                        root_id: r.get(1)?,
                        rel_path: r.get(2)?,
                        filename: r.get(3)?,
                        ext: r.get(4)?,
                        size_bytes: r.get(5)?,
                        state: FileState::parse(&r.get::<_, String>(6)?).unwrap_or(FileState::New),
                        chunk_count: r.get(7)?,
                        token_count: r.get(8)?,
                        preview: r.get(9)?,
                    })
                },
            )
            .optional()?)
        })
    }

    /// File counts by state, for status and the Home screen.
    pub fn file_counts(&self) -> Result<Vec<(String, i64)>> {
        self.read(|c| {
            let mut st = c.prepare("SELECT state, COUNT(*) FROM files GROUP BY state")?;
            let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }
}
