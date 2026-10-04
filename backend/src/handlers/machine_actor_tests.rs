//! VATUSA/OIS#583: a machine credential — a service account (`ois_sa_`) or a user's API key
//! (`ois_pat_`) — can drive the release path, and what it writes names the machine.
//!
//! Every test goes through the real router, so the bearer middleware, `RequirePermission`, the
//! [`Actor`](crate::auth::principal::Actor) extractor and the audit layer are all on the path. Against
//! the code before #583, every service-account and API-key test here gets **401**: the credential
//! cleared `RequirePermission` and was then refused for not being a session user.

use std::collections::HashMap;
use std::sync::Arc;

use axum::http;
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::auth::principal::Principal;
use crate::feed::airports::Airport;
use crate::feed::vatsim::{FlightPlan, Prefile, VatsimData};
use crate::repos::access::{PermissionScope, sha256_hex};
use crate::scope_test_support::{grant, seed_user, session_cookie, test_state};
use crate::state::AppState;

const ROLE: &str = "T583_MACHINE";

/// A service account holding `permission` through a role scoped to `artcc` (`None` = national).
/// Returns `(account id, bearer token)`.
async fn service_account(pool: &PgPool, permission: &str, artcc: Option<&str>) -> (String, String) {
    sqlx::query(
        "insert into access.roles (name, is_system) values ($1, false) on conflict do nothing",
    )
    .bind(ROLE)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "insert into access.role_permissions (role_name, permission_name) values ($1, $2) \
         on conflict do nothing",
    )
    .bind(ROLE)
    .bind(permission)
    .execute(pool)
    .await
    .unwrap();
    let id: String = sqlx::query_scalar(
        "insert into access.service_accounts (key, name) values ('vtbfm', 'vTBFM') returning id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let token = format!("ois_sa_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "insert into access.service_account_credentials (service_account_id, secret_hash) \
         values ($1, $2)",
    )
    .bind(&id)
    .bind(sha256_hex(&token))
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "insert into access.service_account_roles (service_account_id, role_name, artcc_id) \
         values ($1, $2, $3)",
    )
    .bind(&id)
    .bind(ROLE)
    .bind(artcc)
    .execute(pool)
    .await
    .unwrap();
    (id, format!("Bearer {token}"))
}

/// A service account that exists and authenticates but holds nothing.
async fn bare_service_account(pool: &PgPool) -> String {
    let id: String = sqlx::query_scalar(
        "insert into access.service_accounts (key, name) values ('bare', 'Bare') returning id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let token = format!("ois_sa_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "insert into access.service_account_credentials (service_account_id, secret_hash) \
         values ($1, $2)",
    )
    .bind(&id)
    .bind(sha256_hex(&token))
    .execute(pool)
    .await
    .unwrap();
    format!("Bearer {token}")
}

/// A user's API key granted `permission`, its owner holding it too (a key is capped by its owner).
/// Returns `(key id, bearer token)`.
async fn api_key(pool: &PgPool, permission: &str) -> (String, String) {
    let owner = seed_user(pool).await;
    grant(pool, &owner, permission, None).await;
    let token = format!("ois_pat_{}", uuid::Uuid::new_v4().simple());
    let id: String = sqlx::query_scalar(
        "insert into access.api_keys (owner_user_id, name, prefix, secret_hash) \
         values ($1, 'vTBFM key', 'ois_pat_t583', $2) returning id",
    )
    .bind(&owner)
    .bind(sha256_hex(&token))
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "insert into access.api_key_permissions (api_key_id, permission_name) values ($1, $2)",
    )
    .bind(&id)
    .bind(permission)
    .execute(pool)
    .await
    .unwrap();
    (id, format!("Bearer {token}"))
}

/// The `access.actors` row for a principal, by the id column that names it.
async fn actor_of(pool: &PgPool, column: &str, id: &str) -> Option<String> {
    sqlx::query_scalar(&format!("select id from access.actors where {column} = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .unwrap()
}

/// One request through the real router, authenticated by `Authorization` (a bearer) or `Cookie`.
async fn call(
    state: &AppState,
    method: http::Method,
    uri: &str,
    auth: &str,
    body: Option<Value>,
) -> (http::StatusCode, Value) {
    call_with(state, method, uri, &[auth], body).await
}

/// [`call`] carrying several entries at once: a bearer (`Bearer …` → `Authorization`), a session
/// (`ois_session=…` → `Cookie`), or any other header written `Name: value` — e.g. `If-None-Match: *`.
async fn call_with(
    state: &AppState,
    method: http::Method,
    uri: &str,
    auth: &[&str],
    body: Option<Value>,
) -> (http::StatusCode, Value) {
    use tower::ServiceExt;

    let mut builder = http::Request::builder().method(method).uri(uri);
    for entry in auth {
        builder = if entry.starts_with("Bearer ") {
            builder.header(http::header::AUTHORIZATION, *entry)
        } else if entry.starts_with("ois_session=") {
            builder.header(http::header::COOKIE, *entry)
        } else {
            let (name, value) = entry.split_once(": ").expect("`Name: value`");
            builder.header(name, value)
        };
    }
    let request = match body {
        Some(b) => builder
            .header(http::header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(b.to_string())),
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
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn issue_body() -> Value {
    json!({
        "callsign": "AAL1",
        "airport": "KDCA",
        "ready_time": (Utc::now() + Duration::hours(1)).to_rfc3339(),
    })
}

/// `(issued_by, issued_by_actor)` for a CFR.
async fn cfr_attribution(pool: &PgPool, callsign: &str) -> (Option<String>, Option<String>) {
    sqlx::query_as("select issued_by, issued_by_actor from tmu.issued_cfrs where callsign = $1")
        .bind(callsign)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// The audit entry's actor for the newest write to `resource_type`.
async fn audited_actor(pool: &PgPool) -> Option<String> {
    sqlx::query_scalar("select actor_id from access.audit_logs order by created_at desc limit 1")
        .fetch_optional(pool)
        .await
        .unwrap()
        .flatten()
}

/// A state whose feed holds one prefile, `TEST1` (KJFK→KDCA via RBV WHITE SIE), that crosses
/// [`fca`] — so `mark_release` has a real crossing to release.
async fn crossing_state(pool: PgPool) -> AppState {
    crossing_state_of(pool, &["TEST1"]).await
}

/// [`crossing_state`] with one such prefile per callsign.
async fn crossing_state_of(pool: PgPool, callsigns: &[&str]) -> AppState {
    let state = test_state(pool, HashMap::new());
    {
        let mut feed = state.feed.write().await;
        feed.airports = Arc::new(HashMap::from([
            ("KJFK".to_string(), Airport::at(40.64, -73.78)),
            ("KDCA".to_string(), Airport::at(38.85, -77.04)),
        ]));
        feed.snapshot = Some(Arc::new(crate::feed::Snapshot::of(VatsimData {
            prefiles: callsigns
                .iter()
                .map(|callsign| Prefile {
                    callsign: (*callsign).into(),
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

/// A crossing FCA (the JFK→DCA corridor between WHITE and SIE) and its id.
async fn fca(pool: &PgPool) -> String {
    sqlx::query_scalar(
        "insert into flow.fca (name, color, artcc, points, dests, origins, fixes, scope, \
             dir, mode, rate, mit, enabled) \
         values ('T583', '#fff', 'ZDC', $1, '{}', '{}', '{}', '{}', 'any', 'rate', 30, 0, true) \
         returning id",
    )
    .bind(json!([[39.5, -75.6], [39.5, -74.0]]))
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn seed_release(pool: &PgPool, fca_id: &str, callsign: &str, cta: i64) {
    sqlx::query(
        "insert into flow.fca_release (fca_id, callsign, cta_ms, edct_ms) values ($1, $2, $3, $3)",
    )
    .bind(fca_id)
    .bind(callsign)
    .bind(cta)
    .execute(pool)
    .await
    .unwrap();
}

/// A release `service_account` holds — written by it, so a machine acting on its own release is
/// exercised rather than refused as a person's (#585).
async fn seed_owned_release(
    pool: &PgPool,
    fca_id: &str,
    callsign: &str,
    cta: i64,
    service_account: &str,
) {
    let actor =
        crate::repos::audit::resolve_service_account_actor_id(pool, service_account, "vTBFM")
            .await
            .unwrap()
            .unwrap();
    sqlx::query(
        "insert into flow.fca_release (fca_id, callsign, cta_ms, edct_ms, updated_by_actor) \
         values ($1, $2, $3, $3, $4)",
    )
    .bind(fca_id)
    .bind(callsign)
    .bind(cta)
    .bind(actor)
    .execute(pool)
    .await
    .unwrap();
}

/// `(updated_by, updated_by_actor)` for a release.
async fn release_attribution(
    pool: &PgPool,
    fca_id: &str,
    callsign: &str,
) -> Option<(Option<String>, Option<String>)> {
    sqlx::query_as(
        "select updated_by, updated_by_actor from flow.fca_release \
         where fca_id = $1 and callsign = $2",
    )
    .bind(fca_id)
    .bind(callsign)
    .fetch_optional(pool)
    .await
    .unwrap()
}

// ---- issue / release a CFR -------------------------------------------------------------------------

/// AC1 + AC2 + AC6: the row, the response and the audit entry all name the service account.
#[sqlx::test]
async fn a_service_account_issues_a_cfr_attributed_to_itself(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let (sa, auth) = service_account(&pool, "tmu.cfr.assign", None).await;

    let (status, body) = call_with(
        &state,
        http::Method::POST,
        "/api/v1/tmu/cfr",
        &[&auth, "If-None-Match: *"],
        Some(issue_body()),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "{body}");

    let actor = actor_of(&pool, "service_account_id", &sa)
        .await
        .expect("an actor was resolved");
    assert_eq!(
        cfr_attribution(&pool, "AAL1").await,
        (None, Some(actor.clone())),
        "named the machine, not a person"
    );
    assert_eq!(body["issued_by"], "vTBFM", "the response names it too");
    assert_eq!(
        audited_actor(&pool).await,
        Some(actor),
        "and so does the audit entry"
    );
}

#[sqlx::test]
async fn an_api_key_issues_a_cfr_attributed_to_the_key(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let (key, auth) = api_key(&pool, "tmu.cfr.assign").await;

    let (status, body) = call_with(
        &state,
        http::Method::POST,
        "/api/v1/tmu/cfr",
        &[&auth, "If-None-Match: *"],
        Some(issue_body()),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "{body}");

    let actor = actor_of(&pool, "api_key_id", &key)
        .await
        .expect("an actor was resolved");
    assert_eq!(
        cfr_attribution(&pool, "AAL1").await,
        (None, Some(actor.clone()))
    );
    assert_eq!(audited_actor(&pool).await, Some(actor));
}

/// A machine releases the CFR it issued, at the version it was issued at (#585).
#[sqlx::test]
async fn a_service_account_releases_a_cfr(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let (_, auth) = service_account(&pool, "tmu.cfr.assign", None).await;
    assert_eq!(
        call_with(
            &state,
            http::Method::POST,
            "/api/v1/tmu/cfr",
            &[&auth, "If-None-Match: *"],
            Some(issue_body())
        )
        .await
        .0,
        http::StatusCode::OK
    );

    let (status, _) = call_with(
        &state,
        http::Method::DELETE,
        "/api/v1/tmu/cfr/AAL1",
        &[&auth, "If-Match: \"1\""],
        None,
    )
    .await;
    assert_eq!(status, http::StatusCode::NO_CONTENT);
}

/// No regression for people: a session still names its user in the legacy column, and its actor.
#[sqlx::test]
async fn a_user_session_still_attributes_the_user(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let user = seed_user(&pool).await;
    grant(&pool, &user, "tmu.cfr.assign", None).await;
    let cookie = session_cookie(&pool, &user).await;

    let (status, body) = call(
        &state,
        http::Method::POST,
        "/api/v1/tmu/cfr",
        &cookie,
        Some(issue_body()),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "{body}");

    let actor = actor_of(&pool, "user_id", &user).await;
    assert!(actor.is_some());
    assert_eq!(cfr_attribution(&pool, "AAL1").await, (Some(user), actor));
    assert_eq!(body["issued_by"], "Scope Test User");
}

/// #583 review: a request can carry a session cookie **and** a service-account bearer — the middleware
/// resolves both, and `ensure_permission` authorises the **user**. The write must then be attributed
/// to that user too. Otherwise anyone holding a service-account token could make their own actions
/// look machine-made, in the row and in the audit log.
#[sqlx::test]
async fn a_request_carrying_a_session_and_a_service_account_is_the_users(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let user = seed_user(&pool).await;
    grant(&pool, &user, "tmu.cfr.assign", None).await;
    let cookie = session_cookie(&pool, &user).await;
    let (sa, bearer) = service_account(&pool, "tmu.cfr.assign", None).await;

    let (status, body) = call_with(
        &state,
        http::Method::POST,
        "/api/v1/tmu/cfr",
        &[&cookie, &bearer],
        Some(issue_body()),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "{body}");

    let user_actor = actor_of(&pool, "user_id", &user).await;
    assert!(user_actor.is_some());
    assert_eq!(
        cfr_attribution(&pool, "AAL1").await,
        (Some(user), user_actor.clone()),
        "the user who was authorised is the one named"
    );
    assert_eq!(audited_actor(&pool).await, user_actor);
    assert_eq!(
        actor_of(&pool, "service_account_id", &sa).await,
        None,
        "the service account was never acted as"
    );
}

// ---- mark / swap / clear a release ----------------------------------------------------------------

/// AC6 for `mark_release`: a real crossing in the feed, so the write itself runs, not only the gate.
#[sqlx::test]
async fn a_service_account_marks_a_release_attributed_to_itself(pool: PgPool) {
    let state = crossing_state(pool.clone()).await;
    let fca_id = fca(&pool).await;
    let (sa, auth) = service_account(&pool, "flow.fca.update", None).await;

    let (status, body) = call_with(
        &state,
        http::Method::POST,
        &format!("/api/v1/flow/fcas/{fca_id}/release/TEST1"),
        &[&auth, "If-None-Match: *"],
        Some(json!({})),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "{body}");

    let actor = actor_of(&pool, "service_account_id", &sa).await;
    assert!(actor.is_some());
    assert_eq!(
        release_attribution(&pool, &fca_id, "TEST1").await,
        Some((None, actor))
    );
}

#[sqlx::test]
async fn a_service_account_swaps_releases_attributed_to_itself(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let fca_id = fca(&pool).await;
    let (sa, auth) = service_account(&pool, "flow.fca.update", None).await;
    seed_owned_release(&pool, &fca_id, "AAL1", 1_000, &sa).await;
    seed_owned_release(&pool, &fca_id, "UAL2", 2_000, &sa).await;

    let (status, _) = call(
        &state,
        http::Method::POST,
        &format!("/api/v1/flow/fcas/{fca_id}/swap"),
        &auth,
        Some(json!({"a": "AAL1", "b": "UAL2"})),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK);

    let actor = actor_of(&pool, "service_account_id", &sa).await;
    assert!(actor.is_some());
    for cs in ["AAL1", "UAL2"] {
        assert_eq!(
            release_attribution(&pool, &fca_id, cs).await,
            Some((None, actor.clone()))
        );
    }
}

/// A machine clears its own release, at the version it last saw (#585).
#[sqlx::test]
async fn a_service_account_clears_a_release(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let fca_id = fca(&pool).await;
    let (sa, auth) = service_account(&pool, "flow.fca.update", None).await;
    seed_owned_release(&pool, &fca_id, "AAL1", 1_000, &sa).await;

    let (status, _) = call_with(
        &state,
        http::Method::DELETE,
        &format!("/api/v1/flow/fcas/{fca_id}/release/AAL1"),
        &[&auth, "If-Match: \"1\""],
        None,
    )
    .await;
    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(release_attribution(&pool, &fca_id, "AAL1").await, None);
}

// ---- AC5: no widening -----------------------------------------------------------------------------

#[sqlx::test]
async fn a_service_account_without_the_permission_is_refused(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let auth = bare_service_account(&pool).await;

    let (status, _) = call(
        &state,
        http::Method::POST,
        "/api/v1/tmu/cfr",
        &auth,
        Some(issue_body()),
    )
    .await;
    assert_eq!(status, http::StatusCode::UNAUTHORIZED);
    let rows: i64 = sqlx::query_scalar("select count(*) from tmu.issued_cfrs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

/// A role held at ZDC scopes the account to ZDC; only a role held with no ARTCC is national.
#[sqlx::test]
async fn a_service_account_scope_honours_its_roles_artcc(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let (sa, _) = service_account(&pool, "flow.fca.update", Some("ZDC")).await;
    let principal = Principal::ServiceAccount(crate::auth::context::CurrentServiceAccount {
        id: sa.clone(),
        key: "vtbfm".into(),
        name: "vTBFM".into(),
    });

    let scope = principal
        .permission_scope(&state, "flow.fca.update")
        .await
        .unwrap();
    assert!(
        !matches!(scope, PermissionScope::National { .. }),
        "a ZDC role is not national"
    );
    assert!(scope.allows(Some("ZDC")));
    assert!(!scope.allows(Some("ZNY")));
    assert!(!scope.allows(None));

    let unheld = principal
        .permission_scope(&state, "tmu.cfr.assign")
        .await
        .unwrap();
    assert!(
        !unheld.allows(Some("ZDC")),
        "a permission it holds nowhere covers nothing"
    );

    sqlx::query(
        "update access.service_account_roles set artcc_id = null where service_account_id = $1",
    )
    .bind(&sa)
    .execute(&pool)
    .await
    .unwrap();
    let national = principal
        .permission_scope(&state, "flow.fca.update")
        .await
        .unwrap();
    assert!(matches!(national, PermissionScope::National { .. }));
}

// ==== VATUSA/OIS#585: authority, idempotency and conflict for external release writers =============

/// One request's status, `ETag` and body.
async fn send_full(
    state: &AppState,
    method: http::Method,
    uri: &str,
    entries: &[&str],
    body: Option<Value>,
) -> (http::StatusCode, Option<String>, Value) {
    use tower::ServiceExt;

    let mut builder = http::Request::builder().method(method).uri(uri);
    for entry in entries {
        builder = if entry.starts_with("Bearer ") {
            builder.header(http::header::AUTHORIZATION, *entry)
        } else if entry.starts_with("ois_session=") {
            builder.header(http::header::COOKIE, *entry)
        } else {
            let (name, value) = entry.split_once(": ").expect("`Name: value`");
            builder.header(name, value)
        };
    }
    let request = match body {
        Some(b) => builder
            .header(http::header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(b.to_string())),
        None => builder.body(axum::body::Body::empty()),
    }
    .unwrap();
    let response = crate::router::build_router(state.clone())
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let etag = response
        .headers()
        .get(http::header::ETAG)
        .map(|v| v.to_str().unwrap().to_string());
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        etag,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// `(wheels_up, version, issued_by_actor)` for a CFR.
async fn cfr_row(
    pool: &PgPool,
    callsign: &str,
) -> Option<(chrono::DateTime<Utc>, i64, Option<String>)> {
    sqlx::query_as(
        "select wheels_up, version, issued_by_actor from tmu.issued_cfrs where callsign = $1",
    )
    .bind(callsign)
    .fetch_optional(pool)
    .await
    .unwrap()
}

async fn audit_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("select count(*) from access.audit_logs")
        .fetch_one(pool)
        .await
        .unwrap()
}

/// A second service account, holding `permission`, under a distinct key.
async fn other_service_account(pool: &PgPool, permission: &str) -> String {
    sqlx::query("insert into access.role_permissions (role_name, permission_name) values ($1, $2) on conflict do nothing")
        .bind(ROLE)
        .bind(permission)
        .execute(pool)
        .await
        .unwrap();
    let id: String = sqlx::query_scalar(
        "insert into access.service_accounts (key, name) values ('other', 'Other Tool') returning id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let token = format!("ois_sa_{}", uuid::Uuid::new_v4().simple());
    sqlx::query("insert into access.service_account_credentials (service_account_id, secret_hash) values ($1, $2)")
        .bind(&id)
        .bind(sha256_hex(&token))
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "insert into access.service_account_roles (service_account_id, role_name) values ($1, $2)",
    )
    .bind(&id)
    .bind(ROLE)
    .execute(pool)
    .await
    .unwrap();
    format!("Bearer {token}")
}

const CFR: &str = "/api/v1/tmu/cfr";

// ---- AC2: a retry cannot issue twice -------------------------------------------------------------

#[sqlx::test]
async fn a_retried_create_cannot_issue_a_cfr_twice(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let (_, auth) = service_account(&pool, "tmu.cfr.assign", None).await;

    let (status, etag, body) = send_full(
        &state,
        http::Method::POST,
        CFR,
        &[&auth, "If-None-Match: *"],
        Some(issue_body()),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "{body}");
    assert_eq!(etag.as_deref(), Some("\"1\""));
    let first = cfr_row(&pool, "AAL1").await.unwrap();
    let audits = audit_count(&pool).await;

    // The same request again — a client that timed out and retried. A later ready time makes a
    // re-issue observable: it would move `wheels_up`.
    let mut retry = issue_body();
    retry["ready_time"] = json!((Utc::now() + chrono::Duration::hours(2)).to_rfc3339());
    let (status, etag, _) = send_full(
        &state,
        http::Method::POST,
        CFR,
        &[&auth, "If-None-Match: *"],
        Some(retry),
    )
    .await;
    assert_eq!(status, http::StatusCode::PRECONDITION_FAILED);
    assert_eq!(
        etag.as_deref(),
        Some("\"1\""),
        "the ETag says what is there"
    );
    assert_eq!(
        cfr_row(&pool, "AAL1").await.unwrap(),
        first,
        "nothing was re-issued"
    );
    assert_eq!(
        audit_count(&pool).await,
        audits,
        "and nothing was audited as written"
    );
}

#[sqlx::test]
async fn a_retried_create_cannot_mark_a_release_twice(pool: PgPool) {
    let state = crossing_state(pool.clone()).await;
    let fca_id = fca(&pool).await;
    let (_, auth) = service_account(&pool, "flow.fca.update", None).await;
    let uri = format!("/api/v1/flow/fcas/{fca_id}/release/TEST1");

    let (status, etag, body) = send_full(
        &state,
        http::Method::POST,
        &uri,
        &[&auth, "If-None-Match: *"],
        Some(json!({})),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "{body}");
    assert_eq!(etag.as_deref(), Some("\"1\""));
    let first: (i64, i64, i64) = sqlx::query_as(
        "select cta_ms, edct_ms, version from flow.fca_release where callsign = 'TEST1'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    let (status, etag, _) = send_full(
        &state,
        http::Method::POST,
        &uri,
        &[&auth, "If-None-Match: *"],
        Some(json!({"ready": "2359"})),
    )
    .await;
    assert_eq!(status, http::StatusCode::PRECONDITION_FAILED);
    assert_eq!(etag.as_deref(), Some("\"1\""));
    let after: (i64, i64, i64) = sqlx::query_as(
        "select cta_ms, edct_ms, version from flow.fca_release where callsign = 'TEST1'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(after, first);
}

// ---- AC3: a conflicting write is refused, distinguishably ----------------------------------------

#[sqlx::test]
async fn a_stale_if_match_is_refused_with_the_current_version(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let (_, auth) = service_account(&pool, "tmu.cfr.assign", None).await;
    send_full(
        &state,
        http::Method::POST,
        CFR,
        &[&auth, "If-None-Match: *"],
        Some(issue_body()),
    )
    .await;

    let (status, etag, _) = send_full(
        &state,
        http::Method::POST,
        CFR,
        &[&auth, "If-Match: \"1\""],
        Some(issue_body()),
    )
    .await;
    assert_eq!(
        status,
        http::StatusCode::OK,
        "replacing the version it saw is fine"
    );
    assert_eq!(etag.as_deref(), Some("\"2\""));
    let current = cfr_row(&pool, "AAL1").await.unwrap();

    let (status, etag, body) = send_full(
        &state,
        http::Method::POST,
        CFR,
        &[&auth, "If-Match: \"1\""],
        Some(issue_body()),
    )
    .await;
    assert_eq!(status, http::StatusCode::PRECONDITION_FAILED);
    assert_eq!(body["error"], "precondition_failed");
    assert_eq!(etag.as_deref(), Some("\"2\""), "it lost to version 2");
    assert_eq!(
        cfr_row(&pool, "AAL1").await.unwrap(),
        current,
        "the newer one stands"
    );
}

#[sqlx::test]
async fn a_machine_may_not_replace_or_clear_a_persons_release(pool: PgPool) {
    let state = crossing_state(pool.clone()).await;
    let fca_id = fca(&pool).await;
    seed_release(&pool, &fca_id, "TEST1", 1_000).await; // a person's (unattributed legacy rows count as people)
    seed_release(&pool, &fca_id, "UAL2", 2_000).await;
    let (_, auth) = service_account(&pool, "flow.fca.update", None).await;
    let release = format!("/api/v1/flow/fcas/{fca_id}/release/TEST1");

    for (method, uri, entries, body) in [
        (
            http::Method::POST,
            release.clone(),
            vec![auth.as_str(), "If-Match: \"1\""],
            Some(json!({})),
        ),
        (
            http::Method::DELETE,
            release.clone(),
            vec![auth.as_str(), "If-Match: \"1\""],
            None,
        ),
        (
            http::Method::POST,
            format!("/api/v1/flow/fcas/{fca_id}/swap"),
            vec![auth.as_str()],
            Some(json!({"a": "TEST1", "b": "UAL2"})),
        ),
    ] {
        let (status, _, body) = send_full(&state, method.clone(), &uri, &entries, body).await;
        assert_eq!(status, http::StatusCode::CONFLICT, "{method} {uri}");
        assert_eq!(body["error"], "held_by_person");
    }
    let row: (i64, i64) =
        sqlx::query_as("select cta_ms, version from flow.fca_release where callsign = 'TEST1'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(row, (1_000, 1), "the person's release is untouched");
}

#[sqlx::test]
async fn a_machine_may_not_replace_or_release_a_persons_cfr(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let user = seed_user(&pool).await;
    grant(&pool, &user, "tmu.cfr.assign", None).await;
    let cookie = session_cookie(&pool, &user).await;
    assert_eq!(
        send_full(
            &state,
            http::Method::POST,
            CFR,
            &[&cookie],
            Some(issue_body())
        )
        .await
        .0,
        http::StatusCode::OK
    );
    let (_, auth) = service_account(&pool, "tmu.cfr.assign", None).await;

    let (status, _, body) = send_full(
        &state,
        http::Method::POST,
        CFR,
        &[&auth, "If-Match: \"1\""],
        Some(issue_body()),
    )
    .await;
    assert_eq!(
        (status, body["error"].as_str()),
        (http::StatusCode::CONFLICT, Some("held_by_person"))
    );
    let (status, _, body) = send_full(
        &state,
        http::Method::DELETE,
        &format!("{CFR}/AAL1"),
        &[&auth, "If-Match: \"1\""],
        None,
    )
    .await;
    assert_eq!(
        (status, body["error"].as_str()),
        (http::StatusCode::CONFLICT, Some("held_by_person"))
    );
    assert_eq!(cfr_row(&pool, "AAL1").await.unwrap().1, 1);
}

#[sqlx::test]
async fn a_machine_may_not_touch_another_machines_cfr(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let (_, mine) = service_account(&pool, "tmu.cfr.assign", None).await;
    let theirs = other_service_account(&pool, "tmu.cfr.assign").await;
    send_full(
        &state,
        http::Method::POST,
        CFR,
        &[&mine, "If-None-Match: *"],
        Some(issue_body()),
    )
    .await;

    let (status, _, body) = send_full(
        &state,
        http::Method::POST,
        CFR,
        &[&theirs, "If-Match: \"1\""],
        Some(issue_body()),
    )
    .await;
    assert_eq!(
        (status, body["error"].as_str()),
        (http::StatusCode::CONFLICT, Some("held_by_other_machine"))
    );
}

#[sqlx::test]
async fn a_machine_must_send_a_precondition(pool: PgPool) {
    let state = crossing_state(pool.clone()).await;
    let fca_id = fca(&pool).await;
    let (_, auth) = service_account(&pool, "tmu.cfr.assign", None).await;
    sqlx::query("insert into access.role_permissions (role_name, permission_name) values ($1, 'flow.fca.update')")
        .bind(ROLE)
        .execute(&pool)
        .await
        .unwrap();

    let (status, _, body) = send_full(
        &state,
        http::Method::POST,
        CFR,
        &[&auth],
        Some(issue_body()),
    )
    .await;
    assert_eq!(
        (status, body["error"].as_str()),
        (
            http::StatusCode::PRECONDITION_REQUIRED,
            Some("precondition_required")
        )
    );
    let (status, _, _) = send_full(
        &state,
        http::Method::POST,
        &format!("/api/v1/flow/fcas/{fca_id}/release/TEST1"),
        &[&auth],
        Some(json!({})),
    )
    .await;
    assert_eq!(status, http::StatusCode::PRECONDITION_REQUIRED);
    assert!(cfr_row(&pool, "AAL1").await.is_none());
}

/// People keep today's behaviour: no precondition needed, and they override a machine's release.
#[sqlx::test]
async fn a_person_overrides_a_machines_cfr_without_a_precondition(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let (_, auth) = service_account(&pool, "tmu.cfr.assign", None).await;
    send_full(
        &state,
        http::Method::POST,
        CFR,
        &[&auth, "If-None-Match: *"],
        Some(issue_body()),
    )
    .await;
    let user = seed_user(&pool).await;
    grant(&pool, &user, "tmu.cfr.assign", None).await;
    let cookie = session_cookie(&pool, &user).await;

    let (status, etag, body) = send_full(
        &state,
        http::Method::POST,
        CFR,
        &[&cookie],
        Some(issue_body()),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "{body}");
    assert_eq!(etag.as_deref(), Some("\"2\""));
    let user_actor = actor_of(&pool, "user_id", &user).await;
    assert_eq!(
        cfr_row(&pool, "AAL1").await.unwrap().2,
        user_actor,
        "the person holds it now"
    );
}

// ---- AC4: provenance where a controller looks ------------------------------------------------------

#[sqlx::test]
async fn idst_and_traffic_show_a_machine_release_and_its_version(pool: PgPool) {
    let state = crossing_state(pool.clone()).await;
    let fca_id = fca(&pool).await;
    let (_, auth) = service_account(&pool, "flow.fca.update", None).await;
    sqlx::query("insert into access.role_permissions (role_name, permission_name) values ($1, 'flow.fca.read')")
        .bind(ROLE)
        .execute(&pool)
        .await
        .unwrap();
    let (status, _, _) = send_full(
        &state,
        http::Method::POST,
        &format!("/api/v1/flow/fcas/{fca_id}/release/TEST1"),
        &[&auth, "If-None-Match: *"],
        Some(json!({})),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK);

    let (status, _, idst) = send_full(
        &state,
        http::Method::GET,
        "/api/v1/flow/idst?airports=KJFK",
        &[&auth],
        None,
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "{idst}");
    assert_eq!(idst["released"][0]["callsign"], "TEST1");
    assert_eq!(idst["released"][0]["released_by_machine"], "vTBFM");

    let (_, _, traffic) = send_full(
        &state,
        http::Method::GET,
        &format!("/api/v1/flow/fcas/{fca_id}/traffic"),
        &[&auth],
        None,
    )
    .await;
    assert_eq!(traffic[0]["release_version"], 1);
}

#[sqlx::test]
async fn idst_names_no_machine_for_a_persons_release(pool: PgPool) {
    let state = crossing_state(pool.clone()).await;
    let fca_id = fca(&pool).await;
    seed_release(
        &pool,
        &fca_id,
        "TEST1",
        Utc::now().timestamp_millis() + 600_000,
    )
    .await;
    let user = seed_user(&pool).await;
    grant(&pool, &user, "flow.fca.read", None).await;
    let cookie = session_cookie(&pool, &user).await;

    let (_, _, idst) = send_full(
        &state,
        http::Method::GET,
        "/api/v1/flow/idst?airports=KJFK",
        &[&cookie],
        None,
    )
    .await;
    assert_eq!(idst["released"][0]["callsign"], "TEST1");
    assert_eq!(idst["released"][0]["released_by_machine"], Value::Null);
}

// ---- AC5: revocation mid-flight ---------------------------------------------------------------------

/// A committed time stands: revoking the credential stops its *future* writes, not its releases.
#[sqlx::test]
async fn a_revoked_credentials_releases_stand(pool: PgPool) {
    let state = crossing_state(pool.clone()).await;
    let fca_id = fca(&pool).await;
    let (sa, auth) = service_account(&pool, "flow.fca.update", None).await;
    let uri = format!("/api/v1/flow/fcas/{fca_id}/release/TEST1");
    assert_eq!(
        send_full(
            &state,
            http::Method::POST,
            &uri,
            &[&auth, "If-None-Match: *"],
            Some(json!({}))
        )
        .await
        .0,
        http::StatusCode::OK
    );

    sqlx::query("update access.service_account_credentials set revoked_at = now() where service_account_id = $1")
        .bind(&sa)
        .execute(&pool)
        .await
        .unwrap();

    let frozen = crate::repos::flow::list_releases(&pool, &fca_id)
        .await
        .unwrap();
    assert_eq!(
        frozen.len(),
        1,
        "still a frozen release the metering engine spaces around"
    );
    assert_eq!(frozen[0].0, "TEST1");
    let (status, _, _) = send_full(
        &state,
        http::Method::DELETE,
        &uri,
        &[&auth, "If-Match: \"1\""],
        None,
    )
    .await;
    assert_eq!(
        status,
        http::StatusCode::UNAUTHORIZED,
        "but the revoked credential can no longer act"
    );
    assert_eq!(
        crate::repos::flow::list_releases(&pool, &fca_id)
            .await
            .unwrap()
            .len(),
        1
    );
}

/// A stale `If-Match` on a **clear** is refused too — a tool must not remove a release it has not
/// seen the latest of.
#[sqlx::test]
async fn a_stale_if_match_cannot_release_a_cfr_or_clear_a_release(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let (sa, auth) = service_account(&pool, "tmu.cfr.assign", None).await;
    send_full(
        &state,
        http::Method::POST,
        CFR,
        &[&auth, "If-None-Match: *"],
        Some(issue_body()),
    )
    .await;
    send_full(
        &state,
        http::Method::POST,
        CFR,
        &[&auth, "If-Match: \"1\""],
        Some(issue_body()),
    )
    .await; // → v2

    let (status, etag, _) = send_full(
        &state,
        http::Method::DELETE,
        &format!("{CFR}/AAL1"),
        &[&auth, "If-Match: \"1\""],
        None,
    )
    .await;
    assert_eq!(status, http::StatusCode::PRECONDITION_FAILED);
    assert_eq!(etag.as_deref(), Some("\"2\""));
    assert!(
        cfr_row(&pool, "AAL1").await.is_some(),
        "the CFR is still issued"
    );

    sqlx::query("insert into access.role_permissions (role_name, permission_name) values ($1, 'flow.fca.update')")
        .bind(ROLE)
        .execute(&pool)
        .await
        .unwrap();
    let fca_id = fca(&pool).await;
    seed_owned_release(&pool, &fca_id, "AAL9", 1_000, &sa).await;
    sqlx::query("update flow.fca_release set version = 2 where callsign = 'AAL9'")
        .execute(&pool)
        .await
        .unwrap();
    let uri = format!("/api/v1/flow/fcas/{fca_id}/release/AAL9");
    let (status, etag, _) = send_full(
        &state,
        http::Method::DELETE,
        &uri,
        &[&auth, "If-Match: \"1\""],
        None,
    )
    .await;
    assert_eq!(status, http::StatusCode::PRECONDITION_FAILED);
    assert_eq!(etag.as_deref(), Some("\"2\""));
    assert!(
        release_attribution(&pool, &fca_id, "AAL9").await.is_some(),
        "the release still stands"
    );
}

// ---- #585 review: the holder is part of every machine write, not a read before it --------------
//
// The handlers read the holder and refuse with 409 before writing, which gives the caller a precise
// answer. But between that read and the write a person can take the row over, and versions restart
// at 1 after a clear and re-mark, so a machine's `If-Match` can match a row it does not hold. These
// drive the repo writes with exactly that stale state — a version that matches, on a row someone else
// holds — and require the write itself to refuse.

/// The attribution a service account writes with, and a person's for the same database.
async fn machine_and_person(
    pool: &PgPool,
) -> (
    crate::auth::principal::Attribution,
    crate::auth::principal::Attribution,
) {
    let (sa, _) = service_account(pool, "flow.fca.update", None).await;
    let machine = crate::auth::principal::Attribution {
        user_id: None,
        actor_id: crate::repos::audit::resolve_service_account_actor_id(pool, &sa, "vTBFM")
            .await
            .unwrap(),
    };
    let user = seed_user(pool).await;
    let person = crate::auth::principal::Attribution {
        actor_id: crate::repos::audit::resolve_user_actor_id(pool, &user, "Controller")
            .await
            .unwrap(),
        user_id: Some(user),
    };
    (machine, person)
}

#[sqlx::test]
async fn a_machine_conditional_write_never_lands_on_a_persons_release(pool: PgPool) {
    use crate::repos::flow::{self as flow_repo, Expect};
    let id = fca(&pool).await;
    let (machine, person) = machine_and_person(&pool).await;
    let v = flow_repo::upsert_release(&pool, &id, "AAL1", 1_000, 1_000, &person, None)
        .await
        .unwrap()
        .expect("a person's write is unconditional");

    // The version matches; the holder does not.
    let written = flow_repo::upsert_release(
        &pool,
        &id,
        "AAL1",
        9_000,
        9_000,
        &machine,
        Some(Expect::Version(v)),
    )
    .await
    .unwrap();
    assert_eq!(written, None, "refused in the write");
    assert!(
        !flow_repo::delete_release(&pool, &id, "AAL1", Some(v), &machine)
            .await
            .unwrap(),
        "a machine's clear at the matching version does not remove a person's release"
    );
    assert_eq!(
        release_attribution(&pool, &id, "AAL1").await,
        Some((person.user_id.clone(), person.actor_id.clone())),
        "still the person's, untouched"
    );
}

#[sqlx::test]
async fn a_machine_swap_never_takes_a_persons_release(pool: PgPool) {
    use crate::repos::flow as flow_repo;
    let id = fca(&pool).await;
    let (machine, person) = machine_and_person(&pool).await;
    flow_repo::upsert_release(
        &pool,
        &id,
        "OWN1",
        1_000,
        1_000,
        &machine,
        Some(flow_repo::Expect::Absent),
    )
    .await
    .unwrap()
    .unwrap();
    flow_repo::upsert_release(&pool, &id, "PER2", 2_000, 2_000, &person, None)
        .await
        .unwrap()
        .unwrap();

    assert!(
        !flow_repo::swap_releases(&pool, &id, "OWN1", "PER2", &machine)
            .await
            .unwrap()
    );
    let cta: i64 = sqlx::query_scalar(
        "select cta_ms from flow.fca_release where fca_id = $1 and callsign = 'PER2'",
    )
    .bind(&id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(cta, 2_000, "the person's time is not traded away");
    assert_eq!(
        release_attribution(&pool, &id, "PER2").await,
        Some((person.user_id.clone(), person.actor_id.clone()))
    );

    // The control: a person may swap them, and a machine may swap two it holds.
    flow_repo::upsert_release(
        &pool,
        &id,
        "OWN3",
        3_000,
        3_000,
        &machine,
        Some(flow_repo::Expect::Absent),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        flow_repo::swap_releases(&pool, &id, "OWN1", "OWN3", &machine)
            .await
            .unwrap()
    );
    assert!(
        flow_repo::swap_releases(&pool, &id, "OWN1", "PER2", &person)
            .await
            .unwrap()
    );
}

#[sqlx::test]
async fn a_machine_conditional_write_still_reaches_its_own_release(pool: PgPool) {
    use crate::repos::flow::{self as flow_repo, Expect};
    let id = fca(&pool).await;
    let (machine, _) = machine_and_person(&pool).await;
    let v = flow_repo::upsert_release(
        &pool,
        &id,
        "OWN1",
        1_000,
        1_000,
        &machine,
        Some(Expect::Absent),
    )
    .await
    .unwrap()
    .unwrap();
    let v2 = flow_repo::upsert_release(
        &pool,
        &id,
        "OWN1",
        2_000,
        2_000,
        &machine,
        Some(Expect::Version(v)),
    )
    .await
    .unwrap();
    assert_eq!(v2, Some(v + 1));
    assert!(
        flow_repo::delete_release(&pool, &id, "OWN1", Some(v + 1), &machine)
            .await
            .unwrap()
    );
}

#[sqlx::test]
async fn a_machine_conditional_cfr_write_never_lands_on_a_persons_cfr(pool: PgPool) {
    use crate::repos::flow::Expect;
    use crate::repos::tmu as tmu_repo;
    let (machine, person) = machine_and_person(&pool).await;
    let at = Utc::now() + Duration::hours(1);
    let v = tmu_repo::upsert_issued_cfr(&pool, "AAL1", "KDCA", at, &person, None)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        tmu_repo::upsert_issued_cfr(
            &pool,
            "AAL1",
            "KDCA",
            at + Duration::minutes(5),
            &machine,
            Some(Expect::Version(v))
        )
        .await
        .unwrap(),
        None
    );
    assert!(
        !tmu_repo::delete_issued_cfr(&pool, "AAL1", Some(v), &machine)
            .await
            .unwrap()
    );
    assert_eq!(
        cfr_attribution(&pool, "AAL1").await,
        (person.user_id.clone(), person.actor_id.clone())
    );

    // Its own CFR it can replace and release.
    let own =
        tmu_repo::upsert_issued_cfr(&pool, "UAL2", "KDCA", at, &machine, Some(Expect::Absent))
            .await
            .unwrap()
            .unwrap();
    assert!(
        tmu_repo::delete_issued_cfr(&pool, "UAL2", Some(own), &machine)
            .await
            .unwrap()
    );
}

#[test]
fn only_a_machine_attribution_names_an_owner_and_a_missing_actor_fails_closed() {
    use crate::auth::principal::Attribution;
    let person = Attribution {
        user_id: Some("u".into()),
        actor_id: Some("a".into()),
    };
    assert_eq!(person.machine_actor(), None);
    let machine = Attribution {
        user_id: None,
        actor_id: Some("m".into()),
    };
    assert_eq!(machine.machine_actor(), Some("m"));
    let unresolved = Attribution {
        user_id: None,
        actor_id: None,
    };
    assert_eq!(
        unresolved.machine_actor(),
        Some(""),
        "matches no row rather than every row"
    );
}

// ---- #585 QA: every read a writer takes a version from carries it -------------------------------

/// A writer takes versions from whichever flight list it is handed, the clear response included: a
/// released flight there carries its version, not null.
#[sqlx::test]
async fn the_clear_response_carries_the_remaining_releases_versions(pool: PgPool) {
    let state = crossing_state_of(pool.clone(), &["TEST1", "TEST2"]).await;
    let fca_id = fca(&pool).await;
    let (_, auth) = service_account(&pool, "flow.fca.update", None).await;
    let release = |callsign: &'static str, precondition: &'static str| {
        let (state, auth, uri) = (
            state.clone(),
            auth.clone(),
            format!("/api/v1/flow/fcas/{fca_id}/release/{callsign}"),
        );
        async move {
            send_full(
                &state,
                http::Method::POST,
                &uri,
                &[&auth, precondition],
                Some(json!({})),
            )
            .await
            .0
        }
    };
    assert_eq!(
        release("TEST1", "If-None-Match: *").await,
        http::StatusCode::OK
    );
    assert_eq!(
        release("TEST1", "If-Match: \"1\"").await,
        http::StatusCode::OK
    );
    assert_eq!(
        release("TEST2", "If-None-Match: *").await,
        http::StatusCode::OK
    );

    let (status, _, flights) = send_full(
        &state,
        http::Method::DELETE,
        &format!("/api/v1/flow/fcas/{fca_id}/release/TEST2"),
        &[&auth, "If-Match: \"1\""],
        None,
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "{flights}");
    let test1 = flights
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["callsign"] == "TEST1")
        .expect("TEST1 is still in the list");
    assert_eq!(test1["released"], true);
    assert_eq!(test1["release_version"], 2);
}

/// CFR writes need `If-Match`, so the departures list carries each issued CFR's version, rather than
/// leaving a writer to provoke a 412 to learn it.
#[sqlx::test]
async fn the_departures_list_carries_an_issued_cfrs_version(pool: PgPool) {
    let state = crossing_state(pool.clone()).await;
    let (_, auth) = service_account(&pool, "tmu.cfr.assign", None).await;
    sqlx::query("insert into access.role_permissions (role_name, permission_name) values ($1, 'tmu.program.read')")
        .bind(ROLE)
        .execute(&pool)
        .await
        .unwrap();
    let mut body = issue_body();
    body["callsign"] = json!("TEST1");
    for precondition in ["If-None-Match: *", "If-Match: \"1\""] {
        let (status, _, reply) = send_full(
            &state,
            http::Method::POST,
            CFR,
            &[&auth, precondition],
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, http::StatusCode::OK, "{reply}");
    }

    let (status, _, list) = send_full(
        &state,
        http::Method::GET,
        "/api/v1/tmu/departures/KJFK",
        &[&auth],
        None,
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "{list}");
    let test1 = list["departures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["callsign"] == "TEST1")
        .expect("TEST1 departs KJFK");
    assert_eq!(test1["cfr_version"], 2);
}

// ---- #607 PR 1: the TMU, flow and GDP writes ----
//
// Each of these went 401 for a service account before #607: the handler took `CurrentUser` (or
// `Principal::require`, which admits a key but not a service account) and refused it on the line after
// `RequirePermission` let it through.

/// Add `permission` to the role the test service account holds, so one account can drive a sequence.
async fn allow(pool: &PgPool, permission: &str) {
    sqlx::query(
        "insert into access.role_permissions (role_name, permission_name) values ($1, $2) \
         on conflict do nothing",
    )
    .bind(ROLE)
    .bind(permission)
    .execute(pool)
    .await
    .unwrap();
}

/// `(user column, actor column)` of one attribution pair on one row.
async fn attributed(
    pool: &PgPool,
    table: &str,
    user_col: &str,
    id_col: &str,
    id: &str,
) -> (Option<String>, Option<String>) {
    sqlx::query_as(&format!(
        "select {user_col}, {user_col}_actor from {table} where {id_col}::text = $1"
    ))
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// A national service account holding every one of `permissions`. Returns its bearer.
async fn machine(pool: &PgPool, permissions: &[&str]) -> String {
    let (_, bearer) = service_account(pool, permissions[0], None).await;
    for p in &permissions[1..] {
        allow(pool, p).await;
    }
    bearer
}

/// The service account's audit actor — created on its first write, so look it up after one.
async fn machine_actor(pool: &PgPool) -> String {
    let id: String =
        sqlx::query_scalar("select id from access.service_accounts where key = 'vtbfm'")
            .fetch_one(pool)
            .await
            .unwrap();
    actor_of(pool, "service_account_id", &id)
        .await
        .expect("the machine has an actor once it has written")
}

/// AC3, flow: a service account creates, edits and reorders an FCA, and creates, edits and deletes a
/// route. Each row names the machine — never a person, never nobody — and reads back as "vTBFM".
#[sqlx::test]
async fn a_machine_drives_the_flow_writes(pool: PgPool) {
    let auth = machine(
        &pool,
        &["flow.fca.update", "flow.route.update", "flow.route.delete"],
    )
    .await;
    let state = test_state(pool.clone(), HashMap::new());
    let fca = json!({ "name": "T607", "artcc": "ZDC",
                      "points": [[38.0, -77.0], [39.0, -77.0], [39.0, -76.0]] });

    let (status, body) = call(
        &state,
        http::Method::POST,
        "/api/v1/flow/fcas",
        &auth,
        Some(fca.clone()),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "create_fca: {body}");
    let id = body["id"].as_str().unwrap().to_string();
    let actor = machine_actor(&pool).await;
    assert_eq!(
        attributed(&pool, "flow.fca", "created_by", "id", &id).await,
        (None, Some(actor.clone()))
    );
    assert_eq!(body["updated_by"], "vTBFM", "the read names the machine");

    let (status, _) = call(
        &state,
        http::Method::PUT,
        &format!("/api/v1/flow/fcas/{id}"),
        &auth,
        Some(fca),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "update_fca");
    let (status, _) = call(
        &state,
        http::Method::PUT,
        &format!("/api/v1/flow/fcas/{id}/order"),
        &auth,
        Some(json!({ "order": ["AAL1"] })),
    )
    .await;
    assert!(status.is_success(), "reorder_fca: {status}");
    assert_eq!(
        attributed(&pool, "flow.fca", "updated_by", "id", &id).await,
        (None, Some(actor.clone()))
    );

    let route = json!({ "name": "R607", "route": "DCA J149 JFK", "artcc": "ZDC" });
    let (status, body) = call(
        &state,
        http::Method::POST,
        "/api/v1/flow/routes",
        &auth,
        Some(route.clone()),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "create_route: {body}");
    let rid = body["id"].as_str().unwrap().to_string();
    assert_eq!(
        attributed(&pool, "flow.route", "created_by", "id", &rid).await,
        (None, Some(actor.clone()))
    );
    let (status, _) = call(
        &state,
        http::Method::PUT,
        &format!("/api/v1/flow/routes/{rid}"),
        &auth,
        Some(route),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "update_route");
    assert_eq!(
        attributed(&pool, "flow.route", "updated_by", "id", &rid).await,
        (None, Some(actor))
    );
    let (status, _) = call(
        &state,
        http::Method::DELETE,
        &format!("/api/v1/flow/routes/{rid}"),
        &auth,
        None,
    )
    .await;
    assert!(status.is_success(), "delete_route: {status}");
}

/// AC3, TMU: a service account issues and publishes a TMI, sets a rate program, and creates and publishes
/// a ground stop — whose generated advisory is attributed to the machine as well.
#[sqlx::test]
async fn a_machine_drives_the_tmu_writes(pool: PgPool) {
    let auth = machine(
        &pool,
        &[
            "tmu.tmi.create",
            "tmu.tmi.publish",
            "tmu.program.update",
            "tmu.groundstop.create",
            "tmu.groundstop.publish",
        ],
    )
    .await;
    let state = test_state(pool.clone(), HashMap::new());

    let (status, body) = call(
        &state,
        http::Method::POST,
        "/api/v1/tmu/tmis",
        &auth,
        Some(json!({ "requesting": "ZDC", "providing": "ZNY", "restriction": "20 MIT" })),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "create_tmi: {body}");
    let tmi = body["id"].as_str().unwrap().to_string();
    let actor = machine_actor(&pool).await;
    assert_eq!(
        attributed(&pool, "tmu.tmis", "created_by", "id", &tmi).await,
        (None, Some(actor.clone()))
    );
    assert_eq!(body["author"], "vTBFM");
    let (status, _) = call(
        &state,
        http::Method::POST,
        &format!("/api/v1/tmu/tmis/{tmi}/publish"),
        &auth,
        None,
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "publish_tmi");
    assert_eq!(
        attributed(&pool, "tmu.tmis", "published_by", "id", &tmi).await,
        (None, Some(actor.clone()))
    );

    let (status, _) = call(
        &state,
        http::Method::PUT,
        "/api/v1/tmu/programs/KDCA",
        &auth,
        Some(json!({ "aar": 30 })),
    )
    .await;
    assert!(status.is_success(), "upsert_program: {status}");
    assert_eq!(
        attributed(&pool, "tmu.programs", "updated_by", "icao", "KDCA").await,
        (None, Some(actor.clone()))
    );

    let (status, body) = call(
        &state,
        http::Method::POST,
        "/api/v1/tmu/ground-stops",
        &auth,
        Some(json!({ "airport": "KDCA" })),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "create_ground_stop: {body}");
    let gs = body["id"].as_str().unwrap().to_string();
    assert_eq!(
        attributed(&pool, "tmu.ground_stops", "created_by", "id", &gs).await,
        (None, Some(actor.clone()))
    );
    let (status, body) = call(
        &state,
        http::Method::POST,
        &format!("/api/v1/tmu/ground-stops/{gs}/publish"),
        &auth,
        None,
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "publish_ground_stop: {body}");
    assert_eq!(
        attributed(&pool, "tmu.ground_stops", "published_by", "id", &gs).await,
        (None, Some(actor.clone()))
    );
    let adv: String =
        sqlx::query_scalar("select id from tmu.advisories where ground_stop_id::text = $1")
            .bind(&gs)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        attributed(&pool, "tmu.advisories", "created_by", "id", &adv).await,
        (None, Some(actor))
    );
}

/// AC3, advisories: create, edit, publish, cancel, and delete a draft — all as a machine.
#[sqlx::test]
async fn a_machine_drives_the_advisory_writes(pool: PgPool) {
    let auth = machine(
        &pool,
        &["tmu.adv.create", "tmu.adv.update", "tmu.adv.publish"],
    )
    .await;
    let state = test_state(pool.clone(), HashMap::new());
    let draft = json!({ "facility": "ZDC", "kind": "general", "body": "ADVZY 001 ZDC" });

    let (status, body) = call(
        &state,
        http::Method::POST,
        "/api/v1/tmu/advisories",
        &auth,
        Some(draft.clone()),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "create_advisory: {body}");
    let adv = body["id"].as_str().unwrap().to_string();
    let actor = machine_actor(&pool).await;
    assert_eq!(
        attributed(&pool, "tmu.advisories", "created_by", "id", &adv).await,
        (None, Some(actor.clone()))
    );
    let (status, _) = call(
        &state,
        http::Method::PATCH,
        &format!("/api/v1/tmu/advisories/{adv}"),
        &auth,
        Some(json!({ "body": "ADVZY 001 ZDC AMENDED" })),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "update_advisory");
    let (status, _) = call(
        &state,
        http::Method::POST,
        &format!("/api/v1/tmu/advisories/{adv}/publish"),
        &auth,
        None,
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "publish_advisory");
    assert_eq!(
        attributed(&pool, "tmu.advisories", "published_by", "id", &adv).await,
        (None, Some(actor))
    );
    let (status, _) = call(
        &state,
        http::Method::POST,
        &format!("/api/v1/tmu/advisories/{adv}/cancel"),
        &auth,
        None,
    )
    .await;
    assert!(status.is_success(), "cancel_advisory: {status}");

    let (_, body) = call(
        &state,
        http::Method::POST,
        "/api/v1/tmu/advisories",
        &auth,
        Some(draft),
    )
    .await;
    let second = body["id"].as_str().unwrap().to_string();
    let (status, _) = call(
        &state,
        http::Method::DELETE,
        &format!("/api/v1/tmu/advisories/{second}"),
        &auth,
        None,
    )
    .await;
    assert!(status.is_success(), "delete_advisory: {status}");
}

/// AC3, GDP: create, revise and publish — the published GDP's generated advisory names the machine too.
#[sqlx::test]
async fn a_machine_drives_the_gdp_writes(pool: PgPool) {
    let auth = machine(&pool, &["tmu.gdp.create", "tmu.gdp.publish"]).await;
    let state = test_state(pool.clone(), HashMap::new());
    let now = Utc::now();
    let (start, end) = (
        (now + Duration::hours(1)).format("%H%M").to_string(),
        (now + Duration::hours(3)).format("%H%M").to_string(),
    );
    let gdp = json!({ "airport": "KDCA", "aar": 30, "start_time": start, "end_time": end });

    let (status, body) = call(
        &state,
        http::Method::POST,
        "/api/v1/tmu/gdp",
        &auth,
        Some(gdp.clone()),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "create_gdp: {body}");
    let id = body["id"].as_str().unwrap().to_string();
    let actor = machine_actor(&pool).await;
    assert_eq!(
        attributed(&pool, "tmu.gdp", "created_by", "id", &id).await,
        (None, Some(actor.clone()))
    );
    assert_eq!(body["updated_by"], "vTBFM");
    let (status, body) = call(
        &state,
        http::Method::PUT,
        &format!("/api/v1/tmu/gdp/{id}"),
        &auth,
        Some(gdp),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "revise_gdp: {body}");
    assert_eq!(
        attributed(&pool, "tmu.gdp", "updated_by", "id", &id).await,
        (None, Some(actor.clone()))
    );
    let (status, body) = call(
        &state,
        http::Method::POST,
        &format!("/api/v1/tmu/gdp/{id}/publish"),
        &auth,
        None,
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "publish_gdp: {body}");
    assert_eq!(
        attributed(&pool, "tmu.gdp", "published_by", "id", &id).await,
        (None, Some(actor.clone()))
    );
    let adv: String = sqlx::query_scalar("select id from tmu.advisories where gdp_id::text = $1")
        .bind(&id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        attributed(&pool, "tmu.advisories", "created_by", "id", &adv).await,
        (None, Some(actor))
    );
}

/// No regression for people: a signed-in user's write still fills **both** columns, so the legacy user
/// column keeps working and the actor column is populated going forward.
#[sqlx::test]
async fn a_persons_write_still_names_them_in_both_columns(pool: PgPool) {
    let user = seed_user(&pool).await;
    grant(&pool, &user, "tmu.tmi.create", None).await;
    let cookie = session_cookie(&pool, &user).await;
    let state = test_state(pool.clone(), HashMap::new());

    let (status, body) = call(
        &state,
        http::Method::POST,
        "/api/v1/tmu/tmis",
        &cookie,
        Some(json!({ "requesting": "ZDC", "providing": "ZNY", "restriction": "20 MIT" })),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "{body}");
    let (by, by_actor) = attributed(
        &pool,
        "tmu.tmis",
        "created_by",
        "id",
        body["id"].as_str().unwrap(),
    )
    .await;
    assert_eq!(by.as_deref(), Some(user.as_str()));
    assert_eq!(by_actor, actor_of(&pool, "user_id", &user).await);
}

/// AC4: no widening. A ZDC-scoped service account is refused a route and an advisory at ZNY, exactly
/// as a ZDC-scoped person is — the scope checks these handlers already ran now see the machine's roles.
#[sqlx::test]
async fn a_scoped_machine_is_refused_outside_its_artcc(pool: PgPool) {
    let (_, auth) = service_account(&pool, "flow.route.update", Some("ZDC")).await;
    allow(&pool, "tmu.adv.create").await;
    let state = test_state(pool.clone(), HashMap::new());

    let (status, _) = call(
        &state,
        http::Method::POST,
        "/api/v1/flow/routes",
        &auth,
        Some(json!({ "name": "R", "route": "JFK J60 BOS", "artcc": "ZNY" })),
    )
    .await;
    assert_eq!(
        status,
        http::StatusCode::FORBIDDEN,
        "route outside the machine's ARTCC"
    );
    let (status, _) = call(
        &state,
        http::Method::POST,
        "/api/v1/tmu/advisories",
        &auth,
        Some(json!({ "facility": "ZNY", "kind": "general", "body": "x" })),
    )
    .await;
    assert_eq!(
        status,
        http::StatusCode::FORBIDDEN,
        "advisory outside the machine's ARTCC"
    );

    let (status, _) = call(
        &state,
        http::Method::POST,
        "/api/v1/flow/routes",
        &auth,
        Some(json!({ "name": "R", "route": "DCA J149 JFK", "artcc": "ZDC" })),
    )
    .await;
    assert_eq!(
        status,
        http::StatusCode::OK,
        "inside it, the same write goes through"
    );
}

/// The stated behaviour change: a route written with an API key used to be recorded under the key's
/// **owner**. It now names the **key** — #583's rule that a machine's write names the machine, never a
/// person. The owner stays reachable through the key's actor.
#[sqlx::test]
async fn an_api_keys_route_names_the_key_not_its_owner(pool: PgPool) {
    let (key, auth) = api_key(&pool, "flow.route.update").await;
    let state = test_state(pool.clone(), HashMap::new());

    let (status, body) = call(
        &state,
        http::Method::POST,
        "/api/v1/flow/routes",
        &auth,
        Some(json!({ "name": "R", "route": "DCA J149 JFK" })),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "{body}");
    let (by, by_actor) = attributed(
        &pool,
        "flow.route",
        "created_by",
        "id",
        body["id"].as_str().unwrap(),
    )
    .await;
    assert_eq!(by, None);
    assert_eq!(by_actor, actor_of(&pool, "api_key_id", &key).await);
}

/// The bridge the event-package lifecycle job uses (it has no request, so no `Principal`): a person is
/// attributed in both columns, exactly as `Principal::attribution` would; an unknown id is an error,
/// never a silently anonymous write.
#[sqlx::test]
async fn the_lifecycle_jobs_attribution_names_the_person_in_both_columns(pool: PgPool) {
    let user = seed_user(&pool).await;
    let by = crate::auth::principal::Attribution::for_user_id(&pool, &user)
        .await
        .unwrap();
    assert_eq!(by.user_id.as_deref(), Some(user.as_str()));
    assert!(by.actor_id.is_some());
    assert_eq!(by.actor_id, actor_of(&pool, "user_id", &user).await);
    assert!(
        crate::auth::principal::Attribution::for_user_id(&pool, "no-such-user")
            .await
            .is_err()
    );
}

// ---- #607 PR 2: the events planning writes ----
//
// Arming and activating a package, and generating Tier-1 ACE requests, stay user-only (see the
// ratchet): the lifecycle job acts later as the person in `tmi_package.updated_by`.

const EVENT: i64 = 6072;

async fn seed_event(pool: &PgPool) {
    sqlx::query(
        "insert into events.event (id, title, start_time, end_time, facility) \
         values ($1, 'T607', now(), now() + interval '2 hours', 'ZDC')",
    )
    .bind(EVENT)
    .execute(pool)
    .await
    .unwrap();
}

/// `(updated_by, updated_by_actor)` of a row keyed by `where`.
async fn updated(pool: &PgPool, table: &str, filter: &str) -> (Option<String>, Option<String>) {
    sqlx::query_as(&format!(
        "select updated_by, updated_by_actor from {table} where {filter}"
    ))
    .fetch_one(pool)
    .await
    .unwrap()
}

/// AC3, events: a service account writes DCC, facility support, an airport rate, capture, a debrief
/// and an event FCA — each 401 before — and every row names the machine, never a person.
#[sqlx::test]
async fn a_machine_drives_the_event_planning_writes(pool: PgPool) {
    let auth = machine(
        &pool,
        &[
            "events.plan.update",
            "events.plan.read",
            "events.support.update",
            "events.rate.update",
            "stats.capture.update",
            "events.debrief.create",
        ],
    )
    .await;
    seed_event(&pool).await;
    let state = test_state(pool.clone(), HashMap::new());
    let base = format!("/api/v1/events/{EVENT}");
    let put = |path: &str, body: Value| {
        let (state, auth, uri) = (state.clone(), auth.clone(), format!("{base}{path}"));
        async move { call(&state, http::Method::PUT, &uri, &auth, Some(body)).await }
    };

    let (status, body) = put("/dcc", json!({ "status": "requested" })).await;
    assert_eq!(status, http::StatusCode::OK, "dcc: {body}");
    let actor = machine_actor(&pool).await;
    let machine_row = (None, Some(actor.clone()));
    let at_event = format!("event_id = {EVENT}");
    assert_eq!(
        updated(&pool, "events.dcc_request", &at_event).await,
        machine_row
    );
    assert_eq!(body["updated_by"], "vTBFM", "the read names the machine");

    let (status, body) = put("/facilities/ZDC", json!({ "level": "required" })).await;
    assert_eq!(status, http::StatusCode::OK, "facility: {body}");
    assert_eq!(
        updated(&pool, "events.facility_support", &at_event).await,
        machine_row
    );

    let (status, body) = put("/rates/KIAD", json!({ "aar": 40, "adr": 40 })).await;
    assert_eq!(status, http::StatusCode::OK, "rate: {body}");
    assert_eq!(
        updated(&pool, "events.airport_rate", &at_event).await,
        machine_row
    );

    let (status, body) = put("/capture", json!({ "enabled": true })).await;
    assert_eq!(status, http::StatusCode::OK, "capture: {body}");
    assert_eq!(
        updated(&pool, "stats.event_capture", &at_event).await,
        machine_row
    );

    let (status, body) = put("/debrief", json!({ "notes": "went fine" })).await;
    assert_eq!(status, http::StatusCode::OK, "debrief: {body}");
    assert_eq!(
        updated(&pool, "events.event_debrief", &at_event).await,
        machine_row
    );
    let (status, body) = call(
        &state,
        http::Method::GET,
        &format!("{base}/debrief"),
        &auth,
        None,
    )
    .await;
    assert_eq!(status, http::StatusCode::OK);
    assert_eq!(body["updated_by"], "vTBFM");
    assert_eq!(
        body["editable"], true,
        "the machine holds events.debrief.create"
    );

    let fca = json!({ "name": "E607", "artcc": "ZDC",
                      "points": [[38.0, -77.0], [39.0, -77.0], [39.0, -76.0]] });
    let (status, body) = call(
        &state,
        http::Method::POST,
        &format!("{base}/fcas"),
        &auth,
        Some(fca.clone()),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "event fca: {body}");
    let id = body[0]["id"].as_str().unwrap().to_string();
    assert_eq!(
        attributed(&pool, "flow.fca", "created_by", "id", &id).await,
        machine_row
    );
    let (status, _) = put(&format!("/fcas/{id}"), fca).await;
    assert_eq!(status, http::StatusCode::OK, "update event fca");
    assert_eq!(
        attributed(&pool, "flow.fca", "updated_by", "id", &id).await,
        machine_row
    );

    for (path, flag) in [("/facilities", "editable"), ("/rates", "editable")] {
        let (status, body) = call(
            &state,
            http::Method::GET,
            &format!("{base}{path}"),
            &auth,
            None,
        )
        .await;
        assert_eq!(status, http::StatusCode::OK, "{path}");
        assert!(
            body.as_array().unwrap().iter().any(|r| r[flag] == true),
            "{path}: the machine's national scope marks its rows editable: {body}"
        );
    }
    let (_, body) = call(
        &state,
        http::Method::GET,
        &format!("{base}/capture"),
        &auth,
        None,
    )
    .await;
    assert_eq!(body["can_edit"], true);

    for path in ["/facilities/ZDC", "/rates/KIAD"] {
        let (status, _) = call(
            &state,
            http::Method::DELETE,
            &format!("{base}{path}"),
            &auth,
            None,
        )
        .await;
        assert!(status.is_success(), "delete {path}: {status}");
    }
}

/// AC3, packages: a machine creates a package, adds an item and deactivates it, named each time.
/// Then the pair stays consistent when a person arms it — both columns name the person, so the
/// lifecycle job (which acts as `updated_by`) never meets a machine-only row it would silently skip.
#[sqlx::test]
async fn a_machine_builds_a_package_and_a_person_arms_it(pool: PgPool) {
    let auth = machine(&pool, &["events.plan.update"]).await;
    seed_event(&pool).await;
    let state = test_state(pool.clone(), HashMap::new());
    let base = format!("/api/v1/events/{EVENT}/packages");

    let (status, body) = call(
        &state,
        http::Method::POST,
        &base,
        &auth,
        Some(json!({ "name": "Plan" })),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "create: {body}");
    let pkg = body[0]["id"].as_str().unwrap().to_string();
    let at_pkg = format!("id = '{pkg}'");
    let actor = machine_actor(&pool).await;
    assert_eq!(
        updated(&pool, "events.tmi_package", &at_pkg).await,
        (None, Some(actor.clone()))
    );
    let item = json!({ "kind": "restriction",
                       "payload": { "requesting": "ZDC", "providing": "ZNY", "restriction": "20 MIT" } });
    let (status, body) = call(
        &state,
        http::Method::POST,
        &format!("{base}/{pkg}/items"),
        &auth,
        Some(item),
    )
    .await;
    assert!(status.is_success(), "add item: {status} {body}");

    let person = seed_user(&pool).await;
    grant(&pool, &person, "events.plan.update", None).await;
    let cookie = session_cookie(&pool, &person).await;
    let (status, body) = call(
        &state,
        http::Method::PUT,
        &format!("{base}/{pkg}/auto"),
        &cookie,
        Some(json!({ "auto_publish": true })),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "arm: {body}");
    assert_eq!(
        updated(&pool, "events.tmi_package", &at_pkg).await,
        (
            Some(person.clone()),
            actor_of(&pool, "user_id", &person).await
        ),
        "arming names the person in both columns, replacing the machine"
    );
    let due = crate::repos::events::auto_due_packages(&pool)
        .await
        .unwrap();
    assert!(
        due.iter().any(|p| p.0 == pkg),
        "the job picks the armed package up"
    );

    // Activation stays a person's act; the machine then stands the package down.
    let (status, body) = call(
        &state,
        http::Method::POST,
        &format!("{base}/{pkg}/activate"),
        &cookie,
        None,
    )
    .await;
    assert!(status.is_success(), "activate: {status} {body}");
    assert_eq!(
        updated(&pool, "events.tmi_package", &at_pkg).await,
        (
            Some(person.clone()),
            actor_of(&pool, "user_id", &person).await
        ),
        "activating names the person in both columns too"
    );
    let (status, body) = call(
        &state,
        http::Method::POST,
        &format!("{base}/{pkg}/deactivate"),
        &auth,
        None,
    )
    .await;
    assert!(status.is_success(), "deactivate: {status} {body}");
    assert_eq!(
        updated(&pool, "events.tmi_package", &at_pkg).await,
        (None, Some(actor))
    );
}

/// AC4, events: a ZDC-scoped machine is refused support at ZNY, as a ZDC-scoped person is.
#[sqlx::test]
async fn a_scoped_machine_is_refused_another_facilitys_support(pool: PgPool) {
    let (_, auth) = service_account(&pool, "events.support.update", Some("ZDC")).await;
    seed_event(&pool).await;
    let state = test_state(pool.clone(), HashMap::new());
    let uri = |f: &str| format!("/api/v1/events/{EVENT}/facilities/{f}");
    let body = json!({ "level": "required" });

    let (status, _) = call(
        &state,
        http::Method::PUT,
        &uri("ZNY"),
        &auth,
        Some(body.clone()),
    )
    .await;
    assert_eq!(
        status,
        http::StatusCode::FORBIDDEN,
        "outside the machine's ARTCC"
    );
    let (status, _) = call(&state, http::Method::PUT, &uri("ZDC"), &auth, Some(body)).await;
    assert_eq!(status, http::StatusCode::OK, "inside it");
}

// ---- #607 PR 3: airport configurations and surface data ----

/// KIAD under ZDC and KJFK under ZNY, so writes have an owning ARTCC to be scoped against.
fn airport_state(pool: &PgPool) -> AppState {
    use crate::scope_test_support::artcc;
    test_state(
        pool.clone(),
        HashMap::from([
            ("ZDC".to_string(), artcc(&["KIAD"])),
            ("ZNY".to_string(), artcc(&["KJFK"])),
        ]),
    )
}

fn config_body() -> Value {
    json!({ "name": "South flow", "aar": 60, "adr": 60, "wind_from_deg": 150, "wind_to_deg": 250 })
}

fn ring() -> Value {
    json!([[[38.94, -77.46], [38.95, -77.46], [38.95, -77.45]]])
}

/// AC3, airport data: a service account writes a configuration and every kind of surface feature —
/// each refused before #607 — and every row names the machine, never a person. The reads mark the
/// rows editable per the machine's own scope.
#[sqlx::test]
async fn a_machine_drives_the_airport_config_and_surface_writes(pool: PgPool) {
    let auth = machine(
        &pool,
        &[
            "events.config.update",
            "flow.surface_data.update",
            "events.plan.read",
        ],
    )
    .await;
    let state = airport_state(&pool);
    let send = |method: http::Method, path: String, body: Option<Value>| {
        let (state, auth) = (state.clone(), auth.clone());
        async move { call(&state, method, &format!("/api/v1{path}"), &auth, body).await }
    };

    let (status, body) = send(
        http::Method::POST,
        "/airport-configs/KIAD".into(),
        Some(config_body()),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "create config: {body}");
    let id = body["id"].as_str().unwrap().to_string();
    let actor = machine_actor(&pool).await;
    let machine_row = (None, Some(actor));
    assert_eq!(
        attributed(&pool, "flow.airport_config", "updated_by", "id", &id).await,
        machine_row
    );
    assert_eq!(body["updated_by"], "vTBFM", "the read names the machine");
    let (status, _) = send(
        http::Method::PUT,
        format!("/airport-configs/KIAD/{id}"),
        Some(config_body()),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "update config");
    assert_eq!(
        attributed(&pool, "flow.airport_config", "updated_by", "id", &id).await,
        machine_row,
        "the edit names the machine too"
    );
    let (_, list) = send(http::Method::GET, "/airport-configs/KIAD".into(), None).await;
    assert_eq!(list[0]["editable"], true, "{list}");
    let (status, _) = send(
        http::Method::DELETE,
        format!("/airport-configs/KIAD/{id}"),
        None,
    )
    .await;
    assert!(status.is_success(), "delete config: {status}");

    for (kind, table, feature) in [
        (
            "gates",
            "flow.airport_gate",
            json!({ "name": "A1", "lat": 38.95, "lon": -77.45 }),
        ),
        (
            "ramp-areas",
            "flow.airport_ramp_area",
            json!({ "name": "R1", "kind": "ramp", "rings": ring() }),
        ),
        (
            "taxiways",
            "flow.airport_taxiway",
            json!({ "name": "A", "rings": ring() }),
        ),
        (
            "runways",
            "flow.airport_runway",
            json!({ "name": "01/19", "rings": ring() }),
        ),
    ] {
        let base = format!("/airports/KIAD/{kind}");
        let (status, body) = send(http::Method::POST, base.clone(), Some(feature.clone())).await;
        assert_eq!(status, http::StatusCode::OK, "create {kind}: {body}");
        let id = body["id"].as_str().unwrap().to_string();
        assert_eq!(
            attributed(&pool, table, "updated_by", "id", &id).await,
            machine_row,
            "{kind}"
        );
        let (status, _) = send(http::Method::PUT, format!("{base}/{id}"), Some(feature)).await;
        assert_eq!(status, http::StatusCode::OK, "update {kind}");
        assert_eq!(
            attributed(&pool, table, "updated_by", "id", &id).await,
            machine_row,
            "{kind}"
        );
        let (_, surface) = send(http::Method::GET, "/airports/KIAD/surface".into(), None).await;
        let list = surface[kind.replace('-', "_")].as_array().unwrap().clone();
        assert!(
            list.iter().all(|f| f["editable"] == true),
            "{kind}: {surface}"
        );
        let (status, _) = send(http::Method::DELETE, format!("{base}/{id}"), None).await;
        assert!(status.is_success(), "delete {kind}: {status}");
    }

    let (status, body) = send(
        http::Method::POST,
        "/airports/KIAD/surface/repull-faa".into(),
        None,
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "repull: {body}");
}

/// AC4, airport data: a ZDC-scoped machine may edit KIAD (ZDC's) but not KJFK (ZNY's) — the scope check
/// these handlers already ran now sees the machine's roles.
#[sqlx::test]
async fn a_scoped_machine_is_refused_another_artccs_airport(pool: PgPool) {
    let (_, auth) = service_account(&pool, "events.config.update", Some("ZDC")).await;
    allow(&pool, "flow.surface_data.update").await;
    let state = airport_state(&pool);
    let gate = json!({ "name": "A1", "lat": 40.64, "lon": -73.78 });

    for (path, body) in [
        ("/api/v1/airport-configs/KJFK", config_body()),
        ("/api/v1/airports/KJFK/gates", gate.clone()),
    ] {
        let (status, _) = call(&state, http::Method::POST, path, &auth, Some(body)).await;
        assert_eq!(
            status,
            http::StatusCode::FORBIDDEN,
            "{path}: outside the machine's ARTCC"
        );
    }
    for (path, body) in [
        ("/api/v1/airport-configs/KIAD", config_body()),
        ("/api/v1/airports/KIAD/gates", gate),
    ] {
        let (status, _) = call(&state, http::Method::POST, path, &auth, Some(body)).await;
        assert_eq!(status, http::StatusCode::OK, "{path}: inside it");
    }
}

// ---- #607 PR 4: the long tail, and the lifecycle job acting as an actor ----

const PKG_EVENT: i64 = 6074;

/// An event inside the auto-publish window (it starts now), and a draft package on it holding one ZDC
/// advisory. Returns the package id.
async fn advisory_package(pool: &PgPool, state: &AppState, auth: &str) -> String {
    sqlx::query(
        "insert into events.event (id, title, start_time, end_time, facility) \
         values ($1, 'T607d', now(), now() + interval '2 hours', 'ZDC')",
    )
    .bind(PKG_EVENT)
    .execute(pool)
    .await
    .unwrap();
    let base = format!("/api/v1/events/{PKG_EVENT}/packages");
    let (status, body) = call(
        state,
        http::Method::POST,
        &base,
        auth,
        Some(json!({ "name": "Plan" })),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "create package: {body}");
    let pkg = body[0]["id"].as_str().unwrap().to_string();
    let from = Utc::now();
    let item = json!({ "kind": "advisory", "payload": {
        "facility": "ZDC", "kind": "reroute", "body": "vATCSCC ADVZY 607",
        "valid_from": from, "valid_to": from + Duration::hours(2) } });
    let (status, body) = call(
        state,
        http::Method::POST,
        &format!("{base}/{pkg}/items"),
        auth,
        Some(item),
    )
    .await;
    assert!(status.is_success(), "add advisory: {status} {body}");
    pkg
}

async fn arm(state: &AppState, auth: &str, pkg: &str) -> http::StatusCode {
    call(
        state,
        http::Method::PUT,
        &format!("/api/v1/events/{PKG_EVENT}/packages/{pkg}/auto"),
        auth,
        Some(json!({ "auto_publish": true })),
    )
    .await
    .0
}

async fn package_status(pool: &PgPool, pkg: &str) -> String {
    sqlx::query_scalar("select status from events.tmi_package where id = $1")
        .bind(pkg)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// `(created_by, created_by_actor)` of the one advisory a package issued, if any.
async fn issued_advisory(pool: &PgPool) -> Option<(Option<String>, Option<String>)> {
    sqlx::query_as("select created_by, created_by_actor from tmu.advisories")
        .fetch_optional(pool)
        .await
        .unwrap()
}

async fn lifecycle_pass(pool: &PgPool) {
    crate::jobs::event_package_lifecycle_once(pool, &tokio::sync::broadcast::channel(8).0)
        .await
        .unwrap();
}

/// The package handlers' half of AC1: a service account arms auto-publish, and the lifecycle job —
/// which used to act only as a person — publishes the advisory **as the machine**, then archives it as
/// the machine when the event ends.
#[sqlx::test]
async fn a_machine_arms_a_package_and_the_job_publishes_as_it(pool: PgPool) {
    let auth = machine(
        &pool,
        &["events.plan.update", "tmu.adv.create", "tmu.adv.publish"],
    )
    .await;
    let state = test_state(pool.clone(), HashMap::new());
    let pkg = advisory_package(&pool, &state, &auth).await;
    assert_eq!(arm(&state, &auth, &pkg).await, http::StatusCode::OK);
    let actor = machine_actor(&pool).await;
    assert_eq!(
        updated(&pool, "events.tmi_package", &format!("id = '{pkg}'")).await,
        (None, Some(actor.clone()))
    );

    lifecycle_pass(&pool).await;
    assert_eq!(package_status(&pool, &pkg).await, "activated");
    assert_eq!(
        issued_advisory(&pool).await,
        Some((None, Some(actor.clone()))),
        "issued as the machine"
    );

    sqlx::query("update events.event set start_time = now() - interval '3 hours', end_time = now() - interval '1 minute' where id = $1")
        .bind(PKG_EVENT)
        .execute(&pool)
        .await
        .unwrap();
    lifecycle_pass(&pool).await;
    assert_eq!(package_status(&pool, &pkg).await, "archived");
}

/// A machine whose credential stops working between arming and the event issues nothing — exactly as
/// its request would be refused. Archiving still runs on a revoked actor, so a package it activated
/// cannot outlive its event.
#[sqlx::test]
async fn a_disabled_machine_publishes_nothing(pool: PgPool) {
    let auth = machine(
        &pool,
        &["events.plan.update", "tmu.adv.create", "tmu.adv.publish"],
    )
    .await;
    let state = test_state(pool.clone(), HashMap::new());
    let pkg = advisory_package(&pool, &state, &auth).await;
    assert_eq!(arm(&state, &auth, &pkg).await, http::StatusCode::OK);
    sqlx::query("update access.service_accounts set status = 'disabled' where key = 'vtbfm'")
        .execute(&pool)
        .await
        .unwrap();

    lifecycle_pass(&pool).await;
    assert_eq!(package_status(&pool, &pkg).await, "draft");
    assert_eq!(issued_advisory(&pool).await, None);
}

/// AC4 at fire time: an API key arms, then loses its own `tmu.adv.publish` grant while its owner keeps
/// it. The job re-checks the **key** (owner ∩ key), not the owner, so nothing is issued. Before #607 a
/// key's arm was recorded under its owner and the job checked only the owner.
#[sqlx::test]
async fn a_keys_armed_package_is_held_to_the_keys_scope(pool: PgPool) {
    let (key, auth) = api_key(&pool, "events.plan.update").await;
    let owner: String =
        sqlx::query_scalar("select owner_user_id from access.api_keys where id = $1")
            .bind(&key)
            .fetch_one(&pool)
            .await
            .unwrap();
    for p in ["tmu.adv.create", "tmu.adv.publish"] {
        grant(&pool, &owner, p, None).await;
        sqlx::query(
            "insert into access.api_key_permissions (api_key_id, permission_name) values ($1, $2)",
        )
        .bind(&key)
        .bind(p)
        .execute(&pool)
        .await
        .unwrap();
    }
    let state = test_state(pool.clone(), HashMap::new());
    let pkg = advisory_package(&pool, &state, &auth).await;
    assert_eq!(arm(&state, &auth, &pkg).await, http::StatusCode::OK);
    assert_eq!(
        updated(&pool, "events.tmi_package", &format!("id = '{pkg}'")).await,
        (None, actor_of(&pool, "api_key_id", &key).await),
        "the key is recorded, not its owner"
    );

    sqlx::query("delete from access.api_key_permissions where api_key_id = $1 and permission_name = 'tmu.adv.publish'")
        .bind(&key)
        .execute(&pool)
        .await
        .unwrap();
    lifecycle_pass(&pool).await;
    assert_eq!(
        package_status(&pool, &pkg).await,
        "draft",
        "the owner still holds it; the key does not"
    );
    assert_eq!(issued_advisory(&pool).await, None);
}

/// The data migration: a package armed before 0114 has a person in `updated_by` and no actor. The
/// backfill names that person's actor, so the job — which now keys on the actor — still serves it.
#[sqlx::test]
async fn a_package_armed_before_actors_is_still_published(pool: PgPool) {
    let person = seed_user(&pool).await;
    for p in ["events.plan.update", "tmu.adv.create", "tmu.adv.publish"] {
        grant(&pool, &person, p, None).await;
    }
    let cookie = session_cookie(&pool, &person).await;
    let state = test_state(pool.clone(), HashMap::new());
    let pkg = advisory_package(&pool, &state, &cookie).await;
    assert_eq!(arm(&state, &cookie, &pkg).await, http::StatusCode::OK);
    // Back to the pre-0114 shape: a person, no actor (and no actor row for them at all).
    sqlx::query("update events.tmi_package set updated_by_actor = null where id = $1")
        .bind(&pkg)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("delete from access.actors where user_id = $1")
        .bind(&person)
        .execute(&pool)
        .await
        .unwrap();
    let migration = include_str!("../../migrations/0119_machine_long_tail_attribution.sql");
    let backfill = &migration[migration.find("insert into access.actors").unwrap()..];
    sqlx::raw_sql(backfill).execute(&pool).await.unwrap();

    lifecycle_pass(&pool).await;
    assert_eq!(package_status(&pool, &pkg).await, "activated");
    let (by, by_actor) = issued_advisory(&pool).await.unwrap();
    assert_eq!(by.as_deref(), Some(person.as_str()));
    assert_eq!(by_actor, actor_of(&pool, "user_id", &person).await);
}

/// AC3, the long tail: a service account drives each remaining write — each refused or anonymous
/// before #607 — and every row that records who did it names the machine.
#[sqlx::test]
async fn a_machine_drives_the_long_tail_writes(pool: PgPool) {
    let auth = machine(
        &pool,
        &[
            "facilities.docs.update",
            "facilities.docs.read",
            "flow.fca.update",
            "flow.fca.read",
            "flow.aircraft_profiles.update",
            "flow.facility_map.update",
            "flow.runway.update",
            "stats.capture.update",
            "ace.requests.decide",
        ],
    )
    .await;
    let state = test_state(pool.clone(), HashMap::new());
    let send = |method: http::Method, path: String, body: Option<Value>| {
        let (state, auth) = (state.clone(), auth.clone());
        async move { call(&state, method, &format!("/api/v1{path}"), &auth, body).await }
    };

    let (status, body) = send(
        http::Method::POST,
        "/facilities/ZDC/documents".into(),
        Some(json!({ "title": "SOP", "url": "https://example.org/sop.pdf" })),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "document: {body}");
    let (status, body) = send(http::Method::GET, "/facilities/ZDC/documents".into(), None).await;
    assert_eq!(status, http::StatusCode::OK, "documents: {body}");

    let fca = fca(&pool).await;
    let (status, body) = send(
        http::Method::POST,
        format!("/flow/fcas/{fca}/exclusions/BOGUS1"),
        Some(json!({ "reason": "teleporting" })),
    )
    .await;
    assert!(status.is_success(), "exclude: {status} {body}");
    let actor = machine_actor(&pool).await;
    let machine_row = (None, Some(actor.clone()));
    assert_eq!(
        attributed(
            &pool,
            "flow.manual_flight_exclusion",
            "created_by",
            "callsign",
            "BOGUS1"
        )
        .await,
        machine_row
    );
    let (_, list) = send(
        http::Method::GET,
        format!("/flow/fcas/{fca}/exclusions"),
        None,
    )
    .await;
    assert_eq!(list["exclusions"][0]["created_by_name"], "vTBFM", "{list}");
    let (status, _) = send(
        http::Method::DELETE,
        format!("/flow/fcas/{fca}/exclusions/BOGUS1"),
        None,
    )
    .await;
    assert!(status.is_success(), "restore: {status}");

    let profile = json!({ "name": "Test jet", "climb_ias_lo": 250.0, "climb_ias_hi": 290.0, "climb_fpm_lo": 1500.0,
        "climb_fpm_hi": 2500.0, "service_ceiling_ft": 41000.0, "desc_ias_hi": 290.0, "desc_ias_lo": 250.0, "desc_fpm": 2000.0 });
    let (status, body) = send(
        http::Method::PUT,
        "/flow/aircraft-profiles/type/T607".into(),
        Some(profile),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "profile: {body}");
    assert_eq!(
        attributed(&pool, "flow.aircraft_profile", "updated_by", "key", "T607").await,
        machine_row
    );
    assert_eq!(body["updated_by"], "vTBFM");

    let (status, body) = send(
        http::Method::PUT,
        "/facility-map/ZDC/config".into(),
        Some(json!({ "rules": [], "default_color": "#888888" })),
    )
    .await;
    assert!(status.is_success(), "facility map: {status} {body}");
    assert_eq!(
        attributed(
            &pool,
            "flow.facility_map_config",
            "updated_by",
            "facility_id",
            "ZDC"
        )
        .await,
        machine_row
    );
    let (_, body) = send(http::Method::GET, "/facility-map/ZDC/config".into(), None).await;
    assert_eq!(
        body["editable"], true,
        "the machine may edit its own map: {body}"
    );
    let (status, body) = call_with(
        &state,
        http::Method::GET,
        "/api/v1/facility-map/ZDC/config",
        &[],
        None,
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "still public");
    assert_eq!(body["editable"], false);

    let (status, body) = send(
        http::Method::PUT,
        "/flow/runway/KIAD".into(),
        Some(json!({ "active_ends": ["01C"] })),
    )
    .await;
    assert!(status.is_success(), "runway: {status} {body}");
    assert_eq!(
        attributed(&pool, "flow.runway_config", "updated_by", "icao", "KIAD").await,
        machine_row
    );
    let (status, body) = send(
        http::Method::PUT,
        "/flow/runway/KIAD/configs/South".into(),
        Some(json!({ "active_ends": ["19C"], "star_rules": {} })),
    )
    .await;
    assert!(status.is_success(), "saved runway config: {status} {body}");
    assert_eq!(
        attributed(
            &pool,
            "flow.runway_saved_config",
            "updated_by",
            "name",
            "South"
        )
        .await,
        machine_row
    );

    sqlx::query(
        "insert into stats.position (session_id, ts, lat, lon, altitude, groundspeed, heading) \
                 values (1, now() - interval '30 minutes', 38.9, -77.4, 10000, 300, 90)",
    )
    .execute(&pool)
    .await
    .unwrap();
    let now = Utc::now().timestamp();
    let (status, body) = send(
        http::Method::POST,
        "/stats/captures".into(),
        Some(json!({ "label": "T607", "from": now - 3600, "to": now })),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "capture: {body}");
    assert_eq!(
        attributed(&pool, "stats.capture", "created_by", "label", "T607").await,
        machine_row
    );

    let requester = seed_user(&pool).await;
    sqlx::query(
        "insert into events.event (id, title, start_time, end_time, facility) \
                 values (6075, 'ACE', now(), now() + interval '2 hours', 'ZDC')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let req: String = sqlx::query_scalar("insert into ace.requests (event_id, requested_by, slots) values (6075, $1, 1) returning id")
        .bind(&requester)
        .fetch_one(&pool)
        .await
        .unwrap();
    let (status, body) = send(
        http::Method::POST,
        format!("/events/6075/ace/{req}/decide"),
        Some(json!({ "outcome": "completed" })),
    )
    .await;
    assert_eq!(status, http::StatusCode::OK, "decide: {body}");
    assert_eq!(
        attributed(&pool, "ace.requests", "decided_by", "id", &req).await,
        machine_row
    );
    assert_eq!(body["decided_by_name"], "vTBFM");

    let (status, _) = send(http::Method::GET, "/admin/summary".into(), None).await;
    assert_eq!(status, http::StatusCode::OK, "admin summary");
}

/// AC4, the long tail: a ZDC-scoped machine is refused another ARTCC's documents and facility map.
#[sqlx::test]
async fn a_scoped_machine_is_refused_another_artccs_documents_and_map(pool: PgPool) {
    let (_, auth) = service_account(&pool, "facilities.docs.update", Some("ZDC")).await;
    allow(&pool, "flow.facility_map.update").await;
    let state = test_state(pool.clone(), HashMap::new());
    let doc = json!({ "title": "SOP", "url": "https://example.org/sop.pdf" });
    let map = json!({ "rules": [], "default_color": "#888888" });

    for (method, artcc, path, body) in [
        (
            http::Method::POST,
            "ZNY",
            "/api/v1/facilities/ZNY/documents",
            doc.clone(),
        ),
        (
            http::Method::PUT,
            "ZNY",
            "/api/v1/facility-map/ZNY/config",
            map.clone(),
        ),
        (
            http::Method::POST,
            "ZDC",
            "/api/v1/facilities/ZDC/documents",
            doc,
        ),
        (
            http::Method::PUT,
            "ZDC",
            "/api/v1/facility-map/ZDC/config",
            map,
        ),
    ] {
        let (status, _) = call(&state, method, path, &auth, Some(body)).await;
        if artcc == "ZNY" {
            assert_eq!(
                status,
                http::StatusCode::FORBIDDEN,
                "{path}: outside the machine's ARTCC"
            );
        } else {
            assert!(status.is_success(), "{path}: inside it ({status})");
        }
    }
}

/// Archiving only cancels what a package issued, so it runs even after the activating machine is
/// disabled — otherwise its advisory would outlive the event.
#[sqlx::test]
async fn a_disabled_machines_package_is_still_archived(pool: PgPool) {
    let auth = machine(
        &pool,
        &["events.plan.update", "tmu.adv.create", "tmu.adv.publish"],
    )
    .await;
    let state = test_state(pool.clone(), HashMap::new());
    let pkg = advisory_package(&pool, &state, &auth).await;
    assert_eq!(arm(&state, &auth, &pkg).await, http::StatusCode::OK);
    lifecycle_pass(&pool).await;
    assert_eq!(package_status(&pool, &pkg).await, "activated");

    sqlx::query("update access.service_accounts set status = 'disabled' where key = 'vtbfm'")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("update events.event set start_time = now() - interval '3 hours', end_time = now() - interval '1 minute' where id = $1")
        .bind(PKG_EVENT)
        .execute(&pool)
        .await
        .unwrap();
    lifecycle_pass(&pool).await;
    assert_eq!(package_status(&pool, &pkg).await, "archived");
    let actor = machine_actor(&pool).await;
    assert_eq!(
        updated(&pool, "events.tmi_package", &format!("id = '{pkg}'")).await,
        (None, Some(actor))
    );
}
