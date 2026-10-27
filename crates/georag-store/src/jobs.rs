//! Jobs: one row per stage per file; idempotent and resumable.

use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use crate::Result;
use crate::store::Store;
use crate::time::now_rfc3339;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Job {
    pub id: i64,
    pub kind: String,
    pub file_id: Option<i64>,
    pub payload: Option<String>,
    pub state: String,
    pub attempts: i64,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl Store {
    /// Queue a job unless an identical queued or running one exists.
    pub fn job_enqueue(
        &self,
        kind: &str,
        file_id: Option<i64>,
        payload: Option<&str>,
    ) -> Result<i64> {
        let conn = self.writer();
        let existing: Option<i64> = conn
            .query_row(
                "SELECT id FROM jobs WHERE kind = ?1 AND file_id IS ?2 AND state IN ('queued','running') LIMIT 1",
                params![kind, file_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            return Ok(id);
        }
        let now = now_rfc3339();
        conn.execute(
            "INSERT INTO jobs(kind, file_id, payload, state, attempts, created_at, updated_at) VALUES (?1, ?2, ?3, 'queued', 0, ?4, ?4)",
            params![kind, file_id, payload, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Take the oldest queued job of `kind`, marking it running.
    pub fn job_claim(&self, kind: &str) -> Result<Option<Job>> {
        let mut conn = self.writer();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let job = tx
            .query_row(
                "SELECT id, kind, file_id, payload, state, attempts, error, created_at, updated_at FROM jobs WHERE kind = ?1 AND state = 'queued' ORDER BY id LIMIT 1",
                [kind],
                row_to_job,
            )
            .optional()?;
        if let Some(j) = &job {
            tx.execute(
                "UPDATE jobs SET state = 'running', attempts = attempts + 1, updated_at = ?2 WHERE id = ?1",
                params![j.id, now_rfc3339()],
            )?;
        }
        tx.commit()?;
        Ok(job.map(|mut j| {
            j.state = "running".into();
            j.attempts += 1;
            j
        }))
    }

    pub fn job_finish(&self, id: i64, state: &str, error: Option<&str>) -> Result<()> {
        self.writer().execute(
            "UPDATE jobs SET state = ?2, error = ?3, updated_at = ?4 WHERE id = ?1",
            params![id, state, error, now_rfc3339()],
        )?;
        Ok(())
    }

    /// Running jobs left by a crash go back to queued.
    pub fn jobs_requeue_running(&self) -> Result<usize> {
        Ok(self.writer().execute(
            "UPDATE jobs SET state = 'queued', updated_at = ?1 WHERE state = 'running'",
            [now_rfc3339()],
        )?)
    }

    pub fn jobs(&self, states: &[&str], limit: usize) -> Result<Vec<Job>> {
        self.read(|c| {
            let placeholders = vec!["?"; states.len()].join(",");
            let sql = if states.is_empty() {
                "SELECT id, kind, file_id, payload, state, attempts, error, created_at, updated_at FROM jobs ORDER BY id DESC LIMIT ?".to_string()
            } else {
                format!("SELECT id, kind, file_id, payload, state, attempts, error, created_at, updated_at FROM jobs WHERE state IN ({placeholders}) ORDER BY id DESC LIMIT ?")
            };
            let mut st = c.prepare(&sql)?;
            let mut values: Vec<rusqlite::types::Value> = states.iter().map(|s| (*s).to_string().into()).collect();
            values.push((limit as i64).into());
            let rows = st.query_map(rusqlite::params_from_iter(values.iter()), row_to_job)?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    pub fn job_counts(&self) -> Result<Vec<(String, String, i64)>> {
        self.read(|c| {
            let mut st =
                c.prepare("SELECT kind, state, COUNT(*) FROM jobs GROUP BY kind, state")?;
            let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    pub fn job_cancel(&self, id: i64) -> Result<bool> {
        Ok(self.writer().execute(
            "UPDATE jobs SET state = 'cancelled', updated_at = ?2 WHERE id = ?1 AND state IN ('queued','running')",
            params![id, now_rfc3339()],
        )? > 0)
    }

    pub fn jobs_clear_finished(&self) -> Result<usize> {
        Ok(self.writer().execute(
            "DELETE FROM jobs WHERE state IN ('done','failed','cancelled')",
            [],
        )?)
    }

    pub fn activity_add(&self, level: &str, text: &str) -> Result<()> {
        self.writer().execute(
            "INSERT INTO activity(at, level, text) VALUES (?1, ?2, ?3)",
            params![now_rfc3339(), level, text],
        )?;
        Ok(())
    }

    pub fn activity(&self, limit: usize) -> Result<Vec<(String, String, String)>> {
        self.read(|c| {
            let mut st =
                c.prepare("SELECT at, level, text FROM activity ORDER BY id DESC LIMIT ?1")?;
            let rows = st.query_map([limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }
}

fn row_to_job(r: &rusqlite::Row<'_>) -> rusqlite::Result<Job> {
    Ok(Job {
        id: r.get(0)?,
        kind: r.get(1)?,
        file_id: r.get(2)?,
        payload: r.get(3)?,
        state: r.get(4)?,
        attempts: r.get(5)?,
        error: r.get(6)?,
        created_at: r.get(7)?,
        updated_at: r.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enqueue_is_idempotent_and_claim_marks_running() {
        let store = Store::open_in_memory().unwrap();
        let root = store.root_add("lib", std::path::Path::new("/lib")).unwrap();
        let f1 = store
            .file_upsert(root.id, "a.pdf", None, crate::FileState::New)
            .unwrap();
        let f2 = store
            .file_upsert(root.id, "b.pdf", None, crate::FileState::New)
            .unwrap();
        let a = store.job_enqueue("extract", Some(f1), None).unwrap();
        let b = store.job_enqueue("extract", Some(f1), None).unwrap();
        assert_eq!(a, b);
        let c = store.job_enqueue("extract", Some(f2), None).unwrap();
        assert_ne!(a, c);
        let j = store.job_claim("extract").unwrap().unwrap();
        assert_eq!(j.id, a);
        assert_eq!(j.state, "running");
        assert_eq!(j.attempts, 1);
        store.job_finish(j.id, "done", None).unwrap();
        let j2 = store.job_claim("extract").unwrap().unwrap();
        assert_eq!(j2.file_id, Some(f2));
        assert!(store.job_claim("extract").unwrap().is_none());
        assert_eq!(store.jobs_requeue_running().unwrap(), 1);
        assert!(store.job_cancel(j2.id).unwrap());
        assert_eq!(store.jobs(&["done", "cancelled"], 10).unwrap().len(), 2);
    }
}
