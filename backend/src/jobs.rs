//! Background maintenance jobs.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use arc_swap::ArcSwap;
use chrono::Utc;
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

/// Fetch the latest NASR data once and hot-swap it in when the cycle (or point count)
/// changes. Records the fetch time on success. Returns `Ok(true)` when the data changed,
/// `Ok(false)` when it was already current, `Err` when the fetch failed or was empty. The
/// existing data is always kept on failure.
pub async fn refresh_nav_once(
    nav: &Arc<ArcSwap<NavData>>,
    refreshed: &Arc<AtomicI64>,
) -> Result<bool, String> {
    let fresh = nav_source::fetch_latest()
        .await
        .map_err(|e| e.to_string())?;
    if fresh.is_empty() {
        return Err("nav fetch produced an empty database".into());
    }
    let current = nav.load();
    let changed = fresh.cycle() != current.cycle() || fresh.len() != current.len();
    if changed {
        tracing::info!(
            from_cycle = current.cycle(),
            to_cycle = fresh.cycle(),
            points = fresh.len(),
            "nav database refreshed"
        );
        nav.store(Arc::new(fresh));
    }
    refreshed.store(Utc::now().timestamp_millis(), Ordering::Relaxed);
    Ok(changed)
}

/// Fetch winds aloft once and hot-swap them in. Returns `None` if the airport database
/// isn't loaded yet, else `Some(station_count)` (0 = fetch returned nothing; current winds
/// kept). Records the fetch time when stations were loaded.
pub async fn refresh_winds_once(
    feed: &FeedState,
    winds: &Arc<ArcSwap<Winds>>,
    refreshed: &Arc<AtomicI64>,
    client: &reqwest::Client,
) -> Option<usize> {
    let airports = feed.read().await.airports.clone();
    if airports.is_empty() {
        return None;
    }
    let fresh = winds::fetch(client, &airports).await;
    let n = fresh.station_count();
    if n > 0 {
        winds.store(Arc::new(fresh));
        refreshed.store(Utc::now().timestamp_millis(), Ordering::Relaxed);
    }
    Some(n)
}

/// Keep the in-memory nav database current: refresh at startup and every 24h. On any
/// failure the existing data is kept — the server always has a coherent dataset from the
/// compile-time bundle seed.
pub fn spawn_nav_refresh(nav: Arc<ArcSwap<NavData>>, refreshed: Arc<AtomicI64>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(NAV_REFRESH_INTERVAL);
        loop {
            ticker.tick().await;
            if let Err(e) = refresh_nav_once(&nav, &refreshed).await {
                tracing::warn!(error = %e, "nav refresh failed; keeping current data");
            }
        }
    });
}

/// Keep winds aloft current for ETA prediction: once the airport database is loaded, fetch
/// the AWC FB tables and hot-swap them in, then refresh hourly. Fails safe.
pub fn spawn_winds_refresh(feed: FeedState, winds: Arc<ArcSwap<Winds>>, refreshed: Arc<AtomicI64>) {
    tokio::spawn(async move {
        let client = reqwest::Client::builder()
            .user_agent("ois-winds/1.0 (+https://vatusa.net)")
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap_or_default();
        loop {
            match refresh_winds_once(&feed, &winds, &refreshed, &client).await {
                // Airport DB not loaded yet — retry soon.
                None => tokio::time::sleep(Duration::from_secs(30)).await,
                Some(n) => {
                    if n > 0 {
                        tracing::info!(stations = n, "winds aloft refreshed");
                    } else {
                        tracing::warn!("winds refresh returned no stations; keeping current");
                    }
                    tokio::time::sleep(WINDS_REFRESH_INTERVAL).await;
                }
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
