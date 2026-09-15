//! Background maintenance jobs.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use arc_swap::ArcSwap;
use chrono::Utc;
use sqlx::PgPool;

use serde_json::json;

use crate::errors::ApiError;
use crate::feed::FeedState;
use crate::feed::nav::NavData;
use crate::feed::nav_source;
use crate::feed::trajectory::ProfileTable;
use crate::feed::winds::{self, Winds};
use crate::job_registry::{JobRegistry, run_interval};
use crate::models::AirportGateBody;
use crate::realtime::{Events, WsEvent, topic};
use crate::repos::ace as ace_repo;
use crate::repos::aircraft_profiles as aircraft_profiles_repo;
use crate::repos::airport_surface as airport_surface_repo;
use crate::repos::events as events_repo;
use crate::repos::flow as flow_repo;
use crate::repos::stats as stats_repo;
use crate::repos::tmu as tmu_repo;

const CLEANUP_INTERVAL: Duration = Duration::from_secs(15 * 60);

/// How often to run the event-FCA auto-publish / auto-archive pass.
const EVENT_FCA_INTERVAL: Duration = Duration::from_secs(60);

/// How often to age the stats position table.
const STATS_COMPACTION_INTERVAL: Duration = Duration::from_secs(60 * 60);
/// Retention horizon for winds and TM history — unrelated to `stats.position`, unaffected by its
/// compaction ladder rework, still hard-pruned past this age.
const STATS_PRUNE_AFTER_DAYS: i64 = 14;
/// Delay legs are tiny (one row per flight leg) and useful over a longer window than raw positions.
const DELAY_LEG_RETAIN_DAYS: i64 = 30;

/// Weekly compaction ladder for `stats.position`: `(age_days, keep_every)`. When a position's age
/// first crosses `age_days`, keep only every `keep_every`-th sample of the survivors handed down
/// from the previous tier — an *incremental* factor, not a cumulative target. Each pass only looks
/// at the narrow slice of rows crossing that boundary *right now*, one `STATS_COMPACTION_INTERVAL`
/// wide (so consecutive runs tile the timeline with no gap and, just as importantly, no overlap —
/// an overlap would downsample the same rows twice in one tier and compound past the intended
/// ratio), never the whole historical band — reprocessing already-thinned rows on every tick would
/// grind survivors down to nothing well before they're meant to move to the next tier. Because each
/// row is (assuming the job doesn't miss a tick) touched exactly once per boundary it crosses,
/// these incremental ×4 steps compound to the effective density: full fidelity for a week, ~1 min
/// resolution (÷4) for the next, ~4 min (÷16 cumulative) the week after, and ~16 min (÷64
/// cumulative) forever past three weeks — nothing is ever fully deleted, only thinned further. A
/// missed tick leaves a thin gap of not-yet-downsampled rows rather than losing or double-thinning
/// any — the safe direction to fail in.
const COMPACTION_TIERS: &[(i64, i64)] = &[(7, 4), (14, 4), (21, 4)];

/// How often to open/close event stat-capture windows.
const CAPTURE_SCHEDULER_INTERVAL: Duration = Duration::from_secs(60);

/// How often to check the FAA/@squawk sources for a newer NASR cycle.
const NAV_REFRESH_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// How often to refresh winds aloft (AWC FB tables update ~4×/day; hourly keeps us current).
const WINDS_REFRESH_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// How often to reload aircraft performance profiles from the DB (staff edits are rare, and the
/// handler force-refreshes on write, so a slow poll is enough to catch out-of-band changes).
const AIRCRAFT_PROFILES_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// How often to reload airport surface gates from the DB (staff edits are rare, and the handler
/// force-refreshes on write, so a slow poll is enough to catch out-of-band changes).
const AIRPORT_GATES_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// How often to refresh the airport coordinate database (#216). Fast enough that a transient
/// startup failure self-heals within minutes instead of requiring a restart; slow enough not to
/// hammer the upstream (mwgg/Airports on GitHub raw).
const AIRPORTS_REFRESH_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// How often to reload taxi observation samples from the DB (#164 sub-issue E). Observations
/// accrue continuously and slowly from live traffic — no write path needs an instant force-reload
/// the way admin-edited gates do, so a slow poll is enough.
const TAXI_ESTIMATE_SAMPLES_INTERVAL: Duration = Duration::from_secs(10 * 60);

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

/// Fetch the airport coordinate database once and hot-swap it in (#216). The existing data is
/// always kept on failure — a transient boot/network failure no longer permanently strands
/// `FeedInner::airports` empty, since the caller (`spawn_airports_refresh`) retries this
/// periodically.
pub async fn refresh_airports_once(
    feed: &FeedState,
    client: &reqwest::Client,
) -> Result<usize, String> {
    let (db, iata) = crate::feed::airports::fetch(client).await.map_err(|e| {
        // The job registry tracks this failure (visible on the admin Background Tasks page), but
        // a persistent upstream failure — the whole point #216 exists to catch — deserves a log
        // line too, not just a page someone has to remember to open.
        tracing::warn!(error = %e, "feed: airport database fetch failed; retrying next tick");
        e.to_string()
    })?;
    let n = db.len();
    let mut guard = feed.write().await;
    guard.status.airports_loaded = n;
    guard.airports = Arc::new(db);
    guard.iata = Arc::new(iata);
    Ok(n)
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
pub fn spawn_nav_refresh(
    reg: Arc<JobRegistry>,
    nav: Arc<ArcSwap<NavData>>,
    refreshed: Arc<AtomicI64>,
) {
    tokio::spawn(run_interval(
        reg,
        "nav_refresh",
        "Fetch the latest FAA NASR nav cycle",
        NAV_REFRESH_INTERVAL,
        move || {
            let (nav, refreshed) = (nav.clone(), refreshed.clone());
            async move {
                match refresh_nav_once(&nav, &refreshed).await {
                    Ok(true) => Ok("nav cycle updated".to_string()),
                    Ok(false) => Ok("already current".to_string()),
                    Err(e) => Err(e),
                }
            }
        },
    ));
}

/// Keep the airport coordinate database current: fetch it at startup and every 5 min (#216). A
/// failed fetch — including the very first one at boot — is retried on the next tick rather than
/// leaving `FeedInner::airports` permanently empty; the existing data (or the empty default) is
/// kept until a fetch succeeds. Registered in the job registry so a stuck load is visible on the
/// admin Background Tasks page instead of only a warn log.
pub fn spawn_airports_refresh(reg: Arc<JobRegistry>, feed: FeedState) {
    let client = reqwest::Client::builder()
        .user_agent("ois-backend/0.1 (+https://vatusa.net)")
        .timeout(Duration::from_secs(20))
        .build()
        .unwrap_or_default();
    tokio::spawn(run_interval(
        reg,
        "airports_refresh",
        "Fetch the airport coordinate database",
        AIRPORTS_REFRESH_INTERVAL,
        move || {
            let (feed, client) = (feed.clone(), client.clone());
            async move {
                refresh_airports_once(&feed, &client)
                    .await
                    .map(|n| format!("{n} airports"))
            }
        },
    ));
}

/// Keep winds aloft current for ETA prediction: once the airport database is loaded, fetch
/// the AWC FB tables and hot-swap them in, then refresh hourly. Fails safe. When a DB pool is
/// present, each successful refresh also snapshots the winds to `stats.winds` so historical replay
/// can reconstruct past ETAs.
pub fn spawn_winds_refresh(
    reg: Arc<JobRegistry>,
    feed: FeedState,
    winds: Arc<ArcSwap<Winds>>,
    refreshed: Arc<AtomicI64>,
    pool: Option<sqlx::PgPool>,
) {
    tokio::spawn(async move {
        // Not run_interval: winds has a variable cadence (fast retry until the airport DB loads,
        // then hourly), so it's observed but not manually triggerable.
        reg.register(
            "winds_refresh",
            "Fetch winds aloft (AWC FB tables)",
            Some(WINDS_REFRESH_INTERVAL.as_secs()),
            false,
        );
        let client = reqwest::Client::builder()
            .user_agent("ois-winds/1.0 (+https://vatusa.net)")
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap_or_default();
        loop {
            reg.begin("winds_refresh");
            match refresh_winds_once(&feed, &winds, &refreshed, &client).await {
                // Airport DB not loaded yet — retry soon.
                None => {
                    reg.finish("winds_refresh", false, "airport database not loaded yet");
                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
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
                        reg.finish("winds_refresh", true, format!("{n} stations"));
                    } else {
                        tracing::warn!("winds refresh returned no stations; keeping current");
                        reg.finish("winds_refresh", false, "no stations returned");
                    }
                    tokio::time::sleep(WINDS_REFRESH_INTERVAL).await;
                }
            }
        }
    });
}

/// Keep the trajectory model's aircraft performance profiles current: load them from the DB at
/// startup and hot-swap them in, then reload periodically. Fails safe — a failed load keeps the
/// current table (initially the legacy default).
pub fn spawn_aircraft_profiles_refresh(
    reg: Arc<JobRegistry>,
    pool: PgPool,
    profiles: Arc<ArcSwap<ProfileTable>>,
) {
    tokio::spawn(run_interval(
        reg,
        "aircraft_profiles_refresh",
        "Reload aircraft performance profiles from the DB",
        AIRCRAFT_PROFILES_INTERVAL,
        move || {
            let (pool, profiles) = (pool.clone(), profiles.clone());
            async move {
                match aircraft_profiles_repo::load_all(&pool).await {
                    Ok(table) => {
                        profiles.store(Arc::new(table));
                        Ok("reloaded".to_string())
                    }
                    Err(e) => Err(format!("{e:?}")),
                }
            }
        },
    ));
}

/// Keep the airport surface gate cache current for the DB-less feed subsystem
/// (`feed::taxi_observations`'s gate matching): load every gate from the DB at startup and
/// hot-swap it in, then reload periodically. Fails safe — a failed load keeps the current map.
pub fn spawn_airport_gates_refresh(
    reg: Arc<JobRegistry>,
    pool: PgPool,
    gates: Arc<ArcSwap<std::collections::HashMap<String, Vec<AirportGateBody>>>>,
) {
    tokio::spawn(run_interval(
        reg,
        "airport_gates_refresh",
        "Reload airport surface gates from the DB",
        AIRPORT_GATES_INTERVAL,
        move || {
            let (pool, gates) = (pool.clone(), gates.clone());
            async move {
                match airport_surface_repo::load_all_gates(&pool).await {
                    Ok(by_icao) => {
                        gates.store(Arc::new(by_icao));
                        Ok("reloaded".to_string())
                    }
                    Err(e) => Err(format!("{e:?}")),
                }
            }
        },
    ));
}

/// Keep the learned taxi-observation sample cache current for the DB-less feed subsystem
/// (`feed::flow::resolve_ground_allowance_sec`, #164 sub-issue E): load every observation from the
/// DB, group by airport, and hot-swap it in, then reload periodically. Fails safe — a failed load
/// keeps the current map.
pub fn spawn_taxi_estimate_samples_refresh(
    reg: Arc<JobRegistry>,
    pool: PgPool,
    samples: Arc<
        ArcSwap<std::collections::HashMap<String, Vec<crate::feed::taxi_estimate::TaxiSample>>>,
    >,
) {
    tokio::spawn(run_interval(
        reg,
        "taxi_estimate_samples_refresh",
        "Reload taxi observation samples from the DB",
        TAXI_ESTIMATE_SAMPLES_INTERVAL,
        move || {
            let (pool, samples) = (pool.clone(), samples.clone());
            async move {
                match stats_repo::load_all_taxi_samples(&pool).await {
                    Ok(by_airport) => {
                        samples.store(Arc::new(by_airport));
                        Ok("reloaded".to_string())
                    }
                    Err(e) => Err(format!("{e:?}")),
                }
            }
        },
    ));
}

/// Age the stats position time-series through the weekly `COMPACTION_TIERS` ladder — full fidelity
/// for a week, then progressively coarser, forever (nothing is hard-deleted past the ladder
/// anymore). Rows inside an open/saved `stats.capture` window are skipped at every tier (retained
/// at full fidelity). Runs hourly; a slow, batched, saved-window-aware alternative to TimescaleDB
/// retention.
pub fn spawn_stats_compaction(reg: Arc<JobRegistry>, pool: PgPool) {
    // Tracks the hour-bucket (`now / STATS_COMPACTION_INTERVAL`) whose COMPACTION_TIERS slices
    // were last processed. The admin Jobs page can trigger this job on demand (`system.jobs.update`)
    // in addition to its hourly schedule; a trigger landing in the same bucket as the last run must
    // be a no-op for the tiered passes below, or it would re-downsample that tier's already-thinned
    // survivors and compound well past the intended ratio (see `stats_compaction_once`).
    let last_tier_hour = Arc::new(AtomicI64::new(0));
    tokio::spawn(run_interval(
        reg,
        "stats_compaction",
        "Downsample the stats position time-series through the weekly retention ladder",
        STATS_COMPACTION_INTERVAL,
        move || {
            let pool = pool.clone();
            let last_tier_hour = last_tier_hour.clone();
            async move { stats_compaction_once(&pool, &last_tier_hour).await }
        },
    ));
}

/// One stats-compaction pass: downsample the narrow slice of positions crossing each
/// `COMPACTION_TIERS` boundary, and prune everything past the unrelated retention horizons (winds,
/// TM history, flight legs — `stats.position` is never hard-deleted anymore). Best-effort — a
/// failed sub-pass is logged and the others still run; returns a summary of rows removed.
async fn stats_compaction_once(
    pool: &PgPool,
    last_tier_hour: &AtomicI64,
) -> Result<String, String> {
    let now = Utc::now();
    let legs_before = now - chrono::Duration::days(DELAY_LEG_RETAIN_DAYS);
    let prune_before = now - chrono::Duration::days(STATS_PRUNE_AFTER_DAYS);
    // Exactly the run interval, so consecutive ticks tile the timeline with no gap *and* no
    // overlap — an overlap would downsample the same rows twice per tier (see COMPACTION_TIERS).
    let slice = chrono::Duration::from_std(STATS_COMPACTION_INTERVAL)
        .unwrap_or_else(|_| chrono::Duration::hours(1));
    let mut removed: u64 = 0;

    // Each tier's boundary slice is exactly one hour-bucket wide; run it at most once per bucket no
    // matter how many times this fn is invoked within it (the scheduled tick, plus any manual
    // "run now" trigger) — a second run would downsample that slice's already-thinned survivors
    // again. `swap` both checks and immediately claims the bucket, so two near-simultaneous
    // invocations can't both see it as due. A genuinely new bucket (the next scheduled tick, or a
    // manual trigger after the interval has elapsed) still runs normally.
    let hour_bucket = now.timestamp() / STATS_COMPACTION_INTERVAL.as_secs() as i64;
    let tiers_due = last_tier_hour.swap(hour_bucket, Ordering::Relaxed) != hour_bucket;

    let mut passes: Vec<(&str, Result<u64, ApiError>)> = Vec::new();
    if tiers_due {
        for &(age_days, keep_every) in COMPACTION_TIERS {
            let to = now - chrono::Duration::days(age_days);
            let from = to - slice;
            passes.push((
                "downsample",
                stats_repo::downsample_positions(pool, from, to, keep_every).await,
            ));
        }
    }
    passes.push(("winds", stats_repo::prune_winds(pool, prune_before).await));
    passes.push((
        "tm-history",
        crate::repos::tmu::prune_history(pool, prune_before).await,
    ));
    passes.push((
        "flight-legs",
        stats_repo::prune_flight_legs(pool, legs_before).await,
    ));
    passes.push((
        "taxi-observations",
        stats_repo::prune_taxi_observations(pool, legs_before).await,
    ));

    for (label, res) in passes {
        match res {
            Ok(n) => {
                if n > 0 {
                    tracing::info!(deleted = n, pass = label, "stats: compaction");
                }
                removed += n;
            }
            Err(_) => tracing::warn!(pass = label, "stats: compaction sub-pass failed"),
        }
    }
    Ok(format!("{removed} rows removed"))
}

/// Drive per-event stat capture: for each event with capture enabled, open a `stats.capture`
/// window once the event is inside `[start - pre, end + post]`, and close+save it once that window
/// has passed. Runs every minute. Idempotent — it keys off whether an open capture already exists.
pub fn spawn_capture_scheduler(reg: Arc<JobRegistry>, pool: PgPool) {
    tokio::spawn(run_interval(
        reg,
        "capture_scheduler",
        "Open/close per-event stat capture windows",
        CAPTURE_SCHEDULER_INTERVAL,
        move || {
            let pool = pool.clone();
            async move { capture_scheduler_once(&pool).await }
        },
    ));
}

/// One capture-scheduler pass: open a capture for each event now inside its window, and close+save
/// captures whose window has ended. Idempotent. Returns a summary of what changed.
async fn capture_scheduler_once(pool: &PgPool) -> Result<String, String> {
    let rows = stats_repo::list_capture_schedule(pool)
        .await
        .map_err(|_| "capture schedule query failed".to_string())?;
    let now = Utc::now();
    let (mut opened, mut saved) = (0u32, 0u32);
    for r in rows {
        let window_start = r.start_time - chrono::Duration::minutes(r.pre_minutes as i64);
        let window_end = r.end_time + chrono::Duration::minutes(r.post_minutes as i64);
        let in_window = now >= window_start && now <= window_end;

        match (in_window, r.open_capture_id.as_deref()) {
            // Inside the window with no capture yet → open one covering the whole window.
            (true, None) => {
                match stats_repo::create_capture(
                    pool,
                    Some(r.event_id),
                    &r.title,
                    window_start,
                    None,
                )
                .await
                {
                    Ok(id) => {
                        tracing::info!(event = r.event_id, capture = %id, "stats: opened event capture");
                        opened += 1;
                    }
                    Err(_) => tracing::warn!(event = r.event_id, "stats: open capture failed"),
                }
            }
            // Past the window with an open capture → close + save it.
            (false, Some(_)) if now > window_end => {
                if let Ok(n) =
                    stats_repo::close_open_event_captures(pool, r.event_id, window_end).await
                    && n > 0
                {
                    tracing::info!(event = r.event_id, "stats: saved event capture");
                    saved += 1;
                }
            }
            _ => {}
        }
    }
    Ok(if opened == 0 && saved == 0 {
        "no changes".to_string()
    } else {
        format!("{opened} opened, {saved} saved")
    })
}

/// How often to check for ACE claims crossing a reminder threshold.
const ACE_REMINDER_INTERVAL: Duration = Duration::from_secs(15 * 60);

/// Reminder tiers as `(hours_after, hours_before, job_type)` — each tier's own window (see
/// `claims_due_for_reminder`'s doc comment for why the lower bound matters), and the outbound-job
/// type used both to dispatch and to de-duplicate each (via `not exists` against
/// `integration.outbound_jobs`).
const ACE_REMINDER_TIERS: &[(i64, i64, &str)] = &[
    (6, 24, "ace_claim_reminder_24h"),
    (0, 6, "ace_claim_reminder_6h"),
];

/// DM ACE claimers a reminder at T-24h and T-6h before their event starts. Idempotent by
/// construction: each tick re-queries live state (crossed the threshold, event still upcoming, not
/// already reminded), so a released claim or a cancelled request simply stops matching — no
/// separate "cancel the scheduled reminder" step is needed. Runs every 15 minutes.
pub fn spawn_ace_reminder_scheduler(reg: Arc<JobRegistry>, pool: PgPool) {
    tokio::spawn(run_interval(
        reg,
        "ace_reminder_scheduler",
        "DM ACE claimers a reminder at T-24h/T-6h before their event",
        ACE_REMINDER_INTERVAL,
        move || {
            let pool = pool.clone();
            async move { ace_reminder_scheduler_once(&pool).await }
        },
    ));
}

async fn ace_reminder_scheduler_once(pool: &PgPool) -> Result<String, String> {
    let mut sent = 0u32;
    let mut tier_failed = false;
    // Each tier is queried and enqueued independently — a transient failure on one tier's query
    // must not skip the other tier's check for this cycle (they're unrelated thresholds), so errors
    // are logged and accumulated rather than propagated with `?`, which would abort the whole loop
    // on the first failure.
    for &(hours_after, hours_before, job_type) in ACE_REMINDER_TIERS {
        let due = match ace_repo::claims_due_for_reminder(pool, hours_after, hours_before, job_type)
            .await
        {
            Ok(due) => due,
            Err(e) => {
                tracing::warn!(job_type, error = ?e, "ace reminder query failed");
                tier_failed = true;
                continue;
            }
        };
        for r in due {
            let payload = json!({
                "discord_user_id": r.discord_user_id,
                "event_title": r.event_title,
                "position": r.position,
                "reminder": format!("{hours_before}h"),
            });
            match ace_repo::enqueue_reminder_job(pool, job_type, &r.claim_id, &payload).await {
                Ok(inserted) => {
                    if inserted {
                        sent += 1;
                    }
                }
                Err(_) => {
                    tracing::warn!(claim = %r.claim_id, job_type, "ace reminder enqueue failed")
                }
            }
        }
    }
    // Unconditional on `tier_failed`: a persistently-failing tier must always surface to the
    // JobRegistry as a failure, even in a cycle where the *other* tier had genuine hits — masking
    // it behind `sent == 0` would hide an ongoing problem for as long as the healthy tier keeps
    // producing reminders.
    if tier_failed {
        return Err("one or more ace reminder tiers failed to query".to_string());
    }
    Ok(if sent == 0 {
        "no changes".to_string()
    } else {
        format!("{sent} reminder(s) enqueued")
    })
}

/// Periodically expire finished TMIs/ground stops and delete ones that ended over an hour
/// ago. Runs once at startup, then every 15 minutes.
pub fn spawn_cleanup(reg: Arc<JobRegistry>, pool: PgPool) {
    tokio::spawn(run_interval(
        reg,
        "tmu_cleanup",
        "Expire + delete finished TMIs / ground stops",
        CLEANUP_INTERVAL,
        move || {
            let pool = pool.clone();
            async move {
                match tmu_repo::run_cleanup(&pool).await {
                    Ok(stats) => {
                        if stats.expired > 0 || stats.deleted > 0 {
                            tracing::info!(
                                expired = stats.expired,
                                deleted = stats.deleted,
                                "tmu cleanup pass"
                            );
                        }
                        Ok(format!(
                            "{} expired, {} deleted",
                            stats.expired, stats.deleted
                        ))
                    }
                    Err(_) => Err("cleanup pass failed".to_string()),
                }
            }
        },
    ));
}

/// Drive event FCAs through their lifecycle: publish `planned` + auto ones ~30 min before their event
/// starts, and archive still-live ones when it ends. Nudges connected maps (`flow.fca`) whenever
/// anything changed. Runs every minute.
pub fn spawn_event_fca_lifecycle(reg: Arc<JobRegistry>, pool: PgPool, events: Events) {
    tokio::spawn(run_interval(
        reg,
        "event_fca_lifecycle",
        "Auto-publish / archive event FCAs",
        EVENT_FCA_INTERVAL,
        move || {
            let (pool, events) = (pool.clone(), events.clone());
            async move {
                match flow_repo::run_event_fca_lifecycle(&pool).await {
                    Ok(0) => Ok("no changes".to_string()),
                    Ok(changed) => {
                        let _ = events.send(WsEvent {
                            topic: topic::FCA.to_string(),
                        });
                        tracing::info!(changed, "event FCA lifecycle pass");
                        Ok(format!("{changed} changed"))
                    }
                    Err(_) => Err("lifecycle pass failed".to_string()),
                }
            }
        },
    ));
}

/// Drive event TMI packages through their lifecycle: auto-activate draft + auto packages ~30 min
/// before their event starts (materializing live TMU rows), and auto-deactivate (archive) activated
/// ones when it ends. Acts as the package's `updated_by`. Nudges connected clients when anything
/// changed. Runs every minute.
pub fn spawn_event_package_lifecycle(reg: Arc<JobRegistry>, pool: PgPool, events: Events) {
    tokio::spawn(run_interval(
        reg,
        "event_package_lifecycle",
        "Auto-activate / archive event TMI packages",
        EVENT_FCA_INTERVAL,
        move || {
            let (pool, events) = (pool.clone(), events.clone());
            async move { event_package_lifecycle_once(&pool, &events).await }
        },
    ));
}

/// One event-TMI-package lifecycle pass: auto-activate draft+auto packages entering the pre-event
/// window and auto-archive activated ones whose event ended; nudges clients when anything changed.
async fn event_package_lifecycle_once(pool: &PgPool, events: &Events) -> Result<String, String> {
    let mut changed = 0u32;

    // Auto-activate: draft + auto packages entering the 30-min pre-event window.
    match events_repo::auto_due_packages(pool).await {
        Ok(due) => {
            for (package_id, event_id, actor) in due {
                match crate::handlers::events::activate_package(pool, event_id, &package_id, &actor)
                    .await
                {
                    Ok(()) => changed += 1,
                    Err(_) => tracing::warn!(%package_id, "auto-activate package failed"),
                }
            }
        }
        Err(_) => tracing::warn!("auto-due package query failed"),
    }

    // Auto-archive: activated packages whose event has ended.
    match events_repo::ended_activated_packages(pool).await {
        Ok(ended) => {
            for (package_id, _event_id, actor) in ended {
                match crate::handlers::events::deactivate_package(pool, &package_id, &actor).await {
                    Ok(()) => changed += 1,
                    Err(_) => tracing::warn!(%package_id, "auto-archive package failed"),
                }
            }
        }
        Err(_) => tracing::warn!("ended-package query failed"),
    }

    if changed > 0 {
        for t in [topic::PROGRAM, topic::TMI, topic::GROUND_STOP] {
            let _ = events.send(WsEvent {
                topic: t.to_string(),
            });
        }
        tracing::info!(changed, "event package lifecycle pass");
    }
    Ok(if changed > 0 {
        format!("{changed} changed")
    } else {
        "no changes".to_string()
    })
}
