use serde::{Deserialize, Serialize};

fn default_sample_rate() -> i32 {
    16000
}

#[derive(Debug, Deserialize)]
pub struct StartCommand {
    #[serde(default = "default_sample_rate")]
    pub sample_rate: i32,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    #[serde(rename = "start")]
    Start(StartCommand),
    #[serde(rename = "finish")]
    Finish,
    #[serde(rename = "ping")]
    Ping,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    #[serde(rename = "status")]
    Status { state: String },
    #[serde(rename = "partial")]
    Partial {
        text: String,
        segment: u32,
    },
    #[serde(rename = "final")]
    Final {
        text: String,
        segment: u32,
        tokens: Vec<String>,
        timestamps: Vec<f64>,
    },
    #[serde(rename = "error")]
    Error {
        message: String,
        fatal: bool,
    },
    #[serde(rename = "pong")]
    Pong,
}

impl ServerMessage {
    pub fn ready() -> Self {
        ServerMessage::Status {
            state: "ready".into(),
        }
    }

    pub fn listening() -> Self {
        ServerMessage::Status {
            state: "listening".into(),
        }
    }

    pub fn error_non_fatal(msg: impl Into<String>) -> Self {
        ServerMessage::Error {
            message: msg.into(),
            fatal: false,
        }
    }

    pub fn error_fatal(msg: impl Into<String>) -> Self {
        ServerMessage::Error {
            message: msg.into(),
            fatal: true,
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

#[derive(Debug, Serialize)]
pub struct BusyResponse {
    pub error: String,
    pub message: String,
}

impl BusyResponse {
    pub fn to_json() -> String {
        serde_json::to_string(&BusyResponse {
            error: "busy".into(),
            message: "All ASR slots occupied".into(),
        })
        .unwrap_or_default()
    }
}
