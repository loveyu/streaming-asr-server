use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Clone, Debug)]
#[command(name = "asr-server", version, about = "Remote ASR WebSocket server based on Sherpa-ONNX")]
pub struct Config {
    #[arg(long, default_value = "0.0.0.0:6008")]
    pub bind: SocketAddr,

    #[arg(long)]
    pub tls_cert: Option<PathBuf>,

    #[arg(long)]
    pub tls_key: Option<PathBuf>,

    #[arg(long)]
    pub model: PathBuf,

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
}
