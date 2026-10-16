//! Manual reload of ATC sector volumes (#594) into `flow.airspace_sector`.
//!
//! The backend loads them itself: the `airspace_sector_seed` job fetches the pinned vtsd
//! `sectors.json` at start whenever the table isn't already at `SOURCE_REF` (#821). This binary runs
//! the same code by hand and always reloads, even a current table. See
//! `feed::sector_source` for the source, its license, and how to move to a newer revision.
//!
//! ```text
//! RUST_LOG=info cargo run -p ois-backend --bin airspace-sector-importer
//! ```
//! (Without `RUST_LOG=info` only errors print, so a successful run is silent.)
//! Needs `DATABASE_URL` (read from `.env` too) and a migrated database. The whole table is replaced
//! in one transaction. A running server picks the new data up on its next `airspace_sectors_refresh`
//! tick (5 minutes). There is no per-ARTCC mode: a table holding one ARTCC at `SOURCE_REF` would
//! read as current, and the backend would never load the rest. A database loaded that way with the
//! old `--artcc` flag needs one run of this binary to load every ARTCC.

use ois_backend::{feed::sector_source, repos::airspace_sector_seed};

#[tokio::main]
async fn main() -> Result<(), String> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();
    if std::env::args().len() > 1 {
        return Err(
            "airspace-sector-importer takes no arguments (--artcc was removed in #821)".into(),
        );
    }

    let database_url = std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL must be set")?;
    let pool = sqlx::PgPool::connect(&database_url)
        .await
        .map_err(|e| format!("connecting: {e}"))?;
    // The outcome lists every skipped feature with its reason.
    let outcome = airspace_sector_seed::seed(&pool, true, sector_source::fetch).await?;
    tracing::info!("{outcome}");
    Ok(())
}
