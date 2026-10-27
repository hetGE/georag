//! Read-only access to the v1 Python app's `metadata.db` (handoff section 10).
//! Nothing here writes to the v1 folder.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};

use crate::Result;
use crate::StoreError;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct V1File {
    pub id: i64,
    pub relative_path: String,
    pub filename: String,
    pub extension: Option<String>,
    pub size_bytes: Option<i64>,
    pub modified_time: Option<f64>,
    pub content_hash: Option<String>,
    pub scan_status: Option<String>,
    pub extracted_text_preview: Option<String>,
    pub chunk_count: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct V1Tag {
    pub id: i64,
    pub name: String,
    pub display_name: String,
    pub description: Option<String>,
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct V1FileTag {
    pub file_id: i64,
    pub tag_id: i64,
    pub source: Option<String>,
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct V1Conversation {
    pub id: i64,
    pub title: Option<String>,
    pub selected_tags: Vec<String>,
    pub created_at: Option<String>,
    pub deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct V1Message {
    pub conversation_id: i64,
    pub role: String,
    pub content: String,
    pub sources: serde_json::Value,
    pub created_at: Option<String>,
}

/// Everything the importer needs, in one shape for both sources.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct V1Dump {
    pub files: Vec<V1File>,
    pub tags: Vec<V1Tag>,
    pub file_tags: Vec<V1FileTag>,
    pub conversations: Vec<V1Conversation>,
    pub messages: Vec<V1Message>,
}

fn table_exists(conn: &Connection, name: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [name],
        |r| r.get::<_, i64>(0),
    )? > 0)
}

/// Open `metadata.db` read-only and load files, tags, file tags and chats.
pub fn read_metadata_db(path: &Path) -> Result<V1Dump> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|source| StoreError::Open {
        path: path.to_path_buf(),
        source,
    })?;
    for required in ["files", "tags", "file_tags"] {
        if !table_exists(&conn, required)? {
            return Err(StoreError::NotFound(
                "a v1 metadata.db with files, tags and file_tags tables",
            ));
        }
    }
    let mut dump = V1Dump::default();
    {
        let mut st = conn.prepare(
            "SELECT id, relative_path, filename, extension, size_bytes, modified_time, content_hash, scan_status, extracted_text_preview, chunk_count FROM files ORDER BY id",
        )?;
        let rows = st.query_map([], |r| {
            Ok(V1File {
                id: r.get(0)?,
                relative_path: r.get(1)?,
                filename: r.get(2)?,
                extension: r.get(3)?,
                size_bytes: r.get(4)?,
                modified_time: r.get(5)?,
                content_hash: r.get(6)?,
                scan_status: r.get(7)?,
                extracted_text_preview: r.get(8)?,
                chunk_count: r.get(9)?,
            })
        })?;
        dump.files = rows.collect::<std::result::Result<Vec<_>, _>>()?;
    }
    {
        let mut st = conn
            .prepare("SELECT id, name, display_name, description, color FROM tags ORDER BY id")?;
        let rows = st.query_map([], |r| {
            Ok(V1Tag {
                id: r.get(0)?,
                name: r.get(1)?,
                display_name: r.get(2)?,
                description: r.get(3)?,
                color: r.get(4)?,
            })
        })?;
        dump.tags = rows.collect::<std::result::Result<Vec<_>, _>>()?;
    }
    {
        let mut st = conn.prepare("SELECT file_id, tag_id, source, confidence FROM file_tags")?;
        let rows = st.query_map([], |r| {
            Ok(V1FileTag {
                file_id: r.get(0)?,
                tag_id: r.get(1)?,
                source: r.get(2)?,
                confidence: r.get(3)?,
            })
        })?;
        dump.file_tags = rows.collect::<std::result::Result<Vec<_>, _>>()?;
    }
    if table_exists(&conn, "conversations")? {
        let mut st = conn.prepare("SELECT id, title, selected_tags, created_at, deleted_at FROM conversations ORDER BY id")?;
        let rows = st.query_map([], |r| {
            let tags: Option<String> = r.get(2)?;
            let deleted: Option<String> = r.get(4)?;
            Ok(V1Conversation {
                id: r.get(0)?,
                title: r.get(1)?,
                selected_tags: tags
                    .and_then(|t| serde_json::from_str(&t).ok())
                    .unwrap_or_default(),
                created_at: r.get(3)?,
                deleted: deleted.is_some(),
            })
        })?;
        dump.conversations = rows.collect::<std::result::Result<Vec<_>, _>>()?;
        if table_exists(&conn, "messages")? {
            let mut st = conn.prepare("SELECT conversation_id, role, content, sources, created_at FROM messages ORDER BY id")?;
            let rows = st.query_map([], |r| {
                let sources: Option<String> = r.get(3)?;
                Ok(V1Message {
                    conversation_id: r.get(0)?,
                    role: r.get(1)?,
                    content: r.get(2)?,
                    sources: sources
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or(serde_json::Value::Array(Vec::new())),
                    created_at: r.get(4)?,
                })
            })?;
            dump.messages = rows.collect::<std::result::Result<Vec<_>, _>>()?;
        }
    }
    Ok(dump)
}

/// Build a v1-shaped database for tests and fixtures.
pub fn write_fixture_db(path: &Path, dump: &V1Dump) -> Result<()> {
    let conn = Connection::open(path).map_err(|source| StoreError::Open {
        path: path.to_path_buf(),
        source,
    })?;
    conn.execute_batch(
        "CREATE TABLE files(id INTEGER PRIMARY KEY, relative_path TEXT UNIQUE NOT NULL, filename TEXT NOT NULL, extension TEXT, size_bytes INTEGER, modified_time REAL, parent_directory TEXT, content_hash TEXT, scan_status TEXT, extracted_text_preview TEXT, chunk_count INTEGER, processed_at TEXT, auto_tagged INTEGER, auto_tag_confidence REAL);
         CREATE TABLE tags(id INTEGER PRIMARY KEY, name TEXT UNIQUE NOT NULL, display_name TEXT NOT NULL, description TEXT, color TEXT, file_count INTEGER);
         CREATE TABLE file_tags(id INTEGER PRIMARY KEY, file_id INTEGER NOT NULL, tag_id INTEGER NOT NULL, source TEXT, confidence REAL, created_at TEXT);
         CREATE TABLE conversations(id INTEGER PRIMARY KEY, title TEXT, created_at TEXT, updated_at TEXT, deleted_at TEXT, selected_tags TEXT);
         CREATE TABLE messages(id INTEGER PRIMARY KEY, conversation_id INTEGER NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL, sources TEXT, created_at TEXT);",
    )?;
    for f in &dump.files {
        conn.execute(
            "INSERT INTO files(id, relative_path, filename, extension, size_bytes, modified_time, content_hash, scan_status, extracted_text_preview, chunk_count) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            rusqlite::params![f.id, f.relative_path, f.filename, f.extension, f.size_bytes, f.modified_time, f.content_hash, f.scan_status, f.extracted_text_preview, f.chunk_count],
        )?;
    }
    for t in &dump.tags {
        conn.execute(
            "INSERT INTO tags(id, name, display_name, description, color) VALUES (?1,?2,?3,?4,?5)",
            rusqlite::params![t.id, t.name, t.display_name, t.description, t.color],
        )?;
    }
    for ft in &dump.file_tags {
        conn.execute(
            "INSERT INTO file_tags(file_id, tag_id, source, confidence) VALUES (?1,?2,?3,?4)",
            rusqlite::params![ft.file_id, ft.tag_id, ft.source, ft.confidence],
        )?;
    }
    for c in &dump.conversations {
        conn.execute(
            "INSERT INTO conversations(id, title, created_at, deleted_at, selected_tags) VALUES (?1,?2,?3,?4,?5)",
            rusqlite::params![c.id, c.title, c.created_at, if c.deleted { Some("2025-01-01T00:00:00") } else { None }, serde_json::to_string(&c.selected_tags)?],
        )?;
    }
    for m in &dump.messages {
        conn.execute(
            "INSERT INTO messages(conversation_id, role, content, sources, created_at) VALUES (?1,?2,?3,?4,?5)",
            rusqlite::params![m.conversation_id, m.role, m.content, serde_json::to_string(&m.sources)?, m.created_at],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_round_trips() {
        let dir = std::env::temp_dir().join(format!("georag-v1-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("metadata.db");
        let dump = V1Dump {
            files: vec![V1File {
                id: 1,
                relative_path: "Piles/a.pdf".into(),
                filename: "a.pdf".into(),
                extension: Some("pdf".into()),
                size_bytes: Some(10),
                modified_time: Some(1.0),
                content_hash: None,
                scan_status: Some("processed".into()),
                extracted_text_preview: Some("p".into()),
                chunk_count: Some(3),
            }],
            tags: vec![V1Tag {
                id: 1,
                name: "piling".into(),
                display_name: "Piling".into(),
                description: None,
                color: Some("#fff".into()),
            }],
            file_tags: vec![V1FileTag {
                file_id: 1,
                tag_id: 1,
                source: Some("folder_hint".into()),
                confidence: Some(0.7),
            }],
            conversations: vec![V1Conversation {
                id: 1,
                title: Some("t".into()),
                selected_tags: vec!["piling".into()],
                created_at: None,
                deleted: false,
            }],
            messages: vec![V1Message {
                conversation_id: 1,
                role: "user".into(),
                content: "hi".into(),
                sources: serde_json::json!([]),
                created_at: None,
            }],
        };
        write_fixture_db(&db, &dump).unwrap();
        let back = read_metadata_db(&db).unwrap();
        assert_eq!(back, dump);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
