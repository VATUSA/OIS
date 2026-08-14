//! Background maintenance jobs.

use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use sqlx::PgPool;

use crate::feed::FeedState;
use crate::feed::nav::NavData;
use crate::feed::nav_source;
use crate::feed::winds::{self, Winds};
use crate::repos::tmu as tmu_repo;

const CLEANUP_INTERVAL: Duration = Duration::from_secs(15 * 60);

/// How often to check the FAA/@squawk sources for a newer NASR cycle.
const NAV_REFRESH_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// How often to refresh winds aloft (AWC FB tables update ~4×/day; hourly keeps us current).
const WINDS_REFRESH_INTERVAL: Duration = Duration::from_secs(60 * 60);

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

/// Keep winds aloft current for ETA prediction: once the airport database is loaded, fetch
/// the AWC FB tables and hot-swap them in, then refresh hourly. Fails safe — an empty or
/// failed fetch keeps the current winds (or still air), never blocking the feed.
pub fn spawn_winds_refresh(feed: FeedState, winds: Arc<ArcSwap<Winds>>) {
    tokio::spawn(async move {
        let client = reqwest::Client::builder()
            .user_agent("ois-winds/1.0 (+https://vatusa.net)")
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap_or_default();
        loop {
            // Winds stations are keyed to airport coordinates; wait for the feed's airport
            // database before the first fetch.
            let airports = feed.read().await.airports.clone();
            if airports.is_empty() {
                tokio::time::sleep(Duration::from_secs(30)).await;
                continue;
            }
            let fresh = winds::fetch(&client, &airports).await;
            if fresh.is_empty() {
                tracing::warn!("winds refresh returned no stations; keeping current (still air)");
            } else {
                tracing::info!(stations = fresh.station_count(), "winds aloft refreshed");
                winds.store(Arc::new(fresh));
            }
            tokio::time::sleep(WINDS_REFRESH_INTERVAL).await;
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
