//! VATUSA/OIS#913: a failed audit write or actor lookup is logged, and the request it records still
//! succeeds. Every request test goes through the real router, so the bearer and session middleware,
//! the handler and the audit middleware (`audit.rs`) all run; one repo-level test pins
//! `record_audit` for its `?` callers. The database failure is injected with a trigger that raises,
//! which only a `#[sqlx::test]` database can carry.

use std::{collections::HashMap, sync::Arc};

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::{
    feed::sectors::{SectorTable, tests::volume},
    repos::access::sha256_hex,
    scope_test_support::{
        grant,
        log_capture::{Captured, capturing, saw_the_request},
        seed_user, session_cookie, test_state,
    },
    state::AppState,
};

const UPDATE: &str = "flow.sector_limits.update";
const KEY_CREATE: &str = "api_keys.key.create";
/// The middleware-audited write every middleware test sends: `PUT …/{artcc}/{sector_id}`, logged as
/// `update` on `flow.sector-limits`, resource `010`.
const LIMIT_URI: &str = "/api/v1/flow/sector-limits/ZDC/010";
const AUDIT_TARGET: &str = "ois_backend::repos::audit";
/// The `error` field `refuse_inserts(.., "audit insert refused by test")` produces: sqlx's Display of
/// a database error. Its Debug form also contains the message, so only the whole string tells them
/// apart.
const REFUSED_DISPLAY: &str = "error returned from database: audit insert refused by test";

/// ZDC owns sector 010, so `LIMIT_URI` reaches the write.
fn state(pool: PgPool) -> AppState {
    let state = test_state(pool, HashMap::new());
    state.airspace_sectors.store(Arc::new(SectorTable {
        volumes: vec![volume("ZDC", "01001")],
    }));
    state
}

/// Every insert into `table` raises `message`.
async fn refuse_inserts(pool: &PgPool, table: &str, message: &str) {
    sqlx::query(&format!(
        "create function test_refuse() returns trigger language plpgsql as \
         $$ begin raise exception '{message}'; end $$"
    ))
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(&format!(
        "create trigger test_refuse before insert on {table} \
         for each row execute function test_refuse()"
    ))
    .execute(pool)
    .await
    .unwrap();
}

/// One request through the real router. `auth` is a bearer (`Bearer …`) or a session cookie.
async fn request(
    state: &AppState,
    method: Method,
    uri: &str,
    auth: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    use tower::ServiceExt;

    let builder = http::Request::builder().method(method).uri(uri);
    let builder = if auth.starts_with("Bearer ") {
        builder.header(http::header::AUTHORIZATION, auth)
    } else {
        builder.header(http::header::COOKIE, auth)
    };
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

/// The audit repo's events at `level`.
fn audit_events(events: &[Captured], level: tracing::Level) -> Vec<&Captured> {
    events
        .iter()
        .filter(|e| e.level == level && e.target == AUDIT_TARGET)
        .collect()
}

/// `(actor_id, resource_type, resource_id)` of every audit row, oldest first.
async fn audit_rows(pool: &PgPool) -> Vec<(Option<String>, String, Option<String>)> {
    sqlx::query_as(
        "select actor_id, resource_type, resource_id from access.audit_logs order by created_at, id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn stored_limit(pool: &PgPool) -> Option<i32> {
    sqlx::query_scalar(
        "select limit_value from flow.sector_limit where artcc = 'ZDC' and sector_id = '010'",
    )
    .fetch_optional(pool)
    .await
    .unwrap()
}

/// A signed-in user holding `permission` nationally. Returns `(user id, cookie)`.
async fn user_with(pool: &PgPool, permission: &str) -> (String, String) {
    let id = seed_user(pool).await;
    grant(pool, &id, permission, None).await;
    let cookie = session_cookie(pool, &id).await;
    (id, cookie)
}

/// A user's API key granted `UPDATE`, its owner holding it too. Returns `(key id, bearer)`.
async fn api_key(pool: &PgPool) -> (String, String) {
    let (owner, _) = user_with(pool, UPDATE).await;
    let token = format!("ois_pat_{}", uuid::Uuid::new_v4().simple());
    let id: String = sqlx::query_scalar(
        "insert into access.api_keys (owner_user_id, name, prefix, secret_hash) \
         values ($1, 'audit key', 'ois_pat_t913', $2) returning id",
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
    .bind(UPDATE)
    .execute(pool)
    .await
    .unwrap();
    (id, format!("Bearer {token}"))
}

/// A service account holding `UPDATE` nationally through a role. Returns `(account id, bearer)`.
async fn service_account(pool: &PgPool) -> (String, String) {
    for sql in [
        "insert into access.roles (name, is_system) values ('T913_MACHINE', false)",
        "insert into access.role_permissions (role_name, permission_name) \
         values ('T913_MACHINE', 'flow.sector_limits.update')",
    ] {
        sqlx::query(sql).execute(pool).await.unwrap();
    }
    let id: String = sqlx::query_scalar(
        "insert into access.service_accounts (key, name) values ('t913', 'T913') returning id",
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
        "insert into access.service_account_roles (service_account_id, role_name) \
         values ($1, 'T913_MACHINE')",
    )
    .bind(&id)
    .execute(pool)
    .await
    .unwrap();
    (id, format!("Bearer {token}"))
}

// ---- AC 1: the middleware's failed insert -----------------------------------------------------------

#[sqlx::test]
async fn a_failed_audit_insert_is_logged_and_the_write_still_answers_200(pool: PgPool) {
    let state = state(pool.clone());
    let (_, cookie) = user_with(&pool, UPDATE).await;
    refuse_inserts(&pool, "access.audit_logs", "audit insert refused by test").await;

    let ((status, body), events) = capturing(request(
        &state,
        Method::PUT,
        LIMIT_URI,
        &cookie,
        Some(json!({ "limit": 14 })),
    ))
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        stored_limit(&pool).await,
        Some(14),
        "the write itself landed"
    );
    assert!(saw_the_request(&events), "capture was live: {events:?}");
    let errors = audit_events(&events, tracing::Level::ERROR);
    assert_eq!(
        errors.len(),
        1,
        "one ERROR for the one failed insert: {events:?}"
    );
    let error = errors[0];
    assert_eq!(error.fields["action"], "update");
    assert_eq!(error.fields["resource_type"], "flow.sector-limits");
    assert_eq!(error.fields["resource_id"], "010");
    assert_eq!(
        error.fields["error"], REFUSED_DISPLAY,
        "the database error's Display is logged, not its Debug"
    );
    assert!(
        audit_rows(&pool).await.is_empty(),
        "the insert really failed"
    );
}

/// The positive side of AC 1: an insert that succeeds logs nothing and writes its row.
#[sqlx::test]
async fn a_successful_audit_insert_logs_no_error(pool: PgPool) {
    let state = state(pool.clone());
    let (_, cookie) = user_with(&pool, UPDATE).await;

    let ((status, _), events) = capturing(request(
        &state,
        Method::PUT,
        LIMIT_URI,
        &cookie,
        Some(json!({ "limit": 14 })),
    ))
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(saw_the_request(&events));
    assert!(
        audit_events(&events, tracing::Level::ERROR).is_empty(),
        "{events:?}"
    );
    assert!(
        audit_events(&events, tracing::Level::WARN).is_empty(),
        "{events:?}"
    );
    let rows = audit_rows(&pool).await;
    assert_eq!(rows.len(), 1);
    assert!(rows[0].0.is_some(), "the actor resolved");
    assert_eq!(
        (rows[0].1.as_str(), rows[0].2.as_deref()),
        ("flow.sector-limits", Some("010"))
    );
}

/// A write the middleware audits with no path parameter has no resource id; its failed insert logs
/// the field blank rather than dropping it.
#[sqlx::test]
async fn a_failed_insert_with_no_resource_id_logs_it_blank(pool: PgPool) {
    let state = state(pool.clone());
    let (_, cookie) = user_with(&pool, "auth.profile.read").await;
    refuse_inserts(&pool, "access.audit_logs", "audit insert refused by test").await;

    let ((status, body), events) = capturing(request(
        &state,
        Method::POST,
        "/api/v1/dashboards",
        &cookie,
        Some(json!({ "name": "audit test board" })),
    ))
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(saw_the_request(&events), "capture was live: {events:?}");
    let errors = audit_events(&events, tracing::Level::ERROR);
    assert_eq!(errors.len(), 1, "{events:?}");
    assert_eq!(errors[0].fields["action"], "create");
    assert_eq!(errors[0].fields["resource_type"], "dashboards");
    assert_eq!(errors[0].fields["resource_id"], "");
    assert_eq!(errors[0].fields["error"], REFUSED_DISPLAY);
}

/// `record_audit`, which the access editor and the VATUSA reconciler propagate with `?`, still
/// fails its caller when the insert fails.
#[sqlx::test]
async fn record_audit_still_returns_the_failure_to_its_caller(pool: PgPool) {
    refuse_inserts(&pool, "access.audit_logs", "audit insert refused by test").await;
    let entry = || crate::repos::audit::AuditEntry {
        actor_id: None,
        action: "update".into(),
        resource_type: "flow.sector-limits".into(),
        resource_id: Some("010".into()),
        artcc_id: None,
        reason: None,
        before_state: None,
        after_state: None,
        ip_address: None,
    };

    let result = crate::repos::audit::record_audit(&pool, entry()).await;

    assert!(
        matches!(result, Err(crate::errors::ApiError::Internal)),
        "{result:?}"
    );
    sqlx::query("drop trigger test_refuse on access.audit_logs")
        .execute(&pool)
        .await
        .unwrap();
    crate::repos::audit::record_audit(&pool, entry())
        .await
        .expect("positive control: the same entry records once the trigger is gone");
}

// ---- AC 2: audit_key's failed insert, every key action ----------------------------------------------

#[sqlx::test]
async fn every_api_key_action_succeeds_and_logs_its_failed_audit_insert(pool: PgPool) {
    let state = state(pool.clone());
    let (_, cookie) = user_with(&pool, KEY_CREATE).await;
    refuse_inserts(&pool, "access.audit_logs", "audit insert refused by test").await;

    let ((status, body), mut events) = capturing(request(
        &state,
        Method::POST,
        "/api/v1/api-keys",
        &cookie,
        Some(json!({ "name": "audit test key", "permissions": [] })),
    ))
    .await;
    assert_eq!(status, StatusCode::OK, "create: {body}");
    let id = body["key"]["id"]
        .as_str()
        .expect("the created key's id")
        .to_owned();

    let steps: [(Method, String, Option<Value>, StatusCode); 4] = [
        (
            Method::POST,
            format!("/api/v1/api-keys/{id}/rotate"),
            None,
            StatusCode::OK,
        ),
        (
            Method::PUT,
            format!("/api/v1/api-keys/{id}/permissions"),
            Some(json!({ "permissions": [] })),
            StatusCode::OK,
        ),
        (
            Method::POST,
            format!("/api/v1/api-keys/{id}/disable"),
            None,
            StatusCode::NO_CONTENT,
        ),
        (
            Method::DELETE,
            format!("/api/v1/api-keys/{id}"),
            None,
            StatusCode::NO_CONTENT,
        ),
    ];
    for (method, uri, payload, expected) in steps {
        let ((status, body), more) =
            capturing(request(&state, method, &uri, &cookie, payload)).await;
        assert_eq!(status, expected, "{uri}: {body}");
        events.extend(more);
    }

    let logged: Vec<(&str, &str, &str)> = audit_events(&events, tracing::Level::ERROR)
        .iter()
        .map(|e| {
            assert_eq!(e.fields["error"], REFUSED_DISPLAY, "{:?}", e.fields);
            (
                e.fields["action"].as_str(),
                e.fields["resource_type"].as_str(),
                e.fields["resource_id"].as_str(),
            )
        })
        .collect();
    assert_eq!(
        logged,
        ["create", "rotate", "update", "disable", "delete"].map(|action| (
            action,
            "API_KEY",
            id.as_str()
        )),
        "one ERROR per key action, in order"
    );
    assert!(
        audit_rows(&pool).await.is_empty(),
        "every insert really failed"
    );
}

/// The positive side of AC 2: a key action on a healthy database logs nothing and records its row
/// against the acting user's audit actor.
#[sqlx::test]
async fn an_api_key_action_records_its_row_with_the_users_actor(pool: PgPool) {
    let state = state(pool.clone());
    let (user, cookie) = user_with(&pool, KEY_CREATE).await;

    let ((status, body), events) = capturing(request(
        &state,
        Method::POST,
        "/api/v1/api-keys",
        &cookie,
        Some(json!({ "name": "audit test key", "permissions": [] })),
    ))
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let id = body["key"]["id"].as_str().expect("the created key's id");
    assert!(saw_the_request(&events), "capture was live: {events:?}");
    assert!(
        audit_events(&events, tracing::Level::ERROR).is_empty(),
        "{events:?}"
    );
    assert!(
        audit_events(&events, tracing::Level::WARN).is_empty(),
        "{events:?}"
    );
    let actor: String = sqlx::query_scalar(
        "select id from access.actors where actor_type = 'user' and user_id = $1",
    )
    .bind(&user)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        audit_rows(&pool).await,
        [(Some(actor), "API_KEY".to_owned(), Some(id.to_owned()))]
    );
}

// ---- AC 3: a failed actor resolution ---------------------------------------------------------------

/// The WARN for one failed resolution, and the null-actor row written in its place.
async fn assert_warned_and_recorded(
    pool: &PgPool,
    events: &[Captured],
    principal_kind: &str,
    principal_id: &str,
    resource: (&str, &str),
) {
    assert!(saw_the_request(events), "capture was live: {events:?}");
    let warns = audit_events(events, tracing::Level::WARN);
    assert_eq!(
        warns.len(),
        1,
        "one WARN for the one failed lookup: {events:?}"
    );
    assert_eq!(warns[0].fields["principal_kind"], principal_kind);
    assert_eq!(warns[0].fields["principal_id"], principal_id);
    assert!(
        audit_events(events, tracing::Level::ERROR).is_empty(),
        "{events:?}"
    );
    assert_eq!(
        audit_rows(pool).await,
        [(None, resource.0.to_owned(), Some(resource.1.to_owned()))],
        "the entry is still written, with a null actor"
    );
}

#[sqlx::test]
async fn a_users_failed_actor_lookup_warns_and_still_records_the_write(pool: PgPool) {
    let state = state(pool.clone());
    let (user, cookie) = user_with(&pool, UPDATE).await;
    refuse_inserts(&pool, "access.actors", "actor insert refused by test").await;

    let ((status, body), events) = capturing(request(
        &state,
        Method::PUT,
        LIMIT_URI,
        &cookie,
        Some(json!({ "limit": 14 })),
    ))
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_warned_and_recorded(&pool, &events, "user", &user, ("flow.sector-limits", "010")).await;
}

#[sqlx::test]
async fn an_api_keys_failed_actor_lookup_warns_and_still_records_the_write(pool: PgPool) {
    let state = state(pool.clone());
    let (key, bearer) = api_key(&pool).await;
    refuse_inserts(&pool, "access.actors", "actor insert refused by test").await;

    let ((status, body), events) = capturing(request(
        &state,
        Method::PUT,
        LIMIT_URI,
        &bearer,
        Some(json!({ "limit": 14 })),
    ))
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_warned_and_recorded(
        &pool,
        &events,
        "api_key",
        &key,
        ("flow.sector-limits", "010"),
    )
    .await;
}

#[sqlx::test]
async fn a_service_accounts_failed_actor_lookup_warns_and_still_records_the_write(pool: PgPool) {
    let state = state(pool.clone());
    let (account, bearer) = service_account(&pool).await;
    refuse_inserts(&pool, "access.actors", "actor insert refused by test").await;

    let ((status, body), events) = capturing(request(
        &state,
        Method::PUT,
        LIMIT_URI,
        &bearer,
        Some(json!({ "limit": 14 })),
    ))
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_warned_and_recorded(
        &pool,
        &events,
        "service_account",
        &account,
        ("flow.sector-limits", "010"),
    )
    .await;
}

#[sqlx::test]
async fn audit_keys_failed_actor_lookup_warns_and_still_records_the_key_action(pool: PgPool) {
    let state = state(pool.clone());
    let (user, cookie) = user_with(&pool, KEY_CREATE).await;
    refuse_inserts(&pool, "access.actors", "actor insert refused by test").await;

    let ((status, body), events) = capturing(request(
        &state,
        Method::POST,
        "/api/v1/api-keys",
        &cookie,
        Some(json!({ "name": "audit test key", "permissions": [] })),
    ))
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let id = body["key"]["id"].as_str().expect("the created key's id");
    assert_warned_and_recorded(&pool, &events, "user", &user, ("API_KEY", id)).await;
}
