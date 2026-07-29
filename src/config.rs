use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use clap::{Parser, ValueEnum};

use crate::model;

/// Log verbosity. Mapped to a `tracing` level. Default is `info`.
#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            LogLevel::Trace => "trace",
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    }
}

#[derive(Parser, Clone, Debug)]
#[command(name = "asr-server", version, about = "Remote ASR WebSocket server based on Sherpa-ONNX")]
pub struct Config {
    #[arg(long, default_value = "0.0.0.0:6008")]
    pub bind: SocketAddr,

    #[arg(long)]
    pub tls_cert: Option<PathBuf>,

    #[arg(long)]
    pub tls_key: Option<PathBuf>,

    #[arg(long, value_name = "DIR")]
    pub model: Option<PathBuf>,

    #[arg(long)]
    pub model_url: Option<String>,

    #[arg(long)]
    pub auth_token: Option<String>,

    #[arg(long, default_value = "2")]
    pub max_sessions: usize,

    #[arg(long)]
    pub num_threads: Option<i32>,

    #[arg(long, default_value = "greedy_search")]
    pub decoding_method: String,

    #[arg(long, default_value = "4")]
    pub max_active_paths: i32,

    #[arg(long, default_value = "1.2")]
    pub endpoint_silence: f32,

    #[arg(long, default_value = "20.0")]
    pub endpoint_max_utterance: f32,

    #[arg(long, default_value = "16000")]
    pub sample_rate: i32,

    /// Per-session idle timeout in seconds. A client may override per round via
    /// `start.idle_seconds`. Should be >= the client's local endpoint silence
    /// (R4). [default: 60]
    #[arg(long, default_value = "60.0")]
    pub idle_timeout: f64,

    /// Log verbosity. Also set via `$ASR_LOG` / `$RUST_LOG`. [default: info]
    #[arg(long, value_enum)]
    pub log_level: Option<LogLevel>,

    /// Log file path or directory. Defaults to the system log folder. Also set
    /// via `$ASR_LOG_FILE`.
    #[arg(long, value_name = "PATH")]
    pub log_file: Option<PathBuf>,

    /// Disable file logging; write to stderr only.
    #[arg(long, default_value_t = false)]
    pub no_log_file: bool,

    #[arg(skip)]
    pub resolved_model_url: String,

    #[arg(skip)]
    pub model_dir: PathBuf,
}

impl Config {
    pub fn canonicalize(&mut self) -> anyhow::Result<()> {
        self.resolved_model_url = model::resolve_model_url(self.model_url.as_deref());
        self.model_dir = match self.model.take() {
            Some(p) => model::canonicalize(p)?,
            None => {
                let home = std::env::var("HOME")
                    .context("Cannot determine home directory; set --model or $HOME")?;
                PathBuf::from(home).join(".cache/asr-server/models")
            }
        };
        if let Some(ref cert) = self.tls_cert {
            self.tls_cert = Some(model::canonicalize(cert)?);
        }
        if let Some(ref key) = self.tls_key {
            self.tls_key = Some(model::canonicalize(key)?);
        }
        Ok(())
    }

    /// Resolved default idle timeout as a `Duration`.
    pub fn idle_timeout(&self) -> Duration {
        Duration::from_secs_f64(self.idle_timeout.max(1.0))
    }
}
