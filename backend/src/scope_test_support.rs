//! Shared test-only support for exercising the facility/ARTCC-scope authorization boundary
//! (`can_edit`/`require_edit`) that `airport_configs.rs`, `airport_surface.rs`, and
//! `facility_map.rs` each implement identically. See #198: this boundary had zero coverage
//! anywhere — every existing test drove the repo layer directly, never through a handler's
//! `can_edit`/`require_edit`, so a `require_edit` stubbed to `Ok(())` (ARTCC-scope enforcement
//! silently disabled) passed every test in the suite.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicI64;

use arc_swap::ArcSwap;
use sqlx::PgPool;
use tokio::sync::RwLock;

use crate::{
    auth::context::CurrentUser,
    auth::principal::Principal,
    feed::{
        self, airspace::Boundaries, facilities::Facility, nav::NavData, runway_db::RunwayDb,
        trajectory::ProfileTable, winds::Winds,
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
        winds: Arc::new(ArcSwap::from_pointee(Winds::default())),
        aircraft_profiles: Arc::new(ArcSwap::from_pointee(ProfileTable::default())),
        nav_refreshed: Arc::new(AtomicI64::new(0)),
        winds_refreshed: Arc::new(AtomicI64::new(0)),
        metar_cache: Arc::new(Mutex::new(HashMap::new())),
        events: tokio::sync::broadcast::channel(256).0,
        jobs: Arc::new(crate::job_registry::JobRegistry::new()),
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

/// Grant `user_id` `permission_name`, nationally (`artcc = None`) or scoped to one ARTCC —
/// a direct `access.user_permissions` row, deliberately bypassing roles for a minimal setup.
pub(crate) async fn grant(
    pool: &PgPool,
    user_id: &str,
    permission_name: &str,
    artcc: Option<&str>,
) {
    sqlx::query(
        "insert into access.user_permissions (user_id, permission_name, granted, artcc_id) \
         values ($1, $2, true, $3)",
    )
    .bind(user_id)
    .bind(permission_name)
    .bind(artcc)
    .execute(pool)
    .await
    .unwrap();
}
