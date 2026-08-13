//! Background maintenance jobs.

use std::time::Duration;

use sqlx::PgPool;

use crate::repos::tmu as tmu_repo;

const CLEANUP_INTERVAL: Duration = Duration::from_secs(15 * 60);

/// Periodically expire finished TMIs/ground stops and delete ones that ended over an hour
/// ago. Runs once at startup, then every 15 minutes.
pub fn spawn_cleanup(pool: PgPool) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(CLEANUP_INTERVAL);
        loop {
            ticker.tick().await;
            match tmu_repo::run_cleanup(&pool).await {
                Ok(stats) if stats.expired > 0 || stats.deleted > 0 => {
                    tracing::info!(
                        expired = stats.expired,
                        deleted = stats.deleted,
                        "tmu cleanup pass"
                    );
                }
                Ok(_) => {}
                Err(_) => tracing::warn!("tmu cleanup pass failed"),
            }
        }
    });
}
