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
    pub mtime: Option<f64>,
    pub error: Option<String>,
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
                "SELECT id, root_id, rel_path, filename, ext, size_bytes, state, chunk_count, token_count, preview, mtime, error FROM files WHERE id = ?1",
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
                        mtime: r.get(10)?,
                        error: r.get(11)?,
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

/// What the scanner saw for one path.
#[derive(Debug, Clone, PartialEq)]
pub struct ScanRecord {
    pub rel_path: String,
    pub size_bytes: i64,
    pub mtime: f64,
}

/// Filters for the Library screen and `docs list`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileQuery {
    pub text: Option<String>,
    pub state: Option<FileState>,
    pub ext: Option<String>,
    pub tag: Option<String>,
    pub root_id: Option<i64>,
    pub offset: usize,
    pub limit: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct FacetCounts {
    pub total: i64,
    pub by_state: Vec<(String, i64)>,
    pub by_ext: Vec<(String, i64)>,
    pub by_tag: Vec<(String, i64)>,
}

impl Store {
    pub fn root_remove(&self, name: &str) -> Result<bool> {
        Ok(self
            .writer()
            .execute("DELETE FROM library_roots WHERE name = ?1", [name])?
            > 0)
    }

    pub fn root_move(&self, name: &str, path: &Path) -> Result<bool> {
        Ok(self.writer().execute(
            "UPDATE library_roots SET path = ?2 WHERE name = ?1",
            params![name, path.to_string_lossy()],
        )? > 0)
    }

    /// Upsert files seen by a scan. New paths get `new`; changed size or
    /// mtime on an indexed file marks it `stale`; paths not seen become
    /// `skipped` with a note. Returns (new or changed ids, missing count).
    /// Upsert what the walk saw. `unsupported` lists files that are on disk
    /// but of a type georag does not read; they are kept as `skipped` with a
    /// note and never counted as missing. Returns (changed ids, missing).
    pub fn files_apply_scan(
        &self,
        root_id: i64,
        seen: &[ScanRecord],
        unsupported: &[String],
    ) -> Result<(Vec<i64>, usize)> {
        let mut conn = self.writer();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut changed = Vec::new();
        {
            let mut lookup = tx.prepare("SELECT id, size_bytes, mtime, state FROM files WHERE root_id = ?1 AND rel_path = ?2")?;
            let mut insert = tx.prepare(
                "INSERT INTO files(root_id, rel_path, filename, ext, size_bytes, mtime, state) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'new')",
            )?;
            let mut update = tx.prepare("UPDATE files SET size_bytes = ?2, mtime = ?3, state = ?4, error = NULL WHERE id = ?1")?;
            for rec in seen {
                let filename = Path::new(&rec.rel_path)
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default();
                let ext = Path::new(&rec.rel_path)
                    .extension()
                    .map(|e| e.to_string_lossy().to_ascii_lowercase());
                let existing: Option<(i64, Option<i64>, Option<f64>, String)> = lookup
                    .query_row(params![root_id, rec.rel_path], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                    })
                    .optional()?;
                match existing {
                    None => {
                        insert.execute(params![
                            root_id,
                            rec.rel_path,
                            filename,
                            ext,
                            rec.size_bytes,
                            rec.mtime
                        ])?;
                        changed.push(tx.last_insert_rowid());
                    }
                    Some((id, size, mtime, state)) => {
                        let same = size == Some(rec.size_bytes)
                            && mtime
                                .map(|m| (m - rec.mtime).abs() < 0.001)
                                .unwrap_or(false);
                        if state == "skipped" {
                            // Back on disk: treat as new if it was never indexed, else stale.
                            update.execute(params![id, rec.size_bytes, rec.mtime, "new"])?;
                            changed.push(id);
                        } else if !same {
                            let next = if state == "indexed"
                                || state == "tagged"
                                || state == "extracted"
                            {
                                "stale"
                            } else {
                                state.as_str()
                            };
                            update.execute(params![id, rec.size_bytes, rec.mtime, next])?;
                            changed.push(id);
                        }
                    }
                }
            }
        }
        // Paths not seen in this scan.
        let mut missing = 0usize;
        {
            let mut st = tx.prepare("SELECT id, rel_path, state FROM files WHERE root_id = ?1")?;
            let rows: Vec<(i64, String, String)> = st
                .query_map([root_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let seen_set: std::collections::HashSet<&str> =
                seen.iter().map(|s| s.rel_path.as_str()).collect();
            let unsupported_set: std::collections::HashSet<&str> =
                unsupported.iter().map(String::as_str).collect();
            let mut mark =
                tx.prepare("UPDATE files SET state = 'skipped', error = ?2 WHERE id = ?1")?;
            for (id, rel, state) in rows {
                if seen_set.contains(rel.as_str()) {
                    continue;
                }
                if unsupported_set.contains(rel.as_str()) {
                    // Relabelled on every scan so an older note never sticks.
                    mark.execute(params![id, "unsupported file type"])?;
                } else if state != "skipped" {
                    mark.execute(params![id, "missing on disk"])?;
                    missing += 1;
                }
            }
        }
        tx.commit()?;
        Ok((changed, missing))
    }

    pub fn file_set_v1_note(&self, id: i64, note: &str) -> Result<()> {
        self.writer().execute(
            "UPDATE files SET v1_note = ?2 WHERE id = ?1",
            params![id, note],
        )?;
        Ok(())
    }

    pub fn file_set_hash(&self, id: i64, hash: &str) -> Result<()> {
        self.writer().execute(
            "UPDATE files SET hash = ?2 WHERE id = ?1",
            params![id, hash],
        )?;
        Ok(())
    }

    /// Apply freshly computed hashes in one transaction: record the hash,
    /// settle the state and queue `extract` where the content is new or
    /// changed. Returns (queued, unchanged).
    pub fn files_finish_scan(&self, hashes: &[(i64, String)]) -> Result<(usize, usize)> {
        let mut conn = self.writer();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut queued = 0usize;
        let mut unchanged = 0usize;
        {
            let mut current = tx.prepare("SELECT hash, state FROM files WHERE id = ?1")?;
            let mut set_hash =
                tx.prepare("UPDATE files SET hash = ?2, state = ?3, error = NULL WHERE id = ?1")?;
            let mut back_to_indexed =
                tx.prepare("UPDATE files SET state = 'indexed', error = NULL WHERE id = ?1")?;
            let mut has_job = tx.prepare(
                "SELECT 1 FROM jobs WHERE kind = 'extract' AND file_id = ?1 AND state IN ('queued','running') LIMIT 1",
            )?;
            let mut enqueue = tx.prepare(
                "INSERT INTO jobs(kind, file_id, payload, state, attempts, created_at, updated_at) VALUES ('extract', ?1, NULL, 'queued', 0, ?2, ?2)",
            )?;
            let now = crate::time::now_rfc3339();
            for (id, hash) in hashes {
                let Some((previous, state)) = current
                    .query_row([id], |r| {
                        Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?))
                    })
                    .optional()?
                else {
                    continue;
                };
                if previous.as_deref() == Some(hash.as_str()) && state == "stale" {
                    back_to_indexed.execute([id])?;
                    unchanged += 1;
                    continue;
                }
                let next = if state == "imported" {
                    "imported"
                } else {
                    "new"
                };
                set_hash.execute(params![id, hash, next])?;
                if !has_job.exists([id])? {
                    enqueue.execute(params![id, now])?;
                }
                queued += 1;
            }
        }
        tx.commit()?;
        Ok((queued, unchanged))
    }

    pub fn file_hash(&self, id: i64) -> Result<Option<String>> {
        self.read(|c| {
            Ok(
                c.query_row("SELECT hash FROM files WHERE id = ?1", [id], |r| r.get(0))
                    .optional()?
                    .flatten(),
            )
        })
    }

    /// Ids of files in the given states, oldest first.
    pub fn files_in_state(&self, states: &[FileState], limit: usize) -> Result<Vec<i64>> {
        self.read(|c| {
            let placeholders = vec!["?"; states.len()].join(",");
            let mut st = c.prepare(&format!(
                "SELECT id FROM files WHERE state IN ({placeholders}) ORDER BY id LIMIT ?"
            ))?;
            let mut values: Vec<rusqlite::types::Value> = states
                .iter()
                .map(|s| s.as_str().to_string().into())
                .collect();
            values.push((limit as i64).into());
            let rows = st.query_map(rusqlite::params_from_iter(values.iter()), |r| r.get(0))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// Remove `skipped` files (and their chunks through cascade).
    pub fn files_prune_skipped(&self) -> Result<usize> {
        Ok(self
            .writer()
            .execute("DELETE FROM files WHERE state = 'skipped'", [])?)
    }

    pub fn file_delete(&self, id: i64) -> Result<bool> {
        Ok(self
            .writer()
            .execute("DELETE FROM files WHERE id = ?1", [id])?
            > 0)
    }

    /// Query files with filters; `limit` 0 means no limit.
    pub fn files_query(&self, q: &FileQuery) -> Result<Vec<File>> {
        self.read(|c| {
            let (where_sql, values) = file_filter(q);
            let limit = if q.limit == 0 { i64::MAX } else { q.limit as i64 };
            let sql = format!(
                "SELECT f.id, f.root_id, f.rel_path, f.filename, f.ext, f.size_bytes, f.state, f.chunk_count, f.token_count, f.preview, f.mtime, f.error
                 FROM files f {where_sql} ORDER BY f.rel_path LIMIT ? OFFSET ?"
            );
            let mut st = c.prepare(&sql)?;
            let mut values = values;
            values.push(limit.into());
            values.push((q.offset as i64).into());
            let rows = st.query_map(rusqlite::params_from_iter(values.iter()), |r| {
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
                    mtime: r.get(10)?,
                    error: r.get(11)?,
                })
            })?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// Counts by state, extension and tag for the current filter.
    pub fn files_facets(&self, q: &FileQuery) -> Result<FacetCounts> {
        self.read(|c| {
            let (where_sql, values) = file_filter(q);
            let total: i64 = c.query_row(&format!("SELECT COUNT(*) FROM files f {where_sql}"), rusqlite::params_from_iter(values.iter()), |r| r.get(0))?;
            let group = |col: &str| -> Result<Vec<(String, i64)>> {
                let mut st = c.prepare(&format!("SELECT COALESCE(f.{col}, ''), COUNT(*) FROM files f {where_sql} GROUP BY f.{col} ORDER BY 2 DESC, 1"))?;
                let rows = st.query_map(rusqlite::params_from_iter(values.iter()), |r| Ok((r.get(0)?, r.get(1)?)))?;
                Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
            };
            let by_state = group("state")?;
            let by_ext = group("ext")?;
            let mut st = c.prepare(&format!(
                "SELECT t.name, COUNT(DISTINCT f.id) FROM files f JOIN file_tags ft ON ft.file_id = f.id JOIN tags t ON t.id = ft.tag_id {where_sql} GROUP BY t.name ORDER BY 2 DESC, 1"
            ))?;
            let rows = st.query_map(rusqlite::params_from_iter(values.iter()), |r| Ok((r.get(0)?, r.get(1)?)))?;
            let by_tag = rows.collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(FacetCounts { total, by_state, by_ext, by_tag })
        })
    }
}

fn file_filter(q: &FileQuery) -> (String, Vec<rusqlite::types::Value>) {
    let mut clauses: Vec<String> = Vec::new();
    let mut values: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(t) = &q.text
        && !t.trim().is_empty()
    {
        clauses.push("f.rel_path LIKE ?".into());
        values.push(format!("%{}%", t.trim()).into());
    }
    if let Some(s) = q.state {
        clauses.push("f.state = ?".into());
        values.push(s.as_str().to_string().into());
    }
    if let Some(e) = &q.ext {
        clauses.push("f.ext = ?".into());
        values.push(e.to_ascii_lowercase().into());
    }
    if let Some(r) = q.root_id {
        clauses.push("f.root_id = ?".into());
        values.push(r.into());
    }
    if let Some(tag) = &q.tag {
        clauses.push("f.id IN (SELECT ft.file_id FROM file_tags ft JOIN tags t ON t.id = ft.tag_id WHERE t.name = ?)".into());
        values.push(tag.clone().into());
    }
    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };
    (where_sql, values)
}

#[cfg(test)]
mod scan_tests {
    use super::*;

    #[test]
    fn scan_keeps_unsupported_files_as_skipped_not_missing() {
        let store = Store::open_in_memory().unwrap();
        let root = store.root_add("lib", Path::new("/lib")).unwrap();
        let (changed, _) = store
            .files_apply_scan(
                root.id,
                &[ScanRecord {
                    rel_path: "old.doc".into(),
                    size_bytes: 10,
                    mtime: 1.0,
                }],
                &[],
            )
            .unwrap();
        let (changed2, missing) = store
            .files_apply_scan(root.id, &[], &["old.doc".to_string()])
            .unwrap();
        assert!(changed2.is_empty());
        assert_eq!(missing, 0);
        let file = store.file_get(changed[0]).unwrap().unwrap();
        assert_eq!(file.state, FileState::Skipped);
        assert_eq!(file.error.as_deref(), Some("unsupported file type"));
    }

    #[test]
    fn scan_marks_new_stale_and_missing() {
        let store = Store::open_in_memory().unwrap();
        let root = store.root_add("lib", Path::new("/lib")).unwrap();
        let (changed, missing) = store
            .files_apply_scan(
                root.id,
                &[
                    ScanRecord {
                        rel_path: "a.pdf".into(),
                        size_bytes: 10,
                        mtime: 1.0,
                    },
                    ScanRecord {
                        rel_path: "b/c.docx".into(),
                        size_bytes: 20,
                        mtime: 2.0,
                    },
                ],
                &[],
            )
            .unwrap();
        assert_eq!(changed.len(), 2);
        assert_eq!(missing, 0);
        store
            .file_set_state(changed[0], FileState::Indexed, None)
            .unwrap();
        // a.pdf changed, c.docx vanished.
        let (changed2, missing2) = store
            .files_apply_scan(
                root.id,
                &[ScanRecord {
                    rel_path: "a.pdf".into(),
                    size_bytes: 11,
                    mtime: 1.0,
                }],
                &[],
            )
            .unwrap();
        assert_eq!(changed2, vec![changed[0]]);
        assert_eq!(missing2, 1);
        assert_eq!(
            store.file_get(changed[0]).unwrap().unwrap().state,
            FileState::Stale
        );
        assert_eq!(
            store.file_get(changed[1]).unwrap().unwrap().state,
            FileState::Skipped
        );
        let facets = store.files_facets(&FileQuery::default()).unwrap();
        assert_eq!(facets.total, 2);
        assert!(facets.by_ext.iter().any(|(e, n)| e == "pdf" && *n == 1));
        let q = FileQuery {
            state: Some(FileState::Skipped),
            ..Default::default()
        };
        assert_eq!(store.files_query(&q).unwrap().len(), 1);
        assert_eq!(store.files_prune_skipped().unwrap(), 1);
    }

    #[test]
    fn tags_attach_with_manual_precedence() {
        let store = Store::open_in_memory().unwrap();
        let root = store.root_add("lib", Path::new("/lib")).unwrap();
        let id = store
            .file_upsert(root.id, "piles/x.pdf", None, FileState::New)
            .unwrap();
        store
            .tag_upsert("piling", "Piling", Some("piles"), Some("#fff"))
            .unwrap();
        store
            .file_tag_add(id, "piling", "rule", Some(0.7), false)
            .unwrap();
        assert_eq!(store.file_tags(id).unwrap()[0].source, "rule");
        store
            .file_tag_add(id, "piling", "manual", None, false)
            .unwrap();
        store
            .file_tag_add(id, "piling", "llm", Some(0.8), false)
            .unwrap();
        let t = &store.file_tags(id).unwrap()[0];
        assert_eq!(t.source, "manual");
        assert_eq!(t.confidence, Some(1.0));
        assert!(store.file_has_tags(id).unwrap());
        assert_eq!(store.tag_counts().unwrap(), vec![("piling".to_string(), 1)]);
        let q = FileQuery {
            tag: Some("piling".into()),
            ..Default::default()
        };
        assert_eq!(store.files_query(&q).unwrap().len(), 1);
        assert!(store.file_tag_remove(id, "piling").unwrap());
        assert!(!store.file_has_tags(id).unwrap());
    }
}
