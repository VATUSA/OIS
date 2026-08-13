use std::sync::Arc;

use sqlx::{PgPool, postgres::PgPoolOptions};

use crate::feed::{self, FeedState, facilities::FacilityState, nav::NavData};

/// Lean application state: the DB pool (optional so the process can boot without a
/// database, e.g. for `--help`-style runs and tests), the live VATSIM feed, the
/// facility → airports map, and the (immutable) nav-fix database.
#[derive(Clone)]
pub struct AppState {
    pub db: Option<PgPool>,
    pub feed: FeedState,
    pub facilities: FacilityState,
    pub nav: Arc<NavData>,
}

impl AppState {
    pub async fn from_env() -> Result<Self, sqlx::Error> {
        let feed = feed::new_state();
        let facilities = feed::facilities::new_state();
        let nav = Arc::new(NavData::load());
        tracing::info!(nav_points = nav.len(), "nav database loaded");
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
            nav: Arc::new(NavData::default()),
        }
    }
}
