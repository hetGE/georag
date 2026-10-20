//! Hugging Face lookups: the tree API gives file sizes and LFS sha256.

use serde::Deserialize;

use crate::error::{CoreError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    pub path: String,
    pub size: u64,
    pub sha256: Option<String>,
}

#[derive(Deserialize)]
struct RawEntry {
    #[serde(rename = "type")]
    kind: String,
    path: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    lfs: Option<RawLfs>,
}

#[derive(Deserialize)]
struct RawLfs {
    oid: String,
    #[serde(default)]
    size: u64,
}

pub fn client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(format!("georag/{}", crate::VERSION))
        .connect_timeout(std::time::Duration::from_secs(20))
        .timeout(None)
        .build()
        .map_err(|e| CoreError::Network(e.to_string()))
}

/// Split `org/repo[:file]` into repo and optional file.
pub fn parse_spec(spec: &str) -> Result<(String, Option<String>)> {
    let (repo, file) = match spec.split_once(':') {
        Some((r, f)) => (r.trim(), Some(f.trim().to_string())),
        None => (spec.trim(), None),
    };
    let parts: Vec<&str> = repo.split('/').collect();
    if parts.len() != 2 || parts.iter().any(|p| p.is_empty()) {
        return Err(CoreError::Invalid(format!(
            "{spec:?} is not org/repo or org/repo:file"
        )));
    }
    Ok((repo.to_string(), file.filter(|f| !f.is_empty())))
}

/// Files in a repo (recursive), with sizes and LFS sha256 where present.
pub fn tree(client: &reqwest::blocking::Client, repo: &str) -> Result<Vec<TreeEntry>> {
    let url = format!("https://huggingface.co/api/models/{repo}/tree/main?recursive=true");
    let resp = client
        .get(&url)
        .send()
        .map_err(|e| CoreError::Network(format!("{url}: {e}")))?;
    if !resp.status().is_success() {
        return Err(CoreError::Network(format!(
            "{url} returned {}: check the repo name, or whether it needs a licence acceptance on huggingface.co",
            resp.status()
        )));
    }
    let raw: Vec<RawEntry> = resp
        .json()
        .map_err(|e| CoreError::Network(format!("{url}: {e}")))?;
    Ok(raw
        .into_iter()
        .filter(|e| e.kind == "file")
        .map(|e| {
            let (size, sha) = match e.lfs {
                Some(l) => (if l.size > 0 { l.size } else { e.size }, Some(l.oid)),
                None => (e.size, None),
            };
            TreeEntry {
                path: e.path,
                size,
                sha256: sha,
            }
        })
        .collect())
}

pub fn download_url(repo: &str, file: &str) -> String {
    format!("https://huggingface.co/{repo}/resolve/main/{file}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specs_parse() {
        assert_eq!(parse_spec("org/repo").unwrap(), ("org/repo".into(), None));
        assert_eq!(
            parse_spec("org/repo:a.gguf").unwrap(),
            ("org/repo".into(), Some("a.gguf".into()))
        );
        assert!(parse_spec("repo").is_err());
        assert!(parse_spec("a/b/c").is_err());
    }
}
