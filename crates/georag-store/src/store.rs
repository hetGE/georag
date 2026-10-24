use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, Once};

use rusqlite::{Connection, OpenFlags};

use crate::Result;
use crate::error::StoreError;

const MIGRATIONS: &[(&str, &str)] = &[(
    "0001_initial",
    include_str!("../migrations/0001_initial.sql"),
)];

/// The SQLite store. Cheap to share behind an `Arc`.
pub struct Store {
    path: Option<PathBuf>,
    writer: Mutex<Connection>,
    readers: Mutex<Vec<Connection>>,
}

static REGISTER_VEC: Once = Once::new();

fn register_sqlite_vec() {
    REGISTER_VEC.call_once(|| {
        // SAFETY: sqlite3_vec_init has the sqlite3_loadext_entry signature the
        // auto-extension API expects, and is registered once per process
        // before any connection that needs it is opened.
        #[allow(unsafe_code)]
        unsafe {
            type Entry = unsafe extern "C" fn(
                *mut rusqlite::ffi::sqlite3,
                *mut *mut std::os::raw::c_char,
                *const rusqlite::ffi::sqlite3_api_routines,
            ) -> std::os::raw::c_int;
            let f: Entry = std::mem::transmute(sqlite_vec::sqlite3_vec_init as *const ());
            rusqlite::ffi::sqlite3_auto_extension(Some(f));
        }
    });
}

fn configure(conn: &Connection) -> rusqlite::Result<()> {
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(())
}

impl Store {
    /// Open or create the database at `path`, applying migrations.
    pub fn open(path: &Path) -> Result<Store> {
        register_sqlite_vec();
        let writer = Connection::open(path).map_err(|source| StoreError::Open {
            path: path.to_path_buf(),
            source,
        })?;
        configure(&writer).map_err(|source| StoreError::Open {
            path: path.to_path_buf(),
            source,
        })?;
        let store = Store {
            path: Some(path.to_path_buf()),
            writer: Mutex::new(writer),
            readers: Mutex::new(Vec::new()),
        };
        store.migrate()?;
        Ok(store)
    }

    /// An in-memory store for tests. Reads share the writer connection.
    pub fn open_in_memory() -> Result<Store> {
        register_sqlite_vec();
        let writer = Connection::open_in_memory()?;
        writer.pragma_update(None, "foreign_keys", "ON")?;
        let store = Store {
            path: None,
            writer: Mutex::new(writer),
            readers: Mutex::new(Vec::new()),
        };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
        let mut conn = self.writer();
        // Skip the write transaction when every migration is already applied,
        // so a second open (another command, the TUI's list of conversations)
        // never contends with a writer.
        let applied: i64 = conn
            .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'", [], |r| r.get(0))
            .unwrap_or(0);
        if applied == 1 {
            let done: i64 =
                conn.query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r.get(0))?;
            if done as usize >= MIGRATIONS.len() {
                return Ok(());
            }
        }
        // Immediate: take the write lock up front so busy_timeout applies
        // instead of an instant BUSY on a read-to-write upgrade.
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS schema_migrations(name TEXT PRIMARY KEY, applied_at TEXT NOT NULL);")?;
        for (name, sql) in MIGRATIONS {
            let applied: bool = tx.query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE name = ?1",
                [name],
                |r| r.get::<_, i64>(0),
            )? > 0;
            if !applied {
                tx.execute_batch(sql)?;
                tx.execute(
                    "INSERT INTO schema_migrations(name, applied_at) VALUES (?1, ?2)",
                    rusqlite::params![name, crate::time::now_rfc3339()],
                )?;
                tracing::info!(migration = name, "applied");
            }
        }
        tx.execute(
            "INSERT OR IGNORE INTO meta(key, value) VALUES ('schema_version', ?1)",
            [MIGRATIONS.len().to_string()],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO meta(key, value) VALUES ('created_at', ?1)",
            [crate::time::now_rfc3339()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The single write connection. Hold the guard for one transaction only.
    pub(crate) fn writer(&self) -> MutexGuard<'_, Connection> {
        self.writer.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Run a read-only closure on a pooled connection (or the writer for
    /// in-memory stores).
    pub(crate) fn read<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let Some(path) = &self.path else {
            let conn = self.writer();
            return f(&conn);
        };
        let conn = {
            let mut pool = self.readers.lock().unwrap_or_else(|e| e.into_inner());
            pool.pop()
        };
        let conn = match conn {
            Some(c) => c,
            None => {
                let c = Connection::open_with_flags(
                    path,
                    OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
                )?;
                c.busy_timeout(std::time::Duration::from_secs(5))?;
                c
            }
        };
        let out = f(&conn);
        let mut pool = self.readers.lock().unwrap_or_else(|e| e.into_inner());
        if pool.len() < 4 {
            pool.push(conn);
        }
        out
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}
