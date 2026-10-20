use rusqlite::OptionalExtension;

use crate::Result;
use crate::store::Store;

impl Store {
    pub fn meta_get(&self, key: &str) -> Result<Option<String>> {
        self.read(|c| {
            Ok(
                c.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
                    .optional()?,
            )
        })
    }

    pub fn meta_set(&self, key: &str, value: &str) -> Result<()> {
        self.writer().execute(
            "INSERT INTO meta(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [key, value],
        )?;
        Ok(())
    }

    /// Current embedding model id and dimension, if a vector table exists.
    pub fn embed_info(&self) -> Result<Option<(String, usize)>> {
        let model = self.meta_get("embed_model")?;
        let dim = self
            .meta_get("embed_dim")?
            .and_then(|d| d.parse::<usize>().ok());
        Ok(match (model, dim) {
            (Some(m), Some(d)) => Some((m, d)),
            _ => None,
        })
    }

    /// Create (or verify) the vector table for `dim` dimensions and record the
    /// embedding model. A different dimension is an error; the caller drops
    /// vectors explicitly with `reset_vectors` after confirming with the user.
    pub fn ensure_vectors(&self, embed_model: &str, dim: usize) -> Result<()> {
        if let Some((_, existing)) = self.embed_info()?
            && existing != dim
        {
            return Err(crate::StoreError::DimensionMismatch {
                existing,
                requested: dim,
            });
        }
        let conn = self.writer();
        conn.execute_batch(&format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS chunk_vec USING vec0(chunk_id INTEGER PRIMARY KEY, embedding float[{dim}] distance_metric=cosine);"
        ))?;
        conn.execute(
            "INSERT INTO meta(key, value) VALUES ('embed_model', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [embed_model],
        )?;
        conn.execute(
            "INSERT INTO meta(key, value) VALUES ('embed_dim', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [dim.to_string()],
        )?;
        Ok(())
    }

    /// Drop all vectors (used when the embedding model changes) and mark
    /// indexed files for re-embedding.
    pub fn reset_vectors(&self) -> Result<()> {
        let conn = self.writer();
        conn.execute_batch(
            "DROP TABLE IF EXISTS chunk_vec; DELETE FROM meta WHERE key IN ('embed_model','embed_dim');
             UPDATE files SET state = 'extracted' WHERE state = 'indexed';",
        )?;
        Ok(())
    }
}
