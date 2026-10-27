//! Chunks with their vectors. Reprocessing a file deletes and reinserts its
//! chunks; other files' chunk ids are untouched.

use rusqlite::params;
use serde::Serialize;

use crate::Result;
use crate::StoreError;
use crate::store::Store;

#[derive(Debug, Clone, PartialEq)]
pub struct NewChunk {
    pub idx: i64,
    pub text: String,
    pub tokens: i64,
    pub page: Option<i64>,
    pub locator: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Chunk {
    pub id: i64,
    pub file_id: i64,
    pub rel_path: String,
    pub idx: i64,
    pub text: String,
    pub tokens: i64,
    pub page: Option<i64>,
    pub locator: Option<String>,
}

pub(crate) fn vec_to_blob(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

pub(crate) fn blob_to_vec(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

impl Store {
    /// Replace every chunk of `file_id` in one transaction, with vectors when
    /// given (one per chunk, same order). Updates the file's counts, preview
    /// and state.
    pub fn chunks_replace_for_file(
        &self,
        file_id: i64,
        chunks: &[NewChunk],
        embeddings: Option<&[Vec<f32>]>,
    ) -> Result<Vec<i64>> {
        if let Some(e) = embeddings
            && e.len() != chunks.len()
        {
            return Err(StoreError::NotFound("one embedding per chunk"));
        }
        let has_vec = self.embed_info()?.is_some();
        if embeddings.is_some() && !has_vec {
            return Err(StoreError::NoVectorTable);
        }
        let mut conn = self.writer();
        let tx = conn.transaction()?;
        if has_vec {
            tx.execute(
                "DELETE FROM chunk_vec WHERE chunk_id IN (SELECT id FROM chunks WHERE file_id = ?1)",
                [file_id],
            )?;
        }
        tx.execute("DELETE FROM chunks WHERE file_id = ?1", [file_id])?;
        let mut ids = Vec::with_capacity(chunks.len());
        {
            let mut ins = tx.prepare(
                "INSERT INTO chunks(file_id, idx, text, tokens, page, locator) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            let mut ins_vec = if embeddings.is_some() {
                Some(tx.prepare("INSERT INTO chunk_vec(chunk_id, embedding) VALUES (?1, ?2)")?)
            } else {
                None
            };
            for (i, c) in chunks.iter().enumerate() {
                ins.execute(params![file_id, c.idx, c.text, c.tokens, c.page, c.locator])?;
                let id = tx.last_insert_rowid();
                if let (Some(st), Some(e)) = (ins_vec.as_mut(), embeddings) {
                    st.execute(params![id, vec_to_blob(&e[i])])?;
                }
                ids.push(id);
            }
        }
        let tokens: i64 = chunks.iter().map(|c| c.tokens).sum();
        let preview: String = chunks
            .first()
            .map(|c| c.text.chars().take(300).collect())
            .unwrap_or_default();
        let state = if embeddings.is_some() {
            "indexed"
        } else {
            "extracted"
        };
        tx.execute(
            "UPDATE files SET chunk_count = ?2, token_count = ?3, preview = ?4, state = ?5,
               indexed_at = CASE WHEN ?5 = 'indexed' THEN ?6 ELSE indexed_at END WHERE id = ?1",
            params![
                file_id,
                chunks.len() as i64,
                tokens,
                preview,
                state,
                crate::time::now_rfc3339()
            ],
        )?;
        tx.commit()?;
        Ok(ids)
    }

    /// Chunks by id, in the order given, with the file's relative path.
    pub fn chunks_by_ids(&self, ids: &[i64]) -> Result<Vec<Chunk>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        self.read(|c| {
            let placeholders = vec!["?"; ids.len()].join(",");
            let sql = format!(
                "SELECT ch.id, ch.file_id, f.rel_path, ch.idx, ch.text, ch.tokens, ch.page, ch.locator
                 FROM chunks ch JOIN files f ON f.id = ch.file_id WHERE ch.id IN ({placeholders})"
            );
            let mut st = c.prepare(&sql)?;
            let rows = st.query_map(rusqlite::params_from_iter(ids.iter()), |r| {
                Ok(Chunk {
                    id: r.get(0)?,
                    file_id: r.get(1)?,
                    rel_path: r.get(2)?,
                    idx: r.get(3)?,
                    text: r.get(4)?,
                    tokens: r.get(5)?,
                    page: r.get(6)?,
                    locator: r.get(7)?,
                })
            })?;
            let mut found: Vec<Chunk> = rows.collect::<std::result::Result<Vec<_>, _>>()?;
            found.sort_by_key(|ch| ids.iter().position(|i| *i == ch.id).unwrap_or(usize::MAX));
            Ok(found)
        })
    }

    pub fn chunk_count(&self) -> Result<i64> {
        self.read(|c| Ok(c.query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))?))
    }
}

impl Store {
    /// Chunks of one file in order, without vectors.
    pub fn chunks_for_file(&self, file_id: i64) -> Result<Vec<Chunk>> {
        self.read(|c| {
            let mut st = c.prepare(
                "SELECT ch.id, ch.file_id, f.rel_path, ch.idx, ch.text, ch.tokens, ch.page, ch.locator
                 FROM chunks ch JOIN files f ON f.id = ch.file_id WHERE ch.file_id = ?1 ORDER BY ch.idx",
            )?;
            let rows = st.query_map([file_id], |r| {
                Ok(Chunk {
                    id: r.get(0)?,
                    file_id: r.get(1)?,
                    rel_path: r.get(2)?,
                    idx: r.get(3)?,
                    text: r.get(4)?,
                    tokens: r.get(5)?,
                    page: r.get(6)?,
                    locator: r.get(7)?,
                })
            })?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// Attach vectors to existing chunks of a file and mark it indexed.
    pub fn chunks_set_vectors(&self, file_id: i64, vectors: &[(i64, Vec<f32>)]) -> Result<()> {
        if self.embed_info()?.is_none() {
            return Err(StoreError::NoVectorTable);
        }
        let mut conn = self.writer();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM chunk_vec WHERE chunk_id IN (SELECT id FROM chunks WHERE file_id = ?1)",
            [file_id],
        )?;
        {
            let mut ins =
                tx.prepare("INSERT INTO chunk_vec(chunk_id, embedding) VALUES (?1, ?2)")?;
            for (id, v) in vectors {
                ins.execute(params![id, vec_to_blob(v)])?;
            }
        }
        tx.execute(
            "UPDATE files SET state = 'indexed', indexed_at = ?2, error = NULL WHERE id = ?1",
            params![file_id, crate::time::now_rfc3339()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Files whose chunks have no vectors yet (after an embedding model change).
    pub fn files_without_vectors(&self) -> Result<Vec<i64>> {
        self.read(|c| {
            if c.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name = 'chunk_vec'", [], |r| r.get::<_, i64>(0))? == 0 {
                let mut st = c.prepare("SELECT DISTINCT file_id FROM chunks")?;
                let rows = st.query_map([], |r| r.get(0))?;
                return Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?);
            }
            let mut st = c.prepare("SELECT DISTINCT ch.file_id FROM chunks ch LEFT JOIN chunk_vec v ON v.chunk_id = ch.id WHERE v.chunk_id IS NULL")?;
            let rows = st.query_map([], |r| r.get(0))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }
}
