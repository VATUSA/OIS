//! VATUSA/OIS#920: an ACE request is reached only through its own event. Each write route takes the
//! event id and the request id from the path; the request must be resolved with the event id bound
//! in the same query, so another event's request id answers 404 and its row, claims and status are
//! left as they were. Every request goes through the real router.
//!
//! Coverage (VATUSA/OIS#920): one test per route, each walking no credential (401), no permission
//! (401), a ZDC-scoped holder and a national holder (404 through another event's URL with the row
//! unchanged and no ACE topic published, then success through its own event, published once).
//!
//! Every request row carries `artcc_id = 'ZNY'` and every scoped caller holds its permission at ZDC
//! only: the routes take no ARTCC scope check today, so a ZDC holder's success on the right event is
//! the pinned current behavior, and its 404 on the wrong event can only come from the event binding.

use std::collections::HashMap;

use axum::http::{Method, StatusCode};
use serde_json::json;
use sqlx::PgPool;
use tokio::sync::broadcast::Receiver;

use crate::realtime::{WsEvent, topic};
use crate::scope_test_support::{grant, seed_user, send, session_cookie, test_state};

/// How many events of `wanted` arrived since the last drain.
fn drain(rx: &mut Receiver<WsEvent>, wanted: &str) -> usize {
    let mut n = 0;
    while let Ok(event) = rx.try_recv() {
        if event.topic == wanted {
            n += 1;
        }
    }
    n
}

// AC1, AC2, AC3: DELETE /api/v1/events/{id}/ace/{req} binds the event id in the delete.
#[sqlx::test]
async fn delete_answers_404_for_another_events_request_and_keeps_it(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let mut rx = state.events.subscribe();
    let requester = seed_user(&pool).await;
    sqlx::query(
        "insert into events.event (id, title, start_time, end_time) values \
         (9201, 'ACE 920 A', now(), now() + interval '2 hours'), \
         (9202, 'ACE 920 B', now(), now() + interval '2 hours')",
    )
    .execute(&pool)
    .await
    .unwrap();
    // a1 is the target; a2 shares its event (the id predicate's neighbor); b1 is the other event's;
    // a3 is the national holder's fresh request.
    sqlx::query(
        "insert into ace.requests (id, event_id, artcc_id, position, slots, status, requested_by) values \
         ('ace-920-a1', 9201, 'ZNY', 'NY_CTR', 1, 'open', $1), \
         ('ace-920-a2', 9201, 'ZNY', 'N90_APP', 1, 'open', $1), \
         ('ace-920-b1', 9202, 'ZNY', 'NY_CTR', 1, 'open', $1), \
         ('ace-920-a3', 9201, 'ZNY', 'JFK_TWR', 1, 'open', $1)",
    )
    .bind(&requester)
    .execute(&pool)
    .await
    .unwrap();

    // Unauthenticated.
    assert_eq!(
        send(
            &state,
            Method::DELETE,
            "/api/v1/events/9201/ace/ace-920-a1",
            "",
            None
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
    // Signed in, no permission.
    let outsider = seed_user(&pool).await;
    let outsider_cookie = session_cookie(&pool, &outsider).await;
    assert_eq!(
        send(
            &state,
            Method::DELETE,
            "/api/v1/events/9201/ace/ace-920-a1",
            &outsider_cookie,
            None
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
    let a1_count: i64 =
        sqlx::query_scalar("select count(*) from ace.requests where id = 'ace-920-a1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(a1_count, 1, "a refused caller deleted nothing");

    // A ZDC-scoped holder, through the wrong event.
    let scoped = seed_user(&pool).await;
    grant(&pool, &scoped, "ace.requests.decide", Some("ZDC")).await;
    let scoped_cookie = session_cookie(&pool, &scoped).await;
    assert_eq!(
        send(
            &state,
            Method::DELETE,
            "/api/v1/events/9202/ace/ace-920-a1",
            &scoped_cookie,
            None
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        drain(&mut rx, topic::ACE),
        0,
        "a refused cross-event write publishes nothing"
    );
    let a1: Option<(i64, String)> =
        sqlx::query_as("select event_id, status from ace.requests where id = 'ace-920-a1'")
            .fetch_optional(&pool)
            .await
            .unwrap();
    assert_eq!(
        a1,
        Some((9201, "open".to_string())),
        "the wrong event's URL left a1 in place"
    );

    // Positive control: the same holder, through a1's own event.
    assert_eq!(
        send(
            &state,
            Method::DELETE,
            "/api/v1/events/9201/ace/ace-920-a1",
            &scoped_cookie,
            None
        )
        .await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        drain(&mut rx, topic::ACE),
        1,
        "the write through its own event publishes once"
    );
    let left: Vec<String> = sqlx::query_scalar(
        "select id from ace.requests where id in ('ace-920-a1', 'ace-920-a2', 'ace-920-b1') order by id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        left,
        vec!["ace-920-a2".to_string(), "ace-920-b1".to_string()],
        "only a1 is deleted; its event sibling and the other event's request survive"
    );

    // A national holder: the same pair on a fresh request.
    let national = seed_user(&pool).await;
    grant(&pool, &national, "ace.requests.decide", None).await;
    let national_cookie = session_cookie(&pool, &national).await;
    assert_eq!(
        send(
            &state,
            Method::DELETE,
            "/api/v1/events/9202/ace/ace-920-a3",
            &national_cookie,
            None
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        drain(&mut rx, topic::ACE),
        0,
        "a refused cross-event write publishes nothing"
    );
    let a3_count: i64 =
        sqlx::query_scalar("select count(*) from ace.requests where id = 'ace-920-a3'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(a3_count, 1, "national scope does not reach across events");
    assert_eq!(
        send(
            &state,
            Method::DELETE,
            "/api/v1/events/9201/ace/ace-920-a3",
            &national_cookie,
            None
        )
        .await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        drain(&mut rx, topic::ACE),
        1,
        "the write through its own event publishes once"
    );
    let a3_count: i64 =
        sqlx::query_scalar("select count(*) from ace.requests where id = 'ace-920-a3'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(a3_count, 0);
}

// AC1, AC2, AC3: DELETE /api/v1/events/{id}/ace/{req}/claim binds the event id in the claim delete.
// The caller holds a claim on the request in every case, so the 404 comes from the event binding,
// never from a missing claim.
#[sqlx::test]
async fn release_answers_404_for_another_events_request_and_keeps_the_claim(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let mut rx = state.events.subscribe();
    let requester = seed_user(&pool).await;
    sqlx::query(
        "insert into events.event (id, title, start_time, end_time) values \
         (9201, 'ACE 920 A', now(), now() + interval '2 hours'), \
         (9202, 'ACE 920 B', now(), now() + interval '2 hours')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "insert into ace.requests (id, event_id, artcc_id, position, slots, status, requested_by) values \
         ('ace-920-a1', 9201, 'ZNY', 'NY_CTR', 2, 'open', $1), \
         ('ace-920-b1', 9202, 'ZNY', 'NY_CTR', 1, 'open', $1), \
         ('ace-920-a3', 9201, 'ZNY', 'JFK_TWR', 1, 'open', $1)",
    )
    .bind(&requester)
    .execute(&pool)
    .await
    .unwrap();

    let scoped = seed_user(&pool).await;
    grant(&pool, &scoped, "ace.requests.claim", Some("ZDC")).await;
    let scoped_cookie = session_cookie(&pool, &scoped).await;
    let bystander = seed_user(&pool).await;
    // A claim row's cid decodes as non-null in the response body; every real account carries one.
    sqlx::query("update identity.users set cid = 9200001 where id = $1")
        .bind(&bystander)
        .execute(&pool)
        .await
        .unwrap();
    let national = seed_user(&pool).await;
    grant(&pool, &national, "ace.requests.claim", None).await;
    let national_cookie = session_cookie(&pool, &national).await;
    // The scoped claimer holds a1 and b1; a bystander also holds a1 (the claimed_by predicate's
    // neighbor); the national claimer holds a3.
    sqlx::query(
        "insert into ace.claims (request_id, claimed_by, notes) values \
         ('ace-920-a1', $1, 'scoped on a1'), \
         ('ace-920-b1', $1, 'scoped on b1'), \
         ('ace-920-a1', $2, 'bystander on a1'), \
         ('ace-920-a3', $3, 'national on a3')",
    )
    .bind(&scoped)
    .bind(&bystander)
    .bind(&national)
    .execute(&pool)
    .await
    .unwrap();

    // Unauthenticated.
    assert_eq!(
        send(
            &state,
            Method::DELETE,
            "/api/v1/events/9201/ace/ace-920-a1/claim",
            "",
            None
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
    // Signed in, no permission.
    let outsider = seed_user(&pool).await;
    let outsider_cookie = session_cookie(&pool, &outsider).await;
    assert_eq!(
        send(
            &state,
            Method::DELETE,
            "/api/v1/events/9201/ace/ace-920-a1/claim",
            &outsider_cookie,
            None
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
    let claims: i64 = sqlx::query_scalar("select count(*) from ace.claims")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(claims, 4, "a refused caller released nothing");

    // The ZDC-scoped claimer, through the wrong event.
    assert_eq!(
        send(
            &state,
            Method::DELETE,
            "/api/v1/events/9202/ace/ace-920-a1/claim",
            &scoped_cookie,
            None
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        drain(&mut rx, topic::ACE),
        0,
        "a refused cross-event write publishes nothing"
    );
    let scoped_on_a1: i64 = sqlx::query_scalar(
        "select count(*) from ace.claims where request_id = 'ace-920-a1' and claimed_by = $1",
    )
    .bind(&scoped)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        scoped_on_a1, 1,
        "the wrong event's URL left the claim in place"
    );
    let a1_status: String =
        sqlx::query_scalar("select status from ace.requests where id = 'ace-920-a1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(a1_status, "open");

    // Positive control: the same claimer, through a1's own event.
    assert_eq!(
        send(
            &state,
            Method::DELETE,
            "/api/v1/events/9201/ace/ace-920-a1/claim",
            &scoped_cookie,
            None
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        drain(&mut rx, topic::ACE),
        1,
        "the write through its own event publishes once"
    );
    let claims: Vec<String> = sqlx::query_scalar(
        "select notes from ace.claims where request_id in ('ace-920-a1', 'ace-920-b1') order by notes",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        claims,
        vec!["bystander on a1".to_string(), "scoped on b1".to_string()],
        "only the caller's a1 claim is released"
    );

    // A national claimer: the same pair on its own claim.
    assert_eq!(
        send(
            &state,
            Method::DELETE,
            "/api/v1/events/9202/ace/ace-920-a3/claim",
            &national_cookie,
            None
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        drain(&mut rx, topic::ACE),
        0,
        "a refused cross-event write publishes nothing"
    );
    let national_on_a3: i64 = sqlx::query_scalar(
        "select count(*) from ace.claims where request_id = 'ace-920-a3' and claimed_by = $1",
    )
    .bind(&national)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        national_on_a3, 1,
        "national scope does not reach across events"
    );
    assert_eq!(
        send(
            &state,
            Method::DELETE,
            "/api/v1/events/9201/ace/ace-920-a3/claim",
            &national_cookie,
            None
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        drain(&mut rx, topic::ACE),
        1,
        "the write through its own event publishes once"
    );
    let national_on_a3: i64 = sqlx::query_scalar(
        "select count(*) from ace.claims where request_id = 'ace-920-a3' and claimed_by = $1",
    )
    .bind(&national)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(national_on_a3, 0);
}

// AC1, AC2, AC3: POST /api/v1/events/{id}/ace/{req}/decide binds the event id on the `for update`
// select, so the update never runs for another event's request.
#[sqlx::test]
async fn decide_answers_404_for_another_events_request_and_leaves_it_open(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let mut rx = state.events.subscribe();
    let requester = seed_user(&pool).await;
    sqlx::query(
        "insert into events.event (id, title, start_time, end_time) values \
         (9201, 'ACE 920 A', now(), now() + interval '2 hours'), \
         (9202, 'ACE 920 B', now(), now() + interval '2 hours')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "insert into ace.requests (id, event_id, artcc_id, position, slots, status, requested_by) values \
         ('ace-920-a1', 9201, 'ZNY', 'NY_CTR', 1, 'open', $1), \
         ('ace-920-b1', 9202, 'ZNY', 'NY_CTR', 1, 'open', $1), \
         ('ace-920-a3', 9201, 'ZNY', 'JFK_TWR', 1, 'open', $1)",
    )
    .bind(&requester)
    .execute(&pool)
    .await
    .unwrap();

    // Unauthenticated.
    assert_eq!(
        send(
            &state,
            Method::POST,
            "/api/v1/events/9201/ace/ace-920-a1/decide",
            "",
            Some(json!({"outcome": "completed"}))
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
    // Signed in, no permission.
    let outsider = seed_user(&pool).await;
    let outsider_cookie = session_cookie(&pool, &outsider).await;
    assert_eq!(
        send(
            &state,
            Method::POST,
            "/api/v1/events/9201/ace/ace-920-a1/decide",
            &outsider_cookie,
            Some(json!({"outcome": "completed"}))
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
    let a1_status: String =
        sqlx::query_scalar("select status from ace.requests where id = 'ace-920-a1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(a1_status, "open", "a refused caller decided nothing");

    // A ZDC-scoped holder, through the wrong event.
    let scoped = seed_user(&pool).await;
    grant(&pool, &scoped, "ace.requests.decide", Some("ZDC")).await;
    let scoped_cookie = session_cookie(&pool, &scoped).await;
    assert_eq!(
        send(
            &state,
            Method::POST,
            "/api/v1/events/9202/ace/ace-920-a1/decide",
            &scoped_cookie,
            Some(json!({"outcome": "completed"}))
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        drain(&mut rx, topic::ACE),
        0,
        "a refused cross-event write publishes nothing"
    );
    let a1: (String, Option<String>, bool, bool) = sqlx::query_as(
        "select status, decided_by, decided_by_actor is null, decided_at is null \
         from ace.requests where id = 'ace-920-a1'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        a1,
        ("open".to_string(), None, true, true),
        "the wrong event's URL left a1 undecided"
    );

    // Positive control: the same holder, through a1's own event.
    assert_eq!(
        send(
            &state,
            Method::POST,
            "/api/v1/events/9201/ace/ace-920-a1/decide",
            &scoped_cookie,
            Some(json!({"outcome": "completed"}))
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        drain(&mut rx, topic::ACE),
        1,
        "the write through its own event publishes once"
    );
    let a1: (String, Option<String>) =
        sqlx::query_as("select status, decided_by from ace.requests where id = 'ace-920-a1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(a1, ("completed".to_string(), Some(scoped.clone())));
    let b1_status: String =
        sqlx::query_scalar("select status from ace.requests where id = 'ace-920-b1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(b1_status, "open", "the other event's request is untouched");

    // A national holder: the same pair on a fresh request, with the other outcome.
    let national = seed_user(&pool).await;
    grant(&pool, &national, "ace.requests.decide", None).await;
    let national_cookie = session_cookie(&pool, &national).await;
    assert_eq!(
        send(
            &state,
            Method::POST,
            "/api/v1/events/9202/ace/ace-920-a3/decide",
            &national_cookie,
            Some(json!({"outcome": "cancelled"}))
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        drain(&mut rx, topic::ACE),
        0,
        "a refused cross-event write publishes nothing"
    );
    let a3: (String, Option<String>) =
        sqlx::query_as("select status, decided_by from ace.requests where id = 'ace-920-a3'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        a3,
        ("open".to_string(), None),
        "national scope does not reach across events"
    );
    assert_eq!(
        send(
            &state,
            Method::POST,
            "/api/v1/events/9201/ace/ace-920-a3/decide",
            &national_cookie,
            Some(json!({"outcome": "cancelled"}))
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        drain(&mut rx, topic::ACE),
        1,
        "the write through its own event publishes once"
    );
    let a3: (String, Option<String>) =
        sqlx::query_as("select status, decided_by from ace.requests where id = 'ace-920-a3'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(a3, ("cancelled".to_string(), Some(national.clone())));
}

// AC5, AC2, AC3: POST /api/v1/events/{id}/ace/{req}/claim resolves the request with the event id
// bound in the lookup (`get_request_in_event`); dropping that predicate turns this test red.
#[sqlx::test]
async fn claim_answers_404_for_another_events_request_and_writes_no_claim(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let mut rx = state.events.subscribe();
    let requester = seed_user(&pool).await;
    sqlx::query(
        "insert into events.event (id, title, start_time, end_time) values \
         (9201, 'ACE 920 A', now(), now() + interval '2 hours'), \
         (9202, 'ACE 920 B', now(), now() + interval '2 hours')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "insert into ace.requests (id, event_id, artcc_id, position, slots, status, requested_by) values \
         ('ace-920-a1', 9201, 'ZNY', 'NY_CTR', 1, 'open', $1), \
         ('ace-920-b1', 9202, 'ZNY', 'NY_CTR', 1, 'open', $1), \
         ('ace-920-a3', 9201, 'ZNY', 'JFK_TWR', 1, 'open', $1)",
    )
    .bind(&requester)
    .execute(&pool)
    .await
    .unwrap();

    // Unauthenticated.
    assert_eq!(
        send(
            &state,
            Method::POST,
            "/api/v1/events/9201/ace/ace-920-a1/claim",
            "",
            Some(json!({}))
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
    // Signed in, no permission.
    let outsider = seed_user(&pool).await;
    let outsider_cookie = session_cookie(&pool, &outsider).await;
    assert_eq!(
        send(
            &state,
            Method::POST,
            "/api/v1/events/9201/ace/ace-920-a1/claim",
            &outsider_cookie,
            Some(json!({}))
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
    let claims: i64 = sqlx::query_scalar("select count(*) from ace.claims")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(claims, 0, "a refused caller wrote no claim");

    // A ZDC-scoped holder, through the wrong event.
    let scoped = seed_user(&pool).await;
    grant(&pool, &scoped, "ace.requests.claim", Some("ZDC")).await;
    let scoped_cookie = session_cookie(&pool, &scoped).await;
    assert_eq!(
        send(
            &state,
            Method::POST,
            "/api/v1/events/9202/ace/ace-920-a1/claim",
            &scoped_cookie,
            Some(json!({}))
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        drain(&mut rx, topic::ACE),
        0,
        "a refused cross-event write publishes nothing"
    );
    let claims: i64 = sqlx::query_scalar("select count(*) from ace.claims")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(claims, 0, "the wrong event's URL wrote no claim");

    // Positive control: the same holder, through a1's own event.
    assert_eq!(
        send(
            &state,
            Method::POST,
            "/api/v1/events/9201/ace/ace-920-a1/claim",
            &scoped_cookie,
            Some(json!({}))
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        drain(&mut rx, topic::ACE),
        1,
        "the write through its own event publishes once"
    );
    let claims: Vec<(String, String)> =
        sqlx::query_as("select request_id, claimed_by from ace.claims order by request_id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(claims, vec![("ace-920-a1".to_string(), scoped.clone())]);

    // A national holder: the same pair on a fresh request.
    let national = seed_user(&pool).await;
    grant(&pool, &national, "ace.requests.claim", None).await;
    let national_cookie = session_cookie(&pool, &national).await;
    assert_eq!(
        send(
            &state,
            Method::POST,
            "/api/v1/events/9202/ace/ace-920-a3/claim",
            &national_cookie,
            Some(json!({}))
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        drain(&mut rx, topic::ACE),
        0,
        "a refused cross-event write publishes nothing"
    );
    let national_claims: i64 =
        sqlx::query_scalar("select count(*) from ace.claims where claimed_by = $1")
            .bind(&national)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        national_claims, 0,
        "national scope does not reach across events"
    );
    assert_eq!(
        send(
            &state,
            Method::POST,
            "/api/v1/events/9201/ace/ace-920-a3/claim",
            &national_cookie,
            Some(json!({}))
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        drain(&mut rx, topic::ACE),
        1,
        "the write through its own event publishes once"
    );
    let national_claims: Vec<String> =
        sqlx::query_scalar("select request_id from ace.claims where claimed_by = $1")
            .bind(&national)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(national_claims, vec!["ace-920-a3".to_string()]);
}
