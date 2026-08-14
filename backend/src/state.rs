use std::sync::Arc;
use std::sync::atomic::AtomicI64;

use arc_swap::ArcSwap;
use sqlx::{PgPool, postgres::PgPoolOptions};

use crate::feed::{
    self, FeedState, airspace::Boundaries, facilities::FacilityState, nav::NavData,
    runway_db::RunwayDb, winds::Winds,
};

/// Lean application state: the DB pool (optional so the process can boot without a
/// database, e.g. for `--help`-style runs and tests), the live VATSIM feed, the
/// facility → airports map, and the nav-fix database. The nav DB is seeded from the
/// compile-time bundle and hot-swapped by the refresh job (`jobs::spawn_nav_refresh`), so
/// it sits behind an `ArcSwap` for lock-free reads.
#[derive(Clone)]
pub struct AppState {
    pub db: Option<PgPool>,
    pub feed: FeedState,
    pub facilities: FacilityState,
    pub nav: Arc<ArcSwap<NavData>>,
    /// ARTCC boundary polygons, for FCA scope filtering (immutable, compile-time bundled).
    pub airspace: Arc<Boundaries>,
    /// Runway ends per US airport, for the Runway Balancer (immutable, compile-time bundled).
    pub runways: Arc<RunwayDb>,
    /// Winds aloft, for ETA correction. Starts empty (still air) and is hot-swapped by
    /// `jobs::spawn_winds_refresh`, so it sits behind an `ArcSwap` for lock-free reads.
    pub winds: Arc<ArcSwap<Winds>>,
    /// Epoch-ms of the last successful nav / winds fetch (0 = not yet fetched at runtime).
    pub nav_refreshed: Arc<AtomicI64>,
    pub winds_refreshed: Arc<AtomicI64>,
}

impl AppState {
    pub async fn from_env() -> Result<Self, sqlx::Error> {
        let feed = feed::new_state();
        let facilities = feed::facilities::new_state();
        let nav = Arc::new(ArcSwap::from_pointee(NavData::load()));
        let airspace = Arc::new(Boundaries::load());
        let runways = Arc::new(RunwayDb::load());
        let winds = Arc::new(ArcSwap::from_pointee(Winds::default()));
        let nav_refreshed = Arc::new(AtomicI64::new(0));
        let winds_refreshed = Arc::new(AtomicI64::new(0));
        tracing::info!(
            nav_points = nav.load().len(),
            nav_cycle = nav.load().cycle(),
            artccs = airspace.len(),
            "nav database loaded"
        );
        if let Ok(database_url) = std::env::var("DATABASE_URL") {
            let pool = PgPoolOptions::new()
                .max_connections(10)
                .connect(&database_url)
                .await?;
            return Ok(Self {
                db: Some(pool),
                feed,
                facilities,
                nav,
                airspace,
                runways,
                winds,
                nav_refreshed,
                winds_refreshed,
            });
        }

        Ok(Self {
            db: None,
            feed,
            facilities,
            nav,
            airspace,
            runways,
            winds,
            nav_refreshed,
            winds_refreshed,
        })
    }

    pub fn without_db() -> Self {
        Self {
            db: None,
            feed: feed::new_state(),
            facilities: feed::facilities::new_state(),
            nav: Arc::new(ArcSwap::from_pointee(NavData::default())),
            airspace: Arc::new(Boundaries::load()),
            runways: Arc::new(RunwayDb::load()),
            winds: Arc::new(ArcSwap::from_pointee(Winds::default())),
            nav_refreshed: Arc::new(AtomicI64::new(0)),
            winds_refreshed: Arc::new(AtomicI64::new(0)),
        }
    }
}
