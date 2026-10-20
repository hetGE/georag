use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("could not open the index database {path} ({source}): check that data/ is writable")]
    Open {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },
    #[error("database error ({0})")]
    Sqlite(#[from] rusqlite::Error),
    #[error(
        "the vector table was created for {existing} dimensions but the embedding model produces {requested}: re-embed the library after changing the embedding model"
    )]
    DimensionMismatch { existing: usize, requested: usize },
    #[error("the vector table does not exist yet: set an embedding model first")]
    NoVectorTable,
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error("invalid stored JSON ({0})")]
    Json(#[from] serde_json::Error),
}
