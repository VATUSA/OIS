//! #854: the six event FCA write routes (`/api/v1/events/{id}/fcas…`) check the caller's
//! `events.plan.update` scope against the FCA's ARTCC, through the real router.
//!
//! Each route's refusal is its own `*_is_refused_outside_*` test, so deleting one route's scope
//! call reds that route's test and no other route's.
//!
//! The two gates answer differently: `RequirePermission` refuses a caller without the permission
//! with 401 (`auth::middleware::ensure_permission`), the scope check refuses one holding it at
//! another facility with 403.

use http::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::repos::access::sha256_hex;
use crate::scope_test_support::{deny_scoped, grant, seed_user, send, session_cookie, test_state};
use crate::state::AppState;

const PERM: &str = "events.plan.update";

/// Event 7360 with its FCAs, and event 7361 with one FCA of its own:
///
/// | id               | event | artcc | event_status | auto_publish |
/// |------------------|-------|-------|--------------|--------------|
/// | `ev-planned`     | 7360  | ZDC   | planned      | false        |
/// | `ev-planned-2`   | 7360  | ZDC   | planned      | false        |
/// | `ev-published`   | 7360  | ZDC   | published    | false        |
/// | `ev-archived`    | 7360  | ZDC   | archived     | false        |
/// | `ev-ztl`         | 7360  | ZTL   | planned      | false        |
/// | `ev-blank`       | 7360  | `''`  | planned      | false        |
/// | `other-event`    | 7361  | ZDC   | planned      | false        |
async fn seed(pool: &PgPool) {
    sqlx::query(
        "insert into events.event (id, title, start_time, end_time) values \
           (7360, 'Event FCA scope', now() + interval '1 day', now() + interval '2 days'), \
           (7361, 'Other event', now() + interval '1 day', now() + interval '2 days')",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "insert into flow.fca (id, name, artcc, enabled, event_id, event_status, auto_publish) values \
           ('ev-planned',   'Planned',    'ZDC', true, 7360, 'planned',   false), \
           ('ev-planned-2', 'Planned 2',  'ZDC', true, 7360, 'planned',   false), \
           ('ev-published', 'Published',  'ZDC', true, 7360, 'published', false), \
           ('ev-archived',  'Archived',   'ZDC', true, 7360, 'archived',  false), \
           ('ev-ztl',       'ZTL planned','ZTL', true, 7360, 'planned',   false), \
           ('ev-blank',     'Blank',      '',    true, 7360, 'planned',   false), \
           ('other-event',  'Other',      'ZDC', true, 7361, 'planned',   false)",
    )
    .execute(pool)
    .await
    .unwrap();
}

/// `(name, artcc, deleted, event_status, auto_publish)` of a row.
type Row = (String, String, bool, Option<String>, bool);

async fn row(pool: &PgPool, id: &str) -> Row {
    sqlx::query_as(
        "select name, artcc, deleted_at is not null, event_status, auto_publish \
         from flow.fca where id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// FCAs on `event`, deleted ones included.
async fn count(pool: &PgPool, event: i64) -> i64 {
    sqlx::query_scalar("select count(*) from flow.fca where event_id = $1")
        .bind(event)
        .fetch_one(pool)
        .await
        .unwrap()
}

fn fca(name: &str, artcc: &str) -> Value {
    json!({ "name": name, "artcc": artcc, "points": [[0.0, 0.0], [1.0, 1.0]] })
}

/// A signed-in user holding `events.plan.update` at `artcc` (`None` = nationally).
async fn planner(pool: &PgPool, artcc: Option<&str>) -> String {
    let user = seed_user(pool).await;
    grant(pool, &user, PERM, artcc).await;
    session_cookie(pool, &user).await
}

/// A signed-in user holding FCA and read permissions at ZDC, but not `events.plan.update`.
async fn non_planner(pool: &PgPool) -> String {
    let user = seed_user(pool).await;
    grant(pool, &user, "events.plan.read", Some("ZDC")).await;
    grant(pool, &user, "flow.fca.update", Some("ZDC")).await;
    session_cookie(pool, &user).await
}

/// The statuses an anonymous caller, a signed-in non-planner and a ZTL-scoped planner get, in that
/// order (least privileged first).
async fn refusals(
    state: &AppState,
    pool: &PgPool,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> [StatusCode; 3] {
    let anonymous = send(state, method.clone(), uri, "", body.clone()).await;
    let none = non_planner(pool).await;
    let no_permission = send(state, method.clone(), uri, &none, body.clone()).await;
    let ztl = planner(pool, Some("ZTL")).await;
    let out_of_scope = send(state, method, uri, &ztl, body).await;
    [anonymous, no_permission, out_of_scope]
}

const REFUSED: [StatusCode; 3] = [
    StatusCode::UNAUTHORIZED,
    StatusCode::UNAUTHORIZED,
    StatusCode::FORBIDDEN,
];

fn state(pool: &PgPool) -> AppState {
    test_state(pool.clone(), Default::default())
}

#[sqlx::test]
async fn create_is_refused_outside_the_payload_artcc_and_allowed_inside_it(pool: PgPool) {
    let state = state(&pool);
    seed(&pool).await;
    let uri = "/api/v1/events/7360/fcas";

    let got = refusals(
        &state,
        &pool,
        Method::POST,
        uri,
        Some(fca("New ZDC", "ZDC")),
    )
    .await;
    assert_eq!(got, REFUSED);
    assert_eq!(count(&pool, 7360).await, 6, "a refused create wrote a row");

    let zdc = planner(&pool, Some("ZDC")).await;
    let status = send(&state, Method::POST, uri, &zdc, Some(fca("ZDC new", "ZDC"))).await;
    assert_eq!(status, StatusCode::OK);
    let national = planner(&pool, None).await;
    let status = send(
        &state,
        Method::POST,
        uri,
        &national,
        Some(fca("National new", "ZDC")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(count(&pool, 7360).await, 8);
}

/// Create checks the ARTCC the row stores: `bind_fca` trims and uppercases it.
#[sqlx::test]
async fn create_normalises_the_payload_artcc_before_the_scope_check(pool: PgPool) {
    let state = state(&pool);
    seed(&pool).await;
    let uri = "/api/v1/events/7360/fcas";

    let ztl = planner(&pool, Some("ZTL")).await;
    let status = send(
        &state,
        Method::POST,
        uri,
        &ztl,
        Some(fca("Padded", " zdc ")),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(count(&pool, 7360).await, 6);

    let zdc = planner(&pool, Some("ZDC")).await;
    let status = send(
        &state,
        Method::POST,
        uri,
        &zdc,
        Some(fca("Padded", " zdc ")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let artcc: String =
        sqlx::query_scalar("select artcc from flow.fca where event_id = 7360 and name = 'Padded'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(artcc, "ZDC");
}

#[sqlx::test]
async fn update_is_refused_outside_the_stored_artcc_and_allowed_inside_it(pool: PgPool) {
    let state = state(&pool);
    seed(&pool).await;
    let uri = "/api/v1/events/7360/fcas/ev-planned";
    let before = row(&pool, "ev-planned").await;

    let got = refusals(&state, &pool, Method::PUT, uri, Some(fca("Edited", "ZDC"))).await;
    assert_eq!(got, REFUSED);
    assert_eq!(row(&pool, "ev-planned").await, before);

    let zdc = planner(&pool, Some("ZDC")).await;
    let status = send(&state, Method::PUT, uri, &zdc, Some(fca("ZDC edit", "ZDC"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(row(&pool, "ev-planned").await.0, "ZDC edit");
    let national = planner(&pool, None).await;
    let status = send(
        &state,
        Method::PUT,
        uri,
        &national,
        Some(fca("National edit", "ZDC")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(row(&pool, "ev-planned").await.0, "National edit");
}

/// Update checks the payload ARTCC as well as the stored one: an FCA can't be moved out of the
/// caller's facility.
#[sqlx::test]
async fn update_refuses_a_zdc_planner_relabelling_a_zdc_fca_to_ztl(pool: PgPool) {
    let state = state(&pool);
    seed(&pool).await;
    let uri = "/api/v1/events/7360/fcas/ev-planned";
    let before = row(&pool, "ev-planned").await;
    let zdc = planner(&pool, Some("ZDC")).await;

    let status = send(&state, Method::PUT, uri, &zdc, Some(fca("Moved", "ZTL"))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(row(&pool, "ev-planned").await, before);

    let status = send(&state, Method::PUT, uri, &zdc, Some(fca("Kept", "ZDC"))).await;
    assert_eq!(status, StatusCode::OK);
}

/// Update checks the stored ARTCC even when the payload's is in scope (a ZDC planner can't take a
/// ZTL FCA over), and the payload's when the stored one is (a ZTL planner can't hand it to ZDC).
#[sqlx::test]
async fn update_refuses_moving_a_ztl_fca_to_zdc_from_either_facility(pool: PgPool) {
    let state = state(&pool);
    seed(&pool).await;
    let uri = "/api/v1/events/7360/fcas/ev-ztl";
    let before = row(&pool, "ev-ztl").await;

    let zdc = planner(&pool, Some("ZDC")).await;
    let status = send(&state, Method::PUT, uri, &zdc, Some(fca("Taken", "ZDC"))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(row(&pool, "ev-ztl").await, before);

    let ztl = planner(&pool, Some("ZTL")).await;
    let status = send(&state, Method::PUT, uri, &ztl, Some(fca("Given", "ZDC"))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(row(&pool, "ev-ztl").await, before);

    let status = send(&state, Method::PUT, uri, &ztl, Some(fca("ZTL edit", "ZTL"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(row(&pool, "ev-ztl").await.0, "ZTL edit");
}

#[sqlx::test]
async fn delete_is_refused_outside_the_stored_artcc_and_allowed_inside_it(pool: PgPool) {
    let state = state(&pool);
    seed(&pool).await;

    let got = refusals(
        &state,
        &pool,
        Method::DELETE,
        "/api/v1/events/7360/fcas/ev-planned",
        None,
    )
    .await;
    assert_eq!(got, REFUSED);
    assert!(
        !row(&pool, "ev-planned").await.2,
        "a refused delete deleted the FCA"
    );

    let zdc = planner(&pool, Some("ZDC")).await;
    let status = send(
        &state,
        Method::DELETE,
        "/api/v1/events/7360/fcas/ev-planned",
        &zdc,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(row(&pool, "ev-planned").await.2);
    let national = planner(&pool, None).await;
    let status = send(
        &state,
        Method::DELETE,
        "/api/v1/events/7360/fcas/ev-planned-2",
        &national,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(row(&pool, "ev-planned-2").await.2);
}

#[sqlx::test]
async fn publish_is_refused_outside_the_stored_artcc_and_allowed_inside_it(pool: PgPool) {
    let state = state(&pool);
    seed(&pool).await;
    let mut topics = state.events.subscribe();

    let uri = "/api/v1/events/7360/fcas/ev-planned/publish";
    let got = refusals(&state, &pool, Method::POST, uri, None).await;
    assert_eq!(got, REFUSED);
    assert_eq!(row(&pool, "ev-planned").await.3.as_deref(), Some("planned"));
    assert!(
        topics.try_recv().is_err(),
        "a refused publish nudged the maps"
    );

    let zdc = planner(&pool, Some("ZDC")).await;
    assert_eq!(
        send(&state, Method::POST, uri, &zdc, None).await,
        StatusCode::OK
    );
    assert_eq!(
        row(&pool, "ev-planned").await.3.as_deref(),
        Some("published")
    );
    assert_eq!(
        topics.try_recv().unwrap().topic,
        crate::realtime::topic::FCA
    );
    let national = planner(&pool, None).await;
    let uri = "/api/v1/events/7360/fcas/ev-planned-2/publish";
    assert_eq!(
        send(&state, Method::POST, uri, &national, None).await,
        StatusCode::OK
    );
    assert_eq!(
        row(&pool, "ev-planned-2").await.3.as_deref(),
        Some("published")
    );
}

#[sqlx::test]
async fn archive_is_refused_outside_the_stored_artcc_and_allowed_inside_it(pool: PgPool) {
    let state = state(&pool);
    seed(&pool).await;
    let mut topics = state.events.subscribe();

    let uri = "/api/v1/events/7360/fcas/ev-published/archive";
    let got = refusals(&state, &pool, Method::POST, uri, None).await;
    assert_eq!(got, REFUSED);
    assert_eq!(
        row(&pool, "ev-published").await.3.as_deref(),
        Some("published")
    );
    assert!(
        topics.try_recv().is_err(),
        "a refused archive nudged the maps"
    );

    let zdc = planner(&pool, Some("ZDC")).await;
    assert_eq!(
        send(&state, Method::POST, uri, &zdc, None).await,
        StatusCode::OK
    );
    assert_eq!(
        row(&pool, "ev-published").await.3.as_deref(),
        Some("archived")
    );
    assert_eq!(
        topics.try_recv().unwrap().topic,
        crate::realtime::topic::FCA
    );
    let national = planner(&pool, None).await;
    let uri = "/api/v1/events/7360/fcas/ev-planned/archive";
    assert_eq!(
        send(&state, Method::POST, uri, &national, None).await,
        StatusCode::OK
    );
    assert_eq!(
        row(&pool, "ev-planned").await.3.as_deref(),
        Some("archived")
    );
}

/// Arming auto-publish is the write that matters most: the lifecycle job publishes an armed FCA
/// with no principal of its own.
#[sqlx::test]
async fn auto_is_refused_outside_the_stored_artcc_and_allowed_inside_it(pool: PgPool) {
    let state = state(&pool);
    seed(&pool).await;
    let uri = "/api/v1/events/7360/fcas/ev-planned/auto";

    let got = refusals(
        &state,
        &pool,
        Method::PUT,
        uri,
        Some(json!({ "auto_publish": true })),
    )
    .await;
    assert_eq!(got, REFUSED);
    assert!(
        !row(&pool, "ev-planned").await.4,
        "a refused caller armed auto-publish"
    );

    let zdc = planner(&pool, Some("ZDC")).await;
    let on = Some(json!({ "auto_publish": true }));
    assert_eq!(
        send(&state, Method::PUT, uri, &zdc, on).await,
        StatusCode::OK
    );
    assert!(row(&pool, "ev-planned").await.4);
    let national = planner(&pool, None).await;
    let off = Some(json!({ "auto_publish": false }));
    assert_eq!(
        send(&state, Method::PUT, uri, &national, off).await,
        StatusCode::OK
    );
    assert!(!row(&pool, "ev-planned").await.4);
}

/// Scope runs before the lifecycle check, so an out-of-scope caller can't read an FCA's state from
/// a 409.
#[sqlx::test]
async fn publish_answers_403_not_409_to_an_out_of_scope_planner(pool: PgPool) {
    let state = state(&pool);
    seed(&pool).await;
    let uri = "/api/v1/events/7360/fcas/ev-published/publish";
    let before = row(&pool, "ev-published").await;

    let ztl = planner(&pool, Some("ZTL")).await;
    assert_eq!(
        send(&state, Method::POST, uri, &ztl, None).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(row(&pool, "ev-published").await, before);
    let zdc = planner(&pool, Some("ZDC")).await;
    assert_eq!(
        send(&state, Method::POST, uri, &zdc, None).await,
        StatusCode::CONFLICT
    );
}

#[sqlx::test]
async fn archive_answers_403_not_409_to_an_out_of_scope_planner(pool: PgPool) {
    let state = state(&pool);
    seed(&pool).await;
    let uri = "/api/v1/events/7360/fcas/ev-archived/archive";
    let before = row(&pool, "ev-archived").await;

    let ztl = planner(&pool, Some("ZTL")).await;
    assert_eq!(
        send(&state, Method::POST, uri, &ztl, None).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(row(&pool, "ev-archived").await, before);
    let zdc = planner(&pool, Some("ZDC")).await;
    assert_eq!(
        send(&state, Method::POST, uri, &zdc, None).await,
        StatusCode::CONFLICT
    );
}

#[sqlx::test]
async fn update_answers_400_for_an_invalid_body_before_the_scope_check(pool: PgPool) {
    let state = state(&pool);
    seed(&pool).await;
    let uri = "/api/v1/events/7360/fcas/ev-planned";
    let before = row(&pool, "ev-planned").await;
    let ztl = planner(&pool, Some("ZTL")).await;

    let status = send(&state, Method::PUT, uri, &ztl, Some(fca("", "ZDC"))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let status = send(&state, Method::PUT, uri, &ztl, Some(fca("Valid", "ZDC"))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(row(&pool, "ev-planned").await, before);
}

#[sqlx::test]
async fn create_on_a_missing_event_answers_404_not_403(pool: PgPool) {
    let state = state(&pool);
    seed(&pool).await;
    let ztl = planner(&pool, Some("ZTL")).await;

    let status = send(
        &state,
        Method::POST,
        "/api/v1/events/9999/fcas",
        &ztl,
        Some(fca("Nowhere", "ZDC")),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(count(&pool, 9999).await, 0);
    let status = send(
        &state,
        Method::POST,
        "/api/v1/events/7360/fcas",
        &ztl,
        Some(fca("Here", "ZDC")),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

/// An FCA with no ARTCC is writable only at national scope, as a plain FCA is.
#[sqlx::test]
async fn a_blank_artcc_event_fca_is_refused_to_a_scoped_planner(pool: PgPool) {
    let state = state(&pool);
    seed(&pool).await;
    let before = row(&pool, "ev-blank").await;
    let zdc = planner(&pool, Some("ZDC")).await;
    let uri = "/api/v1/events/7360/fcas/ev-blank";

    let cases = [
        (Method::PUT, uri.to_string(), Some(fca("Blank edit", ""))),
        (Method::DELETE, uri.to_string(), None),
        (Method::POST, format!("{uri}/publish"), None),
        (Method::POST, format!("{uri}/archive"), None),
        (
            Method::PUT,
            format!("{uri}/auto"),
            Some(json!({ "auto_publish": true })),
        ),
        (
            Method::POST,
            "/api/v1/events/7360/fcas".to_string(),
            Some(fca("Blank new", "")),
        ),
    ];
    for (method, uri, body) in cases {
        let status = send(&state, method.clone(), &uri, &zdc, body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}");
    }
    assert_eq!(row(&pool, "ev-blank").await, before);
    assert_eq!(count(&pool, 7360).await, 6);
}

#[sqlx::test]
async fn a_blank_artcc_event_fca_is_writable_by_a_national_planner(pool: PgPool) {
    let state = state(&pool);
    seed(&pool).await;
    let national = planner(&pool, None).await;
    let uri = "/api/v1/events/7360/fcas/ev-blank";

    let status = send(
        &state,
        Method::PUT,
        uri,
        &national,
        Some(fca("National blank", "")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(row(&pool, "ev-blank").await.0, "National blank");
    let status = send(
        &state,
        Method::POST,
        "/api/v1/events/7360/fcas",
        &national,
        Some(fca("New blank", "")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(count(&pool, 7360).await, 7);
    assert_eq!(
        send(&state, Method::DELETE, uri, &national, None).await,
        StatusCode::OK
    );
    assert!(row(&pool, "ev-blank").await.2);
}

/// The ownership 404 comes before the scope check, for an out-of-scope caller as for an in-scope one.
#[sqlx::test]
async fn another_events_fca_answers_404_to_an_out_of_scope_planner(pool: PgPool) {
    let state = state(&pool);
    seed(&pool).await;
    let before = row(&pool, "other-event").await;
    let uri = "/api/v1/events/7360/fcas/other-event";

    for cookie in [
        planner(&pool, Some("ZTL")).await,
        planner(&pool, Some("ZDC")).await,
    ] {
        let cases = [
            (Method::PUT, uri.to_string(), Some(fca("Hijack", "ZDC"))),
            (Method::DELETE, uri.to_string(), None),
            (Method::POST, format!("{uri}/publish"), None),
            (Method::POST, format!("{uri}/archive"), None),
            (
                Method::PUT,
                format!("{uri}/auto"),
                Some(json!({ "auto_publish": true })),
            ),
        ];
        for (method, uri, body) in cases {
            let status = send(&state, method.clone(), &uri, &cookie, body).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{method} {uri}");
        }
    }
    assert_eq!(row(&pool, "other-event").await, before);

    let zdc = planner(&pool, Some("ZDC")).await;
    let own = "/api/v1/events/7361/fcas/other-event";
    assert_eq!(
        send(&state, Method::PUT, own, &zdc, Some(fca("Own", "ZDC"))).await,
        StatusCode::OK
    );
}

#[sqlx::test]
async fn a_zdc_deny_beside_a_national_allow_refuses_a_zdc_event_fca(pool: PgPool) {
    let state = state(&pool);
    seed(&pool).await;
    let user = seed_user(&pool).await;
    grant(&pool, &user, PERM, None).await;
    deny_scoped(&pool, &user, PERM, Some("ZDC")).await;
    let cookie = session_cookie(&pool, &user).await;
    let before = row(&pool, "ev-planned").await;

    let uri = "/api/v1/events/7360/fcas/ev-planned";
    let status = send(
        &state,
        Method::PUT,
        uri,
        &cookie,
        Some(fca("Denied", "ZDC")),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(row(&pool, "ev-planned").await, before);
    let uri = "/api/v1/events/7360/fcas/ev-ztl";
    let status = send(
        &state,
        Method::PUT,
        uri,
        &cookie,
        Some(fca("Allowed", "ZTL")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

/// A bearer sent through the real router.
async fn send_bearer(
    state: &AppState,
    method: Method,
    uri: &str,
    bearer: &str,
    body: Option<Value>,
) -> StatusCode {
    use tower::ServiceExt;

    let builder = http::Request::builder()
        .method(method)
        .uri(uri)
        .header(http::header::AUTHORIZATION, bearer);
    let request = match body {
        Some(b) => builder
            .header(http::header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(b.to_string())),
        None => builder.body(axum::body::Body::empty()),
    }
    .unwrap();
    crate::router::build_router(state.clone())
        .oneshot(request)
        .await
        .unwrap()
        .status()
}

/// A service account holding `events.plan.update` through a role scoped to `artcc`, as a bearer.
async fn service_account(pool: &PgPool, key: &str, artcc: &str) -> String {
    sqlx::query("insert into access.roles (name, is_system) values ('T854_PLANNER', false) on conflict do nothing")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "insert into access.role_permissions (role_name, permission_name) \
         values ('T854_PLANNER', $1) on conflict do nothing",
    )
    .bind(PERM)
    .execute(pool)
    .await
    .unwrap();
    let id: String = sqlx::query_scalar(
        "insert into access.service_accounts (key, name) values ($1, $1) returning id",
    )
    .bind(key)
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
         values ($1, 'T854_PLANNER', $2)",
    )
    .bind(&id)
    .bind(artcc)
    .execute(pool)
    .await
    .unwrap();
    format!("Bearer {token}")
}

/// An API key granted `events.plan.update`, its owner holding it only at `artcc`, as a bearer.
async fn api_key(pool: &PgPool, artcc: &str) -> String {
    let owner = seed_user(pool).await;
    grant(pool, &owner, PERM, Some(artcc)).await;
    let token = format!("ois_pat_{}", uuid::Uuid::new_v4().simple());
    let id: String = sqlx::query_scalar(
        "insert into access.api_keys (owner_user_id, name, prefix, secret_hash) \
         values ($1, 'planner key', 'ois_pat_t854', $2) returning id",
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
    .bind(PERM)
    .execute(pool)
    .await
    .unwrap();
    format!("Bearer {token}")
}

/// Publish took no `Actor` before #854, so this drives the service-account path into the new check.
#[sqlx::test]
async fn a_ztl_service_account_is_refused_a_zdc_event_fca_and_zdc_is_not(pool: PgPool) {
    let state = state(&pool);
    seed(&pool).await;
    let uri = "/api/v1/events/7360/fcas/ev-planned/publish";

    let ztl = service_account(&pool, "t854-ztl", "ZTL").await;
    assert_eq!(
        send_bearer(&state, Method::POST, uri, &ztl, None).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(row(&pool, "ev-planned").await.3.as_deref(), Some("planned"));
    let zdc = service_account(&pool, "t854-zdc", "ZDC").await;
    assert_eq!(
        send_bearer(&state, Method::POST, uri, &zdc, None).await,
        StatusCode::OK
    );
    assert_eq!(
        row(&pool, "ev-planned").await.3.as_deref(),
        Some("published")
    );
}

/// A key is capped at its owner's live scope, so a ZTL planner's key can't arm a ZDC FCA.
#[sqlx::test]
async fn an_api_key_of_a_ztl_planner_is_refused_a_zdc_event_fca(pool: PgPool) {
    let state = state(&pool);
    seed(&pool).await;
    let uri = "/api/v1/events/7360/fcas/ev-planned/auto";
    let on = || Some(json!({ "auto_publish": true }));

    let ztl = api_key(&pool, "ZTL").await;
    assert_eq!(
        send_bearer(&state, Method::PUT, uri, &ztl, on()).await,
        StatusCode::FORBIDDEN
    );
    assert!(!row(&pool, "ev-planned").await.4);
    let zdc = api_key(&pool, "ZDC").await;
    assert_eq!(
        send_bearer(&state, Method::PUT, uri, &zdc, on()).await,
        StatusCode::OK
    );
    assert!(row(&pool, "ev-planned").await.4);
}
