//! Conversations, messages and passes.

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::Result;
use crate::store::Store;
use crate::time::now_rfc3339;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Conversation {
    pub id: i64,
    pub title: Option<String>,
    pub tags: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub file_id: i64,
    pub rel_path: String,
    pub locator: Option<String>,
    pub chunk_id: i64,
    pub score: f32,
    /// Citation tag number as shown in the answer, `[S<n>]`.
    pub tag: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Message {
    pub id: i64,
    pub conversation_id: i64,
    pub role: String,
    pub content: String,
    pub sources: Vec<Source>,
    pub mode: Option<String>,
    pub cut_off: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewMessage<'a> {
    pub conversation_id: i64,
    pub role: &'a str,
    pub content: &'a str,
    pub sources: &'a [Source],
    pub mode: Option<&'a str>,
    pub cut_off: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Pass {
    pub id: i64,
    pub message_id: i64,
    pub idx: i64,
    pub status: String,
    pub chunk_ids: Vec<i64>,
    pub answer: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewPass<'a> {
    pub idx: i64,
    pub status: &'a str,
    pub chunk_ids: &'a [i64],
    pub answer: Option<&'a str>,
    pub started_at: Option<&'a str>,
    pub finished_at: Option<&'a str>,
}

impl Store {
    pub fn conversation_create(
        &self,
        title: Option<&str>,
        tags: &[String],
    ) -> Result<Conversation> {
        let now = now_rfc3339();
        let tags_json = serde_json::to_string(tags)?;
        let conn = self.writer();
        conn.execute(
            "INSERT INTO conversations(title, tags, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
            params![title, tags_json, now],
        )?;
        Ok(Conversation {
            id: conn.last_insert_rowid(),
            title: title.map(str::to_string),
            tags: tags.to_vec(),
            created_at: now.clone(),
            updated_at: now,
            deleted_at: None,
        })
    }

    pub fn conversation_get(&self, id: i64) -> Result<Option<Conversation>> {
        self.read(|c| {
            Ok(c.query_row(
                "SELECT id, title, tags, created_at, updated_at, deleted_at FROM conversations WHERE id = ?1",
                [id],
                row_to_conversation,
            )
            .optional()?)
        })
    }

    /// Live conversations, most recently updated first.
    pub fn conversations(&self, include_deleted: bool) -> Result<Vec<Conversation>> {
        self.read(|c| {
            let sql = if include_deleted {
                "SELECT id, title, tags, created_at, updated_at, deleted_at FROM conversations ORDER BY updated_at DESC"
            } else {
                "SELECT id, title, tags, created_at, updated_at, deleted_at FROM conversations WHERE deleted_at IS NULL ORDER BY updated_at DESC"
            };
            let mut st = c.prepare(sql)?;
            let rows = st.query_map([], row_to_conversation)?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    pub fn conversation_set_title(&self, id: i64, title: &str) -> Result<()> {
        self.writer().execute(
            "UPDATE conversations SET title = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, title, now_rfc3339()],
        )?;
        Ok(())
    }

    pub fn conversation_set_tags(&self, id: i64, tags: &[String]) -> Result<()> {
        self.writer().execute(
            "UPDATE conversations SET tags = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, serde_json::to_string(tags)?, now_rfc3339()],
        )?;
        Ok(())
    }

    /// Soft delete (`deleted_at` set); `conversation_restore` undoes it.
    pub fn conversation_delete(&self, id: i64) -> Result<()> {
        self.writer().execute(
            "UPDATE conversations SET deleted_at = ?2 WHERE id = ?1",
            params![id, now_rfc3339()],
        )?;
        Ok(())
    }

    pub fn conversation_restore(&self, id: i64) -> Result<()> {
        self.writer().execute(
            "UPDATE conversations SET deleted_at = NULL WHERE id = ?1",
            [id],
        )?;
        Ok(())
    }

    pub fn message_add(&self, m: &NewMessage<'_>) -> Result<Message> {
        let now = now_rfc3339();
        let sources_json = serde_json::to_string(m.sources)?;
        let conn = self.writer();
        conn.execute(
            "INSERT INTO messages(conversation_id, role, content, sources, mode, cut_off, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![m.conversation_id, m.role, m.content, sources_json, m.mode, m.cut_off as i64, now],
        )?;
        let id = conn.last_insert_rowid();
        conn.execute(
            "UPDATE conversations SET updated_at = ?2 WHERE id = ?1",
            params![m.conversation_id, now],
        )?;
        Ok(Message {
            id,
            conversation_id: m.conversation_id,
            role: m.role.to_string(),
            content: m.content.to_string(),
            sources: m.sources.to_vec(),
            mode: m.mode.map(str::to_string),
            cut_off: m.cut_off,
            created_at: now,
        })
    }

    pub fn messages(&self, conversation_id: i64) -> Result<Vec<Message>> {
        self.read(|c| {
            let mut st = c.prepare(
                "SELECT id, conversation_id, role, content, sources, mode, cut_off, created_at FROM messages WHERE conversation_id = ?1 ORDER BY id",
            )?;
            let rows = st.query_map([conversation_id], |r| {
                let sources: String = r.get(4)?;
                Ok((
                    Message {
                        id: r.get(0)?,
                        conversation_id: r.get(1)?,
                        role: r.get(2)?,
                        content: r.get(3)?,
                        sources: Vec::new(),
                        mode: r.get(5)?,
                        cut_off: r.get::<_, i64>(6)? != 0,
                        created_at: r.get(7)?,
                    },
                    sources,
                ))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (mut m, sources) = row?;
                m.sources = serde_json::from_str(&sources).unwrap_or_default();
                out.push(m);
            }
            Ok(out)
        })
    }

    pub fn messages_conversation(&self, message_id: i64) -> Result<i64> {
        self.read(|c| {
            Ok(c.query_row(
                "SELECT conversation_id FROM messages WHERE id = ?1",
                [message_id],
                |r| r.get(0),
            )?)
        })
    }

    pub fn message_set_content(&self, message_id: i64, content: &str, cut_off: bool) -> Result<()> {
        self.writer().execute(
            "UPDATE messages SET content = ?2, cut_off = ?3 WHERE id = ?1",
            params![message_id, content, cut_off as i64],
        )?;
        Ok(())
    }

    pub fn passes_add(&self, message_id: i64, passes: &[NewPass<'_>]) -> Result<()> {
        let mut conn = self.writer();
        let tx = conn.transaction()?;
        for p in passes {
            tx.execute(
                "INSERT INTO passes(message_id, idx, status, chunk_ids, answer, started_at, finished_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![message_id, p.idx, p.status, serde_json::to_string(p.chunk_ids)?, p.answer, p.started_at, p.finished_at],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn passes(&self, message_id: i64) -> Result<Vec<Pass>> {
        self.read(|c| {
            let mut st = c.prepare(
                "SELECT id, message_id, idx, status, chunk_ids, answer, started_at, finished_at FROM passes WHERE message_id = ?1 ORDER BY idx",
            )?;
            let rows = st.query_map([message_id], |r| {
                let ids: String = r.get(4)?;
                Ok((
                    Pass {
                        id: r.get(0)?,
                        message_id: r.get(1)?,
                        idx: r.get(2)?,
                        status: r.get(3)?,
                        chunk_ids: Vec::new(),
                        answer: r.get(5)?,
                        started_at: r.get(6)?,
                        finished_at: r.get(7)?,
                    },
                    ids,
                ))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (mut p, ids) = row?;
                p.chunk_ids = serde_json::from_str(&ids).unwrap_or_default();
                out.push(p);
            }
            Ok(out)
        })
    }
}

fn row_to_conversation(r: &rusqlite::Row<'_>) -> rusqlite::Result<Conversation> {
    let tags: String = r.get(2)?;
    Ok(Conversation {
        id: r.get(0)?,
        title: r.get(1)?,
        tags: serde_json::from_str(&tags).unwrap_or_default(),
        created_at: r.get(3)?,
        updated_at: r.get(4)?,
        deleted_at: r.get(5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_round_trip_with_passes() {
        let store = Store::open_in_memory().unwrap();
        let conv = store
            .conversation_create(Some("Piles"), &["piling".into()])
            .unwrap();
        let user = store
            .message_add(&NewMessage {
                conversation_id: conv.id,
                role: "user",
                content: "How deep?",
                sources: &[],
                mode: None,
                cut_off: false,
            })
            .unwrap();
        let src = Source {
            file_id: 1,
            rel_path: "a.pdf".into(),
            locator: Some("p.3".into()),
            chunk_id: 7,
            score: 0.8,
            tag: 1,
        };
        let reply = store
            .message_add(&NewMessage {
                conversation_id: conv.id,
                role: "assistant",
                content: "Twelve metres [S1]",
                sources: std::slice::from_ref(&src),
                mode: Some("auto"),
                cut_off: false,
            })
            .unwrap();
        store
            .passes_add(
                reply.id,
                &[
                    NewPass {
                        idx: 1,
                        status: "done",
                        chunk_ids: &[7, 8],
                        answer: Some("partial"),
                        started_at: None,
                        finished_at: None,
                    },
                    NewPass {
                        idx: 0,
                        status: "done",
                        chunk_ids: &[],
                        answer: Some("Twelve metres [S1]"),
                        started_at: None,
                        finished_at: None,
                    },
                ],
            )
            .unwrap();
        let msgs = store.messages(conv.id).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].id, user.id);
        assert_eq!(msgs[1].sources, vec![src]);
        let passes = store.passes(reply.id).unwrap();
        assert_eq!(passes.len(), 2);
        assert_eq!(passes[0].idx, 0);
        assert_eq!(passes[1].chunk_ids, vec![7, 8]);

        store.conversation_delete(conv.id).unwrap();
        assert!(store.conversations(false).unwrap().is_empty());
        assert_eq!(store.conversations(true).unwrap().len(), 1);
        store.conversation_restore(conv.id).unwrap();
        assert_eq!(
            store.conversations(false).unwrap()[0].tags,
            vec!["piling".to_string()]
        );
    }
}
