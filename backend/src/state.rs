use std::sync::Arc;

use arc_swap::ArcSwap;
use sqlx::{PgPool, postgres::PgPoolOptions};

use crate::feed::{self, FeedState, facilities::FacilityState, nav::NavData};

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
}

impl AppState {
    pub async fn from_env() -> Result<Self, sqlx::Error> {
        let feed = feed::new_state();
        let facilities = feed::facilities::new_state();
        let nav = Arc::new(ArcSwap::from_pointee(NavData::load()));
        tracing::info!(
            nav_points = nav.load().len(),
            nav_cycle = nav.load().cycle(),
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
            });
        }

        Ok(Self {
            db: None,
            feed,
            facilities,
            nav,
        })
    }

    pub fn without_db() -> Self {
        Self {
            db: None,
            feed: feed::new_state(),
            facilities: feed::facilities::new_state(),
            nav: Arc::new(ArcSwap::from_pointee(NavData::default())),
        }
    }
}
