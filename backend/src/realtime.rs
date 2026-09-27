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
    pub const EVENT_AVAILABILITY: &str = "events.availability";
    /// Someone's access changed. Payload-free like every topic here, so each client refetches its
    /// own `/me` and works out whether anything it holds actually grew — the socket is broadcast to
    /// everyone, so it must never carry who was granted what (#348).
    pub const ACCESS_GRANTED: &str = "access.granted";
    /// An ACE claim reminder came due. Mirrors the Discord DM the scheduler already sends, so the
    /// desktop app is a second delivery channel for the same decision (#348).
    pub const EVENT_REMINDER: &str = "events.reminder";
}

/// The subprotocol a desktop client offers beside its token, and the only one ever selected.
pub const WS_PROTOCOL: &str = "ois.v1";

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

    // A desktop client authenticates by offering its token as a subprotocol, beside the fixed
    // `ois.v1` marker (see `auth::middleware`). The handshake only completes if the server echoes
    // one offered protocol back — otherwise the browser drops the connection straight after we
    // accept it — so select the marker. Never the token: echoing it would put the credential in
    // the response headers as well. A web client offers nothing, and nothing is selected.
    let rx = state.events.subscribe();
    ws.protocols([WS_PROTOCOL])
        .on_upgrade(move |socket| pump(socket, rx))
        .into_response()
}

/// Forward broadcast events to the client, answer pings, and send a keepalive ping so idle
/// connections survive proxy idle-timeouts. Exits when the client closes or errors.
// The Ping arm's `if send(Pong(p)).await.is_err()` can't collapse into a match guard — guards can't
// `await`, and it would move `p` out of the pattern.
#[allow(clippy::collapsible_match)]
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

/// The desktop app authenticates the socket through the subprotocol list — the only header a
/// browser `WebSocket` can set. Driven through the real router on a real port, because the
/// handshake, the auth middleware and the route all have to agree (VATUSA/OIS#348 review).
#[cfg(test)]
mod handshake_tests {
    use sqlx::PgPool;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use crate::state::AppState;

    const DESKTOP: &str = "ois_dsk_handshake_test";
    const WEB: &str = "web_handshake_test";

    async fn serve(pool: PgPool) -> std::net::SocketAddr {
        sqlx::query(
            "insert into identity.users (id, full_name, display_name, cid, rating, email) \
             values ('ws-user', 'WS User', 'WS User', 9900001, 5, 'ws@example.test')",
        )
        .execute(&pool)
        .await
        .unwrap();
        for (token, kind) in [(DESKTOP, "desktop"), (WEB, "web")] {
            sqlx::query(
                "insert into identity.sessions (session_token, user_id, expires_at, kind) \
                 values ($1, 'ws-user', now() + interval '1 hour', $2)",
            )
            .bind(token)
            .bind(kind)
            .execute(&pool)
            .await
            .unwrap();
        }
        sqlx::query(
            "insert into access.user_permissions (user_id, permission_name, granted) \
             values ('ws-user', 'ace.requests.claim', true)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let state = AppState {
            db: Some(pool),
            ..AppState::without_db()
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, crate::router::build_router(state))
                .await
                .unwrap();
        });
        addr
    }

    /// Sends one raw HTTP/1.1 request and returns the response head, lowercased.
    async fn request(addr: std::net::SocketAddr, head: &str) -> String {
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        stream.write_all(head.as_bytes()).await.unwrap();
        let mut buf = vec![0u8; 4096];
        let mut read = 0;
        while !buf[..read].windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut buf[read..]).await.unwrap();
            if n == 0 {
                break;
            }
            read += n;
        }
        String::from_utf8_lossy(&buf[..read]).to_lowercase()
    }

    async fn upgrade(addr: std::net::SocketAddr, protocols: Option<&str>) -> String {
        let offer = protocols
            .map(|p| format!("Sec-WebSocket-Protocol: {p}\r\n"))
            .unwrap_or_default();
        request(
            addr,
            &format!(
                "GET /api/v1/ws HTTP/1.1\r\nHost: {addr}\r\nConnection: Upgrade\r\n\
                 Upgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\
                 Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n{offer}\r\n"
            ),
        )
        .await
    }

    #[sqlx::test]
    async fn a_desktop_token_offered_beside_the_marker_opens_the_socket(pool: PgPool) {
        let addr = serve(pool).await;

        let head = upgrade(addr, Some(&format!("ois.v1, ois.bearer.{DESKTOP}"))).await;

        assert!(head.starts_with("http/1.1 101"), "{head}");
        // The marker is what completes the handshake; the token must not come back with it.
        assert!(
            head.contains("sec-websocket-protocol: ois.v1\r\n"),
            "{head}"
        );
        assert!(!head.contains(DESKTOP), "the token was echoed: {head}");
    }

    #[sqlx::test]
    async fn anything_but_a_live_desktop_token_is_refused(pool: PgPool) {
        let addr = serve(pool).await;

        for offer in [
            // Nothing at all.
            None,
            // A desktop-shaped token with no session behind it.
            Some("ois.v1, ois.bearer.ois_dsk_not_a_real_session".to_owned()),
            // A live *web* session: that one authenticates with its cookie, not here.
            Some(format!("ois.v1, ois.bearer.{WEB}")),
        ] {
            let head = upgrade(addr, offer.as_deref()).await;
            assert!(head.starts_with("http/1.1 401"), "{offer:?} -> {head}");
        }
    }

    #[sqlx::test]
    async fn the_subprotocol_is_not_a_credential_on_any_other_route(pool: PgPool) {
        let addr = serve(pool).await;
        let get = |auth: String| {
            format!(
                "GET /api/v1/me/ace-claims HTTP/1.1\r\nHost: {addr}\r\n{auth}Connection: close\r\n\r\n"
            )
        };

        // Control: the same token as a bearer is accepted, so a 401 below is about where it rode.
        let as_bearer = request(addr, &get(format!("Authorization: Bearer {DESKTOP}\r\n"))).await;
        assert!(as_bearer.starts_with("http/1.1 200"), "{as_bearer}");

        let as_subprotocol = request(
            addr,
            &get(format!(
                "Sec-WebSocket-Protocol: ois.v1, ois.bearer.{DESKTOP}\r\n"
            )),
        )
        .await;
        assert!(
            as_subprotocol.starts_with("http/1.1 401"),
            "{as_subprotocol}"
        );
    }
}
