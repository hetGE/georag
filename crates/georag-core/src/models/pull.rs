//! Resumable downloads with progress, pause and sha256 verification.
//! A pull writes `<target>.part`, resumes from its length with an HTTP Range
//! request, verifies the hash, then renames. Workers run on plain threads so
//! pulls outlive any screen and the manager can be polled from a tick.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::{CoreError, Result};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum PullState {
    Queued,
    Downloading,
    Paused,
    Verifying,
    Done,
    Failed { error: String },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PullStatus {
    pub id: String,
    pub url: String,
    pub target: PathBuf,
    pub bytes: u64,
    pub total: Option<u64>,
    pub bytes_per_second: f64,
    pub eta_seconds: Option<u64>,
    pub state: PullState,
}

impl PullStatus {
    pub fn fraction(&self) -> Option<f64> {
        self.total
            .filter(|t| *t > 0)
            .map(|t| self.bytes as f64 / t as f64)
    }
}

#[derive(Debug, Clone)]
pub struct PullRequest {
    pub id: String,
    pub url: String,
    pub target: PathBuf,
    pub expected_sha256: Option<String>,
    pub expected_size: Option<u64>,
}

struct Worker {
    cancel: Arc<AtomicBool>,
}

/// Tracks every pull in the process.
#[derive(Default)]
pub struct PullManager {
    statuses: Arc<Mutex<HashMap<String, PullStatus>>>,
    workers: Mutex<HashMap<String, Worker>>,
}

impl PullManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn status(&self, id: &str) -> Option<PullStatus> {
        self.statuses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
    }

    pub fn statuses(&self) -> Vec<PullStatus> {
        let mut v: Vec<PullStatus> = self
            .statuses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    /// Start (or resume) a pull in the background.
    pub fn start(&self, req: PullRequest) -> Result<()> {
        let client = super::hf::client()?;
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut statuses = self.statuses.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(existing) = statuses.get(&req.id)
                && matches!(
                    existing.state,
                    PullState::Downloading | PullState::Verifying | PullState::Queued
                )
            {
                return Err(CoreError::Invalid(format!(
                    "{} is already being pulled",
                    req.id
                )));
            }
            statuses.insert(
                req.id.clone(),
                PullStatus {
                    id: req.id.clone(),
                    url: req.url.clone(),
                    target: req.target.clone(),
                    bytes: part_len(&req.target),
                    total: req.expected_size,
                    bytes_per_second: 0.0,
                    eta_seconds: None,
                    state: PullState::Queued,
                },
            );
        }
        self.workers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                req.id.clone(),
                Worker {
                    cancel: cancel.clone(),
                },
            );
        let statuses = self.statuses.clone();
        std::thread::Builder::new()
            .name(format!("pull-{}", req.id))
            .spawn(move || {
                let id = req.id.clone();
                let result = run_pull(&client, &req, &statuses, &cancel);
                let mut s = statuses.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(st) = s.get_mut(&id) {
                    st.state = match result {
                        Ok(()) => PullState::Done,
                        Err(_) if cancel.load(Ordering::SeqCst) => PullState::Paused,
                        Err(e) => PullState::Failed {
                            error: e.to_string(),
                        },
                    };
                    st.bytes_per_second = 0.0;
                    st.eta_seconds = None;
                }
            })
            .map_err(|e| CoreError::Invalid(format!("could not start the download thread: {e}")))?;
        Ok(())
    }

    /// Pause: the worker stops at the next buffer; the `.part` file stays.
    pub fn pause(&self, id: &str) {
        if let Some(w) = self
            .workers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
        {
            w.cancel.store(true, Ordering::SeqCst);
        }
    }

    /// Block until the pull leaves the active states; for the CLI.
    pub fn wait(&self, id: &str, mut on_tick: impl FnMut(&PullStatus)) -> Result<PullStatus> {
        loop {
            let Some(st) = self.status(id) else {
                return Err(CoreError::Invalid(format!("no pull named {id}")));
            };
            on_tick(&st);
            match st.state {
                PullState::Done => return Ok(st),
                PullState::Failed { ref error } => return Err(CoreError::Network(error.clone())),
                PullState::Paused => return Ok(st),
                _ => std::thread::sleep(Duration::from_millis(200)),
            }
        }
    }
}

fn part_path(target: &Path) -> PathBuf {
    let mut p = target.as_os_str().to_owned();
    p.push(".part");
    PathBuf::from(p)
}

fn part_len(target: &Path) -> u64 {
    fs::metadata(part_path(target))
        .map(|m| m.len())
        .unwrap_or(0)
}

fn set_state(statuses: &Mutex<HashMap<String, PullStatus>>, id: &str, state: PullState) {
    if let Some(st) = statuses
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_mut(id)
    {
        st.state = state;
    }
}

fn run_pull(
    client: &reqwest::blocking::Client,
    req: &PullRequest,
    statuses: &Arc<Mutex<HashMap<String, PullStatus>>>,
    cancel: &AtomicBool,
) -> Result<()> {
    if req.target.is_file() {
        set_state(statuses, &req.id, PullState::Verifying);
        return verify_and_finish(&req.target, req.expected_sha256.as_deref(), None);
    }
    if let Some(dir) = req.target.parent() {
        fs::create_dir_all(dir).map_err(|source| CoreError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
    }
    let part = part_path(&req.target);
    let mut have = part_len(&req.target);
    if let Some(total) = req.expected_size
        && have > total
    {
        // A stale or corrupt partial; start again.
        let _ = fs::remove_file(&part);
        have = 0;
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&part)
        .map_err(|source| CoreError::Io {
            path: part.clone(),
            source,
        })?;

    let mut request = client.get(&req.url);
    if have > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let mut resp = request
        .send()
        .map_err(|e| CoreError::Network(format!("{}: {e}", req.url)))?;
    let status = resp.status();
    if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        // Already complete on disk, fall through to verification.
    } else if !status.is_success() {
        return Err(CoreError::Network(format!("{} returned {status}", req.url)));
    } else {
        if have > 0 && status != reqwest::StatusCode::PARTIAL_CONTENT {
            // Server ignored the range: start over.
            drop(file);
            fs::remove_file(&part).map_err(|source| CoreError::Io {
                path: part.clone(),
                source,
            })?;
            file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&part)
                .map_err(|source| CoreError::Io {
                    path: part.clone(),
                    source,
                })?;
            have = 0;
        }
        let total = req
            .expected_size
            .or_else(|| resp.content_length().map(|l| l + have));
        {
            let mut s = statuses.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(st) = s.get_mut(&req.id) {
                st.total = total;
                st.bytes = have;
                st.state = PullState::Downloading;
            }
        }
        let mut buf = vec![0u8; 1 << 18];
        let started = Instant::now();
        let start_bytes = have;
        let mut last_report = Instant::now();
        loop {
            if cancel.load(Ordering::SeqCst) {
                file.flush().ok();
                return Err(CoreError::Network("paused".into()));
            }
            let n = resp
                .read(&mut buf)
                .map_err(|e| CoreError::Network(format!("{}: {e}", req.url)))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).map_err(|source| CoreError::Io {
                path: part.clone(),
                source,
            })?;
            have += n as u64;
            if last_report.elapsed() >= Duration::from_millis(250) {
                last_report = Instant::now();
                let secs = started.elapsed().as_secs_f64().max(0.001);
                let rate = (have - start_bytes) as f64 / secs;
                let mut s = statuses.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(st) = s.get_mut(&req.id) {
                    st.bytes = have;
                    st.bytes_per_second = rate;
                    st.eta_seconds = total.map(|t| {
                        if rate > 0.0 {
                            ((t.saturating_sub(have)) as f64 / rate) as u64
                        } else {
                            0
                        }
                    });
                }
            }
        }
        file.flush().map_err(|source| CoreError::Io {
            path: part.clone(),
            source,
        })?;
    }
    drop(file);
    {
        let mut s = statuses.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(st) = s.get_mut(&req.id) {
            st.bytes = have;
            st.state = PullState::Verifying;
        }
    }
    verify_and_finish(&req.target, req.expected_sha256.as_deref(), Some(&part))
}

/// Check the hash of `part` (or of the final file) and rename into place.
fn verify_and_finish(target: &Path, expected: Option<&str>, part: Option<&Path>) -> Result<()> {
    let candidate = part.unwrap_or(target);
    if let Some(expected) = expected {
        let actual = sha256_file(candidate)?;
        if !actual.eq_ignore_ascii_case(expected) {
            if part.is_some() {
                let _ = fs::remove_file(candidate);
            }
            return Err(CoreError::Invalid(format!(
                "sha256 mismatch for {}: expected {expected}, got {actual}. The partial file was removed; pull again",
                target.display()
            )));
        }
    }
    if let Some(part) = part {
        fs::rename(part, target).map_err(|source| CoreError::Io {
            path: target.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|source| CoreError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf).map_err(|source| CoreError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    /// A tiny HTTP server that honours Range, for resume tests.
    fn serve(body: Vec<u8>) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            for stream in listener.incoming().take(3) {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut start = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(r) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                        start = r.trim().trim_end_matches('-').parse().unwrap_or(0);
                    }
                }
                let slice = &body[start.min(body.len())..];
                let head = if start > 0 {
                    format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
                        slice.len(),
                        start,
                        body.len() - 1,
                        body.len()
                    )
                } else {
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        slice.len()
                    )
                };
                stream.write_all(head.as_bytes()).unwrap();
                stream.write_all(slice).unwrap();
            }
        });
        (format!("http://{addr}/file.bin"), handle)
    }

    #[test]
    fn resumes_from_partial_and_verifies_hash() {
        let body: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let expected = {
            let mut h = Sha256::new();
            h.update(&body);
            h.finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };
        let dir = std::env::temp_dir().join(format!("georag-pull-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("file.bin");
        // Pretend an earlier run stopped after 50 000 bytes.
        fs::write(part_path(&target), &body[..50_000]).unwrap();

        let (url, server) = serve(body.clone());
        let manager = PullManager::new();
        manager
            .start(PullRequest {
                id: "t".into(),
                url,
                target: target.clone(),
                expected_sha256: Some(expected),
                expected_size: Some(body.len() as u64),
            })
            .unwrap();
        let st = manager.wait("t", |_| {}).unwrap();
        assert_eq!(st.state, PullState::Done);
        assert_eq!(fs::read(&target).unwrap(), body);
        assert!(!part_path(&target).exists());
        let _ = fs::remove_dir_all(&dir);
        drop(server);
    }

    #[test]
    fn hash_mismatch_fails_and_removes_partial() {
        let body = vec![7u8; 10_000];
        let dir = std::env::temp_dir().join(format!("georag-pull-bad-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("bad.bin");
        let (url, _server) = serve(body);
        let manager = PullManager::new();
        manager
            .start(PullRequest {
                id: "b".into(),
                url,
                target: target.clone(),
                expected_sha256: Some("00".repeat(32)),
                expected_size: None,
            })
            .unwrap();
        let err = manager.wait("b", |_| {}).unwrap_err().to_string();
        assert!(err.contains("sha256 mismatch"), "{err}");
        assert!(!target.exists());
        assert!(!part_path(&target).exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
