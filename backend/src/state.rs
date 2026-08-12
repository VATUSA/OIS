use sqlx::{PgPool, postgres::PgPoolOptions};

/// Lean application state for the Phase 0 slice: just the DB pool (optional so the
/// process can boot without a database, e.g. for `--help`-style runs and tests).
#[derive(Clone)]
pub struct AppState {
    pub db: Option<PgPool>,
}

impl AppState {
    pub async fn from_env() -> Result<Self, sqlx::Error> {
        if let Ok(database_url) = std::env::var("DATABASE_URL") {
            let pool = PgPoolOptions::new()
                .max_connections(10)
                .connect(&database_url)
                .await?;
            return Ok(Self { db: Some(pool) });
        }

        Ok(Self { db: None })
    }

    pub fn without_db() -> Self {
        Self { db: None }
    }
}
