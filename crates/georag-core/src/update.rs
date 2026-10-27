//! The updater (handoff section 11): a signed manifest on GitHub Releases,
//! an asset chosen by this binary's flavour, resumable download, hash
//! check, staging, swap with `.old`, a self-check of the new binary, and
//! rollback.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::home::Home;
use crate::models::pull::{PullManager, PullRequest, PullState, sha256_file};

/// Distribution repository; override with `GEORAG_UPDATE_BASE` for tests
/// and mirrors (a URL that serves `manifest.json` and the assets).
pub const RELEASES_BASE: &str = "https://github.com/hetGE/georag/releases";

/// Minisign public key compiled into the binary. `None` until the release
/// key exists (section 11.3); manifests are then accepted unsigned with a
/// warning in the report.
pub const MINISIGN_PUBLIC_KEY: Option<&str> = None;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ManifestAsset {
    pub file: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    pub version: String,
    pub channel: String,
    pub published: String,
    pub min_version_for_update: String,
    pub notes: String,
    pub assets: std::collections::BTreeMap<String, ManifestAsset>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckReport {
    pub current: String,
    pub latest: String,
    pub channel: String,
    pub newer: bool,
    pub can_update_directly: bool,
    pub signature_verified: bool,
    pub asset: Option<ManifestAsset>,
    pub notes_url: String,
    pub notes: Vec<String>,
}

fn base_url(channel: &str) -> String {
    let base = std::env::var("GEORAG_UPDATE_BASE").unwrap_or_else(|_| RELEASES_BASE.to_string());
    let base = base.trim_end_matches('/').to_string();
    if std::env::var("GEORAG_UPDATE_BASE").is_ok() {
        return base;
    }
    if channel == "nightly" {
        format!("{base}/download/nightly")
    } else {
        format!("{base}/latest/download")
    }
}

fn client() -> Result<reqwest::blocking::Client> {
    crate::models::hf::client()
}

fn fetch_text(client: &reqwest::blocking::Client, url: &str) -> Result<Option<String>> {
    let resp = client
        .get(url)
        .send()
        .map_err(|e| CoreError::Network(format!("{url}: {e}")))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(CoreError::Network(format!(
            "{url} returned {}",
            resp.status()
        )));
    }
    Ok(Some(
        resp.text()
            .map_err(|e| CoreError::Network(format!("{url}: {e}")))?,
    ))
}

/// Verify `data` against a minisign signature with the compiled-in key.
/// Returns Ok(false) when no key is compiled in.
pub fn verify_signature(data: &[u8], signature: Option<&str>) -> Result<bool> {
    let Some(key_b64) = MINISIGN_PUBLIC_KEY else {
        return Ok(false);
    };
    let Some(sig_text) = signature else {
        return Err(CoreError::Invalid(
            "the release manifest has no signature: refusing to update".into(),
        ));
    };
    let key = minisign_verify::PublicKey::from_base64(key_b64)
        .map_err(|e| CoreError::Invalid(format!("built-in public key is invalid: {e}")))?;
    let sig = minisign_verify::Signature::decode(sig_text)
        .map_err(|e| CoreError::Invalid(format!("signature is malformed: {e}")))?;
    key.verify(data, &sig, false).map_err(|e| {
        CoreError::Invalid(format!(
            "signature check failed: {e}. The download may be tampered with; not updating"
        ))
    })?;
    Ok(true)
}

/// Fetch and check the manifest for `channel`.
pub fn check(channel: &str) -> Result<CheckReport> {
    let client = client()?;
    let base = base_url(channel);
    let url = format!("{base}/manifest.json");
    let text = fetch_text(&client, &url)?
        .ok_or_else(|| CoreError::Network(format!("no release manifest at {url}")))?;
    let signature = fetch_text(&client, &format!("{url}.minisig"))?;
    let signature_verified = verify_signature(text.as_bytes(), signature.as_deref())?;
    let manifest: Manifest = serde_json::from_str(&text)
        .map_err(|e| CoreError::Invalid(format!("manifest.json is invalid: {e}")))?;
    let mut notes = Vec::new();
    if !signature_verified {
        notes.push(
            "manifest signature not verified: no release key is compiled into this build"
                .to_string(),
        );
    }
    let current = semver::Version::parse(crate::VERSION)
        .map_err(|e| CoreError::Invalid(format!("built-in version: {e}")))?;
    let latest = semver::Version::parse(&manifest.version)
        .map_err(|e| CoreError::Invalid(format!("manifest version {:?}: {e}", manifest.version)))?;
    let min = semver::Version::parse(&manifest.min_version_for_update)
        .unwrap_or_else(|_| semver::Version::new(0, 0, 0));
    // A dev or pre-release build counts as its base version for the gate.
    let current_base = semver::Version::new(current.major, current.minor, current.patch);
    let asset = manifest.assets.get(crate::FLAVOUR).cloned();
    if asset.is_none() {
        notes.push(format!(
            "the release has no asset for flavour {}",
            crate::FLAVOUR
        ));
    }
    Ok(CheckReport {
        current: current.to_string(),
        latest: latest.to_string(),
        channel: manifest.channel.clone(),
        newer: latest > current,
        can_update_directly: current_base >= min,
        signature_verified,
        asset,
        notes_url: format!("{base}/{}", manifest.notes),
        notes,
    })
}

/// Fetch the release notes named by a check report.
pub fn fetch_notes(report: &CheckReport) -> Result<String> {
    let client = client()?;
    Ok(fetch_text(&client, &report.notes_url)?.unwrap_or_default())
}

/// Cached notice for the status bar, written by `check_for_notice`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Notice {
    pub checked_at: String,
    pub latest: String,
    pub newer: bool,
}

pub fn notice_path(home: &Home) -> PathBuf {
    home.data_dir().join("update-notice.json")
}

pub fn read_notice(home: &Home) -> Option<Notice> {
    let text = fs::read_to_string(notice_path(home)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Run a check at most once per 24 hours and cache the result. Never
/// installs anything.
pub fn check_for_notice(
    home: &Home,
    settings: &mut crate::config::Settings,
) -> Result<Option<Notice>> {
    if !settings.update.auto_check {
        return Ok(read_notice(home));
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Some(n) = read_notice(home)
        && let Some(then) = parse_rfc3339_secs(&n.checked_at)
        && now.saturating_sub(then) < 86_400
    {
        return Ok(Some(n));
    }
    let report = check(&settings.update.channel)?;
    let notice = Notice {
        checked_at: georag_store::time::rfc3339_from_unix(now),
        latest: report.latest,
        newer: report.newer,
    };
    let path = notice_path(home);
    fs::write(&path, serde_json::to_string(&notice)?)
        .map_err(|source| CoreError::Io { path, source })?;
    settings.app.version_checked_at = notice.checked_at.clone();
    let _ = settings.save(&home.settings_path());
    Ok(Some(notice))
}

fn parse_rfc3339_secs(s: &str) -> Option<u64> {
    // YYYY-MM-DDTHH:MM:SSZ
    let b = s.as_bytes();
    if b.len() < 20 {
        return None;
    }
    let num = |a: usize, l: usize| s.get(a..a + l)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, sec) = (
        num(0, 4)?,
        num(5, 2)?,
        num(8, 2)?,
        num(11, 2)?,
        num(14, 2)?,
        num(17, 2)?,
    );
    let (y2, m2) = if mo <= 2 {
        (y - 1, mo + 9)
    } else {
        (y, mo - 3)
    };
    let era = y2.div_euclid(400);
    let yoe = y2.rem_euclid(400);
    let doy = (153 * m2 + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some((days * 86_400 + h * 3600 + mi * 60 + sec).max(0) as u64)
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadReport {
    pub bundle: PathBuf,
    pub sha256_verified: bool,
}

/// Download the asset for this flavour into `data/updates/` with resume.
pub fn download(
    home: &Home,
    report: &CheckReport,
    channel: &str,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<DownloadReport> {
    let asset = report
        .asset
        .clone()
        .ok_or_else(|| CoreError::Invalid(format!("no asset for flavour {}", crate::FLAVOUR)))?;
    let dir = home.updates_dir();
    fs::create_dir_all(&dir).map_err(|source| CoreError::Io {
        path: dir.clone(),
        source,
    })?;
    let target = dir.join(&asset.file);
    let url = format!("{}/{}", base_url(channel), asset.file);
    let pulls = PullManager::new();
    pulls.start(PullRequest {
        id: "update".into(),
        url,
        target: target.clone(),
        expected_sha256: Some(asset.sha256.clone()),
        expected_size: Some(asset.size),
    })?;
    let status = pulls.wait("update", |st| progress(st.bytes, st.total))?;
    if status.state != PullState::Done {
        return Err(CoreError::Network(format!(
            "download did not finish: {:?}",
            status.state
        )));
    }
    Ok(DownloadReport {
        bundle: target,
        sha256_verified: true,
    })
}

/// Unpack a bundle into `data/updates/staging/<name>/` and return the folder
/// that holds `georag`.
pub fn stage(home: &Home, bundle: &Path) -> Result<PathBuf> {
    let staging = home.updates_dir().join("staging");
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(|source| CoreError::Io {
            path: staging.clone(),
            source,
        })?;
    }
    fs::create_dir_all(&staging).map_err(|source| CoreError::Io {
        path: staging.clone(),
        source,
    })?;
    let name = bundle
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let file = fs::File::open(bundle).map_err(|source| CoreError::Io {
        path: bundle.to_path_buf(),
        source,
    })?;
    if name.ends_with(".zip") {
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|e| CoreError::Invalid(format!("{}: {e}", bundle.display())))?;
        archive
            .extract(&staging)
            .map_err(|e| CoreError::Invalid(format!("extracting {}: {e}", bundle.display())))?;
    } else {
        let gz = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(gz);
        archive
            .unpack(&staging)
            .map_err(|e| CoreError::Invalid(format!("extracting {}: {e}", bundle.display())))?;
    }
    let exe = exe_name();
    // The archive holds one top-level folder.
    let mut found = None;
    for entry in fs::read_dir(&staging).map_err(|source| CoreError::Io {
        path: staging.clone(),
        source,
    })? {
        let entry = entry.map_err(|source| CoreError::Io {
            path: staging.clone(),
            source,
        })?;
        if entry.path().join(exe).is_file() {
            found = Some(entry.path());
        }
    }
    found.ok_or_else(|| {
        CoreError::Invalid(format!(
            "the bundle does not contain {exe} in a top-level folder"
        ))
    })
}

pub fn exe_name() -> &'static str {
    if cfg!(windows) {
        "georag.exe"
    } else {
        "georag"
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplyReport {
    pub replaced_exe: PathBuf,
    pub replaced_engine: bool,
    pub self_check_ok: bool,
    pub rolled_back: bool,
}

/// Swap the staged folder into the home: `georag` to `georag.old`,
/// `engine/` to `engine.old/`, then run the new binary with `--self-check`.
/// On failure the `.old` copies are restored.
pub fn apply(home: &Home, staged: &Path, current_exe: &Path) -> Result<ApplyReport> {
    home.check_writable()?;
    let exe = exe_name();
    let new_exe = staged.join(exe);
    let old_exe = with_suffix(current_exe, ".old");
    let engine_dir = home.engine_dir();
    let old_engine = home.root().join("engine.old");

    // Engine folder first (plain rename), then the executable.
    let _ = fs::remove_dir_all(&old_engine);
    let mut replaced_engine = false;
    if staged.join("engine").is_dir() {
        if engine_dir.is_dir() {
            fs::rename(&engine_dir, &old_engine).map_err(|source| CoreError::Io {
                path: engine_dir.clone(),
                source,
            })?;
        }
        copy_tree(&staged.join("engine"), &engine_dir)?;
        replaced_engine = true;
    }
    let config_default = staged.join("config");
    if config_default.is_dir() {
        for entry in fs::read_dir(&config_default)
            .map_err(|source| CoreError::Io {
                path: config_default.clone(),
                source,
            })?
            .flatten()
        {
            let _ = fs::copy(entry.path(), home.config_dir().join(entry.file_name()));
        }
    }

    let _ = fs::remove_file(&old_exe);
    replace_executable(current_exe, &new_exe, &old_exe)?;

    let check = std::process::Command::new(current_exe)
        .arg("--self-check")
        .status();
    let self_check_ok = matches!(check, Ok(s) if s.success());
    if self_check_ok {
        let _ = fs::remove_file(&old_exe);
        let _ = fs::remove_dir_all(&old_engine);
        Ok(ApplyReport {
            replaced_exe: current_exe.to_path_buf(),
            replaced_engine,
            self_check_ok,
            rolled_back: false,
        })
    } else {
        tracing::error!("self-check of the new binary failed; restoring the previous version");
        restore_old(current_exe, &old_exe, &engine_dir, &old_engine)?;
        Ok(ApplyReport {
            replaced_exe: current_exe.to_path_buf(),
            replaced_engine,
            self_check_ok,
            rolled_back: true,
        })
    }
}

fn replace_executable(current: &Path, new: &Path, old: &Path) -> Result<()> {
    if cfg!(windows) {
        // A running executable cannot be renamed over on Windows.
        fs::copy(current, old).map_err(|source| CoreError::Io {
            path: old.to_path_buf(),
            source,
        })?;
        self_replace::self_replace(new)
            .map_err(|e| CoreError::Invalid(format!("replacing the executable: {e}")))?;
        Ok(())
    } else {
        fs::rename(current, old).map_err(|source| CoreError::Io {
            path: current.to_path_buf(),
            source,
        })?;
        if let Err(e) = fs::copy(new, current) {
            let _ = fs::rename(old, current);
            return Err(CoreError::Io {
                path: current.to_path_buf(),
                source: e,
            });
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(current, fs::Permissions::from_mode(0o755));
        }
        Ok(())
    }
}

fn restore_old(current: &Path, old: &Path, engine_dir: &Path, old_engine: &Path) -> Result<()> {
    if old.is_file() {
        if cfg!(windows) {
            self_replace::self_replace(old)
                .map_err(|e| CoreError::Invalid(format!("restoring the executable: {e}")))?;
            let _ = fs::remove_file(old);
        } else {
            fs::rename(old, current).map_err(|source| CoreError::Io {
                path: current.to_path_buf(),
                source,
            })?;
        }
    }
    if old_engine.is_dir() {
        let _ = fs::remove_dir_all(engine_dir);
        fs::rename(old_engine, engine_dir).map_err(|source| CoreError::Io {
            path: engine_dir.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

/// `--rollback`: put the `.old` copies back.
pub fn rollback(home: &Home, current_exe: &Path) -> Result<bool> {
    let old_exe = with_suffix(current_exe, ".old");
    let old_engine = home.root().join("engine.old");
    if !old_exe.is_file() && !old_engine.is_dir() {
        return Ok(false);
    }
    restore_old(current_exe, &old_exe, &home.engine_dir(), &old_engine)?;
    Ok(true)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst).map_err(|source| CoreError::Io {
        path: dst.to_path_buf(),
        source,
    })?;
    for entry in fs::read_dir(src).map_err(|source| CoreError::Io {
        path: src.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| CoreError::Io {
            path: src.to_path_buf(),
            source,
        })?;
        let target = dst.join(entry.file_name());
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target).map_err(|source| CoreError::Io {
                path: target.clone(),
                source,
            })?;
        }
    }
    Ok(())
}

/// Write `notes.md` next to the bundle for the Update screen.
pub fn save_notes(home: &Home, text: &str) -> Result<PathBuf> {
    let path = home.updates_dir().join("notes.md");
    let mut f = fs::File::create(&path).map_err(|source| CoreError::Io {
        path: path.clone(),
        source,
    })?;
    f.write_all(text.as_bytes())
        .map_err(|source| CoreError::Io {
            path: path.clone(),
            source,
        })?;
    Ok(path)
}

/// Validate an offline bundle's hash against a manifest asset when known.
pub fn verify_bundle(bundle: &Path, expected_sha256: Option<&str>) -> Result<String> {
    let actual = sha256_file(bundle)?;
    if let Some(e) = expected_sha256
        && !e.eq_ignore_ascii_case(&actual)
    {
        return Err(CoreError::Invalid(format!(
            "{} does not match the manifest (expected {e}, got {actual})",
            bundle.display()
        )));
    }
    Ok(actual)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_parses() {
        assert_eq!(parse_rfc3339_secs("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_rfc3339_secs("2023-11-14T22:13:20Z"),
            Some(1_700_000_000)
        );
        assert_eq!(parse_rfc3339_secs("nope"), None);
    }

    #[test]
    fn unsigned_manifest_is_accepted_without_a_key() {
        assert!(!verify_signature(b"{}", None).unwrap());
        assert_eq!(MINISIGN_PUBLIC_KEY, None);
    }

    #[test]
    fn manifest_shape_round_trips() {
        let text = r#"{ "version": "2.1.0", "channel": "stable", "published": "2026-11-02T10:00:00Z",
  "min_version_for_update": "2.0.0", "notes": "notes.md",
  "assets": { "aarch64-apple-darwin": {"file": "georag-2.1.0-aarch64-apple-darwin.tar.gz", "sha256": "ab", "size": 123 } } }"#;
        let m: Manifest = serde_json::from_str(text).unwrap();
        assert_eq!(m.assets["aarch64-apple-darwin"].size, 123);
        assert!(
            semver::Version::parse(&m.version).unwrap()
                > semver::Version::parse(crate::VERSION).unwrap()
        );
    }

    #[cfg(unix)]
    #[test]
    fn stage_apply_and_rollback_with_a_fake_bundle() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("georag-update-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let home = Home::at(dir.join("GeoRAG"));
        home.init().unwrap();
        // The "current" executable is a script that reports version 1.
        let exe = home.root().join("georag");
        fs::write(&exe, "#!/bin/sh\necho v1\nexit 0\n").unwrap();
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(home.engine_dir().join("old.txt"), "old").unwrap();

        // Build a bundle whose georag passes --self-check.
        let stage_src = dir.join("georag-2.1.0-test");
        fs::create_dir_all(stage_src.join("engine")).unwrap();
        fs::create_dir_all(stage_src.join("config")).unwrap();
        fs::write(stage_src.join("georag"), "#!/bin/sh\necho v2\nexit 0\n").unwrap();
        fs::set_permissions(stage_src.join("georag"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(stage_src.join("engine/new.txt"), "new").unwrap();
        fs::write(stage_src.join("config/settings.toml.default"), "x").unwrap();
        let bundle = dir.join("georag-2.1.0-test.tar.gz");
        {
            let f = fs::File::create(&bundle).unwrap();
            let gz = flate2::write::GzEncoder::new(f, flate2::Compression::default());
            let mut t = tar::Builder::new(gz);
            t.append_dir_all("georag-2.1.0-test", &stage_src).unwrap();
            t.into_inner().unwrap().finish().unwrap();
        }

        let staged = stage(&home, &bundle).unwrap();
        assert!(staged.join("georag").is_file());
        let report = apply(&home, &staged, &exe).unwrap();
        assert!(report.self_check_ok, "{report:?}");
        assert!(!report.rolled_back);
        assert!(fs::read_to_string(&exe).unwrap().contains("v2"));
        assert!(home.engine_dir().join("new.txt").is_file());
        assert!(!home.engine_dir().join("old.txt").exists());
        assert!(home.config_dir().join("settings.toml.default").is_file());
        // Nothing to roll back after a successful apply.
        assert!(!rollback(&home, &exe).unwrap());

        // A failing self-check restores the old files.
        fs::write(stage_src.join("georag"), "#!/bin/sh\nexit 3\n").unwrap();
        fs::write(stage_src.join("engine/new.txt"), "broken").unwrap();
        let staged2 = dir.join("staged2");
        fs::create_dir_all(&staged2).unwrap();
        copy_tree(&stage_src, &staged2).unwrap();
        let report = apply(&home, &staged2, &exe).unwrap();
        assert!(!report.self_check_ok);
        assert!(report.rolled_back);
        assert!(fs::read_to_string(&exe).unwrap().contains("v2"));
        assert_eq!(
            fs::read_to_string(home.engine_dir().join("new.txt")).unwrap(),
            "new"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
