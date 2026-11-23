use std::time::Duration;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::{header, HeaderMap},
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;

use crate::{
    auth::decode_token,
    error::{AppError, AppResult},
    state::AppState,
    util::now_iso,
};

#[derive(Debug, Deserialize)]
pub(crate) struct WsQuery {
    token: Option<String>,
}

pub(crate) async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<WsQuery>,
) -> AppResult<Response> {
    let protocol_token = headers
        .get(header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value
                .split(',')
                .map(str::trim)
                .find_map(|item| item.strip_prefix("bearer."))
        });
    let token = protocol_token
        .map(ToString::to_string)
        .or_else(|| {
            if state.config.production {
                None
            } else {
                query.token.filter(|value| !value.is_empty())
            }
        })
        .ok_or_else(|| AppError::unauthorized("缺少 WebSocket 访问令牌"))?;
    let claims = decode_token(&state.jwt, &token)?;
    let user = claims.username;
    let mut receiver = state.bus.subscribe();
    let metrics = state.metrics.clone();
    let initial = serde_json::json!({
        "type": "ready",
        "username": user,
        "server_time": now_iso()
    });

    Ok(ws.on_upgrade(move |socket| async move {
        use std::sync::atomic::Ordering;
        metrics.ws_connections.fetch_add(1, Ordering::Relaxed);
        let _ = handle_socket(socket, &mut receiver, initial).await;
        metrics.ws_connections.fetch_sub(1, Ordering::Relaxed);
    }))
}

async fn handle_socket(
    socket: WebSocket,
    receiver: &mut tokio::sync::broadcast::Receiver<crate::realtime::RealtimeEvent>,
    initial: serde_json::Value,
) -> Result<(), ()> {
    let (mut sender, mut incoming) = socket.split();
    if sender
        .send(Message::Text(initial.to_string().into()))
        .await
        .is_err()
    {
        return Ok(());
    }

    let mut ping = tokio::time::interval(Duration::from_secs(20));
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            event = receiver.recv() => {
                match event {
                    Ok(event) => {
                        let Ok(text) = serde_json::to_string(&event) else { continue };
                        if sender.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        let resync = serde_json::json!({
                            "type": "resync",
                            "skipped": skipped
                        });
                        if sender.send(Message::Text(resync.to_string().into())).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            message = incoming.next() => {
                match message {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(payload))) => {
                        if sender.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Pong(_))) | Some(Ok(Message::Text(_))) | Some(Ok(Message::Binary(_))) => {}
                    Some(Err(_)) => break,
                }
            }
            _ = ping.tick() => {
                if sender.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
        }
    }
    Ok(())
}
