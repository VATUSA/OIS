//! VATUSA/OIS#922: a presented credential whose lookup fails on the database answers
//! `503 service_unavailable` and logs the cause at ERROR with the credential kind, never the token.
//! A credential that resolves to nothing still answers as anonymous, and a request carrying no
//! credential runs no lookup at all.
//!
//! Every test goes through the real router (`resolve_current_user` → reqlog → rate limit → audit →
//! handler). The fault is DDL inside the test's own `#[sqlx::test]` database: a renamed resolver
//! table, or a `before update` trigger that raises on the `last_used_at` bookkeeping write. The
//! valid-credential control runs before the fault, in the same test.
//!
//! Events are captured with a thread-default subscriber: `#[sqlx::test]` runs a current-thread
//! runtime, so the oneshot router and the websocket test's spawned server tasks all emit on this
//! thread. A middleware 503 returns before reqlog runs, so the resolver's ERROR is the only line.
//!
//! Coverage matrix (VATUSA/OIS#922)
//!
//! Requirement                                   | Test
//! ----------------------------------------------|------------------------------------------------------------------------------
//! AC1+AC3 session cookie lookup fails           | a_session_cookie_whose_lookup_fails_answers_503_and_logs_the_kind
//! AC1+AC3 desktop bearer lookup fails           | a_desktop_bearer_whose_lookup_fails_answers_503_and_logs_the_kind
//! AC1+AC3 api key lookup fails                  | an_api_key_whose_lookup_fails_answers_503_and_logs_the_kind
//! AC1+AC3 service account lookup fails          | a_service_account_whose_lookup_fails_answers_503_and_logs_the_kind
//! AC1+D70 desktop token on the ws upgrade       | a_desktop_token_on_the_websocket_upgrade_whose_lookup_fails_answers_503
//! AC1+D70 cookie on a public route              | a_session_cookie_on_a_public_route_whose_lookup_fails_answers_503
//! AC1 api key last_used_at write fails         | an_api_key_whose_last_used_write_fails_answers_503_and_logs_the_kind
//! AC1 SA last_used_at write fails              | a_service_account_whose_last_used_write_fails_answers_503_and_logs_the_kind
//! AC2 session: revoked, expired, unknown        | a_dead_session_cookie_is_anonymous_and_logs_no_error
//! AC2 desktop: revoked, expired, unknown        | a_dead_desktop_bearer_is_anonymous_and_logs_no_error
//! AC2 api key: revoked, expired, disabled, unkn.| a_dead_api_key_is_anonymous_and_logs_no_error
//! AC2 SA: revoked, expired, disabled, unknown   | a_dead_service_account_token_is_anonymous_and_logs_no_error
//! AC4 no credential, every table broken         | a_request_without_a_credential_is_unaffected_by_a_broken_credential_store
//! AC4 bearer of neither prefix                  | a_bearer_of_neither_prefix_is_unaffected_by_a_broken_credential_store
//! AC4 subprotocol token off the ws route        | a_subprotocol_token_off_the_websocket_route_is_not_looked_up
//! AC5 `.ok().flatten()` restored per resolver   | the AC1+AC3 test of that resolver (`bin/mutate.sh`, PR evidence)

use std::sync::{Arc, Mutex};

use axum::http;
use serde_json::{Value, json};
use sqlx::PgPool;
use tracing_subscriber::layer::SubscriberExt;

use super::machine_actor_tests::{api_key, call_with, service_account};
use crate::repos::access::sha256_hex;
use crate::scope_test_support::{grant, seed_user, session_cookie, test_state};

/// One event emitted while the test ran.
#[derive(Debug, Clone)]
struct Captured {
    level: tracing::Level,
    target: String,
    message: String,
    fields: std::collections::BTreeMap<String, String>,
}

impl Captured {
    /// The message and every field value, for the "the token never appears" check.
    fn text(&self) -> String {
        let mut text = self.message.clone();
        for (name, value) in &self.fields {
            text.push_str(&format!(" {name}={value}"));
        }
        text
    }
}

struct Recorder(Arc<Mutex<Vec<Captured>>>);

#[derive(Default)]
struct Visitor {
    message: String,
    fields: std::collections::BTreeMap<String, String>,
}

impl tracing::field::Visit for Visitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_owned();
        } else {
            self.fields
                .insert(field.name().to_owned(), value.to_owned());
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        } else {
            self.fields
                .insert(field.name().to_owned(), format!("{value:?}"));
        }
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Recorder {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        let mut visitor = Visitor::default();
        event.record(&mut visitor);
        self.0
            .lock()
            .expect("the recorder's mutex is only held for a push")
            .push(Captured {
                level: *event.metadata().level(),
                target: event.metadata().target().to_owned(),
                message: visitor.message,
                fields: visitor.fields,
            });
    }
}

/// Install a recorder as this thread's default subscriber until the guard drops.
fn record() -> (Arc<Mutex<Vec<Captured>>>, tracing::subscriber::DefaultGuard) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(Recorder(Arc::clone(&events)));
    (events, tracing::subscriber::set_default(subscriber))
}

/// Take everything recorded so far, leaving the log empty for the next request.
fn drain(events: &Arc<Mutex<Vec<Captured>>>) -> Vec<Captured> {
    std::mem::take(&mut *events.lock().expect("held only for a push"))
}

/// ERROR events this crate emitted (sqlx's own events are not ours to pin).
fn backend_errors(events: &[Captured]) -> Vec<&Captured> {
    events
        .iter()
        .filter(|e| e.level == tracing::Level::ERROR && e.target.starts_with("ois_backend"))
        .collect()
}

/// Positive control that capture saw a request reach reqlog (any level).
fn saw_reqlog(events: &[Captured]) -> bool {
    events.iter().any(|e| e.target == "ois_backend::reqlog")
}

fn service_unavailable() -> Value {
    json!({ "error": "service_unavailable" })
}

// ---------------------------------------------------------------------------------------------
// AC1 + AC3: a failed lookup answers 503 and logs the credential kind, per credential kind.
// ---------------------------------------------------------------------------------------------

// AC1, AC3, AC5 (session resolver): a cookie session whose lookup fails is 503, not anonymous.
#[sqlx::test]
async fn a_session_cookie_whose_lookup_fails_answers_503_and_logs_the_kind(pool: PgPool) {
    let (events, _guard) = record();
    let state = test_state(pool.clone(), Default::default());
    let user = seed_user(&pool).await;
    grant(&pool, &user, "auth.profile.read", None).await;
    let cookie = session_cookie(&pool, &user).await;
    let token = cookie.trim_start_matches("ois_session=").to_owned();

    // Control: the same cookie answers 200 while the store is healthy.
    let (status, body) = call_with(&state, http::Method::GET, "/api/v1/me", &[&cookie], None).await;
    assert_eq!(status, http::StatusCode::OK, "{body}");
    drain(&events);

    sqlx::query("alter table identity.sessions rename to sessions_t922")
        .execute(&pool)
        .await
        .unwrap();

    let (status, body) = call_with(&state, http::Method::GET, "/api/v1/me", &[&cookie], None).await;
    let captured = drain(&events);

    assert_eq!(status, http::StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body, service_unavailable());
    let errors = backend_errors(&captured);
    assert_eq!(errors.len(), 1, "exactly one ERROR: {captured:#?}");
    assert_eq!(
        errors[0].fields.get("credential").map(String::as_str),
        Some("session_cookie"),
        "{:#?}",
        errors[0]
    );
    assert!(
        errors[0]
            .fields
            .get("error")
            .is_some_and(|e| e.contains("sessions")),
        "the cause names the failed relation: {:#?}",
        errors[0]
    );
    for event in &captured {
        assert!(!event.text().contains(&token), "token logged: {event:#?}");
    }
}

// AC1, AC3, AC5 (session resolver, desktop transport): an `ois_dsk_` bearer is 503 under the fault.
#[sqlx::test]
async fn a_desktop_bearer_whose_lookup_fails_answers_503_and_logs_the_kind(pool: PgPool) {
    let (events, _guard) = record();
    let state = test_state(pool.clone(), Default::default());
    let user = seed_user(&pool).await;
    grant(&pool, &user, "auth.profile.read", None).await;
    // The session lookup decodes `cid` as non-null.
    sqlx::query("update identity.users set cid = 1922001 where id = $1")
        .bind(&user)
        .execute(&pool)
        .await
        .unwrap();
    let token = format!("ois_dsk_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "insert into identity.sessions (session_token, user_id, kind, expires_at) \
         values ($1, $2, 'desktop', now() + interval '1 hour')",
    )
    .bind(&token)
    .bind(&user)
    .execute(&pool)
    .await
    .unwrap();
    let bearer = format!("Bearer {token}");

    let (status, body) = call_with(&state, http::Method::GET, "/api/v1/me", &[&bearer], None).await;
    assert_eq!(status, http::StatusCode::OK, "{body}");
    drain(&events);

    sqlx::query("alter table identity.sessions rename to sessions_t922")
        .execute(&pool)
        .await
        .unwrap();

    let (status, body) = call_with(&state, http::Method::GET, "/api/v1/me", &[&bearer], None).await;
    let captured = drain(&events);

    assert_eq!(status, http::StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body, service_unavailable());
    let errors = backend_errors(&captured);
    assert_eq!(errors.len(), 1, "exactly one ERROR: {captured:#?}");
    assert_eq!(
        errors[0].fields.get("credential").map(String::as_str),
        Some("desktop_session"),
        "{:#?}",
        errors[0]
    );
    for event in &captured {
        assert!(!event.text().contains(&token), "token logged: {event:#?}");
    }
}

// AC1, AC3, AC5 (api-key resolver): an `ois_pat_` key whose lookup fails is 503.
#[sqlx::test]
async fn an_api_key_whose_lookup_fails_answers_503_and_logs_the_kind(pool: PgPool) {
    let (events, _guard) = record();
    let state = test_state(pool.clone(), Default::default());
    let (_key_id, bearer) = api_key(&pool, "system.jobs.read").await;
    let token = bearer.trim_start_matches("Bearer ").to_owned();

    let (status, body) = call_with(
        &state,
        http::Method::GET,
        "/api/v1/admin/jobs",
        &[&bearer],
        None,
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "{body}");
    drain(&events);

    sqlx::query("alter table access.api_keys rename to api_keys_t922")
        .execute(&pool)
        .await
        .unwrap();

    let (status, body) = call_with(
        &state,
        http::Method::GET,
        "/api/v1/admin/jobs",
        &[&bearer],
        None,
    )
    .await;
    let captured = drain(&events);

    assert_eq!(status, http::StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body, service_unavailable());
    let errors = backend_errors(&captured);
    assert_eq!(errors.len(), 1, "exactly one ERROR: {captured:#?}");
    assert_eq!(
        errors[0].fields.get("credential").map(String::as_str),
        Some("api_key"),
        "{:#?}",
        errors[0]
    );
    assert!(
        errors[0]
            .fields
            .get("error")
            .is_some_and(|e| e.contains("api_keys")),
        "the cause names the failed relation: {:#?}",
        errors[0]
    );
    for event in &captured {
        assert!(!event.text().contains(&token), "token logged: {event:#?}");
    }
}

// AC1, AC3, AC5 (service-account resolver): an `ois_sa_` token whose lookup fails is 503.
#[sqlx::test]
async fn a_service_account_whose_lookup_fails_answers_503_and_logs_the_kind(pool: PgPool) {
    let (events, _guard) = record();
    let state = test_state(pool.clone(), Default::default());
    let (_sa_id, bearer) = service_account(&pool, "system.jobs.read", None).await;
    let token = bearer.trim_start_matches("Bearer ").to_owned();

    let (status, body) = call_with(
        &state,
        http::Method::GET,
        "/api/v1/admin/jobs",
        &[&bearer],
        None,
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "{body}");
    drain(&events);

    sqlx::query(
        "alter table access.service_account_credentials rename to service_account_credentials_t922",
    )
    .execute(&pool)
    .await
    .unwrap();

    let (status, body) = call_with(
        &state,
        http::Method::GET,
        "/api/v1/admin/jobs",
        &[&bearer],
        None,
    )
    .await;
    let captured = drain(&events);

    assert_eq!(status, http::StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body, service_unavailable());
    let errors = backend_errors(&captured);
    assert_eq!(errors.len(), 1, "exactly one ERROR: {captured:#?}");
    assert_eq!(
        errors[0].fields.get("credential").map(String::as_str),
        Some("service_account"),
        "{:#?}",
        errors[0]
    );
    assert!(
        errors[0]
            .fields
            .get("error")
            .is_some_and(|e| e.contains("service_account_credentials")),
        "the cause names the failed relation: {:#?}",
        errors[0]
    );
    for event in &captured {
        assert!(!event.text().contains(&token), "token logged: {event:#?}");
    }
}

// ---------------------------------------------------------------------------------------------
// D70: 503 wherever a credential was presented, the websocket upgrade and public routes included.
// ---------------------------------------------------------------------------------------------

/// One raw HTTP/1.1 websocket upgrade against a real listener, returning the response head
/// lowercased. A real port because a `oneshot` request carries no `OnUpgrade`, so the upgrade
/// extractor would refuse it whatever the credential (the pattern is `realtime::handshake_tests`).
async fn ws_upgrade(addr: std::net::SocketAddr, offer: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // RFC 6455's sample nonce, not a secret.
    let nonce = "dGhlIHNhbXBsZSBub25jZQ=="; // gitleaks:allow
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let head = format!(
        "GET /api/v1/ws HTTP/1.1\r\nHost: {addr}\r\nConnection: Upgrade\r\n\
         Upgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: {nonce}\r\nSec-WebSocket-Protocol: {offer}\r\n\r\n"
    );
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

// AC1, D70: a desktop token offered as a websocket subprotocol is a presented credential.
#[sqlx::test]
async fn a_desktop_token_on_the_websocket_upgrade_whose_lookup_fails_answers_503(pool: PgPool) {
    let (events, _guard) = record();
    let state = test_state(pool.clone(), Default::default());
    let user = seed_user(&pool).await;
    sqlx::query("update identity.users set cid = 1922002 where id = $1")
        .bind(&user)
        .execute(&pool)
        .await
        .unwrap();
    let token = format!("ois_dsk_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "insert into identity.sessions (session_token, user_id, kind, expires_at) \
         values ($1, $2, 'desktop', now() + interval '1 hour')",
    )
    .bind(&token)
    .bind(&user)
    .execute(&pool)
    .await
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = crate::router::build_router(state.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let offer = format!("ois.v1, ois.bearer.{token}");

    // Control: the live desktop token opens the socket.
    let head = ws_upgrade(addr, &offer).await;
    assert!(head.starts_with("http/1.1 101"), "{head}");
    drain(&events);

    sqlx::query("alter table identity.sessions rename to sessions_t922")
        .execute(&pool)
        .await
        .unwrap();

    let head = ws_upgrade(addr, &offer).await;
    let captured = drain(&events);

    assert!(head.starts_with("http/1.1 503"), "{head}");
    let errors = backend_errors(&captured);
    assert_eq!(errors.len(), 1, "exactly one ERROR: {captured:#?}");
    assert_eq!(
        errors[0].fields.get("credential").map(String::as_str),
        Some("desktop_session"),
        "{:#?}",
        errors[0]
    );
    for event in &captured {
        assert!(!event.text().contains(&token), "token logged: {event:#?}");
    }
}

// AC1, D70: a public route still answers 503 when the cookie it was sent cannot be resolved.
#[sqlx::test]
async fn a_session_cookie_on_a_public_route_whose_lookup_fails_answers_503(pool: PgPool) {
    let (events, _guard) = record();
    let state = test_state(pool.clone(), Default::default());
    let user = seed_user(&pool).await;
    let cookie = session_cookie(&pool, &user).await;

    let (status, body) = call_with(
        &state,
        http::Method::GET,
        "/api/v1/flow/traffic",
        &[&cookie],
        None,
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "{body}");
    drain(&events);

    sqlx::query("alter table identity.sessions rename to sessions_t922")
        .execute(&pool)
        .await
        .unwrap();

    let (status, body) = call_with(
        &state,
        http::Method::GET,
        "/api/v1/flow/traffic",
        &[&cookie],
        None,
    )
    .await;
    let captured = drain(&events);

    assert_eq!(status, http::StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body, service_unavailable());
    let errors = backend_errors(&captured);
    assert_eq!(errors.len(), 1, "exactly one ERROR: {captured:#?}");
    assert_eq!(
        errors[0].fields.get("credential").map(String::as_str),
        Some("session_cookie")
    );
}

// ---------------------------------------------------------------------------------------------
// AC1: a failed `last_used_at` write answers 503 like a failed lookup.
// ---------------------------------------------------------------------------------------------

// The select succeeds and the bookkeeping update fails; the `error` field must carry the
// trigger's message, which tells this failure apart from a failed lookup.
#[sqlx::test]
async fn an_api_key_whose_last_used_write_fails_answers_503_and_logs_the_kind(pool: PgPool) {
    let (events, _guard) = record();
    let state = test_state(pool.clone(), Default::default());
    let (key_id, bearer) = api_key(&pool, "system.jobs.read").await;
    let token = bearer.trim_start_matches("Bearer ").to_owned();

    let (status, body) = call_with(
        &state,
        http::Method::GET,
        "/api/v1/admin/jobs",
        &[&bearer],
        None,
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "{body}");
    drain(&events);

    // The control stamped `last_used_at`; clear it so the once-a-minute update fires again.
    sqlx::query("update access.api_keys set last_used_at = null where id = $1")
        .bind(&key_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "create function public.t922_refuse() returns trigger language plpgsql as \
         $$ begin raise exception 't922 injected last_used_at fault'; end $$",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "create trigger t922_refuse before update on access.api_keys \
         for each row execute function public.t922_refuse()",
    )
    .execute(&pool)
    .await
    .unwrap();

    let (status, body) = call_with(
        &state,
        http::Method::GET,
        "/api/v1/admin/jobs",
        &[&bearer],
        None,
    )
    .await;
    let captured = drain(&events);

    assert_eq!(status, http::StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body, service_unavailable());
    let errors = backend_errors(&captured);
    assert_eq!(errors.len(), 1, "exactly one ERROR: {captured:#?}");
    assert_eq!(
        errors[0].fields.get("credential").map(String::as_str),
        Some("api_key")
    );
    assert!(
        errors[0]
            .fields
            .get("error")
            .is_some_and(|e| e.contains("t922 injected last_used_at fault")),
        "the update, not the lookup, failed: {:#?}",
        errors[0]
    );
    for event in &captured {
        assert!(!event.text().contains(&token), "token logged: {event:#?}");
    }
}

#[sqlx::test]
async fn a_service_account_whose_last_used_write_fails_answers_503_and_logs_the_kind(pool: PgPool) {
    let (events, _guard) = record();
    let state = test_state(pool.clone(), Default::default());
    let (sa_id, bearer) = service_account(&pool, "system.jobs.read", None).await;
    let token = bearer.trim_start_matches("Bearer ").to_owned();

    let (status, body) = call_with(
        &state,
        http::Method::GET,
        "/api/v1/admin/jobs",
        &[&bearer],
        None,
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "{body}");
    drain(&events);

    sqlx::query(
        "update access.service_account_credentials set last_used_at = null \
         where service_account_id = $1",
    )
    .bind(&sa_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "create function public.t922_refuse() returns trigger language plpgsql as \
         $$ begin raise exception 't922 injected last_used_at fault'; end $$",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "create trigger t922_refuse before update on access.service_account_credentials \
         for each row execute function public.t922_refuse()",
    )
    .execute(&pool)
    .await
    .unwrap();

    let (status, body) = call_with(
        &state,
        http::Method::GET,
        "/api/v1/admin/jobs",
        &[&bearer],
        None,
    )
    .await;
    let captured = drain(&events);

    assert_eq!(status, http::StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body, service_unavailable());
    let errors = backend_errors(&captured);
    assert_eq!(errors.len(), 1, "exactly one ERROR: {captured:#?}");
    assert_eq!(
        errors[0].fields.get("credential").map(String::as_str),
        Some("service_account")
    );
    assert!(
        errors[0]
            .fields
            .get("error")
            .is_some_and(|e| e.contains("t922 injected last_used_at fault")),
        "the update, not the lookup, failed: {:#?}",
        errors[0]
    );
    for event in &captured {
        assert!(!event.text().contains(&token), "token logged: {event:#?}");
    }
}

// ---------------------------------------------------------------------------------------------
// AC2: a credential that resolves to nothing is still anonymous: 401, and no ERROR.
// ---------------------------------------------------------------------------------------------

// AC2 (cookie): revoked, expired and unknown sessions are anonymous; a live one is the control.
#[sqlx::test]
async fn a_dead_session_cookie_is_anonymous_and_logs_no_error(pool: PgPool) {
    let (events, _guard) = record();
    let state = test_state(pool.clone(), Default::default());
    let user = seed_user(&pool).await;
    grant(&pool, &user, "auth.profile.read", None).await;
    let live = session_cookie(&pool, &user).await;

    let (status, body) = call_with(&state, http::Method::GET, "/api/v1/me", &[&live], None).await;
    assert_eq!(status, http::StatusCode::OK, "control: {body}");

    for (token, revoked_at, expires_at) in [
        (
            "t922revokedsession",
            "now() - interval '1 minute'",
            "now() + interval '1 hour'",
        ),
        ("t922expiredsession", "null", "now() - interval '1 hour'"),
    ] {
        // Trusted literals from the table above, not input.
        sqlx::query(&format!(
            "insert into identity.sessions (session_token, user_id, kind, revoked_at, expires_at) \
             values ($1, $2, 'web', {revoked_at}, {expires_at})"
        ))
        .bind(token)
        .bind(&user)
        .execute(&pool)
        .await
        .unwrap();
    }

    for (case, cookie) in [
        ("revoked", "ois_session=t922revokedsession"),
        ("expired", "ois_session=t922expiredsession"),
        ("unknown", "ois_session=t922nosuchsession"),
    ] {
        drain(&events);
        let (status, body) =
            call_with(&state, http::Method::GET, "/api/v1/me", &[cookie], None).await;
        let captured = drain(&events);
        assert_eq!(status, http::StatusCode::UNAUTHORIZED, "{case}: {body}");
        assert!(saw_reqlog(&captured), "{case}: capture saw the request");
        assert!(
            backend_errors(&captured).is_empty(),
            "{case}: {captured:#?}"
        );
    }
}

// AC2 (desktop bearer): revoked, expired and unknown `ois_dsk_` tokens are anonymous.
#[sqlx::test]
async fn a_dead_desktop_bearer_is_anonymous_and_logs_no_error(pool: PgPool) {
    let (events, _guard) = record();
    let state = test_state(pool.clone(), Default::default());
    let user = seed_user(&pool).await;
    grant(&pool, &user, "auth.profile.read", None).await;
    sqlx::query("update identity.users set cid = 1922003 where id = $1")
        .bind(&user)
        .execute(&pool)
        .await
        .unwrap();

    for (token, revoked_at, expires_at) in [
        ("ois_dsk_t922live", "null", "now() + interval '1 hour'"),
        (
            "ois_dsk_t922revoked",
            "now() - interval '1 minute'",
            "now() + interval '1 hour'",
        ),
        ("ois_dsk_t922expired", "null", "now() - interval '1 hour'"),
    ] {
        // Trusted literals from the table above, not input.
        sqlx::query(&format!(
            "insert into identity.sessions (session_token, user_id, kind, revoked_at, expires_at) \
             values ($1, $2, 'desktop', {revoked_at}, {expires_at})"
        ))
        .bind(token)
        .bind(&user)
        .execute(&pool)
        .await
        .unwrap();
    }

    let (status, body) = call_with(
        &state,
        http::Method::GET,
        "/api/v1/me",
        &["Bearer ois_dsk_t922live"],
        None,
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "control: {body}");

    for (case, bearer) in [
        ("revoked", "Bearer ois_dsk_t922revoked"),
        ("expired", "Bearer ois_dsk_t922expired"),
        ("unknown", "Bearer ois_dsk_t922nosuchsession"),
    ] {
        drain(&events);
        let (status, body) =
            call_with(&state, http::Method::GET, "/api/v1/me", &[bearer], None).await;
        let captured = drain(&events);
        assert_eq!(status, http::StatusCode::UNAUTHORIZED, "{case}: {body}");
        assert!(saw_reqlog(&captured), "{case}: capture saw the request");
        assert!(
            backend_errors(&captured).is_empty(),
            "{case}: {captured:#?}"
        );
    }
}

// AC2 (api key): revoked, expired, disabled and unknown `ois_pat_` keys are anonymous.
#[sqlx::test]
async fn a_dead_api_key_is_anonymous_and_logs_no_error(pool: PgPool) {
    let (events, _guard) = record();
    let state = test_state(pool.clone(), Default::default());
    let owner = seed_user(&pool).await;
    grant(&pool, &owner, "system.jobs.read", None).await;

    for (token, status, revoked_at, expires_at) in [
        ("ois_pat_t922live", "active", "null", "null"),
        (
            "ois_pat_t922revoked",
            "active",
            "now() - interval '1 minute'",
            "null",
        ),
        (
            "ois_pat_t922expired",
            "active",
            "null",
            "now() - interval '1 hour'",
        ),
        ("ois_pat_t922disabled", "disabled", "null", "null"),
    ] {
        // Trusted literals from the table above, not input.
        let id: String = sqlx::query_scalar(&format!(
            "insert into access.api_keys \
               (owner_user_id, name, prefix, secret_hash, status, revoked_at, expires_at) \
             values ($1, $2, 'ois_pat_t922', $3, $4, {revoked_at}, {expires_at}) returning id"
        ))
        .bind(&owner)
        .bind(token)
        .bind(sha256_hex(token))
        .bind(status)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into access.api_key_permissions (api_key_id, permission_name) \
             values ($1, 'system.jobs.read')",
        )
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
    }

    let (status, body) = call_with(
        &state,
        http::Method::GET,
        "/api/v1/admin/jobs",
        &["Bearer ois_pat_t922live"],
        None,
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "control: {body}");

    for (case, bearer) in [
        ("revoked", "Bearer ois_pat_t922revoked"),
        ("expired", "Bearer ois_pat_t922expired"),
        ("disabled", "Bearer ois_pat_t922disabled"),
        ("unknown", "Bearer ois_pat_t922nosuchkey"),
    ] {
        drain(&events);
        let (status, body) = call_with(
            &state,
            http::Method::GET,
            "/api/v1/admin/jobs",
            &[bearer],
            None,
        )
        .await;
        let captured = drain(&events);
        assert_eq!(status, http::StatusCode::UNAUTHORIZED, "{case}: {body}");
        assert!(saw_reqlog(&captured), "{case}: capture saw the request");
        assert!(
            backend_errors(&captured).is_empty(),
            "{case}: {captured:#?}"
        );
    }
}

// AC2 (service account): revoked or expired credentials, a disabled account and an unknown
// `ois_sa_` token are anonymous.
#[sqlx::test]
async fn a_dead_service_account_token_is_anonymous_and_logs_no_error(pool: PgPool) {
    let (events, _guard) = record();
    let state = test_state(pool.clone(), Default::default());

    for (key, token, account_status, revoked_at, expires_at) in [
        (
            "t922live",
            "ois_sa_t922live",
            "active",
            "null",
            "now() + interval '1 day'",
        ),
        (
            "t922revoked",
            "ois_sa_t922revoked",
            "active",
            "now() - interval '1 minute'",
            "now() + interval '1 day'",
        ),
        (
            "t922expired",
            "ois_sa_t922expired",
            "active",
            "null",
            "now() - interval '1 hour'",
        ),
        (
            "t922disabled",
            "ois_sa_t922disabled",
            "disabled",
            "null",
            "now() + interval '1 day'",
        ),
    ] {
        let id: String = sqlx::query_scalar(
            "insert into access.service_accounts (key, name, status) values ($1, $1, $2) \
             returning id",
        )
        .bind(key)
        .bind(account_status)
        .fetch_one(&pool)
        .await
        .unwrap();
        // Trusted literals from the table above, not input.
        sqlx::query(&format!(
            "insert into access.service_account_credentials \
               (service_account_id, secret_hash, revoked_at, expires_at) \
             values ($1, $2, {revoked_at}, {expires_at})"
        ))
        .bind(&id)
        .bind(sha256_hex(token))
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into access.service_account_permissions (service_account_id, permission_name) \
             values ($1, 'system.jobs.read')",
        )
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
    }

    let (status, body) = call_with(
        &state,
        http::Method::GET,
        "/api/v1/admin/jobs",
        &["Bearer ois_sa_t922live"],
        None,
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "control: {body}");

    for (case, bearer) in [
        ("revoked", "Bearer ois_sa_t922revoked"),
        ("expired", "Bearer ois_sa_t922expired"),
        ("disabled", "Bearer ois_sa_t922disabled"),
        ("unknown", "Bearer ois_sa_t922nosuchtoken"),
    ] {
        drain(&events);
        let (status, body) = call_with(
            &state,
            http::Method::GET,
            "/api/v1/admin/jobs",
            &[bearer],
            None,
        )
        .await;
        let captured = drain(&events);
        assert_eq!(status, http::StatusCode::UNAUTHORIZED, "{case}: {body}");
        assert!(saw_reqlog(&captured), "{case}: capture saw the request");
        assert!(
            backend_errors(&captured).is_empty(),
            "{case}: {captured:#?}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// AC4: no credential, no lookup: a broken credential store does not reach the request.
// ---------------------------------------------------------------------------------------------

// AC4: with every resolver table broken, a credential-less request is public on a public route
// and 401 on a protected one, and nothing is logged at ERROR. The cookie request is the control
// that the fault is live.
#[sqlx::test]
async fn a_request_without_a_credential_is_unaffected_by_a_broken_credential_store(pool: PgPool) {
    let (events, _guard) = record();
    let state = test_state(pool.clone(), Default::default());
    let user = seed_user(&pool).await;
    let cookie = session_cookie(&pool, &user).await;
    for ddl in [
        "alter table identity.sessions rename to sessions_t922",
        "alter table access.api_keys rename to api_keys_t922",
        "alter table access.service_account_credentials rename to service_account_credentials_t922",
    ] {
        sqlx::query(ddl).execute(&pool).await.unwrap();
    }

    let (status, body) =
        call_with(&state, http::Method::GET, "/api/v1/flow/traffic", &[], None).await;
    assert_eq!(status, http::StatusCode::OK, "{body}");
    assert_eq!(body, json!([]));
    let (status, body) = call_with(&state, http::Method::GET, "/api/v1/me", &[], None).await;
    assert_eq!(status, http::StatusCode::UNAUTHORIZED, "{body}");
    let captured = drain(&events);
    assert!(saw_reqlog(&captured));
    assert!(backend_errors(&captured).is_empty(), "{captured:#?}");

    // Control: the fault is live for a request that does present a credential.
    let (status, _) = call_with(
        &state,
        http::Method::GET,
        "/api/v1/flow/traffic",
        &[&cookie],
        None,
    )
    .await;
    assert_eq!(status, http::StatusCode::SERVICE_UNAVAILABLE);
}

// AC4: a bearer of neither prefix (the `METRICS_TOKEN` Prometheus sends) routes to no resolver.
#[sqlx::test]
async fn a_bearer_of_neither_prefix_is_unaffected_by_a_broken_credential_store(pool: PgPool) {
    let (events, _guard) = record();
    let state = test_state(pool.clone(), Default::default());
    for ddl in [
        "alter table identity.sessions rename to sessions_t922",
        "alter table access.api_keys rename to api_keys_t922",
        "alter table access.service_account_credentials rename to service_account_credentials_t922",
    ] {
        sqlx::query(ddl).execute(&pool).await.unwrap();
    }
    let bearer = "Bearer s3cret-metrics-token";

    let (status, body) = call_with(
        &state,
        http::Method::GET,
        "/api/v1/flow/traffic",
        &[bearer],
        None,
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "{body}");
    let (status, body) = call_with(&state, http::Method::GET, "/api/v1/me", &[bearer], None).await;
    assert_eq!(status, http::StatusCode::UNAUTHORIZED, "{body}");
    let captured = drain(&events);
    assert!(saw_reqlog(&captured));
    assert!(backend_errors(&captured).is_empty(), "{captured:#?}");

    // Control: a prefixed bearer under the same fault is looked up and fails.
    let (status, _) = call_with(
        &state,
        http::Method::GET,
        "/api/v1/flow/traffic",
        &["Bearer ois_pat_t922anykey"],
        None,
    )
    .await;
    assert_eq!(status, http::StatusCode::SERVICE_UNAVAILABLE);
}

// AC4: off `/api/v1/ws` the subprotocol list is not a credential, so it costs no lookup.
#[sqlx::test]
async fn a_subprotocol_token_off_the_websocket_route_is_not_looked_up(pool: PgPool) {
    let (events, _guard) = record();
    let state = test_state(pool.clone(), Default::default());
    for ddl in [
        "alter table identity.sessions rename to sessions_t922",
        "alter table access.api_keys rename to api_keys_t922",
        "alter table access.service_account_credentials rename to service_account_credentials_t922",
    ] {
        sqlx::query(ddl).execute(&pool).await.unwrap();
    }
    let offer = "Sec-WebSocket-Protocol: ois.v1, ois.bearer.ois_dsk_t922offered";

    let (status, body) = call_with(&state, http::Method::GET, "/api/v1/me", &[offer], None).await;
    let captured = drain(&events);
    assert_eq!(status, http::StatusCode::UNAUTHORIZED, "{body}");
    assert!(saw_reqlog(&captured));
    assert!(backend_errors(&captured).is_empty(), "{captured:#?}");

    // Control: the same token as a bearer is a presented credential and fails on the fault.
    let (status, _) = call_with(
        &state,
        http::Method::GET,
        "/api/v1/me",
        &["Bearer ois_dsk_t922offered"],
        None,
    )
    .await;
    assert_eq!(status, http::StatusCode::SERVICE_UNAVAILABLE);
}
