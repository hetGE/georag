-- Schema from handoff section 4.1. chunk_vec is created separately once the
-- embedding dimension is known (meta.embed_dim).

CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);

CREATE TABLE library_roots(
  id INTEGER PRIMARY KEY, name TEXT UNIQUE NOT NULL, path TEXT NOT NULL, added_at TEXT NOT NULL);

CREATE TABLE files(
  id INTEGER PRIMARY KEY,
  root_id INTEGER NOT NULL REFERENCES library_roots(id) ON DELETE CASCADE,
  rel_path TEXT NOT NULL, filename TEXT NOT NULL, ext TEXT,
  size_bytes INTEGER, mtime REAL, hash TEXT,
  state TEXT NOT NULL CHECK(state IN ('new','imported','extracted','tagged','indexed','failed','stale','skipped')),
  error TEXT, preview TEXT, chunk_count INTEGER DEFAULT 0, token_count INTEGER DEFAULT 0,
  indexed_at TEXT, v1_note TEXT,
  UNIQUE(root_id, rel_path));
CREATE INDEX files_state ON files(state);
CREATE INDEX files_ext ON files(ext);

CREATE TABLE tags(
  id INTEGER PRIMARY KEY, name TEXT UNIQUE NOT NULL, display_name TEXT NOT NULL,
  description TEXT, color TEXT);

CREATE TABLE file_tags(
  file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
  source TEXT NOT NULL CHECK(source IN ('manual','rule','llm')),
  confidence REAL, imported INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL,
  PRIMARY KEY(file_id, tag_id));

CREATE TABLE chunks(
  id INTEGER PRIMARY KEY,
  file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
  idx INTEGER NOT NULL, text TEXT NOT NULL, tokens INTEGER NOT NULL,
  page INTEGER, locator TEXT,
  UNIQUE(file_id, idx));

CREATE VIRTUAL TABLE chunk_fts USING fts5(text, content='chunks', content_rowid='id', tokenize='unicode61');

CREATE TRIGGER chunks_ai AFTER INSERT ON chunks BEGIN
  INSERT INTO chunk_fts(rowid, text) VALUES (new.id, new.text);
END;
CREATE TRIGGER chunks_ad AFTER DELETE ON chunks BEGIN
  INSERT INTO chunk_fts(chunk_fts, rowid, text) VALUES ('delete', old.id, old.text);
END;
CREATE TRIGGER chunks_au AFTER UPDATE ON chunks BEGIN
  INSERT INTO chunk_fts(chunk_fts, rowid, text) VALUES ('delete', old.id, old.text);
  INSERT INTO chunk_fts(rowid, text) VALUES (new.id, new.text);
END;

CREATE TABLE conversations(
  id INTEGER PRIMARY KEY, title TEXT, tags TEXT NOT NULL DEFAULT '[]',
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL, deleted_at TEXT);

CREATE TABLE messages(
  id INTEGER PRIMARY KEY, conversation_id INTEGER NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
  role TEXT NOT NULL CHECK(role IN ('user','assistant')), content TEXT NOT NULL,
  sources TEXT NOT NULL DEFAULT '[]',
  mode TEXT, cut_off INTEGER DEFAULT 0, created_at TEXT NOT NULL);
CREATE INDEX messages_conversation ON messages(conversation_id);

CREATE TABLE passes(
  id INTEGER PRIMARY KEY, message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
  idx INTEGER NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('done','skipped','stopped','accepted')),
  chunk_ids TEXT NOT NULL, answer TEXT, started_at TEXT, finished_at TEXT);

CREATE TABLE jobs(
  id INTEGER PRIMARY KEY, kind TEXT NOT NULL,
  file_id INTEGER REFERENCES files(id) ON DELETE CASCADE, payload TEXT,
  state TEXT NOT NULL CHECK(state IN ('queued','running','done','failed','cancelled')),
  attempts INTEGER DEFAULT 0, error TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
CREATE INDEX jobs_state ON jobs(state);

CREATE TABLE activity(id INTEGER PRIMARY KEY, at TEXT NOT NULL, level TEXT NOT NULL, text TEXT NOT NULL);
