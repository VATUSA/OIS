//! Background maintenance jobs.

use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use sqlx::PgPool;

use crate::feed::nav::NavData;
use crate::feed::nav_source;
use crate::repos::tmu as tmu_repo;

const CLEANUP_INTERVAL: Duration = Duration::from_secs(15 * 60);

/// How often to check the FAA/@squawk sources for a newer NASR cycle.
const NAV_REFRESH_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Keep the in-memory nav database current: fetch the latest NASR data at startup and
/// every 24h, hot-swapping it in when the cycle (or point count) changes. On any failure
/// the existing data is kept — the server always has a coherent dataset from the
/// compile-time bundle seed.
pub fn spawn_nav_refresh(nav: Arc<ArcSwap<NavData>>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(NAV_REFRESH_INTERVAL);
        loop {
            ticker.tick().await;
            match nav_source::fetch_latest().await {
                Ok(fresh) => {
                    let current = nav.load();
                    let changed = fresh.cycle() != current.cycle() || fresh.len() != current.len();
                    if fresh.is_empty() {
                        tracing::warn!("nav refresh produced an empty database; keeping current");
                    } else if changed {
                        tracing::info!(
                            from_cycle = current.cycle(),
                            to_cycle = fresh.cycle(),
                            points = fresh.len(),
                            "nav database refreshed"
                        );
                        nav.store(Arc::new(fresh));
                    } else {
                        tracing::debug!(cycle = current.cycle(), "nav refresh: already current");
                    }
                }
                Err(e) => tracing::warn!(error = %e, "nav refresh failed; keeping current data"),
            }
        }
    });
}

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
