//! `tracing` setup: daily rotating files in `data/logs` (14 kept), `RUST_LOG`
//! honoured, and an optional stderr layer for warnings when not in the TUI.

use std::fs;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

use crate::error::{CoreError, Result};
use crate::home::Home;

/// Keeps the background log writer alive; drop it last in `main`.
pub struct LogGuard {
    _worker: Option<WorkerGuard>,
}

/// Number of daily log files kept in `data/logs`.
pub const LOG_FILES_KEPT: usize = 14;

/// Initialise logging. When the logs directory cannot be created (home not
/// initialised or read only) file logging is skipped and `Ok` is still
/// returned so read-only commands such as `doctor` keep working.
pub fn init(home: &Home, warnings_to_stderr: bool) -> Result<LogGuard> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let logs_dir = home.logs_dir();
    let file_layer = match fs::create_dir_all(&logs_dir) {
        Ok(()) => {
            let appender = RollingFileAppender::builder()
                .rotation(Rotation::DAILY)
                .max_log_files(LOG_FILES_KEPT)
                .filename_prefix("georag")
                .filename_suffix("log")
                .build(&logs_dir)
                .map_err(|e| CoreError::Logging(e.to_string()))?;
            let (writer, guard) = tracing_appender::non_blocking(appender);
            let layer = tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(writer);
            Some((layer, guard))
        }
        Err(_) => None,
    };

    let stderr_layer = warnings_to_stderr.then(|| {
        tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_target(false)
            .without_time()
            .with_filter(LevelFilter::WARN)
    });

    let (file_layer, worker) = match file_layer {
        Some((layer, guard)) => (Some(layer), Some(guard)),
        None => (None, None),
    };

    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(stderr_layer)
        .try_init()
        .map_err(|e| CoreError::Logging(e.to_string()))?;

    Ok(LogGuard { _worker: worker })
}
