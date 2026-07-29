use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::ws::{close_code, CloseFrame, Message, Utf8Bytes, WebSocket};
use bytes::Bytes;
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use tokio::time::{interval, MissedTickBehavior};

use crate::asr::{AsrEngine, AsrStream};
use crate::protocol::{ClientMessage, ServerMessage};
use crate::session::SessionGuard;

/// How often to probe liveness and re-check the idle deadline (R3).
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
/// Clamp bounds for a client-suggested idle timeout (R4).
const MIN_IDLE: Duration = Duration::from_secs(5);
const MAX_IDLE: Duration = Duration::from_secs(600);

fn clamp_idle(d: Duration) -> Duration {
    if d < MIN_IDLE {
        MIN_IDLE
    } else if d > MAX_IDLE {
        MAX_IDLE
    } else {
        d
    }
}

fn text_msg(msg: &ServerMessage) -> Message {
    Message::Text(Utf8Bytes::from(msg.to_json()))
}

fn close_frame(reason: &str) -> Message {
    Message::Close(Some(CloseFrame {
        code: close_code::NORMAL,
        reason: Utf8Bytes::from(reason.to_string()),
    }))
}

/// Build a `final` message from whatever the engine has decoded so far.
/// Returns `None` when there is no recognized text yet.
fn build_final(engine: &AsrEngine, stream: &mut AsrStream, segment: u32) -> Option<ServerMessage> {
    engine.flush(stream).map(|(text, tokens, timestamps)| ServerMessage::Final {
        text,
        segment,
        tokens,
        timestamps,
    })
}

/// Flush the current round, emit its `final`, reset the stream and bump the
/// segment counter. When `always_emit` is true an empty `final` is sent even
/// with no text (used on idle, R1) so the client never loses the round boundary.
async fn finalize_round(
    sender: &mut SplitSink<WebSocket, Message>,
    engine: &AsrEngine,
    stream: &mut Option<AsrStream>,
    segment: &mut u32,
    always_emit: bool,
) {
    let Some(s) = stream.as_mut() else {
        return;
    };
    let final_msg = build_final(engine, s, *segment);
    let to_send = if always_emit {
        Some(final_msg.unwrap_or_else(|| ServerMessage::Final {
            text: String::new(),
            segment: *segment,
            tokens: vec![],
            timestamps: vec![],
        }))
    } else {
        final_msg
    };
    if let Some(m) = to_send {
        let _ = sender.send(text_msg(&m)).await;
    }
    engine.reset(s);
    *segment += 1;
}

pub async fn handle_ws(ws: WebSocket, engine: Arc<AsrEngine>, _guard: SessionGuard, idle_default: Duration) {
    let (mut sender, mut receiver) = ws.split();

    if sender.send(text_msg(&ServerMessage::ready())).await.is_err() {
        return;
    }
    tracing::info!("session established");

    let mut stream: Option<AsrStream> = None;
    let mut segment: u32 = 0;
    let mut idle_timeout = idle_default;
    let mut last_activity = Instant::now();
    let mut heartbeat = interval(HEARTBEAT_INTERVAL);
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
    heartbeat.tick().await; // discard the immediate first tick

    loop {
        tokio::select! {
            biased;

            msg = receiver.next() => {
                last_activity = Instant::now();
                let msg = match msg {
                    None => {
                        tracing::debug!("client disconnected (EOF)");
                        break;
                    }
                    Some(Err(e)) => {
                        // R3: link tore down — try to deliver any buffered text,
                        // then close cleanly rather than dropping on the floor.
                        tracing::warn!("websocket error: {e}");
                        finalize_round(&mut sender, &engine, &mut stream, &mut segment, false).await;
                        let _ = sender.send(text_msg(&ServerMessage::error_connection("connection closed"))).await;
                        let _ = sender.send(close_frame("connection error")).await;
                        return; // already closed by us
                    }
                    Some(Ok(msg)) => msg,
                };

                match msg {
                    Message::Binary(data) => {
                        if stream.is_none() {
                            let _ = sender
                                .send(text_msg(&ServerMessage::error_protocol(
                                    "not listening, send start first",
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
                            let _ = sender.send(text_msg(&msg)).await;
                        }
                    }
                    Message::Text(text) => {
                        let cmd: ClientMessage = match serde_json::from_str(&text) {
                            Ok(cmd) => cmd,
                            Err(e) => {
                                let _ = sender
                                    .send(text_msg(&ServerMessage::error_protocol(format!(
                                        "invalid message: {e}"
                                    ))))
                                    .await;
                                continue;
                            }
                        };

                        match cmd {
                            ClientMessage::Start(cmd) => {
                                // R6: finalize any in-flight round before (re)starting.
                                finalize_round(&mut sender, &engine, &mut stream, &mut segment, false).await;

                                let sample_rate = cmd.sample_rate;
                                stream = Some(engine.create_stream(sample_rate));

                                // R4: adopt client-suggested idle, clamped; else default.
                                idle_timeout = cmd
                                    .idle_seconds
                                    .filter(|&s| s > 0.0)
                                    .map(|s| clamp_idle(Duration::from_secs_f64(s)))
                                    .unwrap_or(idle_default);
                                last_activity = Instant::now();

                                let _ = sender.send(text_msg(&ServerMessage::listening())).await;
                                tracing::info!(
                                    "listening: sample_rate={sample_rate} idle_timeout={}s segment={segment}",
                                    idle_timeout.as_secs()
                                );
                            }
                            ClientMessage::Finish => {
                                finalize_round(&mut sender, &engine, &mut stream, &mut segment, false).await;
                                let _ = sender.send(text_msg(&ServerMessage::ready())).await;
                                tracing::debug!("round finished, back to ready");
                            }
                            ClientMessage::Ping => {
                                let _ = sender.send(text_msg(&ServerMessage::Pong)).await;
                            }
                        }
                    }
                    Message::Ping(_) | Message::Pong(_) => {
                        // axum auto-replies to pings; counted as activity above.
                    }
                    Message::Close(_) => {
                        tracing::debug!("client sent close frame");
                        break;
                    }
                }
            }

            _ = heartbeat.tick() => {
                // R1: idle is a recoverable, business-level end of round.
                if last_activity.elapsed() >= idle_timeout {
                    tracing::info!("idle timeout after {}s, ending round", idle_timeout.as_secs());
                    finalize_round(&mut sender, &engine, &mut stream, &mut segment, true).await;
                    let _ = sender
                        .send(text_msg(&ServerMessage::error_idle("idle timeout")))
                        .await;
                    break;
                }
                // R3: probe half-open connections with a WS ping.
                let _ = sender.send(Message::Ping(Bytes::from_static(b"hb"))).await;
            }
        }
    }

    // Graceful close (R3): send a close frame instead of a bare TCP tear-down.
    let _ = sender.send(close_frame("bye")).await;
}
