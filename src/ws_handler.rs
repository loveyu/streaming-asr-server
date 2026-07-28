use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use futures_util::{SinkExt, StreamExt};
use tokio::time::timeout;

use crate::asr::{AsrEngine, AsrStream};
use crate::protocol::{ClientMessage, ServerMessage};

const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

pub async fn handle_ws(ws: WebSocket, engine: Arc<AsrEngine>) {
    let (mut sender, mut receiver) = ws.split();

    if sender
        .send(Message::Text(Utf8Bytes::from(ServerMessage::ready().to_json())))
        .await
        .is_err()
    {
        return;
    }

    let mut stream: Option<AsrStream> = None;
    let mut segment: u32 = 0;

    loop {
        let msg = match timeout(IDLE_TIMEOUT, receiver.next()).await {
            Ok(Some(Ok(msg))) => msg,
            Ok(Some(Err(e))) => {
                tracing::warn!("WebSocket error: {e}");
                break;
            }
            Ok(None) => {
                tracing::debug!("Client disconnected");
                break;
            }
            Err(_) => {
                tracing::debug!("Idle timeout, closing connection");
                let _ = sender
                    .send(Message::Text(Utf8Bytes::from(
                        ServerMessage::error_fatal("idle timeout").to_json(),
                    )))
                    .await;
                break;
            }
        };

        match msg {
            Message::Binary(data) => {
                if stream.is_none() {
                    let _ = sender
                        .send(Message::Text(Utf8Bytes::from(
                            ServerMessage::error_non_fatal("not listening, send start first")
                                .to_json(),
                        )))
                        .await;
                    continue;
                }

                let samples: &[i16] = unsafe {
                    std::slice::from_raw_parts(
                        data.as_ptr() as *const i16,
                        data.len() / std::mem::size_of::<i16>(),
                    )
                };

                if let Some(partial_text) =
                    engine.accept_waveform(stream.as_mut().unwrap(), samples)
                {
                    let msg = ServerMessage::Partial {
                        text: partial_text,
                        segment,
                    };
                    let _ = sender
                        .send(Message::Text(Utf8Bytes::from(msg.to_json())))
                        .await;
                }
            }
            Message::Text(text) => {
                let cmd: ClientMessage = match serde_json::from_str(&text) {
                    Ok(cmd) => cmd,
                    Err(e) => {
                        let _ = sender
                            .send(Message::Text(Utf8Bytes::from(
                                ServerMessage::error_non_fatal(format!(
                                    "invalid message: {e}"
                                ))
                                .to_json(),
                            )))
                            .await;
                        continue;
                    }
                };

                match cmd {
                    ClientMessage::Start => {
                        stream = Some(engine.create_stream());
                        segment = 0;
                        let _ = sender
                            .send(Message::Text(Utf8Bytes::from(
                                ServerMessage::listening().to_json(),
                            )))
                            .await;
                    }
                    ClientMessage::Finish => {
                        if let Some(ref mut s) = stream {
                            if let Some((text, tokens, timestamps)) = engine.flush(s) {
                                let msg = ServerMessage::Final {
                                    text,
                                    segment,
                                    tokens,
                                    timestamps,
                                };
                                let _ = sender
                                    .send(Message::Text(Utf8Bytes::from(msg.to_json())))
                                    .await;
                            }
                            engine.reset(s);
                            segment += 1;
                        }
                        let _ = sender
                            .send(Message::Text(Utf8Bytes::from(
                                ServerMessage::ready().to_json(),
                            )))
                            .await;
                    }
                    ClientMessage::Ping => {
                        let _ = sender
                            .send(Message::Text(Utf8Bytes::from(
                                ServerMessage::Pong.to_json(),
                            )))
                            .await;
                    }
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
}
