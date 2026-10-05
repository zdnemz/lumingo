//! The event stream: `GET /ws`.

use std::sync::Arc;

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use tokio::sync::broadcast::error::RecvError;

use crate::AppState;
use crate::types::ServerEvent;

/// Messages from the browser are never read as commands, so the limit is small.
const MAX_INCOMING_BYTES: usize = 1024;

pub async fn events(State(state): State<Arc<AppState>>, upgrade: WebSocketUpgrade) -> Response {
    upgrade
        .max_message_size(MAX_INCOMING_BYTES)
        .on_upgrade(move |socket| run(socket, state))
}

async fn run(mut socket: WebSocket, state: Arc<AppState>) {
    // Subscribe before taking the snapshot so no event falls between the two.
    let mut rx = state.hub.subscribe();
    let first = state.hub.snapshot_event();
    let mut last_seq = first.seq();
    if send(&mut socket, &first).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            () = state.shutdown.cancelled() => {
                let _ = socket.send(Message::Close(None)).await;
                return;
            }
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_)) | Err(_)) | None => return,
                Some(Ok(_)) => {}
            },
            event = rx.recv() => match event {
                Ok(event) if event.seq() > last_seq => {
                    last_seq = event.seq();
                    if send(&mut socket, &event).await.is_err() {
                        return;
                    }
                }
                Ok(_) => {}
                Err(RecvError::Lagged(_)) => {
                    let snapshot = state.hub.snapshot_event();
                    last_seq = snapshot.seq();
                    if send(&mut socket, &snapshot).await.is_err() {
                        return;
                    }
                }
                Err(RecvError::Closed) => return,
            },
        }
    }
}

async fn send(socket: &mut WebSocket, event: &ServerEvent) -> Result<(), ()> {
    let Ok(text) = serde_json::to_string(event) else {
        return Err(());
    };
    socket
        .send(Message::Text(text.into()))
        .await
        .map_err(|_| ())
}
