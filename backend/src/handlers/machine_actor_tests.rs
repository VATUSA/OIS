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
    let state = test_state(pool, HashMap::new());
    {
        let mut feed = state.feed.write().await;
        feed.airports = Arc::new(HashMap::from([
            ("KJFK".to_string(), Airport::at(40.64, -73.78)),
            ("KDCA".to_string(), Airport::at(38.85, -77.04)),
        ]));
        feed.snapshot = Some(Arc::new(crate::feed::Snapshot::of(VatsimData {
            prefiles: vec![Prefile {
                callsign: "TEST1".into(),
                flight_plan: Some(FlightPlan {
                    departure: "KJFK".into(),
                    arrival: "KDCA".into(),
                    route: "RBV WHITE SIE".into(),
                    ..Default::default()
                }),
                ..Default::default()
            }],
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
        !matches!(scope, PermissionScope::National),
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
    assert!(matches!(national, PermissionScope::National));
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
