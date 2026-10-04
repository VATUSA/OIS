//! Shared test-only support for exercising the facility/ARTCC-scope authorization boundary
//! (`can_edit`/`require_edit`) that `airport_configs.rs`, `airport_surface.rs`, and
//! `facility_map.rs` each implement identically. See #198: this boundary had zero coverage
//! anywhere — every existing test drove the repo layer directly, never through a handler's
//! `can_edit`/`require_edit`, so a `require_edit` stubbed to `Ok(())` (ARTCC-scope enforcement
//! silently disabled) passed every test in the suite.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI64};

use arc_swap::ArcSwap;
use sqlx::PgPool;
use tokio::sync::RwLock;

use crate::{
    auth::context::CurrentUser,
    auth::principal::Principal,
    feed::{
        self, airspace::Boundaries, facilities::Facility, nav::NavData, runway_db::RunwayDb,
        sectors::SectorTable, trajectory::ProfileTable, winds::Winds,
    },
    state::AppState,
};

/// A minimal `AppState` wired to `pool` and a caller-supplied facility map (for the
/// `owning_artcc`/`artcc_for_airport` lookups these handlers make). Every other field is the same
/// cheap, in-memory default `AppState::from_env()` boots with — these handlers never touch feed,
/// nav, airspace, runways, winds, or aircraft-profile data, so there's nothing meaningful to fake
/// beyond `facilities`.
pub(crate) fn test_state(pool: PgPool, facilities: HashMap<String, Facility>) -> AppState {
    AppState {
        db: Some(pool),
        feed: feed::new_state(),
        facilities: Arc::new(RwLock::new(facilities)),
        tracons: feed::tracon::new_state(),
        nav: Arc::new(ArcSwap::from_pointee(NavData::load())),
        airspace: Arc::new(Boundaries::load()),
        runways: Arc::new(RunwayDb::load()),
        gates: Arc::new(ArcSwap::from_pointee(HashMap::new())),
        flight_exclusions: Arc::new(ArcSwap::from_pointee(HashMap::new())),
        taxi_estimate_samples: Arc::new(ArcSwap::from_pointee(HashMap::new())),
        winds: Arc::new(ArcSwap::from_pointee(Winds::default())),
        aircraft_profiles: Arc::new(ArcSwap::from_pointee(ProfileTable::default())),
        airspace_sectors: Arc::new(ArcSwap::from_pointee(SectorTable::default())),
        sector_maps: Arc::new(ArcSwap::from_pointee(Default::default())),
        sector_consolidations: Arc::new(ArcSwap::from_pointee(Default::default())),
        nav_refreshed: Arc::new(AtomicI64::new(0)),
        winds_refreshed: Arc::new(AtomicI64::new(0)),
        data_refresh_in_flight: Arc::new(AtomicBool::new(false)),
        metar_cache: Arc::new(Mutex::new(HashMap::new())),
        events: tokio::sync::broadcast::channel(256).0,
        jobs: Arc::new(crate::job_registry::JobRegistry::new()),
        metrics: crate::metrics::handle(),
        metrics_token: None,
    }
}

/// One ARTCC owning `airports`, for a `test_state` facility map.
pub(crate) fn artcc(airports: &[&str]) -> Facility {
    Facility {
        kind: "artcc".to_string(),
        airports: airports.iter().map(|a| a.to_string()).collect(),
    }
}

/// A bare `identity.users` row, returning its id.
pub(crate) async fn seed_user(pool: &PgPool) -> String {
    sqlx::query_scalar::<_, String>(
        "insert into identity.users (full_name, display_name) \
         values ('Scope Test User', 'Scope Test User') returning id",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

/// A `Principal::User` wrapping a bare user id — `can_edit`/`require_edit` only read
/// `permission_scope`/`user_id`, neither of which touches the other `CurrentUser` fields.
pub(crate) fn principal_for(user_id: &str) -> Principal {
    Principal::User(CurrentUser {
        id: user_id.to_string(),
        cid: 0,
        email: String::new(),
        display_name: String::new(),
        rating: None,
        primary_role: None,
    })
}

/// Deny `user_id` `permission_name`, nationally (`artcc = None`) or at one ARTCC.
///
/// The counterpart to [`grant`], which hardcodes `granted = true` and so could not express a deny
/// at all — which is why the scope × deny interaction went untested until #543. Note the unique
/// index is on `(user_id, permission_name, coalesce(artcc_id, ''))`, so a deny and an allow cannot
/// coexist at the *same* scope; a national deny beside a scoped allow is the interesting case.
pub(crate) async fn deny_scoped(
    pool: &PgPool,
    user_id: &str,
    permission_name: &str,
    artcc: Option<&str>,
) {
    sqlx::query(
        "insert into access.user_permissions (user_id, permission_name, granted, artcc_id, source) \
         values ($1, $2, false, $3, 'manual')",
    )
    .bind(user_id)
    .bind(permission_name)
    .bind(artcc)
    .execute(pool)
    .await
    .unwrap();
}

/// Grant `user_id` `permission_name`, nationally (`artcc = None`) or scoped to one ARTCC —
/// a direct `access.user_permissions` row, deliberately bypassing roles for a minimal setup.
pub(crate) async fn grant(
    pool: &PgPool,
    user_id: &str,
    permission_name: &str,
    artcc: Option<&str>,
) {
    sqlx::query(
        "insert into access.user_permissions (user_id, permission_name, granted, artcc_id, source) \
         values ($1, $2, true, $3, 'manual')",
    )
    .bind(user_id)
    .bind(permission_name)
    .bind(artcc)
    .execute(pool)
    .await
    .unwrap();
}

/// A signed-in session for `user_id`, as the `Cookie` header value the router reads it from.
pub(crate) async fn session_cookie(pool: &PgPool, user_id: &str) -> String {
    // Every real account has a VATSIM CID, and the session lookup decodes it as non-null — a
    // `seed_user` row has none, so without one the caller would silently resolve as signed out.
    sqlx::query(
        "update identity.users \
         set cid = coalesce(cid, (select coalesce(max(cid), 0) + 1 from identity.users)) \
         where id = $1",
    )
    .bind(user_id)
    .execute(pool)
    .await
    .unwrap();
    let token = uuid::Uuid::new_v4().simple().to_string();
    crate::repos::auth::insert_session(pool, &token, user_id)
        .await
        .unwrap();
    format!("ois_session={token}")
}

/// Send one request through the real router — `resolve_current_user`, `RequirePermission` and the
/// handler's own scope check all on the path — and return its status (VATUSA/OIS#364).
///
/// This is what the helper-level tests above cannot do: `RequirePermission` has a private field, so
/// a gated handler can't be called directly, and a test of `can_edit`/`require_artcc_scope` alone
/// stays green when a handler stops calling them. The two gates answer differently, which is what
/// lets a test tell which one fired: a missing permission is **401**, a wrong facility **403**.
pub(crate) async fn send(
    state: &AppState,
    method: http::Method,
    uri: &str,
    cookie: &str,
    json: Option<serde_json::Value>,
) -> http::StatusCode {
    use tower::ServiceExt;

    let builder = http::Request::builder()
        .method(method)
        .uri(uri)
        .header(http::header::COOKIE, cookie);
    let request = match json {
        Some(body) => builder
            .header(http::header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(body.to_string())),
        None => builder.body(axum::body::Body::empty()),
    }
    .unwrap();
    crate::router::build_router(state.clone())
        .oneshot(request)
        .await
        .unwrap()
        .status()
}

/// [`send`], but also returning the decoded JSON body (`Null` when there isn't one).
///
/// For a test that has to check *what* a handler returned, not only that it answered — the wiring,
/// rather than a helper called directly.
pub(crate) async fn send_json(
    state: &AppState,
    method: http::Method,
    uri: &str,
    cookie: &str,
) -> (http::StatusCode, serde_json::Value) {
    use tower::ServiceExt;

    let request = http::Request::builder()
        .method(method)
        .uri(uri)
        .header(http::header::COOKIE, cookie)
        .body(axum::body::Body::empty())
        .unwrap();
    let response = crate::router::build_router(state.clone())
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, body)
}
