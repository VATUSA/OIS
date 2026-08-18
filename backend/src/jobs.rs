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
use crate::repos::stats as stats_repo;
use crate::repos::tmu as tmu_repo;

const CLEANUP_INTERVAL: Duration = Duration::from_secs(15 * 60);

/// How often to age the stats position table.
const STATS_COMPACTION_INTERVAL: Duration = Duration::from_secs(60 * 60);
/// Positions this old are downsampled (keep 1-of-N); older than the prune horizon they're dropped.
const STATS_DOWNSAMPLE_AFTER_DAYS: i64 = 2;
const STATS_PRUNE_AFTER_DAYS: i64 = 14;
/// Keep every Nth 15s sample in the downsample band (4 → ~1-minute resolution).
const STATS_KEEP_EVERY: i64 = 4;

/// How often to open/close event stat-capture windows.
const CAPTURE_SCHEDULER_INTERVAL: Duration = Duration::from_secs(60);

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
/// the AWC FB tables and hot-swap them in, then refresh hourly. Fails safe. When a DB pool is
/// present, each successful refresh also snapshots the winds to `stats.winds` so historical replay
/// can reconstruct past ETAs.
pub fn spawn_winds_refresh(
    feed: FeedState,
    winds: Arc<ArcSwap<Winds>>,
    refreshed: Arc<AtomicI64>,
    pool: Option<sqlx::PgPool>,
) {
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
                        if let Some(pool) = &pool {
                            let snap = winds.load_full();
                            if let Err(e) =
                                crate::repos::stats::upsert_winds(pool, Utc::now(), &snap).await
                            {
                                tracing::warn!(error = ?e, "winds snapshot store failed");
                            }
                        }
                    } else {
                        tracing::warn!("winds refresh returned no stations; keeping current");
                    }
                    tokio::time::sleep(WINDS_REFRESH_INTERVAL).await;
                }
            }
        }
    });
}

/// Age the stats position time-series: downsample the 2–14 day band to ~1-minute resolution and
/// drop raw positions past the 14-day horizon (Tier-1 simplified tracks on `stats.flight` survive).
/// Rows inside an open/saved `stats.capture` window are skipped (retained at full fidelity). Runs
/// hourly; a slow, batched, saved-window-aware alternative to TimescaleDB retention.
pub fn spawn_stats_compaction(pool: PgPool) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(STATS_COMPACTION_INTERVAL);
        loop {
            ticker.tick().await;
            let now = Utc::now();
            let downsample_before = now - chrono::Duration::days(STATS_DOWNSAMPLE_AFTER_DAYS);
            let prune_before = now - chrono::Duration::days(STATS_PRUNE_AFTER_DAYS);

            match stats_repo::downsample_positions(
                &pool,
                prune_before,
                downsample_before,
                STATS_KEEP_EVERY,
            )
            .await
            {
                Ok(n) if n > 0 => tracing::info!(deleted = n, "stats: downsampled positions"),
                Ok(_) => {}
                Err(_) => tracing::warn!("stats: downsample pass failed"),
            }

            match stats_repo::prune_positions(&pool, prune_before).await {
                Ok(n) if n > 0 => tracing::info!(deleted = n, "stats: pruned old positions"),
                Ok(_) => {}
                Err(_) => tracing::warn!("stats: prune pass failed"),
            }

            match stats_repo::prune_winds(&pool, prune_before).await {
                Ok(n) if n > 0 => tracing::info!(deleted = n, "stats: pruned old winds"),
                Ok(_) => {}
                Err(_) => tracing::warn!("stats: winds prune pass failed"),
            }

            // Retain traffic-management history (published TMIs/GDPs/ground stops kept for replay)
            // for the same window; hard-drop only rows that ran past the horizon.
            match crate::repos::tmu::prune_history(&pool, prune_before).await {
                Ok(n) if n > 0 => tracing::info!(deleted = n, "stats: pruned old TM history"),
                Ok(_) => {}
                Err(_) => tracing::warn!("stats: TM history prune pass failed"),
            }
        }
    });
}

/// Drive per-event stat capture: for each event with capture enabled, open a `stats.capture`
/// window once the event is inside `[start - pre, end + post]`, and close+save it once that window
/// has passed. Runs every minute. Idempotent — it keys off whether an open capture already exists.
pub fn spawn_capture_scheduler(pool: PgPool) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(CAPTURE_SCHEDULER_INTERVAL);
        loop {
            ticker.tick().await;
            let rows = match stats_repo::list_capture_schedule(&pool).await {
                Ok(r) => r,
                Err(_) => {
                    tracing::warn!("stats: capture schedule query failed");
                    continue;
                }
            };
            let now = Utc::now();
            for r in rows {
                let window_start = r.start_time - chrono::Duration::minutes(r.pre_minutes as i64);
                let window_end = r.end_time + chrono::Duration::minutes(r.post_minutes as i64);
                let in_window = now >= window_start && now <= window_end;

                match (in_window, r.open_capture_id.as_deref()) {
                    // Inside the window with no capture yet → open one covering the whole window.
                    (true, None) => {
                        match stats_repo::create_capture(
                            &pool,
                            Some(r.event_id),
                            &r.title,
                            window_start,
                            None,
                        )
                        .await
                        {
                            Ok(id) => tracing::info!(
                                event = r.event_id,
                                capture = %id,
                                "stats: opened event capture"
                            ),
                            Err(_) => {
                                tracing::warn!(event = r.event_id, "stats: open capture failed")
                            }
                        }
                    }
                    // Past the window with an open capture → close + save it.
                    (false, Some(_)) if now > window_end => {
                        match stats_repo::close_open_event_captures(&pool, r.event_id, window_end)
                            .await
                        {
                            Ok(n) if n > 0 => {
                                tracing::info!(event = r.event_id, "stats: saved event capture")
                            }
                            _ => {}
                        }
                    }
                    _ => {}
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
