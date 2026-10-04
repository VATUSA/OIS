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

/// [`call`] carrying several credentials at once — each a bearer (`Authorization`) or a cookie.
async fn call_with(
    state: &AppState,
    method: http::Method,
    uri: &str,
    auth: &[&str],
    body: Option<Value>,
) -> (http::StatusCode, Value) {
    use tower::ServiceExt;

    let mut builder = http::Request::builder().method(method).uri(uri);
    for credential in auth {
        let header = if credential.starts_with("Bearer ") {
            http::header::AUTHORIZATION
        } else {
            http::header::COOKIE
        };
        builder = builder.header(header, *credential);
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

    let (status, body) = call(
        &state,
        http::Method::POST,
        "/api/v1/tmu/cfr",
        &auth,
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

    let (status, body) = call(
        &state,
        http::Method::POST,
        "/api/v1/tmu/cfr",
        &auth,
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

/// Regression pin: a path that never read `CurrentUser`, so it already worked for a machine.
#[sqlx::test]
async fn a_service_account_releases_a_cfr(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let (_, auth) = service_account(&pool, "tmu.cfr.assign", None).await;
    assert_eq!(
        call(
            &state,
            http::Method::POST,
            "/api/v1/tmu/cfr",
            &auth,
            Some(issue_body())
        )
        .await
        .0,
        http::StatusCode::OK
    );

    let (status, _) = call(
        &state,
        http::Method::DELETE,
        "/api/v1/tmu/cfr/AAL1",
        &auth,
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
    let state = test_state(pool.clone(), HashMap::new());
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
    let fca_id = fca(&pool).await;
    let (sa, auth) = service_account(&pool, "flow.fca.update", None).await;

    let (status, body) = call(
        &state,
        http::Method::POST,
        &format!("/api/v1/flow/fcas/{fca_id}/release/TEST1"),
        &auth,
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
    seed_release(&pool, &fca_id, "AAL1", 1_000).await;
    seed_release(&pool, &fca_id, "UAL2", 2_000).await;
    let (sa, auth) = service_account(&pool, "flow.fca.update", None).await;

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

/// Regression pin: clearing never read `CurrentUser`.
#[sqlx::test]
async fn a_service_account_clears_a_release(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let fca_id = fca(&pool).await;
    seed_release(&pool, &fca_id, "AAL1", 1_000).await;
    let (_, auth) = service_account(&pool, "flow.fca.update", None).await;

    let (status, _) = call(
        &state,
        http::Method::DELETE,
        &format!("/api/v1/flow/fcas/{fca_id}/release/AAL1"),
        &auth,
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

/// #636: deleting an FCA stays open to a machine credential, as it was before the ARTCC gate — and the
/// gate applies to it too. A ZDC-scoped service account deletes a ZDC FCA, not a ZNY one.
#[sqlx::test]
async fn a_scoped_service_account_deletes_only_its_artccs_fcas(pool: PgPool) {
    let state = test_state(pool.clone(), HashMap::new());
    let (_, bearer) = service_account(&pool, "flow.fca.delete", Some("ZDC")).await;
    let owner = seed_user(&pool).await;
    let mut ids = HashMap::new();
    for artcc in ["ZDC", "ZNY"] {
        let req = serde_json::from_value(
            json!({ "name": artcc, "artcc": artcc, "points": [[0.0, 0.0], [1.0, 1.0]] }),
        )
        .unwrap();
        ids.insert(
            artcc,
            crate::repos::flow::create_fca(&pool, &req, &owner)
                .await
                .unwrap(),
        );
    }
    let uri = |artcc: &str| format!("/api/v1/flow/fcas/{}", ids[artcc]);

    let (status, _) = call(&state, http::Method::DELETE, &uri("ZNY"), &bearer, None).await;
    assert_eq!(status, http::StatusCode::FORBIDDEN);
    let (status, _) = call(&state, http::Method::DELETE, &uri("ZDC"), &bearer, None).await;
    assert_eq!(status, http::StatusCode::NO_CONTENT);
}
