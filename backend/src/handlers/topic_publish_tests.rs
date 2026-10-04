//! VATUSA/OIS#645 and #646: the ACE board and the runway view are told when they change, by the
//! writes themselves. Every write goes through the real router with a receiver on the hub, so a test
//! fails if a handler stops publishing — or publishes for a write that did not happen.

use std::collections::HashMap;

use axum::http::Method;
use serde_json::json;
use sqlx::PgPool;
use tokio::sync::broadcast::Receiver;

use crate::realtime::{WsEvent, topic};
use crate::scope_test_support::{grant, seed_user, send, session_cookie, test_state};

/// How many events of `topic` arrived since the last drain.
fn drain(rx: &mut Receiver<WsEvent>, wanted: &str) -> usize {
    let mut n = 0;
    while let Ok(event) = rx.try_recv() {
        if event.topic == wanted {
            n += 1;
        }
    }
    n
}

async fn seed_event(pool: &PgPool) {
    sqlx::query(
        "insert into events.event (id, title, start_time, end_time) \
         values (645, 'ACE Topic', now(), now() + interval '2 hours')",
    )
    .execute(pool)
    .await
    .unwrap();
}

async fn newest_request(pool: &PgPool) -> String {
    sqlx::query_scalar("select id from ace.requests order by created_at desc limit 1")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test]
async fn every_ace_write_tells_the_board_once_it_has_happened(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let mut rx = state.events.subscribe();
    seed_event(&pool).await;
    let user = seed_user(&pool).await;
    for p in [
        "ace.requests.create",
        "ace.requests.claim",
        "ace.requests.decide",
    ] {
        grant(&pool, &user, p, None).await;
    }
    let cookie = session_cookie(&pool, &user).await;
    let base = "/api/v1/events/645/ace";

    let status = send(
        &state,
        Method::POST,
        base,
        &cookie,
        Some(json!({"details": "Need help at ZDC", "slots": 2})),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(drain(&mut rx, topic::ACE), 1, "create");
    let req = newest_request(&pool).await;

    let claim = format!("{base}/{req}/claim");
    assert_eq!(
        send(&state, Method::POST, &claim, &cookie, Some(json!({}))).await,
        200
    );
    assert_eq!(drain(&mut rx, topic::ACE), 1, "claim");

    assert_eq!(
        send(&state, Method::DELETE, &claim, &cookie, None).await,
        200
    );
    assert_eq!(drain(&mut rx, topic::ACE), 1, "release");

    let decide = format!("{base}/{req}/decide");
    assert_eq!(
        send(
            &state,
            Method::POST,
            &decide,
            &cookie,
            Some(json!({"outcome": "completed"}))
        )
        .await,
        200
    );
    assert_eq!(drain(&mut rx, topic::ACE), 1, "decide");

    assert_eq!(
        send(
            &state,
            Method::DELETE,
            &format!("{base}/{req}"),
            &cookie,
            None
        )
        .await,
        204
    );
    assert_eq!(drain(&mut rx, topic::ACE), 1, "delete");
}

/// The publish follows the write: a refused write — no permission, a bad body, nothing to delete —
/// tells no one.
#[sqlx::test]
async fn a_refused_ace_write_tells_no_one(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let mut rx = state.events.subscribe();
    seed_event(&pool).await;
    let base = "/api/v1/events/645/ace";

    let outsider = seed_user(&pool).await;
    let outsider_cookie = session_cookie(&pool, &outsider).await;
    let status = send(
        &state,
        Method::POST,
        base,
        &outsider_cookie,
        Some(json!({"details": "x", "slots": 1})),
    )
    .await;
    assert_eq!(status, 401);

    let user = seed_user(&pool).await;
    for p in ["ace.requests.create", "ace.requests.decide"] {
        grant(&pool, &user, p, None).await;
    }
    let cookie = session_cookie(&pool, &user).await;
    assert_eq!(
        send(
            &state,
            Method::POST,
            base,
            &cookie,
            Some(json!({"details": "x", "slots": 0}))
        )
        .await,
        400
    );
    assert_eq!(
        send(
            &state,
            Method::DELETE,
            &format!("{base}/no-such-request"),
            &cookie,
            None
        )
        .await,
        404
    );

    assert_eq!(drain(&mut rx, topic::ACE), 0);
}

#[sqlx::test]
async fn every_runway_write_tells_other_clients(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let mut rx = state.events.subscribe();
    let user = seed_user(&pool).await;
    grant(&pool, &user, "flow.runway.update", None).await;
    let cookie = session_cookie(&pool, &user).await;
    let base = "/api/v1/flow/runway/KDCA";

    // With no live feed the board can't be built after the write (503), but the write itself is
    // committed, so other clients are still told.
    let status = send(
        &state,
        Method::PUT,
        base,
        &cookie,
        Some(json!({"active_ends": ["1"]})),
    )
    .await;
    assert!(matches!(status.as_u16(), 200 | 503), "{status}");
    assert_eq!(drain(&mut rx, topic::RUNWAY), 1, "change");

    let saved = format!("{base}/configs/SOUTH");
    assert_eq!(
        send(
            &state,
            Method::PUT,
            &saved,
            &cookie,
            Some(json!({"active_ends": ["1"]}))
        )
        .await,
        204
    );
    assert_eq!(drain(&mut rx, topic::RUNWAY), 1, "save");

    assert_eq!(
        send(&state, Method::DELETE, &saved, &cookie, None).await,
        204
    );
    assert_eq!(drain(&mut rx, topic::RUNWAY), 1, "delete");

    assert_eq!(
        send(&state, Method::DELETE, &saved, &cookie, None).await,
        404
    );
    assert_eq!(
        drain(&mut rx, topic::RUNWAY),
        0,
        "nothing left to delete: no one told"
    );
}
