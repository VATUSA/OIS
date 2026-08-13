use sqlx::{PgPool, postgres::PgPoolOptions};

use crate::feed::{self, FeedState};

/// Lean application state: the DB pool (optional so the process can boot without a
/// database, e.g. for `--help`-style runs and tests) and the live VATSIM feed.
#[derive(Clone)]
pub struct AppState {
    pub db: Option<PgPool>,
    pub feed: FeedState,
}

impl AppState {
    pub async fn from_env() -> Result<Self, sqlx::Error> {
        let feed = feed::new_state();
        if let Ok(database_url) = std::env::var("DATABASE_URL") {
            let pool = PgPoolOptions::new()
                .max_connections(10)
                .connect(&database_url)
                .await?;
            return Ok(Self {
                db: Some(pool),
                feed,
            });
        }

        Ok(Self { db: None, feed })
    }

    pub fn without_db() -> Self {
        Self {
            db: None,
            feed: feed::new_state(),
        }
    }
}
