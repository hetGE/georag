//! Model operations behind the CLI and the Models screen: pull, link, use,
//! remove, verify, list.

use std::path::{Path, PathBuf};

use georag_engine::GgufInfo;
use serde::Serialize;

use super::catalog;
use super::hf;
use super::pull::{PullManager, PullRequest, PullStatus, sha256_file};
use super::registry::{ModelEntry, ModelKind, ModelRole, Registry, id_from_filename};
use crate::error::{CoreError, Result};
use crate::home::Home;

/// A registry entry with what the header and the disk say about it.
#[derive(Debug, Clone, Serialize)]
pub struct ModelView {
    #[serde(flatten)]
    pub entry: ModelEntry,
    pub present: bool,
    pub resolved_path: PathBuf,
    pub header: Option<GgufInfo>,
    pub pull: Option<PullStatus>,
}

pub fn list(home: &Home, registry: &Registry, pulls: Option<&PullManager>) -> Vec<ModelView> {
    registry
        .models
        .iter()
        .map(|m| {
            let resolved_path = m.resolve_path(home);
            let present = resolved_path.is_file();
            let header = if present {
                GgufInfo::read(&resolved_path).ok()
            } else {
                None
            };
            ModelView {
                entry: m.clone(),
                present,
                resolved_path,
                header,
                pull: pulls.and_then(|p| p.status(&m.id)),
            }
        })
        .collect()
}

/// What `models pull` will fetch for a spec, resolved against the catalogue
/// and the Hugging Face tree.
#[derive(Debug, Clone)]
pub struct PullPlan {
    pub id: String,
    pub repo: String,
    pub file: String,
    pub role: ModelRole,
    pub size: Option<u64>,
    pub sha256: Option<String>,
    pub target: PathBuf,
    pub pairs_with: Option<String>,
}

/// Resolve `org/repo[:file]` or a catalogue id. Without a file, the repo must
/// hold exactly one GGUF (projectors excluded) or the error lists the choices.
pub fn plan_pull(home: &Home, spec: &str, role: Option<ModelRole>) -> Result<PullPlan> {
    if let Some(entry) = catalog::find(spec) {
        return Ok(PullPlan {
            id: entry.id.to_string(),
            repo: entry.repo.to_string(),
            file: entry.file.to_string(),
            role: role.unwrap_or(entry.role),
            size: Some(entry.size_bytes),
            sha256: None,
            target: home.models_dir().join(entry.repo).join(entry.file),
            pairs_with: entry.pairs_with.map(str::to_string),
        });
    }
    let (repo, file) = hf::parse_spec(spec)?;
    let client = hf::client()?;
    let tree = hf::tree(&client, &repo)?;
    let ggufs: Vec<&hf::TreeEntry> = tree
        .iter()
        .filter(|e| e.path.to_ascii_lowercase().ends_with(".gguf"))
        .collect();
    let chosen = match &file {
        Some(f) => ggufs
            .iter()
            .find(|e| e.path == *f || e.path.ends_with(&format!("/{f}")))
            .copied()
            .ok_or_else(|| {
                CoreError::Invalid(format!(
                    "{f} is not in {repo}. Files there: {}",
                    ggufs
                        .iter()
                        .map(|e| e.path.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            })?,
        None => {
            let candidates: Vec<&hf::TreeEntry> = ggufs
                .iter()
                .filter(|e| !e.path.to_ascii_lowercase().contains("mmproj"))
                .copied()
                .collect();
            match candidates.as_slice() {
                [one] => *one,
                [] => return Err(CoreError::Invalid(format!("{repo} has no GGUF files"))),
                many => {
                    return Err(CoreError::Invalid(format!(
                        "{repo} has several GGUF files; pick one with {repo}:FILE. Choices: {}",
                        many.iter()
                            .map(|e| e.path.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )));
                }
            }
        }
    };
    let filename = chosen
        .path
        .rsplit('/')
        .next()
        .unwrap_or(&chosen.path)
        .to_string();
    let lower = filename.to_ascii_lowercase();
    let role = role.unwrap_or(if lower.contains("mmproj") {
        ModelRole::Vision
    } else if lower.contains("embed") || lower.contains("bge") || lower.contains("e5") {
        ModelRole::Embed
    } else {
        ModelRole::Chat
    });
    let id = catalog::find_by_file(&repo, &filename)
        .map(|e| e.id.to_string())
        .unwrap_or_else(|| id_from_filename(&filename));
    Ok(PullPlan {
        id,
        repo: repo.clone(),
        file: chosen.path.clone(),
        role,
        size: Some(chosen.size),
        sha256: chosen.sha256.clone(),
        target: home.models_dir().join(&repo).join(&chosen.path),
        pairs_with: None,
    })
}

/// Fill in the sha256 from the tree API when the plan lacks one.
pub fn complete_plan(plan: &mut PullPlan) -> Result<()> {
    if plan.sha256.is_some() {
        return Ok(());
    }
    let client = hf::client()?;
    let tree = hf::tree(&client, &plan.repo)?;
    if let Some(e) = tree.iter().find(|e| e.path == plan.file) {
        plan.sha256 = e.sha256.clone();
        plan.size = Some(e.size);
    }
    Ok(())
}

/// Start the download in the background.
pub fn start_pull(pulls: &PullManager, plan: &PullPlan) -> Result<()> {
    pulls.start(PullRequest {
        id: plan.id.clone(),
        url: hf::download_url(&plan.repo, &plan.file),
        target: plan.target.clone(),
        expected_sha256: plan.sha256.clone(),
        expected_size: plan.size,
    })
}

/// Record a finished pull in the registry (idempotent) and return the entry.
pub fn register_pulled(
    home: &Home,
    registry: &mut Registry,
    plan: &PullPlan,
) -> Result<ModelEntry> {
    let header = GgufInfo::read(&plan.target)?;
    let rel = plan
        .target
        .strip_prefix(home.root())
        .unwrap_or(&plan.target)
        .to_string_lossy()
        .replace('\\', "/");
    let entry = ModelEntry {
        id: plan.id.clone(),
        role: plan.role,
        kind: ModelKind::Managed,
        path: rel,
        sha256: plan.sha256.clone(),
        n_ctx_train: Some(header.n_ctx_train),
        size_bytes: Some(header.weight_bytes),
        dim: if plan.role == ModelRole::Embed {
            Some(header.embed_dim)
        } else {
            None
        },
        pairs_with: plan.pairs_with.clone(),
        repo: Some(plan.repo.clone()),
        in_use: registry.get(&plan.id).map(|m| m.in_use).unwrap_or(false),
    };
    registry.upsert(entry.clone());
    registry.ensure_in_use(plan.role);
    registry.save(home)?;
    Ok(registry.get(&plan.id).cloned().expect("just inserted"))
}

pub fn link(
    home: &Home,
    registry: &mut Registry,
    path: &Path,
    role: ModelRole,
    id: Option<&str>,
) -> Result<ModelEntry> {
    let path = path.canonicalize().map_err(|source| CoreError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let header = GgufInfo::read(&path)?;
    let file = path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    let id = id
        .map(str::to_string)
        .unwrap_or_else(|| id_from_filename(&file));
    let entry = ModelEntry {
        id: id.clone(),
        role,
        kind: ModelKind::Linked,
        path: path.to_string_lossy().to_string(),
        sha256: None,
        n_ctx_train: Some(header.n_ctx_train),
        size_bytes: Some(header.weight_bytes),
        dim: if role == ModelRole::Embed {
            Some(header.embed_dim)
        } else {
            None
        },
        pairs_with: None,
        repo: None,
        in_use: false,
    };
    registry.upsert(entry);
    registry.ensure_in_use(role);
    registry.save(home)?;
    Ok(registry.get(&id).cloned().expect("just inserted"))
}

pub fn use_model(home: &Home, registry: &mut Registry, id: &str) -> Result<ModelEntry> {
    let entry = registry.set_in_use(id)?.clone();
    registry.save(home)?;
    Ok(entry)
}

/// Remove from the registry; managed files are deleted from `models/`.
pub fn remove(home: &Home, registry: &mut Registry, id: &str) -> Result<ModelEntry> {
    let entry = registry
        .remove(id)
        .ok_or_else(|| CoreError::Invalid(format!("no model with id {id:?}")))?;
    if entry.kind == ModelKind::Managed {
        let path = entry.resolve_path(home);
        if path.is_file() {
            std::fs::remove_file(&path).map_err(|source| CoreError::Io {
                path: path.clone(),
                source,
            })?;
        }
        let mut part = path.as_os_str().to_owned();
        part.push(".part");
        let _ = std::fs::remove_file(PathBuf::from(part));
    }
    registry.ensure_in_use(entry.role);
    registry.save(home)?;
    Ok(entry)
}

#[derive(Debug, Clone, Serialize)]
pub struct VerifyReport {
    pub id: String,
    pub path: PathBuf,
    pub expected: Option<String>,
    pub actual: String,
    pub ok: Option<bool>,
}

/// Hash the file and compare with the recorded sha256 (None when unknown).
pub fn verify(home: &Home, registry: &Registry, id: &str) -> Result<VerifyReport> {
    let entry = registry
        .get(id)
        .ok_or_else(|| CoreError::Invalid(format!("no model with id {id:?}")))?;
    let path = entry.resolve_path(home);
    let actual = sha256_file(&path)?;
    let ok = entry
        .sha256
        .as_ref()
        .map(|e| e.eq_ignore_ascii_case(&actual));
    Ok(VerifyReport {
        id: id.to_string(),
        path,
        expected: entry.sha256.clone(),
        actual,
        ok,
    })
}
