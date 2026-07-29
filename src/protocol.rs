use serde::{Deserialize, Serialize};

fn default_sample_rate() -> i32 {
    16000
}

/// Reason codes carried by every `error` frame (R2).
///
/// Fatal semantics (`fatal == true`): `connection`, `auth`, `internal`.
/// Recoverable (`fatal == false`): `idle`, `overload`, `protocol`.
pub mod error_code {
    pub const IDLE: &str = "idle";
    pub const CONNECTION: &str = "connection";
    pub const AUTH: &str = "auth";
    pub const OVERLOAD: &str = "overload";
    pub const PROTOCOL: &str = "protocol";
    /// Reserved per the protocol spec; emitted on server-internal failures.
    #[allow(dead_code)]
    pub const INTERNAL: &str = "internal";
}

#[derive(Debug, Deserialize)]
pub struct StartCommand {
    #[serde(default = "default_sample_rate")]
    pub sample_rate: i32,

    /// Client-suggested idle timeout in seconds for this round (R4).
    /// The server adopts it, clamped to a sane range.
    #[serde(default)]
    pub idle_seconds: Option<f64>,
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
        code: String,
        message: String,
        fatal: bool,
        retry: bool,
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

    /// Fully specified error frame.
    pub fn error(code: &str, message: impl Into<String>, fatal: bool, retry: bool) -> Self {
        ServerMessage::Error {
            code: code.into(),
            message: message.into(),
            fatal,
            retry,
        }
    }

    /// Recoverable client-protocol mistake (e.g. audio before `start`, bad JSON).
    /// Non-fatal, no retry needed.
    pub fn error_protocol(msg: impl Into<String>) -> Self {
        Self::error(error_code::PROTOCOL, msg, false, false)
    }

    /// Idle timeout — business-level end of round, recoverable (R1/R2).
    pub fn error_idle(msg: impl Into<String>) -> Self {
        Self::error(error_code::IDLE, msg, false, true)
    }

    /// Link-level break — fatal but the client may reconnect (R2).
    pub fn error_connection(msg: impl Into<String>) -> Self {
        Self::error(error_code::CONNECTION, msg, true, true)
    }

    /// Auth failure — fatal, do not retry without fixing the token (R5).
    /// Reserved: auth currently fails at the HTTP layer before WS upgrade.
    #[allow(dead_code)]
    pub fn error_auth(msg: impl Into<String>) -> Self {
        Self::error(error_code::AUTH, msg, true, false)
    }

    /// Server-internal failure — fatal (R2).
    #[allow(dead_code)]
    pub fn error_internal(msg: impl Into<String>) -> Self {
        Self::error(error_code::INTERNAL, msg, true, false)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// Structured body returned for HTTP-level rejections (503 busy / 401 auth).
/// Mirrors the WS `error` frame shape so clients share one parser (R2/R5).
#[derive(Debug, Serialize)]
pub struct HttpError {
    pub error: String,
    pub code: String,
    pub message: String,
    pub fatal: bool,
    pub retry: bool,
}

impl HttpError {
    /// All ASR slots occupied — recoverable, client should back off / retry (R2).
    pub fn busy() -> Self {
        Self::error(error_code::OVERLOAD, "All ASR slots occupied", false, true)
    }

    /// Bad or missing token — fatal, client must fix its token config (R5).
    pub fn auth() -> Self {
        Self::error(error_code::AUTH, "Unauthorized", true, false)
    }

    fn error(code: &str, message: impl Into<String>, fatal: bool, retry: bool) -> Self {
        HttpError {
            // Keep a short back-compat label for clients that keyed on the old field.
            error: code.into(),
            code: code.into(),
            message: message.into(),
            fatal,
            retry,
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}
