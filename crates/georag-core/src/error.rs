use std::path::PathBuf;

/// Errors raised by the core crate. Every message says what happened and what
/// the user can do about it.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("could not work out the home folder ({0}): pass --home DIR or set GEORAG_HOME")]
    HomeUnresolved(String),

    #[error(
        "the home folder {path} is not writable ({source}): move GeoRAG to a writable location or pass --home DIR"
    )]
    HomeReadOnly {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("could not access {path} ({source})")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "the settings file {path} is invalid ({source}): fix the line shown, or delete the file and run `georag init` to restore defaults"
    )]
    SettingsParse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("could not serialise settings ({0})")]
    SettingsSerialise(#[from] toml::ser::Error),

    #[error("could not initialise logging ({0})")]
    Logging(String),

    #[error("{0}")]
    Invalid(String),

    #[error("network error: {0}")]
    Network(String),

    #[error("invalid JSON ({0})")]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Engine(#[from] georag_engine::EngineError),

    #[error(transparent)]
    Store(#[from] georag_store::StoreError),
}

pub type Result<T> = std::result::Result<T, CoreError>;
