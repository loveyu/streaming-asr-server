mod asr;
mod auth;
mod config;
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
struct AppState {
    engine: Arc<AsrEngine>,
    session_manager: Arc<SessionManager>,
    auth_token: Option<String>,
}

async fn ws_handler(
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

    let _guard = match state.session_manager.try_acquire() {
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

    ws.on_upgrade(move |socket| ws_handler::handle_ws(socket, state.engine.clone()))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = Config::parse();

    if (config.tls_cert.is_some()) != (config.tls_key.is_some()) {
        anyhow::bail!("Both --tls-cert and --tls-key must be provided together");
    }

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
