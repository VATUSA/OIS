use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicI64;

use arc_swap::ArcSwap;
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::sync::broadcast;

use crate::feed::{
    self, FeedState, airspace::Boundaries, facilities::FacilityState, nav::NavData,
    runway_db::RunwayDb, tracon::TraconState, winds::Winds,
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
    /// SimAware TRACON boundaries for the ATC layer. Starts empty, refreshed daily
    /// (`feed::tracon::spawn_refresh`); behind `ArcSwap` for lock-free reads.
    pub tracons: TraconState,
    /// Runway ends per US airport, for the Runway Balancer (immutable, compile-time bundled).
    pub runways: Arc<RunwayDb>,
    /// Winds aloft, for ETA correction. Starts empty (still air) and is hot-swapped by
    /// `jobs::spawn_winds_refresh`, so it sits behind an `ArcSwap` for lock-free reads.
    pub winds: Arc<ArcSwap<Winds>>,
    /// Epoch-ms of the last successful nav / winds fetch (0 = not yet fetched at runtime).
    pub nav_refreshed: Arc<AtomicI64>,
    pub winds_refreshed: Arc<AtomicI64>,
    /// Per-airport METAR cache `(info, fetched_ms)` for the runway board (server-side fetch).
    pub metar_cache: Arc<Mutex<HashMap<String, (feed::metar::MetarInfo, i64)>>>,
    /// Realtime push hub: mutation handlers publish a topic here; connected websockets fan it out to
    /// clients, which then refetch via REST (see `crate::realtime`).
    pub events: crate::realtime::Events,
}

impl AppState {
    /// Publish a realtime nudge to every connected websocket. No-op error when nobody's listening.
    pub fn publish(&self, topic: &str) {
        let _ = self.events.send(crate::realtime::WsEvent {
            topic: topic.to_string(),
        });
    }
}

impl AppState {
    pub async fn from_env() -> Result<Self, sqlx::Error> {
        let feed = feed::new_state();
        let facilities = feed::facilities::new_state();
        let tracons = feed::tracon::new_state();
        let nav = Arc::new(ArcSwap::from_pointee(NavData::load()));
        let airspace = Arc::new(Boundaries::load());
        let runways = Arc::new(RunwayDb::load());
        let winds = Arc::new(ArcSwap::from_pointee(Winds::default()));
        let nav_refreshed = Arc::new(AtomicI64::new(0));
        let winds_refreshed = Arc::new(AtomicI64::new(0));
        let metar_cache = Arc::new(Mutex::new(HashMap::new()));
        let events = broadcast::channel(256).0;
        tracing::info!(
            nav_points = nav.load().len(),
            nav_cycle = nav.load().cycle(),
            artccs = airspace.len(),
            "nav database loaded"
        );
        if let Ok(database_url) = std::env::var("DATABASE_URL") {
            // `max_connections` is configurable so a busier deployment can be given headroom without a
            // rebuild. `acquire_timeout` is the important one: without it a checkout on an exhausted
            // pool waits forever, so a single leaked connection silently wedges *every* request
            // (health included) into an apparent hang. With it, exhaustion surfaces as a fast 5xx that
            // shows up in logs instead of a stuck backend.
            let max_connections = std::env::var("DATABASE_MAX_CONNECTIONS")
                .ok()
                .and_then(|v| v.trim().parse::<u32>().ok())
                .filter(|v| *v > 0)
                .unwrap_or(20);
            let pool = PgPoolOptions::new()
                .max_connections(max_connections)
                .acquire_timeout(std::time::Duration::from_secs(10))
                .connect(&database_url)
                .await?;
            return Ok(Self {
                db: Some(pool),
                feed,
                facilities,
                tracons,
                nav,
                airspace,
                runways,
                winds,
                nav_refreshed,
                winds_refreshed,
                metar_cache,
                events,
            });
        }

        Ok(Self {
            db: None,
            feed,
            facilities,
            tracons,
            nav,
            airspace,
            runways,
            winds,
            nav_refreshed,
            winds_refreshed,
            metar_cache,
            events,
        })
    }

    pub fn without_db() -> Self {
        Self {
            db: None,
            feed: feed::new_state(),
            facilities: feed::facilities::new_state(),
            tracons: feed::tracon::new_state(),
            nav: Arc::new(ArcSwap::from_pointee(NavData::default())),
            airspace: Arc::new(Boundaries::load()),
            runways: Arc::new(RunwayDb::load()),
            winds: Arc::new(ArcSwap::from_pointee(Winds::default())),
            nav_refreshed: Arc::new(AtomicI64::new(0)),
            winds_refreshed: Arc::new(AtomicI64::new(0)),
            metar_cache: Arc::new(Mutex::new(HashMap::new())),
            events: broadcast::channel(256).0,
        }
    }
}
