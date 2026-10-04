//! Additive websocket push layer. A broadcast hub (in `AppState`) carries small "something changed"
//! nudges — a topic string — to every connected client, which then refetches the matching data
//! through the normal REST API. REST stays the single source of truth; the socket only lowers latency
//! versus polling, and if it drops the app degrades cleanly to the existing polls (every key the
//! socket nudges also polls, at least every `SOCKET_FALLBACK_MS` in `web/src/lib/realtime.ts`).
//!
//! **Deployment contract (#649):** any number of backend replicas may share one Postgres. A nudge is
//! delivered to this process's sockets at once and to every other replica through Postgres
//! `LISTEN/NOTIFY` on [`NOTIFY_CHANNEL`] — no extra infrastructure. Delivery is best-effort: a nudge
//! lost while a listener reconnects is healed by the client's fallback poll.

use std::{collections::HashSet, sync::Arc, time::Duration};

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
use sqlx::{PgPool, postgres::PgListener};
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

/// The Postgres channel replicas fan nudges out on.
pub const NOTIFY_CHANNEL: &str = "ois_realtime";

/// The realtime hub: this process's subscribers, plus — with a database — every other replica's.
#[derive(Clone)]
pub struct Events {
    local: broadcast::Sender<WsEvent>,
    pool: Option<PgPool>,
    /// Tags this process's notifications, so it can drop its own echo instead of delivering twice.
    instance: Arc<str>,
}

impl Events {
    pub fn new(pool: Option<PgPool>) -> Self {
        Self {
            local: broadcast::channel(256).0,
            pool,
            instance: uuid::Uuid::new_v4().simple().to_string().into(),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<WsEvent> {
        self.local.subscribe()
    }

    /// Nudges this process's sockets now, then tells the other replicas. Never fails: with no
    /// listener anywhere, or the database unreachable, the nudge simply goes no further.
    pub fn publish(&self, topic: &str) {
        self.deliver(topic);
        let (Some(pool), Ok(runtime)) = (self.pool.clone(), tokio::runtime::Handle::try_current())
        else {
            return;
        };
        let payload = format!("{}:{topic}", self.instance);
        runtime.spawn(async move {
            let notified = sqlx::query("select pg_notify($1, $2)")
                .bind(NOTIFY_CHANNEL)
                .bind(&payload)
                .execute(&pool)
                .await;
            if let Err(e) = notified {
                tracing::warn!(error = %e, "realtime: could not notify the other replicas");
            }
        });
    }

    /// Nudges this process's sockets only. For a signal every replica raises for itself — the feed
    /// tick (#648): each replica polls VATSIM and installs its own snapshot, so fanning its tick out
    /// would tell every client N times, and before the other replicas had the data.
    pub fn publish_local(&self, topic: &str) {
        self.deliver(topic);
    }

    fn deliver(&self, topic: &str) {
        // An error only means nobody on this process is listening right now.
        let _ = self.local.send(WsEvent {
            topic: topic.to_string(),
        });
    }

    /// The topic of a notification from another replica; `None` for this process's own echo.
    fn foreign_topic<'a>(&self, payload: &'a str) -> Option<&'a str> {
        let (from, topic) = payload.split_once(':')?;
        (from != &*self.instance).then_some(topic)
    }

    /// Starts forwarding the other replicas' nudges into this hub. Returns once it is listening, so a
    /// nudge published after this call is not missed; a no-op without a database. If the listening
    /// connection is lost it reconnects, and the clients' fallback poll covers the gap.
    pub async fn start_listener(&self) -> Result<(), sqlx::Error> {
        let Some(pool) = self.pool.clone() else {
            return Ok(());
        };
        let mut listener = listen(&pool).await?;
        let hub = self.clone();
        tokio::spawn(async move {
            loop {
                match listener.recv().await {
                    Ok(notification) => {
                        if let Some(topic) = hub.foreign_topic(notification.payload()) {
                            hub.deliver(topic);
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "realtime: listener lost; reconnecting");
                        listener = loop {
                            tokio::time::sleep(Duration::from_secs(5)).await;
                            match listen(&pool).await {
                                Ok(listener) => break listener,
                                Err(e) => tracing::warn!(error = %e, "realtime: reconnect failed"),
                            }
                        };
                    }
                }
            }
        });
        Ok(())
    }
}

async fn listen(pool: &PgPool) -> Result<PgListener, sqlx::Error> {
    let mut listener = PgListener::connect_with(pool).await?;
    listener.listen(NOTIFY_CHANNEL).await?;
    Ok(listener)
}

/// Topic strings — kept in sync with the frontend invalidation map (`web/src/lib/realtime.ts`).
pub mod topic {
    pub const RELEASE: &str = "flow.release";
    pub const FCA: &str = "flow.fca";
    pub const GDP: &str = "tmu.gdp";
    pub const TMI: &str = "tmu.tmi";
    pub const GROUND_STOP: &str = "tmu.groundstop";
    pub const PROGRAM: &str = "tmu.program";
    /// An advisory was created, edited, published, cancelled or deleted (#643).
    pub const ADVISORY: &str = "tmu.advisory";
    pub const CFR: &str = "flow.cfr";
    pub const EVENT_AVAILABILITY: &str = "events.availability";
    /// Someone's access changed. Payload-free like every topic here, so each client refetches its
    /// own `/me` and works out whether anything it holds actually grew — the socket is broadcast to
    /// everyone, so it must never carry who was granted what (#348).
    pub const ACCESS_GRANTED: &str = "access.granted";
    /// An ACE claim reminder came due. Mirrors the Discord DM the scheduler already sends, so the
    /// desktop app is a second delivery channel for the same decision (#348).
    pub const EVENT_REMINDER: &str = "events.reminder";
    /// The VATSIM feed ingested a new upstream publish (#648). Every feed-derived view refetches.
    pub const FEED_TICK: &str = "feed.tick";

    /// An ACE support request was created, claimed, released, decided or deleted (#645). Every viewer's
    /// board refetches, so two controllers don't race the same request on a stale view.
    pub const ACE: &str = "events.ace";
    /// A runway configuration was changed, saved or deleted (#646) — low-frequency, but it changes
    /// what every arrival is sequenced against, so other clients see it at once.
    pub const RUNWAY: &str = "flow.runway";

    /// Every topic a client may subscribe to. A new topic must be added here too, or a subscriber
    /// asking for it is refused as `unknown_topic`.
    pub const ALL: [&str; 14] = [
        RELEASE,
        FCA,
        GDP,
        TMI,
        GROUND_STOP,
        PROGRAM,
        ADVISORY,
        CFR,
        EVENT_AVAILABILITY,
        ACCESS_GRANTED,
        EVENT_REMINDER,
        FEED_TICK,
        ACE,
        RUNWAY,
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

/// The largest message a client may send. The only thing a client says is a subscribe frame, and one
/// naming every topic is under 200 bytes. Without a cap axum allows 64 MiB, which `Subscription::apply`
/// would parse into a `Vec<String>` and echo back — about 1 GB for one frame from any signed-in
/// caller. A larger message, whole or in fragments, closes the socket.
const MAX_CLIENT_MESSAGE: usize = 4096;

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
        .max_message_size(MAX_CLIENT_MESSAGE)
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
    fn the_feed_tick_is_a_topic_a_client_can_subscribe_to() {
        let mut sub = super::Subscription::default();
        let ack = sub.apply(r#"{"subscribe":["feed.tick"]}"#);
        assert!(
            ack.contains("subscribed") && ack.contains("feed.tick"),
            "{ack}"
        );
        assert!(sub.wants(super::topic::FEED_TICK));
        assert!(
            !sub.wants(super::topic::RELEASE),
            "and only what it asked for"
        );
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
            "insert into access.user_permissions (user_id, permission_name, granted, source) \
             values ('ws-user', 'ace.requests.claim', true, 'manual')",
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
            // #584 gave a credential an expiry; this fixture only needs it live for the test.
            chrono::Utc::now() + chrono::Duration::days(1),
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

    /// The socket is gone, and nothing was said on the way out: a frame the server refused must not
    /// have been parsed and answered.
    async fn assert_closed_unanswered(client: &mut Client) {
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(5), client.next())
                .await
                .expect("the server acts on the frame within 5s")
            {
                Some(Ok(Message::Text(text))) => panic!("an oversized frame was answered: {text}"),
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                _ => return, // closed, reset or ended
            }
        }
    }

    /// Uncapped, one 64 MiB subscribe frame of junk names cost the server about 1 GB (#589 QA). Sizes
    /// are absolute, not derived from `MAX_CLIENT_MESSAGE`, so raising the cap fails here too.
    #[sqlx::test]
    async fn an_oversized_frame_closes_the_socket_unanswered(pool: PgPool) {
        let (addr, _) = serve(pool).await;
        let mut client = connect_as_service_account(addr).await;

        // 4,015 bytes: just under 4 KiB, and still answered.
        let fits = format!("{{\"subscribe\":[{}]}}", vec!["\"zz\""; 800].join(","));
        assert_eq!(fits.len(), 4_015);
        client.send(Message::Text(fits.into())).await.unwrap();
        assert!(
            next_text(&mut client)
                .await
                .starts_with(r#"{"error":"unknown_topic""#)
        );

        client
            .send(Message::Text(" ".repeat(64 * 1024).into()))
            .await
            .ok();
        assert_closed_unanswered(&mut client).await;
    }

    /// The same limit holds for a message sent in pieces: sixteen 1 KiB continuation frames, each
    /// under the cap, still make a 16 KiB message.
    #[sqlx::test]
    async fn a_fragmented_oversized_message_closes_the_socket_unanswered(pool: PgPool) {
        use tokio_tungstenite::tungstenite::protocol::frame::{
            Frame,
            coding::{Data, OpCode},
        };

        let (addr, _) = serve(pool).await;
        let mut client = connect_as_service_account(addr).await;

        let piece = " ".repeat(1024);
        for i in 0..16 {
            let opcode = if i == 0 { Data::Text } else { Data::Continue };
            let frame = Frame::message(piece.clone(), OpCode::Data(opcode), i == 15);
            if client.send(Message::Frame(frame)).await.is_err() {
                break; // already closed mid-message
            }
        }
        assert_closed_unanswered(&mut client).await;
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

    // --- #649: Postgres fan-out across replicas ---

    /// #649: two replicas share one database. A nudge published on one reaches the other's sockets
    /// through Postgres, and reaches its own sockets exactly once — its own echo is dropped.
    #[sqlx::test]
    async fn a_nudge_reaches_every_replica_once(pool: sqlx::PgPool) {
        use std::time::Duration;
        use tokio::{sync::broadcast::error::TryRecvError, time::timeout};

        let a = super::Events::new(Some(pool.clone()));
        let b = super::Events::new(Some(pool.clone()));
        a.start_listener().await.unwrap();
        b.start_listener().await.unwrap();
        let (mut on_a, mut on_b) = (a.subscribe(), b.subscribe());

        a.publish(super::topic::RELEASE);

        let crossed = timeout(Duration::from_secs(10), on_b.recv())
            .await
            .expect("the other replica hears it")
            .unwrap();
        assert_eq!(crossed.topic, "flow.release");
        assert_eq!(
            on_a.try_recv().unwrap().topic,
            "flow.release",
            "delivered locally at once"
        );
        // Give A's own echo time to arrive (it travels with B's copy), then check it was dropped.
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(
            matches!(on_a.try_recv(), Err(TryRecvError::Empty)),
            "no duplicate from the echo"
        );
        assert!(
            matches!(on_b.try_recv(), Err(TryRecvError::Empty)),
            "and B heard it once"
        );
    }

    #[test]
    fn a_hub_without_a_database_stays_local() {
        let hub = super::Events::new(None);
        let mut rx = hub.subscribe();
        hub.publish(super::topic::GDP);
        assert_eq!(rx.try_recv().unwrap().topic, "tmu.gdp");
    }
}
