mod asr;
mod auth;
mod config;
mod model;
mod protocol;
mod session;
mod ws_handler;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{ConnectInfo, WebSocketUpgrade};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use clap::Parser;

use crate::asr::AsrEngine;
use crate::config::Config;
use crate::protocol::BusyResponse;
use crate::session::SessionManager;

#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<AsrEngine>,
    pub session_manager: Arc<SessionManager>,
    pub auth_token: Option<String>,
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> Response {
    if let Some(ref expected_token) = state.auth_token {
        if !auth::verify_token(&headers, expected_token) {
            tracing::warn!("Auth failed from {addr}");
            return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
        }
    }

    let guard = match state.session_manager.try_acquire() {
        Some(guard) => guard,
        None => {
            tracing::warn!("No available slots for {addr}");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                [(header::CONTENT_TYPE, "application/json")],
                BusyResponse::to_json(),
            )
                .into_response();
        }
    };

    tracing::info!("WS connection from {addr}");

    ws.on_upgrade(move |socket| ws_handler::handle_ws(socket, state.engine.clone(), guard))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let mut config = Config::parse();
    config.canonicalize()?;

    if (config.tls_cert.is_some()) != (config.tls_key.is_some()) {
        anyhow::bail!("Both --tls-cert and --tls-key must be provided together");
    }

    model::ensure(&config.model_dir, &config.resolved_model_url).await?;

    let engine = Arc::new(AsrEngine::new(config.clone())?);
    let session_manager = Arc::new(SessionManager::new(config.max_sessions));

    let state = Arc::new(AppState {
        engine,
        session_manager,
        auth_token: config.auth_token.clone(),
    });

    let app = Router::new()
        .route("/", get(ws_handler))
        .with_state(state);

    let bind_addr = config.bind;
    let use_tls = config.tls_cert.is_some();

    tracing::info!(
        "Starting ASR server on {} (TLS: {})",
        bind_addr,
        use_tls
    );

    if let (Some(cert), Some(key)) = (&config.tls_cert, &config.tls_key) {
        let rustls_config = axum_server::tls_rustls::RustlsConfig::from_pem_file(cert, key)
            .await?;

        axum_server::bind_rustls(bind_addr, rustls_config)
            .serve(app.into_make_service_with_connect_info::<SocketAddr>())
            .await?;
    } else {
        let listener = tokio::net::TcpListener::bind(bind_addr).await?;
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::path::PathBuf;
    use std::sync::Arc;

    use axum::Router;
    use futures_util::{SinkExt, StreamExt};
    use tokio::net::TcpListener;
    use tokio::time::{timeout, Duration};
    use tokio_tungstenite::connect_async;
    use tokio_tungstenite::tungstenite::Message;

    use super::*;
    use crate::asr::AsrEngine;
    use crate::config::Config;

    fn dummy_config() -> Config {
        let model_dir = std::env::var("HOME")
            .map(|h| PathBuf::from(h).join(".cache/asr-server/models"))
            .unwrap_or_else(|_| "/tmp/asr-test-model".into());
        Config {
            bind: "0.0.0.0:0".parse().unwrap(),
            tls_cert: None,
            tls_key: None,
            model_url: None,
            resolved_model_url: String::new(),
            model: None,
            model_dir,
            auth_token: None,
            max_sessions: 2,
            num_threads: Some(4),
            decoding_method: "greedy_search".into(),
            max_active_paths: 4,
            endpoint_silence: 1.2,
            endpoint_max_utterance: 20.0,
            sample_rate: 16000,
        }
    }

    async fn test_server() -> (SocketAddr, tokio::task::JoinHandle<()>) {
        let config = dummy_config();
        let engine = Arc::new(AsrEngine::new(config.clone()).unwrap());
        let session_manager = Arc::new(SessionManager::new(config.max_sessions));

        let state = Arc::new(AppState {
            engine,
            session_manager,
            auth_token: None,
        });

        let app = Router::new()
            .route("/", axum::routing::get(ws_handler))
            .with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        (addr, handle)
    }

    fn field(json: &str, name: &str) -> String {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        v[name].as_str().map(|s| s.to_string()).unwrap_or_default()
    }

    #[tokio::test]
    async fn connect_receives_ready() {
        let (addr, _handle) = test_server().await;
        let (ws, _) = connect_async(format!("ws://{addr}/")).await.unwrap();
        let (_, mut read) = ws.split();

        let msg = read.next().await.unwrap().unwrap();
        let text = msg.into_text().unwrap();
        assert_eq!(field(&text, "type"), "status");
        assert_eq!(field(&text, "state"), "ready");
    }

    #[tokio::test]
    async fn start_transitions_to_listening() {
        let (addr, _handle) = test_server().await;
        let (ws, _) = connect_async(format!("ws://{addr}/")).await.unwrap();
        let (mut write, mut read) = ws.split();

        // consume ready
        read.next().await.unwrap().unwrap();

        write.send(Message::Text(r#"{"type":"start"}"#.into())).await.unwrap();

        let msg = read.next().await.unwrap().unwrap();
        let text = msg.into_text().unwrap();
        assert_eq!(field(&text, "type"), "status");
        assert_eq!(field(&text, "state"), "listening");
    }

    #[tokio::test]
    async fn ping_responds_pong() {
        let (addr, _handle) = test_server().await;
        let (ws, _) = connect_async(format!("ws://{addr}/")).await.unwrap();
        let (mut write, mut read) = ws.split();

        read.next().await.unwrap().unwrap(); // ready

        write.send(Message::Text(r#"{"type":"ping"}"#.into())).await.unwrap();

        let msg = read.next().await.unwrap().unwrap();
        let text = msg.into_text().unwrap();
        assert_eq!(field(&text, "type"), "pong");
    }

    #[tokio::test]
    async fn finish_returns_to_ready() {
        let (addr, _handle) = test_server().await;
        let (ws, _) = connect_async(format!("ws://{addr}/")).await.unwrap();
        let (mut write, mut read) = ws.split();

        read.next().await.unwrap().unwrap(); // ready

        write.send(Message::Text(r#"{"type":"start"}"#.into())).await.unwrap();
        read.next().await.unwrap().unwrap(); // listening

        write.send(Message::Text(r#"{"type":"finish"}"#.into())).await.unwrap();

        // drain messages until ready (may include final result from real engine)
        let final_state = loop {
            let msg = timeout(Duration::from_secs(5), read.next()).await;
            match msg {
                Ok(Some(Ok(msg))) => {
                    let text = msg.into_text().unwrap();
                    let t = field(&text, "type");
                    let s = field(&text, "state");
                    if t == "status" && s == "ready" {
                        break Some(s);
                    }
                }
                _ => break None,
            }
        };
        assert_eq!(final_state.as_deref(), Some("ready"), "should return to ready");
    }

    #[tokio::test]
    async fn binary_before_start_errors() {
        let (addr, _handle) = test_server().await;
        let (ws, _) = connect_async(format!("ws://{addr}/")).await.unwrap();
        let (mut write, mut read) = ws.split();

        read.next().await.unwrap().unwrap(); // ready

        // send audio without start
        let pcm: Vec<u8> = vec![0; 3200];
        write.send(Message::Binary(pcm.into())).await.unwrap();

        let msg = read.next().await.unwrap().unwrap();
        let text = msg.into_text().unwrap();
        assert_eq!(field(&text, "type"), "error");
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(!v["fatal"].as_bool().unwrap_or(true));
    }

    #[tokio::test]
    async fn invalid_json_errors() {
        let (addr, _handle) = test_server().await;
        let (ws, _) = connect_async(format!("ws://{addr}/")).await.unwrap();
        let (mut write, mut read) = ws.split();

        read.next().await.unwrap().unwrap(); // ready

        write.send(Message::Text("bad json".into())).await.unwrap();

        let msg = read.next().await.unwrap().unwrap();
        let text = msg.into_text().unwrap();
        assert_eq!(field(&text, "type"), "error");
    }

    #[tokio::test]
    async fn slot_limit_rejects() {
        let config = dummy_config();
        let engine = Arc::new(AsrEngine::new(config.clone()).unwrap());
        let session_manager = Arc::new(SessionManager::new(1)); // only 1 slot

        let state = Arc::new(AppState {
            engine,
            session_manager,
            auth_token: None,
        });

        let app = Router::new()
            .route("/", axum::routing::get(ws_handler))
            .with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });

        let (ws1, _) = connect_async(format!("ws://{addr}/")).await.unwrap();

        let result = connect_async(format!("ws://{addr}/")).await;
        assert!(result.is_err(), "second connection should be rejected");

        drop(ws1);
    }

    #[tokio::test]
    async fn auth_rejects_wrong_token() {
        let config = dummy_config();
        let engine = Arc::new(AsrEngine::new(config.clone()).unwrap());
        let session_manager = Arc::new(SessionManager::new(2));

        let state = Arc::new(AppState {
            engine,
            session_manager,
            auth_token: Some("secret".into()),
        });

        let app = Router::new()
            .route("/", axum::routing::get(ws_handler))
            .with_state(state);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });

        let result = connect_async(format!("ws://{addr}/")).await;
        assert!(result.is_err(), "unauthenticated should be rejected");
    }

    #[tokio::test]
    async fn audio_after_start_accepted() {
        let (addr, _handle) = test_server().await;
        let (ws, _) = connect_async(format!("ws://{addr}/")).await.unwrap();
        let (mut write, mut read) = ws.split();

        read.next().await.unwrap().unwrap(); // ready

        write.send(Message::Text(r#"{"type":"start"}"#.into())).await.unwrap();
        read.next().await.unwrap().unwrap(); // listening

        // send 100ms PCM 16kHz mono 16bit LE (3200 bytes)
        let pcm: Vec<u8> = vec![0; 3200];
        write.send(Message::Binary(pcm.into())).await.unwrap();

        // send 50ms more
        let pcm: Vec<u8> = vec![0; 1600];
        write.send(Message::Binary(pcm.into())).await.unwrap();

        // no error should arrive; confirm with a short timeout
        let result = timeout(Duration::from_millis(200), read.next()).await;
        match result {
            Ok(Some(Ok(msg))) => {
                // placeholder engine returns nothing, so no message expected
                // but if we get something, it must not be an error
                if let Ok(text) = msg.into_text() {
                    assert_ne!(field(&text, "type"), "error");
                }
            }
            Ok(Some(Err(_))) => panic!("unexpected WS error"),
            Ok(None) | Err(_) => {} // timeout or close is fine
        }
    }

    #[tokio::test]
    async fn audio_real_file_pipeline() {
        let (addr, _handle) = test_server().await;
        let (ws, _) = connect_async(format!("ws://{addr}/")).await.unwrap();
        let (mut write, mut read) = ws.split();

        read.next().await.unwrap().unwrap(); // ready

        write.send(Message::Text(r#"{"type":"start"}"#.into())).await.unwrap();
        let msg = read.next().await.unwrap().unwrap();
        assert_eq!(field(&msg.into_text().unwrap(), "state"), "listening");

        let pcm_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/zh-test.pcm");
        if !pcm_path.exists() {
            eprintln!("Skipping: no test_audio.pcm (place next to project root)");
            return;
        }
        let pcm_data = tokio::fs::read(&pcm_path).await.unwrap();

        const CHUNK: usize = 3200; // 100ms
        for chunk in pcm_data.chunks(CHUNK) {
            write.send(Message::Binary(chunk.to_vec().into())).await.unwrap();
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        write.send(Message::Text(r#"{"type":"finish"}"#.into())).await.unwrap();

        let final_state = loop {
            let msg = timeout(Duration::from_secs(2), read.next()).await;
            match msg {
                Ok(Some(Ok(msg))) => {
                    let text = msg.into_text().unwrap();
                    let t = field(&text, "type");
                    let s = field(&text, "state");
                    if t == "status" && s == "ready" {
                        break Some(s);
                    }
                    eprintln!("  <- {text}");
                }
                Ok(Some(Err(_))) => break None,
                Ok(None) | Err(_) => break None,
            }
        };
        assert_eq!(final_state.as_deref(), Some("ready"), "should return to ready after finish");
    }
}
