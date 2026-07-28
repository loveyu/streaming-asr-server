use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;

use crate::model;

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
}
