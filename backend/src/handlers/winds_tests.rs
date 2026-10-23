//! VATUSA/OIS#836: the winds source on the routes that report or replay it, through the real
//! router.
//!
//! `GET /api/v1/flow/data-status` is public by design (`auth_annotation_tests::PUBLIC`), so its
//! requests carry no cookie.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use chrono::{DateTime, Utc};
use http::{Method, StatusCode};
use sqlx::PgPool;

use crate::feed::winds::Winds;
use crate::scope_test_support::{grant, seed_user, send_json, session_cookie, test_state};

const STATUS: &str = "/api/v1/flow/data-status";

fn at(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

// data-status carries the source and the RAP valid time.
#[sqlx::test]
async fn data_status_reports_a_rap_field_and_its_valid_time(pool: PgPool) {
    let state = test_state(pool, HashMap::new());
    let valid = at("2026-10-16T01:00:00Z");
    state.winds.store(Arc::new(Winds::test_grid(
        (38.0, -78.0),
        (2, 2),
        &[34_000.0],
        valid,
        |_, _, _| (-40, 0, -45),
    )));
    state.winds_refreshed.store(
        at("2026-10-16T01:20:00Z").timestamp_millis(),
        Ordering::Relaxed,
    );
    let (status, body) = send_json(&state, Method::GET, STATUS, "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["winds_source"], "rap");
    assert_eq!(body["winds_valid_at"], "2026-10-16T01:00:00Z");
    assert_eq!(body["winds_refreshed"], "2026-10-16T01:20:00Z");
    assert!(
        body.get("winds_stations").is_none(),
        "the FB station count is gone"
    );
}

// Still air: source "none".
#[sqlx::test]
async fn data_status_reports_no_source_for_still_air(pool: PgPool) {
    let state = test_state(pool, HashMap::new());
    state.winds.store(Arc::new(Winds::default()));
    let (status, body) = send_json(&state, Method::GET, STATUS, "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["winds_source"], "none");
    assert!(body["winds_valid_at"].is_null());
    assert!(body["winds_refreshed"].is_null());
}

// AC4 through the route the replay dashboard calls: a pre-#836 `stats.winds` row must not turn
// historical flow into a 500.
#[sqlx::test]
async fn historical_flow_replays_over_a_snapshot_taken_before_rap(pool: PgPool) {
    let insert = |ts: &'static str, json: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query("insert into stats.winds (ts, data) values ($1::timestamptz, $2::jsonb)")
                .bind(ts)
                .bind(json)
                .execute(&pool)
                .await
                .unwrap();
        }
    };
    insert(
        "2026-10-15T18:00:00Z",
        r#"{"stations":[{"lat":40.0,"lon":-90.0,"levels":[[3000,{"dir":180.0,"spd":17.0}]]}]}"#,
    )
    .await;
    insert("2026-10-15T19:00:00Z", r#"{"stations":"x"}"#).await;
    let state = test_state(pool.clone(), HashMap::new());
    let user = seed_user(&pool).await;
    grant(&pool, &user, "stats.data.read", None).await;
    let cookie = session_cookie(&pool, &user).await;
    let uri = |t: &str| format!("/api/v1/stats/hist/flow/KDCA?at={}", at(t).timestamp());

    let (status, body) =
        send_json(&state, Method::GET, &uri("2026-10-15T18:30:00Z"), &cookie).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // Positive control: the route really decodes the snapshot, so a malformed one fails it.
    let (bad, _) = send_json(&state, Method::GET, &uri("2026-10-15T19:30:00Z"), &cookie).await;
    assert_eq!(bad, StatusCode::INTERNAL_SERVER_ERROR);
}
