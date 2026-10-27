//! Tags and file tags.

use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use crate::Result;
use crate::store::Store;
use crate::time::now_rfc3339;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Tag {
    pub id: i64,
    pub name: String,
    pub display_name: String,
    pub description: Option<String>,
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileTag {
    pub tag: String,
    pub display_name: String,
    pub source: String,
    pub confidence: Option<f64>,
    pub imported: bool,
}

impl Store {
    /// Insert or update a tag by name; returns its id.
    pub fn tag_upsert(
        &self,
        name: &str,
        display_name: &str,
        description: Option<&str>,
        color: Option<&str>,
    ) -> Result<i64> {
        let conn = self.writer();
        conn.execute(
            "INSERT INTO tags(name, display_name, description, color) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(name) DO UPDATE SET display_name = excluded.display_name,
               description = COALESCE(excluded.description, tags.description), color = COALESCE(excluded.color, tags.color)",
            params![name, display_name, description, color],
        )?;
        Ok(conn.query_row("SELECT id FROM tags WHERE name = ?1", [name], |r| r.get(0))?)
    }

    pub fn tags(&self) -> Result<Vec<Tag>> {
        self.read(|c| {
            let mut st = c.prepare(
                "SELECT id, name, display_name, description, color FROM tags ORDER BY name",
            )?;
            let rows = st.query_map([], |r| {
                Ok(Tag {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    display_name: r.get(2)?,
                    description: r.get(3)?,
                    color: r.get(4)?,
                })
            })?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    pub fn tag_by_name(&self, name: &str) -> Result<Option<Tag>> {
        self.read(|c| {
            Ok(c.query_row(
                "SELECT id, name, display_name, description, color FROM tags WHERE name = ?1",
                [name],
                |r| {
                    Ok(Tag {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        display_name: r.get(2)?,
                        description: r.get(3)?,
                        color: r.get(4)?,
                    })
                },
            )
            .optional()?)
        })
    }

    pub fn tag_remove(&self, name: &str) -> Result<bool> {
        Ok(self
            .writer()
            .execute("DELETE FROM tags WHERE name = ?1", [name])?
            > 0)
    }

    /// Tag counts over files, for facets.
    pub fn tag_counts(&self) -> Result<Vec<(String, i64)>> {
        self.read(|c| {
            let mut st = c.prepare("SELECT t.name, COUNT(ft.file_id) FROM tags t LEFT JOIN file_tags ft ON ft.tag_id = t.id GROUP BY t.id ORDER BY t.name")?;
            let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// Attach a tag. Manual tags are never overwritten by rule or llm tags;
    /// a manual call replaces any other source.
    pub fn file_tag_add(
        &self,
        file_id: i64,
        tag: &str,
        source: &str,
        confidence: Option<f64>,
        imported: bool,
    ) -> Result<()> {
        let conn = self.writer();
        let tag_id: Option<i64> = conn
            .query_row("SELECT id FROM tags WHERE name = ?1", [tag], |r| r.get(0))
            .optional()?;
        let Some(tag_id) = tag_id else {
            return Err(crate::StoreError::NotFound("tag"));
        };
        if source == "manual" {
            conn.execute(
                "INSERT INTO file_tags(file_id, tag_id, source, confidence, imported, created_at) VALUES (?1, ?2, 'manual', 1.0, ?3, ?4)
                 ON CONFLICT(file_id, tag_id) DO UPDATE SET source = 'manual', confidence = 1.0",
                params![file_id, tag_id, imported as i64, now_rfc3339()],
            )?;
        } else {
            conn.execute(
                "INSERT INTO file_tags(file_id, tag_id, source, confidence, imported, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(file_id, tag_id) DO UPDATE SET source = CASE WHEN file_tags.source = 'manual' THEN 'manual' ELSE excluded.source END,
                   confidence = CASE WHEN file_tags.source = 'manual' THEN file_tags.confidence ELSE excluded.confidence END",
                params![file_id, tag_id, source, confidence, imported as i64, now_rfc3339()],
            )?;
        }
        Ok(())
    }

    pub fn file_tag_remove(&self, file_id: i64, tag: &str) -> Result<bool> {
        Ok(self.writer().execute(
            "DELETE FROM file_tags WHERE file_id = ?1 AND tag_id = (SELECT id FROM tags WHERE name = ?2)",
            params![file_id, tag],
        )? > 0)
    }

    pub fn file_tags(&self, file_id: i64) -> Result<Vec<FileTag>> {
        self.read(|c| {
            let mut st = c.prepare(
                "SELECT t.name, t.display_name, ft.source, ft.confidence, ft.imported FROM file_tags ft JOIN tags t ON t.id = ft.tag_id WHERE ft.file_id = ?1 ORDER BY t.name",
            )?;
            let rows = st.query_map([file_id], |r| {
                Ok(FileTag { tag: r.get(0)?, display_name: r.get(1)?, source: r.get(2)?, confidence: r.get(3)?, imported: r.get::<_, i64>(4)? != 0 })
            })?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    pub fn file_has_tags(&self, file_id: i64) -> Result<bool> {
        self.read(|c| {
            Ok(c.query_row(
                "SELECT COUNT(*) FROM file_tags WHERE file_id = ?1",
                [file_id],
                |r| r.get::<_, i64>(0),
            )? > 0)
        })
    }
}
