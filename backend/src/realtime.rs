//! Additive websocket push layer. An in-process broadcast hub (in `AppState`) carries small
//! "something changed" nudges — a topic string — to every connected client, which then refetches the
//! matching data through the normal REST API. REST stays the single source of truth; the socket only
//! lowers latency versus polling, and if it drops the app degrades cleanly to the existing polls.

use std::{collections::HashSet, time::Duration};

use axum::{
    Extension,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::{
    auth::context::{CurrentApiKey, CurrentServiceAccount, CurrentUser},
    state::AppState,
};

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

    /// Every topic a client may subscribe to. A new topic must be added here too, or a subscriber
    /// asking for it is refused as `unknown_topic`.
    pub const ALL: [&str; 10] = [
        RELEASE,
        FCA,
        GDP,
        TMI,
        GROUND_STOP,
        PROGRAM,
        CFR,
        EVENT_AVAILABILITY,
        ACCESS_GRANTED,
        EVENT_REMINDER,
    ];
}

/// Which topics one connection is forwarded (#589). Until the client sends a subscribe frame it gets
/// every topic, which is what the web and desktop apps rely on.
#[derive(Debug, Default)]
struct Subscription {
    topics: Option<HashSet<&'static str>>,
}

#[derive(Deserialize)]
struct SubscribeFrame {
    subscribe: Vec<String>,
}

impl Subscription {
    fn wants(&self, topic: &str) -> bool {
        self.topics
            .as_ref()
            .is_none_or(|topics| topics.contains(topic))
    }

    /// Handle one client text frame and return the reply. `{"subscribe":[…]}` replaces the set and is
    /// acked with `{"subscribed":[…]}`; a frame naming any unknown topic changes nothing and gets
    /// `{"error":"unknown_topic","topics":[…]}`, so a typo surfaces instead of silently never firing.
    fn apply(&mut self, text: &str) -> String {
        let Ok(frame) = serde_json::from_str::<SubscribeFrame>(text) else {
            return serde_json::json!({ "error": "bad_request" }).to_string();
        };
        let unknown: Vec<&str> = frame
            .subscribe
            .iter()
            .map(String::as_str)
            .filter(|name| !topic::ALL.contains(name))
            .collect();
        if !unknown.is_empty() {
            return serde_json::json!({ "error": "unknown_topic", "topics": unknown }).to_string();
        }
        let topics: HashSet<&'static str> = topic::ALL
            .into_iter()
            .filter(|known| frame.subscribe.iter().any(|name| name == known))
            .collect();
        let mut subscribed: Vec<&str> = topics.iter().copied().collect();
        subscribed.sort_unstable();
        self.topics = Some(topics);
        serde_json::json!({ "subscribed": subscribed }).to_string()
    }
}

/// The subprotocol a desktop client offers beside its token, and the only one ever selected.
pub const WS_PROTOCOL: &str = "ois.v1";

/// `GET /api/v1/ws` — upgrade to a websocket that streams realtime nudges. Requires a signed-in user
/// (the `ois_session` cookie, or a desktop token) or a live API key or service account (#589): the
/// router's auth middleware resolves them from the upgrade GET just as for a REST handler. Any caller
/// may receive every topic, because a nudge carries nothing but its topic name.
pub async fn ws(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(api_key): Extension<Option<CurrentApiKey>>,
    Extension(service_account): Extension<Option<CurrentServiceAccount>>,
) -> Response {
    if current_user.is_none() && api_key.is_none() && service_account.is_none() {
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

/// Forward the broadcast events the client subscribed to, answer its subscribe frames and pings, and
/// send a keepalive ping so idle connections survive proxy idle-timeouts. Exits when the client
/// closes or errors.
// The Ping arm's `if send(Pong(p)).await.is_err()` can't collapse into a match guard — guards can't
// `await`, and it would move `p` out of the pattern.
#[allow(clippy::collapsible_match)]
async fn pump(mut socket: WebSocket, mut rx: broadcast::Receiver<WsEvent>) {
    let mut keepalive = tokio::time::interval(Duration::from_secs(30));
    let mut subscription = Subscription::default();
    loop {
        tokio::select! {
            ev = rx.recv() => match ev {
                Ok(ev) if !subscription.wants(&ev.topic) => continue,
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
                Some(Ok(Message::Text(text))) => {
                    let reply = subscription.apply(&text);
                    if socket.send(Message::Text(reply.into())).await.is_err() {
                        break;
                    }
                }
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

    // --- topic subscription (#589) ---

    #[test]
    fn a_new_connection_gets_every_topic() {
        let sub = super::Subscription::default();
        assert!(super::topic::ALL.iter().all(|t| sub.wants(t)));
    }

    #[test]
    fn a_subscribe_frame_replaces_the_set_and_is_acked() {
        let mut sub = super::Subscription::default();
        let ack = sub.apply(r#"{"subscribe":["tmu.tmi","flow.release","flow.release"]}"#);
        assert_eq!(ack, r#"{"subscribed":["flow.release","tmu.tmi"]}"#);
        assert!(sub.wants("flow.release") && sub.wants("tmu.tmi"));
        assert!(!sub.wants("tmu.gdp"));

        assert_eq!(
            sub.apply(r#"{"subscribe":["tmu.gdp"]}"#),
            r#"{"subscribed":["tmu.gdp"]}"#
        );
        assert!(
            sub.wants("tmu.gdp") && !sub.wants("flow.release"),
            "replaced, not added to"
        );

        sub.apply(r#"{"subscribe":[]}"#);
        assert!(
            !super::topic::ALL.iter().any(|t| sub.wants(t)),
            "an empty list is nothing"
        );
    }

    #[test]
    fn an_unknown_topic_refuses_the_whole_frame_and_keeps_the_old_set() {
        let mut sub = super::Subscription::default();
        sub.apply(r#"{"subscribe":["flow.release"]}"#);
        let reply = sub.apply(r#"{"subscribe":["tmu.tmi","flow.releases"]}"#);
        assert_eq!(
            reply,
            r#"{"error":"unknown_topic","topics":["flow.releases"]}"#
        );
        assert!(
            sub.wants("flow.release"),
            "the previous subscription still holds"
        );
        assert!(
            !sub.wants("tmu.tmi"),
            "and nothing from the refused frame was applied"
        );
    }

    #[test]
    fn a_frame_that_is_not_a_subscription_is_a_bad_request() {
        let mut sub = super::Subscription::default();
        for text in ["hello", "{}", r#"{"subscribe":"flow.release"}"#] {
            assert_eq!(sub.apply(text), r#"{"error":"bad_request"}"#, "{text}");
        }
        assert!(sub.wants("flow.release"), "still every topic");
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
    const SERVICE_ACCOUNT: &str = "ois_sa_handshake_test";
    const API_KEY: &str = "ois_pat_handshake_test";

    async fn serve(pool: PgPool) -> (std::net::SocketAddr, AppState) {
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

        let sha = crate::repos::access::sha256_hex;
        crate::repos::service_accounts::create_service_account(
            &pool,
            "ws-bot",
            "WS bot",
            None,
            &sha(SERVICE_ACCOUNT),
        )
        .await
        .unwrap();
        sqlx::query(
            "insert into access.api_keys (owner_user_id, name, prefix, secret_hash) \
             values ('ws-user', 'ws key', 'ois_pat_hand', $1)",
        )
        .bind(sha(API_KEY))
        .execute(&pool)
        .await
        .unwrap();

        let state = AppState {
            db: Some(pool),
            ..AppState::without_db()
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = crate::router::build_router(state.clone());
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (addr, state)
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
        upgrade_with(addr, &offer).await
    }

    /// An upgrade carrying `extra` raw header lines (each ending `\r\n`).
    async fn upgrade_with(addr: std::net::SocketAddr, offer: &str) -> String {
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
        let (addr, _) = serve(pool).await;

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
        let (addr, _) = serve(pool).await;

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

    /// #589 AC1: an integration opens the socket with its own credential — a service account by
    /// header, as a server-side client would, or an API key by subprotocol, as a browser must.
    #[sqlx::test]
    async fn an_api_key_or_service_account_opens_the_socket(pool: PgPool) {
        let (addr, _) = serve(pool).await;

        let by_header = upgrade_with(
            addr,
            &format!("Authorization: Bearer {SERVICE_ACCOUNT}\r\n"),
        )
        .await;
        assert!(by_header.starts_with("http/1.1 101"), "{by_header}");

        let by_subprotocol = upgrade(addr, Some(&format!("ois.v1, ois.bearer.{API_KEY}"))).await;
        assert!(
            by_subprotocol.starts_with("http/1.1 101"),
            "{by_subprotocol}"
        );
        assert!(
            !by_subprotocol.contains(API_KEY),
            "the key was echoed: {by_subprotocol}"
        );
    }

    /// #589 AC5: a machine-shaped token that resolves to nothing is still a plain 401, by either route.
    #[sqlx::test]
    async fn a_machine_token_with_nothing_behind_it_is_refused(pool: PgPool) {
        let (addr, _) = serve(pool).await;

        for extra in [
            "Authorization: Bearer ois_sa_not_a_real_account\r\n".to_owned(),
            "Authorization: Bearer ois_pat_not_a_real_key\r\n".to_owned(),
            "Sec-WebSocket-Protocol: ois.v1, ois.bearer.ois_sa_not_a_real_account\r\n".to_owned(),
            "Sec-WebSocket-Protocol: ois.v1, ois.bearer.ois_pat_not_a_real_key\r\n".to_owned(),
        ] {
            let head = upgrade_with(addr, &extra).await;
            assert!(head.starts_with("http/1.1 401"), "{extra:?} -> {head}");
        }
    }

    // --- a real client on the socket (#589) ---

    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

    type Client = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    async fn connect_as_service_account(addr: std::net::SocketAddr) -> Client {
        let mut request = format!("ws://{addr}/api/v1/ws")
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            "authorization",
            format!("Bearer {SERVICE_ACCOUNT}").parse().unwrap(),
        );
        tokio_tungstenite::connect_async(request).await.unwrap().0
    }

    /// The next text frame, skipping the server's keepalive pings.
    async fn next_text(client: &mut Client) -> String {
        loop {
            let frame = tokio::time::timeout(std::time::Duration::from_secs(5), client.next())
                .await
                .expect("a frame within 5s")
                .expect("the socket is open")
                .unwrap();
            if let Message::Text(text) = frame {
                return text.to_string();
            }
        }
    }

    /// #589 AC1 + AC3: a service account is told a release changed, and told nothing else — the frame
    /// is the topic name alone, so a machine learns no more than any signed-in user (#348).
    #[sqlx::test]
    async fn a_service_account_receives_release_nudges_with_no_payload(pool: PgPool) {
        let (addr, state) = serve(pool).await;
        let mut client = connect_as_service_account(addr).await;

        state.publish(super::topic::RELEASE);

        let frame: serde_json::Value = serde_json::from_str(&next_text(&mut client).await).unwrap();
        assert_eq!(frame, serde_json::json!({ "topic": "flow.release" }));
    }

    /// #589 AC2: after subscribing to releases, a TMI change doesn't wake the client; and a typo'd
    /// topic is refused without losing the subscription it already had.
    #[sqlx::test]
    async fn a_subscriber_receives_only_its_topics(pool: PgPool) {
        let (addr, state) = serve(pool).await;
        let mut client = connect_as_service_account(addr).await;

        client
            .send(Message::Text(r#"{"subscribe":["flow.release"]}"#.into()))
            .await
            .unwrap();
        assert_eq!(
            next_text(&mut client).await,
            r#"{"subscribed":["flow.release"]}"#
        );

        client
            .send(Message::Text(r#"{"subscribe":["flow.releases"]}"#.into()))
            .await
            .unwrap();
        assert_eq!(
            next_text(&mut client).await,
            r#"{"error":"unknown_topic","topics":["flow.releases"]}"#
        );

        state.publish(super::topic::TMI);
        state.publish(super::topic::RELEASE);
        assert_eq!(
            next_text(&mut client).await,
            r#"{"topic":"flow.release"}"#,
            "the TMI nudge was filtered out and the release still arrives"
        );
    }

    #[sqlx::test]
    async fn the_subprotocol_is_not_a_credential_on_any_other_route(pool: PgPool) {
        let (addr, _) = serve(pool).await;
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
