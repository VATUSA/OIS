//! VATUSA/OIS#746: an unpublished event FCA (`planned` or `archived`) isn't live, so the four
//! live-operation FCA routes — reorder, mark a release, clear a release, swap two releases — answer
//! `404` for it to anyone who isn't an event planner, and write nothing. The check runs before the
//! ARTCC scope check, so a non-planner at another facility gets the `404` a missing id gets rather than
//! a `403` that confirms the FCA exists. An event planner keeps all four on a planned or archived FCA
//! (owner decision: the rule #736 applies to `PUT`/`DELETE`, with the same planner test). A published
//! event FCA and an ordinary FCA behave as before.
//!
//! Everything goes through the real router with a feed that has real crossings, so each allowed call
//! actually writes and each refused one would have written. A handler that stops calling the guard,
//! or calls it after the scope check, turns these red. Each route gets its own tests, so a failure
//! names the route.

use std::collections::HashMap;
use std::sync::Arc;

use http::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::auth::principal::Attribution;
use crate::feed::airports::Airport;
use crate::feed::vatsim::{FlightPlan, Prefile, VatsimData};
use crate::repos::flow as flow_repo;
use crate::scope_test_support::{grant, seed_user, session_cookie, test_state};
use crate::state::AppState;

/// The event the hidden FCAs and the published neighbour belong to (not #736's 7360).
const EVENT: i64 = 7461;
const PLANNED: &str = "vis-planned";
const ARCHIVED: &str = "vis-archived";
/// Neighbour for the status predicate: same event, but published.
const PUBLISHED: &str = "vis-published";
/// Neighbour for the event predicate: same ARTCC, no event.
const ORDINARY: &str = "vis-ordinary";
const HIDDEN: [&str; 2] = [PLANNED, ARCHIVED];
const LIVE: [&str; 2] = [PUBLISHED, ORDINARY];

/// AAL1's and AAL2's seeded releases, which the swap trades; AAL3 crosses with no release, for mark.
const AAL1: (i64, i64) = (1_000, 900);
const AAL2: (i64, i64) = (2_000, 1_900);

/// The four routes #746 guards.
#[derive(Clone, Copy, Debug)]
enum Op {
    Reorder,
    Mark,
    Clear,
    Swap,
}

impl Op {
    fn request(self, id: &str) -> (Method, String, Option<Value>) {
        let base = format!("/api/v1/flow/fcas/{id}");
        match self {
            Op::Reorder => (
                Method::PUT,
                format!("{base}/order"),
                Some(json!({ "order": ["AAL2", "AAL1"] })),
            ),
            Op::Mark => (
                Method::POST,
                format!("{base}/release/AAL3"),
                Some(json!({})),
            ),
            Op::Clear => (Method::DELETE, format!("{base}/release/AAL1"), None),
            Op::Swap => (
                Method::POST,
                format!("{base}/swap"),
                Some(json!({ "a": "AAL1", "b": "AAL2" })),
            ),
        }
    }
}

/// Send `op` on FCA `id` through the real router; the status and the error code, if any.
async fn call(state: &AppState, op: Op, id: &str, cookie: &str) -> (StatusCode, Option<String>) {
    use tower::ServiceExt;
    let (method, uri, body) = op.request(id);
    let builder = http::Request::builder()
        .method(method)
        .uri(uri)
        .header(http::header::COOKIE, cookie);
    let request = match body {
        Some(body) => builder
            .header(http::header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(body.to_string())),
        None => builder.body(axum::body::Body::empty()),
    }
    .unwrap();
    let response = crate::router::build_router(state.clone())
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let code = serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|v| v["error"].as_str().map(str::to_string));
    (status, code)
}

/// Event 7461 with a planned, a published and an archived FCA at ZDC, plus an ordinary ZDC FCA. All
/// four are the JFK→DCA corridor between WHITE and SIE (as `machine_actor_tests`), and each holds
/// releases for AAL1 and AAL2, so "nothing changed" is non-vacuous for clear and swap.
///
/// The returned state's feed holds AAL1–AAL3 as KJFK→KDCA prefiles, all crossing every FCA, with
/// AAL1 and AAL2 assigned runway 31L — so mark and swap can succeed, not just get past the guard.
async fn seed(pool: &PgPool) -> AppState {
    sqlx::query(
        "insert into events.event (id, title, start_time, end_time) values \
           ($1, 'Event FCA visibility', now() + interval '1 day', now() + interval '2 days')",
    )
    .bind(EVENT)
    .execute(pool)
    .await
    .unwrap();
    for (id, event, status) in [
        (PLANNED, Some(EVENT), Some("planned")),
        (PUBLISHED, Some(EVENT), Some("published")),
        (ARCHIVED, Some(EVENT), Some("archived")),
        (ORDINARY, None, None),
    ] {
        sqlx::query(
            "insert into flow.fca (id, name, color, artcc, points, dests, origins, fixes, scope, \
                 dir, mode, rate, mit, enabled, event_id, event_status) \
             values ($1, $1, '#fff', 'ZDC', $2, '{}', '{}', '{}', '{}', 'any', 'rate', 30, 0, \
                 true, $3, $4)",
        )
        .bind(id)
        .bind(json!([[39.5, -75.6], [39.5, -74.0]]))
        .bind(event)
        .bind(status)
        .execute(pool)
        .await
        .unwrap();
    }
    let owner = seed_user(pool).await;
    let by = Attribution {
        user_id: Some(owner),
        actor_id: None,
    };
    for id in HIDDEN.into_iter().chain(LIVE) {
        for (callsign, (cta, edct)) in [("AAL1", AAL1), ("AAL2", AAL2)] {
            flow_repo::upsert_release(pool, id, callsign, cta, edct, &by, None)
                .await
                .unwrap()
                .unwrap();
        }
    }
    for callsign in ["AAL1", "AAL2"] {
        crate::repos::departure_runway::assign(
            pool,
            "KJFK",
            callsign,
            "31L",
            crate::repos::departure_runway::RunwaySource::Config,
            None,
        )
        .await
        .unwrap();
    }

    let state = test_state(pool.clone(), Default::default());
    {
        let mut feed = state.feed.write().await;
        feed.airports = Arc::new(HashMap::from([
            ("KJFK".to_string(), Airport::at(40.64, -73.78)),
            ("KDCA".to_string(), Airport::at(38.85, -77.04)),
        ]));
        feed.snapshot = Some(Arc::new(crate::feed::Snapshot::of(VatsimData {
            prefiles: ["AAL1", "AAL2", "AAL3"]
                .into_iter()
                .map(|callsign| Prefile {
                    callsign: callsign.into(),
                    flight_plan: Some(FlightPlan {
                        departure: "KJFK".into(),
                        arrival: "KDCA".into(),
                        route: "RBV WHITE SIE".into(),
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        })));
    }
    state
}

/// An FCA's sequencing state and every release on it, to prove a refused call wrote nothing.
#[derive(Debug, PartialEq)]
struct Snapshot {
    manual_order: Vec<String>,
    manual_seq: bool,
    releases: Vec<(String, i64, i64, i64)>,
}

impl Snapshot {
    fn release(&self, callsign: &str) -> Option<(i64, i64)> {
        self.releases
            .iter()
            .find(|r| r.0 == callsign)
            .map(|r| (r.1, r.2))
    }
}

async fn snapshot(pool: &PgPool, id: &str) -> Snapshot {
    let (manual_order, manual_seq): (Vec<String>, bool) =
        sqlx::query_as("select manual_order, manual_seq from flow.fca where id = $1")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap();
    let releases = sqlx::query_as(
        "select callsign, cta_ms, edct_ms, version from flow.fca_release \
          where fca_id = $1 order by callsign",
    )
    .bind(id)
    .fetch_all(pool)
    .await
    .unwrap();
    Snapshot {
        manual_order,
        manual_seq,
        releases,
    }
}

/// A CONTROLLER at `artcc`, granted the way the VATUSA sync grants it (#730), as a session cookie.
/// Not an event planner.
async fn controller(pool: &PgPool, artcc: &str) -> String {
    use crate::repos::access::{GrantSource, set_user_role_scoped};
    let user = seed_user(pool).await;
    let mut tx = pool.begin().await.unwrap();
    set_user_role_scoped(
        &mut tx,
        &user,
        "CONTROLLER",
        true,
        Some(artcc),
        GrantSource::Vatusa,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    session_cookie(pool, &user).await
}

/// An event planner for `artcc`'s events who may also run `artcc`'s FCAs, as #736's planner test
/// grants it.
async fn planner(pool: &PgPool, artcc: &str) -> String {
    let user = seed_user(pool).await;
    for p in ["events.plan.update", "flow.fca.update"] {
        grant(pool, &user, p, Some(artcc)).await;
    }
    session_cookie(pool, &user).await
}

/// `op` on `id` succeeded and wrote what it writes: reorder `204` and the order stored, mark `200` and
/// AAL3 released, clear `200` and AAL1's release gone, swap `200` and AAL1/AAL2's times traded.
async fn assert_operable(pool: &PgPool, state: &AppState, op: Op, id: &str, cookie: &str) {
    let before = snapshot(pool, id).await;
    let got = call(state, op, id, cookie).await;
    let after = snapshot(pool, id).await;
    match op {
        Op::Reorder => {
            assert_eq!(got, (StatusCode::NO_CONTENT, None), "{op:?} on {id}");
            assert_eq!(after.manual_order, ["AAL2", "AAL1"], "{id}'s order written");
            assert!(after.manual_seq, "{id} is in manual mode");
            assert_eq!(after.releases, before.releases);
        }
        Op::Mark => {
            assert_eq!(got, (StatusCode::OK, None), "{op:?} on {id}");
            assert!(before.release("AAL3").is_none());
            assert!(after.release("AAL3").is_some(), "{id}'s AAL3 released");
            assert_eq!(after.release("AAL1"), before.release("AAL1"));
        }
        Op::Clear => {
            assert_eq!(got, (StatusCode::OK, None), "{op:?} on {id}");
            let left: Vec<_> = after.releases.iter().map(|r| r.0.as_str()).collect();
            assert_eq!(left, ["AAL2"], "{id}'s AAL1 release cleared");
        }
        Op::Swap => {
            assert_eq!(got, (StatusCode::OK, None), "{op:?} on {id}");
            assert_eq!(after.release("AAL1"), Some(AAL2), "{id}'s times traded");
            assert_eq!(after.release("AAL2"), Some(AAL1), "{id}'s times traded");
        }
    }
}

/// AC1: a ZDC controller — in scope for ZDC's FCAs, not an event planner — gets `404` on a planned and
/// on an archived event FCA, and neither FCA's order nor its releases change.
async fn hidden_fcas_404_in_scope(pool: PgPool, op: Op) {
    let state = seed(&pool).await;
    let zdc = controller(&pool, "ZDC").await;
    for id in HIDDEN {
        let before = snapshot(&pool, id).await;
        assert_eq!(
            call(&state, op, id, &zdc).await.0,
            StatusCode::NOT_FOUND,
            "{op:?} on {id}"
        );
        assert_eq!(
            snapshot(&pool, id).await,
            before,
            "{op:?} left {id} unchanged"
        );
    }
}

/// AC1: a ZTL controller gets the same `404` on ZDC's hidden event FCAs as on an id that doesn't
/// exist — not the `403` the scope check would give, which would confirm the FCA exists.
async fn hidden_fcas_404_out_of_scope_like_a_missing_id(pool: PgPool, op: Op) {
    let state = seed(&pool).await;
    let ztl = controller(&pool, "ZTL").await;
    let missing = call(&state, op, "no-such-fca", &ztl).await;
    assert_eq!(missing.0, StatusCode::NOT_FOUND, "{op:?} on a missing id");
    for id in HIDDEN {
        let before = snapshot(&pool, id).await;
        assert_eq!(call(&state, op, id, &ztl).await, missing, "{op:?} on {id}");
        assert_eq!(
            snapshot(&pool, id).await,
            before,
            "{op:?} left {id} unchanged"
        );
    }
}

/// AC2: a published event FCA and an ordinary FCA stay operable by a ZDC controller.
async fn live_fcas_stay_operable(pool: PgPool, op: Op) {
    let state = seed(&pool).await;
    let zdc = controller(&pool, "ZDC").await;
    for id in LIVE {
        assert_operable(&pool, &state, op, id, &zdc).await;
    }
}

/// AC2: the ARTCC scope check still runs on a live FCA — a ZTL controller gets `403` on ZDC's
/// published and ordinary FCAs, and nothing changes.
async fn live_fcas_403_out_of_scope(pool: PgPool, op: Op) {
    let state = seed(&pool).await;
    let ztl = controller(&pool, "ZTL").await;
    for id in LIVE {
        let before = snapshot(&pool, id).await;
        assert_eq!(
            call(&state, op, id, &ztl).await.0,
            StatusCode::FORBIDDEN,
            "{op:?} on {id}"
        );
        assert_eq!(
            snapshot(&pool, id).await,
            before,
            "{op:?} left {id} unchanged"
        );
    }
}

/// Owner decision on #746: an event planner keeps all four operations on a planned and on an archived
/// event FCA — the exemption #736 gives planners on `PUT`/`DELETE`.
async fn a_planner_operates_hidden_fcas(pool: PgPool, op: Op) {
    let state = seed(&pool).await;
    let cookie = planner(&pool, "ZDC").await;
    for id in HIDDEN {
        assert_operable(&pool, &state, op, id, &cookie).await;
    }
}

/// #736's planner test is "holds `events.plan.update` anywhere", and #746 reuses it verbatim, so a
/// planner whose grants are all at ZTL passes the visibility check on ZDC's hidden FCAs and is then
/// refused by the ARTCC scope check: `403`, as on `PUT`/`DELETE`, and nothing changes.
async fn a_planner_elsewhere_gets_403_on_hidden_fcas(pool: PgPool, op: Op) {
    let state = seed(&pool).await;
    let ztl = planner(&pool, "ZTL").await;
    for id in HIDDEN {
        let before = snapshot(&pool, id).await;
        assert_eq!(
            call(&state, op, id, &ztl).await.0,
            StatusCode::FORBIDDEN,
            "{op:?} on {id}"
        );
        assert_eq!(
            snapshot(&pool, id).await,
            before,
            "{op:?} left {id} unchanged"
        );
    }
}

/// One module per route, so a failing test names the route whose check broke.
macro_rules! route_tests {
    ($route:ident, $op:expr) => {
        mod $route {
            use sqlx::PgPool;

            #[sqlx::test]
            async fn hidden_fcas_404_in_scope(pool: PgPool) {
                super::hidden_fcas_404_in_scope(pool, $op).await;
            }

            #[sqlx::test]
            async fn hidden_fcas_404_out_of_scope_like_a_missing_id(pool: PgPool) {
                super::hidden_fcas_404_out_of_scope_like_a_missing_id(pool, $op).await;
            }

            #[sqlx::test]
            async fn live_fcas_stay_operable(pool: PgPool) {
                super::live_fcas_stay_operable(pool, $op).await;
            }

            #[sqlx::test]
            async fn live_fcas_403_out_of_scope(pool: PgPool) {
                super::live_fcas_403_out_of_scope(pool, $op).await;
            }

            #[sqlx::test]
            async fn a_planner_operates_hidden_fcas(pool: PgPool) {
                super::a_planner_operates_hidden_fcas(pool, $op).await;
            }

            #[sqlx::test]
            async fn a_planner_elsewhere_gets_403_on_hidden_fcas(pool: PgPool) {
                super::a_planner_elsewhere_gets_403_on_hidden_fcas(pool, $op).await;
            }
        }
    };
}

route_tests!(reorder, super::Op::Reorder);
route_tests!(mark_release, super::Op::Mark);
route_tests!(clear_release, super::Op::Clear);
route_tests!(swap_releases, super::Op::Swap);
