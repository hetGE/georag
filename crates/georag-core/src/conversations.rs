//! Conversation commands for the CLI and the Chat screen, without the
//! engine: list, show, delete, restore, export.

use std::path::PathBuf;

use georag_store::{Conversation, Message, Pass, Store};
use serde::Serialize;

use crate::error::{CoreError, Result};
use crate::home::Home;

fn open(home: &Home) -> Result<Store> {
    Ok(Store::open(&home.db_path())?)
}

pub fn list(home: &Home, include_deleted: bool) -> Result<Vec<Conversation>> {
    open(home)?
        .conversations(include_deleted)
        .map_err(Into::into)
}

#[derive(Debug, Clone, Serialize)]
pub struct MessageWithPasses {
    pub message: Message,
    pub passes: Vec<Pass>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConversationView {
    pub conversation: Conversation,
    pub messages: Vec<MessageWithPasses>,
}

pub fn show(home: &Home, id: i64) -> Result<ConversationView> {
    let store = open(home)?;
    show_in(&store, id)
}

pub fn show_in(store: &Store, id: i64) -> Result<ConversationView> {
    let conversation = store
        .conversation_get(id)?
        .ok_or_else(|| CoreError::Invalid(format!("no conversation {id}")))?;
    let mut messages = Vec::new();
    for m in store.messages(id)? {
        let passes = store.passes(m.id)?;
        messages.push(MessageWithPasses { message: m, passes });
    }
    Ok(ConversationView {
        conversation,
        messages,
    })
}

pub fn delete(home: &Home, id: i64) -> Result<()> {
    open(home)?.conversation_delete(id).map_err(Into::into)
}

pub fn restore(home: &Home, id: i64) -> Result<()> {
    open(home)?.conversation_restore(id).map_err(Into::into)
}

/// Render as markdown.
pub fn to_markdown(view: &ConversationView) -> String {
    let mut md = format!(
        "# {}\n\n",
        view.conversation.title.as_deref().unwrap_or("Conversation")
    );
    for m in &view.messages {
        let heading = if m.message.role == "user" {
            "Question"
        } else {
            "Answer"
        };
        md.push_str(&format!("## {heading}\n\n{}\n\n", m.message.content));
        if !m.message.sources.is_empty() {
            md.push_str("Sources:\n\n");
            for s in &m.message.sources {
                md.push_str(&format!(
                    "- [S{}] {} {}\n",
                    s.tag,
                    s.rel_path,
                    s.locator.as_deref().unwrap_or("")
                ));
            }
            md.push('\n');
        }
    }
    md
}

/// Write `exports/conversation-<id>.md` and return its path.
pub fn export(home: &Home, id: i64) -> Result<PathBuf> {
    let view = show(home, id)?;
    export_view(home, &view)
}

pub fn export_view(home: &Home, view: &ConversationView) -> Result<PathBuf> {
    let dir = home.exports_dir();
    std::fs::create_dir_all(&dir).map_err(|source| CoreError::Io {
        path: dir.clone(),
        source,
    })?;
    let path = dir.join(format!("conversation-{}.md", view.conversation.id));
    std::fs::write(&path, to_markdown(view)).map_err(|source| CoreError::Io {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}
