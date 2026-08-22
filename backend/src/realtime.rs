//! Additive websocket push layer. An in-process broadcast hub (in `AppState`) carries small
//! "something changed" nudges — a topic string — to every connected client, which then refetches the
//! matching data through the normal REST API. REST stays the single source of truth; the socket only
//! lowers latency versus polling, and if it drops the app degrades cleanly to the existing polls.

use std::time::Duration;

use axum::{
    Extension,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use tokio::sync::broadcast;

use crate::{auth::context::CurrentUser, state::AppState};

/// A realtime nudge. The client maps `topic` to the React Query keys to invalidate + refetch.
#[derive(Debug, Clone, Serialize)]
pub struct WsEvent {
    pub topic: String,
}

pub type Events = broadcast::Sender<WsEvent>;

/// Topic strings — kept in sync with the frontend invalidation map (`web/src/lib/realtime.ts`).
pub mod topic {
    pub const RELEASE: &str = "flow.release";
    pub const FCA: &str = "flow.fca";
    pub const GDP: &str = "tmu.gdp";
    pub const TMI: &str = "tmu.tmi";
    pub const GROUND_STOP: &str = "tmu.groundstop";
    pub const PROGRAM: &str = "tmu.program";
    pub const CFR: &str = "flow.cfr";
}

/// `GET /api/v1/ws` — upgrade to a websocket that streams realtime nudges. Requires an authenticated
/// session; the `ois_session` cookie rides the upgrade GET, so the router's auth middleware populates
/// `current_user` just like a REST handler.
pub async fn ws(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Extension(current_user): Extension<Option<CurrentUser>>,
) -> Response {
    if current_user.is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let rx = state.events.subscribe();
    ws.on_upgrade(move |socket| pump(socket, rx))
        .into_response()
}

/// Forward broadcast events to the client, answer pings, and send a keepalive ping so idle
/// connections survive proxy idle-timeouts. Exits when the client closes or errors.
async fn pump(mut socket: WebSocket, mut rx: broadcast::Receiver<WsEvent>) {
    let mut keepalive = tokio::time::interval(Duration::from_secs(30));
    loop {
        tokio::select! {
            ev = rx.recv() => match ev {
                Ok(ev) => {
                    let text = serde_json::to_string(&ev).unwrap_or_default();
                    if socket.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                // Fell behind the broadcast buffer — the next event still triggers a refetch, and
                // polling is the safety net, so just keep going.
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            },
            msg = socket.recv() => match msg {
                Some(Ok(Message::Ping(p))) => {
                    if socket.send(Message::Pong(p)).await.is_err() {
                        break;
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                _ => {}
            },
            _ = keepalive.tick() => {
                if socket.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::state::AppState;

    #[tokio::test]
    async fn publish_reaches_subscribers() {
        let state = AppState::without_db();
        let mut rx = state.events.subscribe();
        state.publish(super::topic::RELEASE);
        let ev = rx.recv().await.expect("event delivered");
        assert_eq!(ev.topic, "flow.release");
    }

    #[tokio::test]
    async fn publish_with_no_subscribers_is_a_noop() {
        // send() errors when nobody is listening; publish() must swallow it (no panic).
        AppState::without_db().publish(super::topic::GDP);
    }
}
