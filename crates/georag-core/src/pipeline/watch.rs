//! Folder watching (handoff 6.2): notify with a 2 s debounce; changed roots
//! are rescanned and the pipeline runs.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_full::{DebounceEventResult, new_debouncer};

use crate::error::{CoreError, Result};

/// A watcher over every root; `recv` yields the affected root paths.
pub struct Watcher {
    _debouncer: notify_debouncer_full::Debouncer<
        notify::RecommendedWatcher,
        notify_debouncer_full::RecommendedCache,
    >,
    rx: mpsc::Receiver<Vec<PathBuf>>,
    roots: Vec<PathBuf>,
}

impl Watcher {
    pub fn new(roots: &[PathBuf], debounce: Duration) -> Result<Watcher> {
        let (tx, rx) = mpsc::channel::<Vec<PathBuf>>();
        let mut debouncer =
            new_debouncer(
                debounce,
                None,
                move |result: DebounceEventResult| match result {
                    Ok(events) => {
                        let paths: Vec<PathBuf> = events
                            .into_iter()
                            .flat_map(|e| e.event.paths.clone())
                            .collect();
                        if !paths.is_empty() {
                            let _ = tx.send(paths);
                        }
                    }
                    Err(errors) => {
                        for e in errors {
                            tracing::warn!("watch: {e}");
                        }
                    }
                },
            )
            .map_err(|e| CoreError::Invalid(format!("could not start the folder watcher: {e}")))?;
        let mut watched = Vec::new();
        for root in roots {
            if !root.is_dir() {
                tracing::warn!("root {} is not reachable; not watching it", root.display());
                continue;
            }
            match debouncer.watch(root, RecursiveMode::Recursive) {
                Ok(()) => watched.push(root.clone()),
                Err(e) => tracing::warn!("could not watch {}: {e}", root.display()),
            }
        }
        if watched.is_empty() && !roots.is_empty() {
            return Err(CoreError::Invalid(
                "none of the library roots is reachable, nothing to watch".into(),
            ));
        }
        Ok(Watcher {
            _debouncer: debouncer,
            rx,
            roots: watched,
        })
    }

    /// Roots touched by changes, waiting up to `timeout`.
    pub fn changed_roots(&self, timeout: Duration) -> Vec<PathBuf> {
        let mut touched: Vec<PathBuf> = Vec::new();
        let mut first = true;
        loop {
            let got = if first {
                self.rx.recv_timeout(timeout)
            } else {
                self.rx
                    .try_recv()
                    .map_err(|_| mpsc::RecvTimeoutError::Timeout)
            };
            first = false;
            match got {
                Ok(paths) => {
                    for p in paths {
                        if let Some(root) = self.roots.iter().find(|r| p.starts_with(r))
                            && !touched.contains(root)
                            && !is_ignored(&p)
                        {
                            touched.push(root.clone());
                        }
                    }
                }
                Err(_) => break,
            }
        }
        touched
    }
}

fn is_ignored(path: &Path) -> bool {
    path.components().any(|c| {
        let s = c.as_os_str().to_string_lossy();
        s.starts_with('.') || super::scan::SKIP_DIRS.contains(&s.as_ref())
    })
}
