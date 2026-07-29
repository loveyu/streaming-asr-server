//! Logging initialization.
//!
//! - **Level**: resolved from `--log-level` > `$ASR_LOG` > `$RUST_LOG` > `info`.
//!   Accepts a bare level (`debug`, `info`, `warn`, `error`, `trace`) or a full
//!   `tracing` directive (e.g. `streaming_asr_server=debug,sherpa_onnx=info`).
//! - **File**: writes to a single append-only file (`--log-file` / `$ASR_LOG_FILE`
//!   is used verbatim). Defaults to the system log folder, with a graceful
//!   fallback to the user state dir when not writable. Pass `--no-log-file` to
//!   disable file output and log to stderr only. For long-running deployments,
//!   point `--log-file` at a logrotate-managed path for size-based rotation.
//!
//! File writes are synchronous (not buffered behind a background worker) so
//! that logs survive even a non-graceful process exit such as `SIGTERM`/`SIGKILL`
//! — each event is handed to the kernel page cache immediately. The log volume
//! of this server is low enough that blocking writes are not a concern.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use crate::config::LogLevel;

const DEFAULT_LOG_FILENAME: &str = "asr-server.log";

/// Resolve the effective log filter directive.
///
/// Priority: explicit CLI level → `$ASR_LOG` → `$RUST_LOG` → `info`.
pub fn resolve_level(cli: Option<LogLevel>) -> String {
    if let Some(level) = cli {
        return level.as_str().to_string();
    }
    for var in ["ASR_LOG", "RUST_LOG"] {
        if let Ok(val) = std::env::var(var) {
            let val = val.trim();
            if !val.is_empty() {
                return val.to_string();
            }
        }
    }
    "info".to_string()
}

/// Decide the log file location, honoring an explicit override and falling back
/// through writable candidates. Returns `None` when file logging is disabled or
/// no candidate is writable.
fn resolve_log_file(cli_file: Option<PathBuf>, disable_file: bool) -> Option<PathBuf> {
    if disable_file {
        return None;
    }

    // Explicit override (CLI or $ASR_LOG_FILE): trust the caller as-is.
    if let Some(path) = cli_file.or_else(env_log_file) {
        return Some(path);
    }

    // Default: prefer the system log folder, fall back to a per-user dir.
    for candidate in default_candidates() {
        if let Some(parent) = candidate.parent() {
            if std::fs::create_dir_all(parent).is_ok() && is_writable_dir(parent) {
                return Some(candidate);
            }
        }
    }
    None
}

fn env_log_file() -> Option<PathBuf> {
    std::env::var("ASR_LOG_FILE")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

/// Candidate default locations, most-preferred first.
fn default_candidates() -> Vec<PathBuf> {
    let mut v = vec![PathBuf::from("/var/log/asr-server").join(DEFAULT_LOG_FILENAME)];

    // Per-user state dir (XDG_STATE_HOME / ~/.local/state).
    if let Some(state) = dirs::state_dir() {
        v.push(state.join("asr-server").join(DEFAULT_LOG_FILENAME));
    }

    // Last-resort temp location.
    v.push(std::env::temp_dir().join("asr-server").join(DEFAULT_LOG_FILENAME));
    v
}

/// Probe whether a directory is writable by creating (and removing) a temp file.
fn is_writable_dir(dir: &Path) -> bool {
    let tmp = dir.join(".asr-write-probe");
    let ok = std::fs::File::create(&tmp).is_ok();
    if ok {
        let _ = std::fs::remove_file(&tmp);
    }
    ok
}

/// Initialize the global tracing subscriber.
///
/// Logs to stderr always; additionally writes a daily-rotated file when a path
/// is resolved.
pub fn init(level: Option<LogLevel>, cli_file: Option<PathBuf>, disable_file: bool) -> Result<()> {
    let level_str = resolve_level(level);
    let filter = EnvFilter::try_new(&level_str)
        .with_context(|| format!("invalid log level/directive: {level_str}"))?;

    let stderr_layer = fmt::layer().with_writer(std::io::stderr);

    let log_file = resolve_log_file(cli_file, disable_file);
    if let Some(path) = log_file {
        let (dir, file_name) = split_file_target(&path);
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("cannot create log directory {}", dir.display()))?;
        let appender = tracing_appender::rolling::never(&dir, &file_name);
        let file_layer = fmt::layer().with_ansi(false).with_writer(appender);

        tracing_subscriber::registry()
            .with(filter)
            .with(stderr_layer)
            .with(file_layer)
            .init();

        tracing::info!("Logging to file: {}", dir.join(&file_name).display());
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(stderr_layer)
            .init();
        tracing::info!("File logging disabled; stderr only");
    }

    Ok(())
}

/// Split an absolute target path into (parent_dir, file_name).
/// A bare filename becomes (".", filename); a directory becomes itself + the
/// default filename.
fn split_file_target(path: &Path) -> (PathBuf, String) {
    if path.is_dir() {
        return (path.to_path_buf(), DEFAULT_LOG_FILENAME.to_string());
    }
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => (p.to_path_buf(), file_name_of(path)),
        _ => (PathBuf::from("."), file_name_of(path)),
    }
}

fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| DEFAULT_LOG_FILENAME.to_string())
}
