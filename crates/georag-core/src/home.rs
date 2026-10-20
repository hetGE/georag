//! The home folder: the directory that holds the executable, its settings,
//! models, index and logs. Nothing is ever written outside it.

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::DEFAULT_SETTINGS_TOML;
use crate::error::{CoreError, Result};

/// Resolved home folder with helpers for every path the app uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Home {
    root: PathBuf,
}

/// What `Home::init` did, for the CLI to report.
#[derive(Debug, Default, Clone)]
pub struct InitReport {
    pub created_dirs: Vec<PathBuf>,
    pub created_files: Vec<PathBuf>,
    pub already_present: Vec<PathBuf>,
}

impl Home {
    /// Resolution order: explicit override (`--home`, already merged with
    /// `GEORAG_HOME` by the CLI), then the directory holding the executable.
    pub fn resolve(explicit: Option<&Path>) -> Result<Home> {
        let root = match explicit {
            Some(p) => p.to_path_buf(),
            None => {
                let exe = std::env::current_exe()
                    .map_err(|e| CoreError::HomeUnresolved(e.to_string()))?;
                exe.parent().map(Path::to_path_buf).ok_or_else(|| {
                    CoreError::HomeUnresolved("the executable has no parent directory".into())
                })?
            }
        };
        let root = if root.is_absolute() {
            root
        } else {
            std::env::current_dir()
                .map_err(|e| CoreError::HomeUnresolved(e.to_string()))?
                .join(root)
        };
        Ok(Home { root })
    }

    /// Build a `Home` at an exact path without any resolution. Used by tests
    /// and by callers that already resolved the path.
    pub fn at(root: impl Into<PathBuf>) -> Home {
        Home { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn engine_dir(&self) -> PathBuf {
        self.root.join("engine")
    }
    pub fn config_dir(&self) -> PathBuf {
        self.root.join("config")
    }
    pub fn settings_path(&self) -> PathBuf {
        self.config_dir().join("settings.toml")
    }
    pub fn models_toml_path(&self) -> PathBuf {
        self.config_dir().join("models.toml")
    }
    pub fn tags_toml_path(&self) -> PathBuf {
        self.config_dir().join("tags.toml")
    }
    pub fn models_dir(&self) -> PathBuf {
        self.root.join("models")
    }
    pub fn data_dir(&self) -> PathBuf {
        self.root.join("data")
    }
    pub fn db_path(&self) -> PathBuf {
        self.data_dir().join("georag.db")
    }
    pub fn daemon_json_path(&self) -> PathBuf {
        self.data_dir().join("daemon.json")
    }
    pub fn updates_dir(&self) -> PathBuf {
        self.data_dir().join("updates")
    }
    pub fn logs_dir(&self) -> PathBuf {
        self.data_dir().join("logs")
    }
    pub fn cache_dir(&self) -> PathBuf {
        self.data_dir().join("cache")
    }
    pub fn exports_dir(&self) -> PathBuf {
        self.root.join("exports")
    }

    /// Every directory `georag init` creates, in creation order.
    pub fn layout_dirs(&self) -> Vec<PathBuf> {
        vec![
            self.root.clone(),
            self.engine_dir(),
            self.config_dir(),
            self.models_dir(),
            self.data_dir(),
            self.updates_dir(),
            self.logs_dir(),
            self.cache_dir(),
            self.exports_dir(),
        ]
    }

    /// True when the layout exists (the config and data directories are present).
    pub fn is_initialised(&self) -> bool {
        self.config_dir().is_dir() && self.data_dir().is_dir()
    }

    /// Create the folder layout and the default `settings.toml` if absent.
    /// Idempotent. Fails with a clear message when the folder is read only.
    pub fn init(&self) -> Result<InitReport> {
        let mut report = InitReport::default();
        for dir in self.layout_dirs() {
            if dir.is_dir() {
                report.already_present.push(dir);
            } else {
                fs::create_dir_all(&dir).map_err(|e| read_only_or_io(&dir, e))?;
                report.created_dirs.push(dir);
            }
        }
        let settings = self.settings_path();
        if settings.is_file() {
            report.already_present.push(settings);
        } else {
            fs::write(&settings, DEFAULT_SETTINGS_TOML)
                .map_err(|e| read_only_or_io(&settings, e))?;
            report.created_files.push(settings);
        }
        self.check_writable()?;
        Ok(report)
    }

    /// Verify that files can be created inside `data/`. The probe file is
    /// removed before returning.
    pub fn check_writable(&self) -> Result<()> {
        let data = self.data_dir();
        fs::create_dir_all(&data).map_err(|e| read_only_or_io(&data, e))?;
        let probe = data.join(".write-check");
        fs::write(&probe, b"ok").map_err(|e| read_only_or_io(&probe, e))?;
        let _ = fs::remove_file(&probe);
        Ok(())
    }
}

fn read_only_or_io(path: &Path, source: std::io::Error) -> CoreError {
    use std::io::ErrorKind::{PermissionDenied, ReadOnlyFilesystem};
    if matches!(source.kind(), PermissionDenied | ReadOnlyFilesystem) {
        CoreError::HomeReadOnly {
            path: path.to_path_buf(),
            source,
        }
    } else {
        CoreError::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_home(tag: &str) -> Home {
        let dir =
            std::env::temp_dir().join(format!("georag-home-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Home::at(dir)
    }

    #[test]
    fn init_creates_layout_and_is_idempotent() {
        let home = temp_home("init");
        let first = home.init().unwrap();
        assert_eq!(first.created_dirs.len(), home.layout_dirs().len());
        assert_eq!(first.created_files, vec![home.settings_path()]);
        assert!(home.is_initialised());

        let second = home.init().unwrap();
        assert!(second.created_dirs.is_empty());
        assert!(second.created_files.is_empty());
        let _ = fs::remove_dir_all(home.root());
    }

    #[test]
    fn explicit_relative_home_is_made_absolute() {
        let home = Home::resolve(Some(Path::new("relative-home"))).unwrap();
        assert!(home.root().is_absolute());
        assert!(home.root().ends_with("relative-home"));
    }
}
