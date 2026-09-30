//! Background maintenance jobs.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use arc_swap::ArcSwap;
use chrono::{NaiveDate, Utc};
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
use crate::repos::flight_exclusions as flight_exclusions_repo;
use crate::repos::flow as flow_repo;
use crate::repos::integration as integration_repo;
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

/// How long a leased outbound job may stay `in_progress` before it is treated as abandoned (#446).
///
/// The bot leases, performs one Discord call and acks — seconds of work. Five minutes is far beyond
/// any legitimate run, so a job still `in_progress` after it did not finish: its worker died between
/// leasing and acking, and nothing else will ever move it.
const OUTBOUND_JOB_LEASE_TIMEOUT_MINS: i64 = 5;

/// How long `access.audit_logs` rows are kept (#444).
///
/// Six months: long enough to span a full VATUSA event season, so "what changed before that event"
/// is still answerable, and short enough that the table stops growing without bound — it had no
/// retention at all, and grew for the life of the deployment.
///
/// One window for every `resource_type` deliberately. A per-type table would have to track the types
/// `audit::derive` invents from the request path, which nothing centrally registers, so it would
/// drift silently the first time a route was added.
const AUDIT_RETAIN_DAYS: i64 = 180;

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
const NAV_REFRESH_JOB: &str = "nav_refresh";

/// How often to refresh winds aloft (AWC FB tables update ~4×/day; hourly keeps us current).
const WINDS_REFRESH_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// How often to reload aircraft performance profiles from the DB (staff edits are rare, and the
/// handler force-refreshes on write, so a slow poll is enough to catch out-of-band changes).
const AIRCRAFT_PROFILES_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// How often to reload airport surface gates from the DB (staff edits are rare, and the handler
/// force-refreshes on write, so a slow poll is enough to catch out-of-band changes).
const AIRPORT_GATES_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// How often to reload manually excluded ("bogus") flights and clear the ones whose callsign has
/// left the feed (#342). The write handler force-refreshes, so this poll is only about the
/// auto-clear — it wants to be brisk enough that a corrected flight reappears promptly.
const FLIGHT_EXCLUSIONS_INTERVAL: Duration = Duration::from_secs(60);

/// How often to run the FAA airport surface seed (#230/#231). The seed only fills airports that have
/// no `faa` rows yet (existing rows — and facility edits to them — are never touched), so this
/// interval isn't about freshness, just a nominal cadence like every other registry entry; the
/// meaningful trigger is `run_interval`'s immediate first tick on every boot (picking up airports a
/// redeployed extract newly covers), plus the admin Background Tasks page's (#40) on-demand run.
const FAA_SURFACE_SEED_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// How often to refresh the airport coordinate database (#216). Fast enough that a transient
/// startup failure self-heals within minutes instead of requiring a restart; slow enough not to
/// hammer the upstream (mwgg/Airports on GitHub raw).
const AIRPORTS_REFRESH_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// How often to reload taxi observation samples from the DB (#164 sub-issue E). Observations
/// accrue continuously and slowly from live traffic — no write path needs an instant force-reload
/// the way admin-edited gates do, so a slow poll is enough.
const TAXI_ESTIMATE_SAMPLES_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// Fetch the latest NASR data once and hot-swap it in when it changes — but never for an older
/// cycle than the one loaded, so a failed fetch's bundle fallback can't replace good live data.
/// `Ok(true)`/`Ok(false)` (updated / already current) only for a **healthy** refresh: the live FAA
/// source at the current cycle, which is also the only case that records the refresh time. A
/// degraded result (a fallback source, or a cycle behind current) is `Err` with the reason, so the
/// job shows failed on the Background Tasks page (VATUSA/OIS#317). Existing data is always kept on
/// a fetch failure.
pub async fn refresh_nav_once(
    nav: &Arc<ArcSwap<NavData>>,
    refreshed: &Arc<AtomicI64>,
) -> Result<bool, String> {
    let fresh = nav_source::fetch_latest()
        .await
        .map_err(|e| e.to_string())?;
    apply_nav_refresh(nav, refreshed, fresh, nav_source::current_cycle())
}

/// [`refresh_nav_once`] after the network fetch: swap `fresh` in, judge its health against the
/// `expected` cycle, and record the refresh time only when healthy.
fn apply_nav_refresh(
    nav: &ArcSwap<NavData>,
    refreshed: &AtomicI64,
    fresh: NavData,
    expected: NaiveDate,
) -> Result<bool, String> {
    if fresh.is_empty() {
        return Err("nav fetch produced an empty database".into());
    }
    let current = nav.load();
    let swap = should_swap_nav(
        (fresh.cycle(), fresh.len()),
        (current.cycle(), current.len()),
    );
    if swap {
        tracing::info!(
            from_cycle = current.cycle(),
            to_cycle = fresh.cycle(),
            points = fresh.len(),
            source = fresh.source(),
            "nav database refreshed"
        );
    }
    let health = nav_health(
        fresh.source(),
        fresh.cycle(),
        nav_source::cycles_behind(fresh.cycle(), expected),
        &expected.format("%Y-%m-%d").to_string(),
    );
    if swap {
        nav.store(Arc::new(fresh));
    }
    match health {
        Ok(()) => {
            refreshed.store(Utc::now().timestamp_millis(), Ordering::Relaxed);
            Ok(swap)
        }
        Err((detail, cycles_behind)) => {
            if cycles_behind.is_none_or(|n| n >= 2) {
                tracing::error!(detail, "nav refresh degraded");
            } else {
                tracing::warn!(detail, "nav refresh degraded");
            }
            Err(detail)
        }
    }
}

/// Whether fetched nav data `(cycle, points)` should replace the loaded data: it differs, and its
/// cycle (`YYYY-MM-DD`, so string order is date order) is not older than the loaded one.
fn should_swap_nav(fresh: (&str, usize), current: (&str, usize)) -> bool {
    fresh != current && fresh.0 >= current.0
}

/// Whether a fetched nav dataset is a healthy refresh: the live FAA source, at the current cycle.
/// Otherwise the reason, with how many cycles behind it is (`None` = the cycle couldn't be read).
fn nav_health(
    source: &str,
    cycle: &str,
    cycles_behind: Option<u32>,
    current: &str,
) -> Result<(), (String, Option<u32>)> {
    match cycles_behind {
        Some(0) if source == nav_source::FAA_SOURCE => Ok(()),
        Some(n) => Err((
            format!("degraded: {source}, cycle {cycle} is {n} cycle(s) behind current {current}"),
            Some(n),
        )),
        None => Err((
            format!("degraded: {source}, unreadable cycle {cycle:?} (current {current})"),
            None,
        )),
    }
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

/// Keep the in-memory nav database current: refresh at startup, every 24h, and as each new
/// cycle takes effect (so a healthy host isn't a cycle behind until its next 24h tick,
/// VATUSA/OIS#317). On any failure the existing data is kept — the server always has a
/// coherent dataset from the compile-time bundle seed.
pub fn spawn_nav_refresh(
    reg: Arc<JobRegistry>,
    nav: Arc<ArcSwap<NavData>>,
    refreshed: Arc<AtomicI64>,
) {
    let rollover_reg = reg.clone();
    tokio::spawn(async move {
        loop {
            let now = Utc::now();
            let wait = (nav_source::next_cycle_start(now) - now)
                .to_std()
                .unwrap_or_default();
            tokio::time::sleep(wait).await;
            rollover_reg.trigger(NAV_REFRESH_JOB);
        }
    });
    tokio::spawn(run_interval(
        reg,
        NAV_REFRESH_JOB,
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

/// Keep the manual flight-exclusion cache current for the DB-less flow surfaces (#342), and run the
/// auto-clear: an exclusion whose callsign has left the VATSIM feed is deleted, so a corrected or
/// returning flight is never hidden forever. `expires_at` is the TTL backstop for a callsign that
/// never cleanly departs the feed; readers filter on it, so an expired row needs no deletion here.
/// Fails safe — a failed load keeps the current map.
pub fn spawn_flight_exclusions_refresh(
    reg: Arc<JobRegistry>,
    pool: PgPool,
    feed: FeedState,
    exclusions: Arc<ArcSwap<std::collections::HashMap<String, std::collections::HashSet<String>>>>,
) {
    tokio::spawn(run_interval(
        reg,
        "flight_exclusions_refresh",
        "Reload manual flight exclusions and clear ones that left the feed",
        FLIGHT_EXCLUSIONS_INTERVAL,
        move || {
            let (pool, feed, exclusions) = (pool.clone(), feed.clone(), exclusions.clone());
            async move {
                // Callsigns currently in the feed — pilots and prefiles alike, since a prefile can
                // be just as bogus as a connected aircraft.
                // Clone the Arc and drop the feed lock before touching the DB. An absent snapshot
                // (feed not loaded yet) yields no callsigns, and `clear_departed` treats that as a
                // no-op rather than deleting every exclusion.
                let snapshot = feed.read().await.snapshot.clone();
                let live: Vec<String> = snapshot
                    .as_ref()
                    .map(|s| {
                        s.data
                            .pilots
                            .iter()
                            .map(|p| p.callsign.clone())
                            .chain(s.data.prefiles.iter().map(|p| p.callsign.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                let cleared = flight_exclusions_repo::clear_departed(&pool, &live)
                    .await
                    .map_err(|e| format!("{e:?}"))?;
                match flight_exclusions_repo::load_all(&pool).await {
                    Ok(by_artcc) => {
                        exclusions.store(Arc::new(by_artcc));
                        Ok(format!("reloaded, cleared {cleared}"))
                    }
                    Err(e) => Err(format!("{e:?}")),
                }
            }
        },
    ));
}

/// Seed `flow.airport_ramp_area` / `flow.airport_taxiway` from the bundled FAA Aerodrome Mapping
/// extract for airports with no `faa` rows yet (#230/#231) — `run_interval`'s immediate first tick means this runs
/// once on every boot, in addition to being visible/triggerable on the admin Background Tasks page.
/// No `AppState` cache to hot-swap here: unlike gates, nothing in the feed subsystem reads ramp/
/// taxiway data — the map editor queries Postgres directly per request.
pub fn spawn_faa_surface_seed(reg: Arc<JobRegistry>, pool: PgPool) {
    tokio::spawn(run_interval(
        reg,
        "faa_surface_seed",
        "Seed airport ramp/taxiway geometry from the bundled FAA AM extract",
        FAA_SURFACE_SEED_INTERVAL,
        move || {
            let pool = pool.clone();
            async move {
                crate::repos::faa_surface_seed::seed(&pool)
                    .await
                    .map(|summary| summary.to_string())
                    .map_err(|e| format!("{e:?}"))
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

/// Freeze one event's movement counts over its own window (#433).
///
/// Movements are counted from `stats.flight_leg`, which is pruned at [`DELAY_LEG_RETAIN_DAYS`], so
/// without this an event's numbers would quietly fall to zero a month after it ran. Taken over the
/// event's own window, matching what the stats endpoint reports.
///
/// Best-effort: a failure here costs the frozen copy, not the capture lifecycle the scheduler is
/// actually responsible for, and the endpoint still computes from legs until they age out. So it logs
/// and returns rather than propagating. The next tick retries, since the missing row is exactly what
/// makes the event match again.
///
/// Returns whether a row was actually written, so the pass's summary — which the admin Jobs page
/// shows as `last_detail` — counts freezes rather than attempts.
async fn snapshot_event_movements(
    pool: &PgPool,
    event_id: i64,
    start_time: chrono::DateTime<Utc>,
    end_time: chrono::DateTime<Utc>,
) -> bool {
    let icaos: Vec<String> = match events_repo::list_airport_rates(pool, event_id).await {
        Ok(rates) => rates.into_iter().map(|r| r.icao).collect(),
        Err(_) => {
            tracing::warn!(
                event = event_id,
                "stats: snapshot skipped, airports unreadable"
            );
            return false;
        }
    };
    if icaos.is_empty() {
        return false;
    }

    match stats_repo::event_airport_breakdown(pool, &icaos, start_time, end_time).await {
        Ok(rows) => {
            // Never freeze an all-zero breakdown. For an event whose legs have already been pruned
            // the recomputed movements are zero for that reason alone, and freezing them would make
            // an artifact of retention permanent — the row is what makes the freeze pass stop
            // matching the event, so no later pass could do better. Leaving it unfrozen costs nothing: the read
            // path computes the same zero, and keeps the option open (#433 review).
            if rows.iter().all(|r| r.arrivals + r.departures == 0) {
                tracing::debug!(
                    event = event_id,
                    "stats: no observed movements to freeze, left to compute"
                );
                return false;
            }
            match stats_repo::snapshot_event_movements(pool, event_id, start_time, end_time, &rows)
                .await
            {
                Ok(n) => {
                    tracing::info!(event = event_id, airports = n, "stats: froze movements");
                    true
                }
                Err(_) => {
                    tracing::warn!(event = event_id, "stats: snapshot write failed");
                    false
                }
            }
        }
        Err(_) => {
            tracing::warn!(event = event_id, "stats: snapshot breakdown failed");
            false
        }
    }
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
    // Freeze the movements of every finished event that isn't frozen yet (#433 review).
    //
    // This is deliberately one pass rather than a snapshot on the close transition plus a separate
    // backfill. The close arm only fires for an event with an *open* capture, so on its own it could
    // never reach anything that closed before `stats.event_movements` existed — those events kept
    // computing from `stats.flight_leg` and fell to zero as their legs crossed
    // DELAY_LEG_RETAIN_DAYS. Asking "which finished events have no snapshot" answers both cases
    // with one query, and it runs in the same invocation as the close above, so a capture that
    // closes on this tick is still frozen on this tick. A call in the close arm as well was
    // redundant: deleting it left every test green, because this pass had already done the work.
    //
    // Idempotent and self-limiting — writing the row is what stops the event matching. Events whose
    // legs are already gone are declined by the all-zero guard in `snapshot_event_movements`, and
    // the query is bounded to the same retention horizon so it does not re-ask about them every
    // minute for the life of the deployment.
    let mut frozen = 0u32;
    match stats_repo::events_missing_movement_snapshot(pool, DELAY_LEG_RETAIN_DAYS).await {
        Ok(unfrozen) => {
            for e in unfrozen {
                if snapshot_event_movements(pool, e.event_id, e.start_time, e.end_time).await {
                    frozen += 1;
                }
            }
        }
        Err(_) => tracing::warn!("stats: unfrozen-event query failed"),
    }

    Ok(if opened == 0 && saved == 0 && frozen == 0 {
        "no changes".to_string()
    } else {
        format!("{opened} opened, {saved} saved, {frozen} frozen")
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
pub fn spawn_ace_reminder_scheduler(reg: Arc<JobRegistry>, pool: PgPool, events: Events) {
    tokio::spawn(run_interval(
        reg,
        "ace_reminder_scheduler",
        "DM ACE claimers a reminder at T-24h/T-6h before their event",
        ACE_REMINDER_INTERVAL,
        move || {
            let (pool, events) = (pool.clone(), events.clone());
            async move { ace_reminder_scheduler_once(&pool, &events).await }
        },
    ));
}

async fn ace_reminder_scheduler_once(pool: &PgPool, events: &Events) -> Result<String, String> {
    let mut sent = 0u32;
    let mut in_window = 0i64;
    let mut tier_failed = false;
    // Each tier is queried and enqueued independently — a transient failure on one tier's query
    // must not skip the other tier's check for this cycle (they're unrelated thresholds), so errors
    // are logged and accumulated rather than propagated with `?`, which would abort the whole loop
    // on the first failure.
    for &(hours_after, hours_before, job_type) in ACE_REMINDER_TIERS {
        // Every live claim in this tier's window, Discord or not — what decides the client nudge below.
        match ace_repo::claims_in_reminder_window(pool, hours_after, hours_before).await {
            Ok(n) => in_window += n,
            Err(e) => {
                tracing::warn!(job_type, error = ?e, "ace reminder window count failed");
                tier_failed = true;
            }
        }
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
    // Nudge connected clients whenever any claim is inside a reminder window, so the desktop app can
    // surface the reminder natively (#348). Not keyed on `sent`: that counts Discord DMs, so a desktop
    // user without a linked Discord account was never nudged and never reminded. Payload-free: each
    // client re-checks its own claims, and its notifier fires once per claim and tier.
    if in_window > 0 {
        let _ = events.send(WsEvent {
            topic: topic::EVENT_REMINDER.to_string(),
        });
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

/// Delete one-time desktop auth codes long past their 60-second life (VATUSA/OIS#346). Nothing
/// else removes them, so without this the table only grows. Same cadence as the TMU cleanup.
pub fn spawn_desktop_auth_code_prune(reg: Arc<JobRegistry>, pool: PgPool) {
    tokio::spawn(run_interval(
        reg,
        "desktop_auth_code_prune",
        "Delete expired one-time desktop sign-in codes",
        CLEANUP_INTERVAL,
        move || {
            let pool = pool.clone();
            async move {
                crate::repos::auth::prune_desktop_auth_codes(&pool)
                    .await
                    .map(|n| format!("{n} deleted"))
                    .map_err(|_| "prune failed".to_string())
            }
        },
    ));
}

/// Return outbound jobs stranded `in_progress` to the queue (#446).
///
/// Only the bot acking a job moves it out of `in_progress`, so a worker that dies mid-job leaves it
/// there forever — never retried, never delivered, and with no error to notice, because nothing
/// failed. Every redeploy is a chance to hit that window.
///
/// The recovery reuses the failed-ack transition, so the retry policy lives in one place.
pub fn spawn_outbound_job_reaper(reg: Arc<JobRegistry>, pool: PgPool) {
    tokio::spawn(run_interval(
        reg,
        "outbound_job_reaper",
        "Requeue Discord jobs whose worker never acked",
        CLEANUP_INTERVAL,
        move || {
            let pool = pool.clone();
            async move { outbound_job_reaper_once(&pool).await }
        },
    ));
}

/// Delete audit rows past [`AUDIT_RETAIN_DAYS`] (#444). Nothing removed them before, so the table
/// only grew — most recently at the Discord bot's job-queue poll rate until #430 stopped that.
///
/// Its own job rather than another pass inside `stats_compaction_once`: an audit trail's retention is
/// a policy decision, not stats housekeeping, and a separate entry is what makes it visible (and
/// runnable) in the admin jobs view. Same cadence as the other cleanups.
pub fn spawn_audit_log_prune(reg: Arc<JobRegistry>, pool: PgPool) {
    tokio::spawn(run_interval(
        reg,
        "audit_log_prune",
        "Delete audit-log rows past their retention window",
        CLEANUP_INTERVAL,
        move || {
            let pool = pool.clone();
            async move {
                let before = Utc::now() - chrono::Duration::days(AUDIT_RETAIN_DAYS);
                crate::repos::audit::prune_audit_logs(&pool, before)
                    .await
                    .map(|n| format!("{n} deleted"))
                    .map_err(|_| "prune failed".to_string())
            }
        },
    ));
}

/// One reaper pass: anything `in_progress` past [`OUTBOUND_JOB_LEASE_TIMEOUT_MINS`] goes back to the
/// queue.
///
/// Split out of the spawn so the cutoff can be tested, mirroring `capture_scheduler_once` and
/// `ace_reminder_scheduler_once`. `reap_stranded_jobs` takes the cutoff as a parameter, which is what
/// makes its own tests precise — but it also meant the *constant* was outside every test, and a constant
/// is exactly the kind of thing that gets "tuned" without anyone noticing what it turns off (#446 review).
async fn outbound_job_reaper_once(pool: &PgPool) -> Result<String, String> {
    let stranded_before = Utc::now() - chrono::Duration::minutes(OUTBOUND_JOB_LEASE_TIMEOUT_MINS);
    integration_repo::reap_stranded_jobs(pool, stranded_before)
        .await
        .map(|n| format!("{n} requeued"))
        .map_err(|_| "reap failed".to_string())
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

#[cfg(test)]
mod nav_health_tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI64, Ordering};

    use arc_swap::ArcSwap;
    use chrono::NaiveDate;
    use serde_json::json;

    use super::{apply_nav_refresh, nav_health, should_swap_nav};
    use crate::feed::nav::NavData;
    use crate::feed::nav_source::FAA_SOURCE;

    const BUNDLE: &str = "runtime fetch (bundle)";
    const LAST_GOOD_MS: i64 = 1_000;

    /// A tiny dataset with `points` fixes at `cycle` from `source`.
    fn dataset(cycle: &str, source: &str, points: usize) -> NavData {
        let fixes: serde_json::Map<_, _> = (0..points)
            .map(|i| (format!("FIX{i}"), json!([[40.0, -75.0]])))
            .collect();
        let meta = json!({ "nasrCycleDate": cycle, "source": source });
        NavData::from_json(
            "{}",
            &serde_json::Value::Object(fixes).to_string(),
            "{}",
            "{}",
            "{}",
            &meta.to_string(),
            "{}",
        )
    }

    /// Apply `fresh` over loaded `(cycle, source, points)` with 2026-09-03 current; returns the
    /// result, the dataset left loaded, and the refresh timestamp afterwards.
    fn apply(
        loaded: (&str, &str, usize),
        fresh: NavData,
    ) -> (Result<bool, String>, Arc<NavData>, i64) {
        let nav = ArcSwap::from_pointee(dataset(loaded.0, loaded.1, loaded.2));
        let refreshed = AtomicI64::new(LAST_GOOD_MS);
        let current = NaiveDate::from_ymd_opt(2026, 9, 3).unwrap();
        let result = apply_nav_refresh(&nav, &refreshed, fresh, current);
        (result, nav.load_full(), refreshed.load(Ordering::Relaxed))
    }

    #[test]
    fn a_healthy_refresh_swaps_in_and_records_the_refresh_time() {
        let (result, loaded, refreshed) = apply(
            ("2026-08-06", FAA_SOURCE, 2),
            dataset("2026-09-03", FAA_SOURCE, 3),
        );
        assert_eq!(result, Ok(true));
        assert_eq!(loaded.cycle(), "2026-09-03");
        assert!(refreshed > LAST_GOOD_MS);
    }

    #[test]
    fn an_already_current_healthy_refresh_is_ok_without_a_swap() {
        let (result, _, refreshed) = apply(
            ("2026-09-03", FAA_SOURCE, 3),
            dataset("2026-09-03", FAA_SOURCE, 3),
        );
        assert_eq!(result, Ok(false));
        assert!(refreshed > LAST_GOOD_MS);
    }

    #[test]
    fn an_older_bundle_fallback_fails_and_keeps_the_live_data_and_refresh_time() {
        // The production incident: the fetch fell back to the 2026-07-09 bundle.
        let (result, loaded, refreshed) = apply(
            ("2026-09-03", FAA_SOURCE, 3),
            dataset("2026-07-09", BUNDLE, 5),
        );
        let detail = result.unwrap_err();
        assert!(detail.contains("2 cycle(s) behind"), "{detail}");
        assert_eq!(
            (loaded.cycle(), loaded.source()),
            ("2026-09-03", FAA_SOURCE)
        );
        assert_eq!(refreshed, LAST_GOOD_MS);
    }

    #[test]
    fn a_newer_fallback_swaps_in_but_is_still_degraded() {
        let (result, loaded, refreshed) = apply(
            ("2026-07-09", BUNDLE, 5),
            dataset("2026-09-03", "runtime fetch (squawk)", 3),
        );
        assert!(result.unwrap_err().contains("squawk"));
        assert_eq!(loaded.cycle(), "2026-09-03");
        assert_eq!(refreshed, LAST_GOOD_MS);
    }

    #[test]
    fn an_empty_fetch_fails_and_changes_nothing() {
        let (result, loaded, refreshed) = apply(
            ("2026-09-03", FAA_SOURCE, 3),
            dataset("2026-09-03", FAA_SOURCE, 0),
        );
        assert!(result.is_err());
        assert_eq!(loaded.len(), 3);
        assert_eq!(refreshed, LAST_GOOD_MS);
    }

    #[test]
    fn a_newer_or_same_cycle_swaps_but_an_older_fallback_never_does() {
        assert!(should_swap_nav(
            ("2026-09-03", 90_000),
            ("2026-08-06", 90_000)
        ));
        assert!(should_swap_nav(
            ("2026-09-03", 91_000),
            ("2026-09-03", 90_000)
        ));
        assert!(!should_swap_nav(
            ("2026-09-03", 90_000),
            ("2026-09-03", 90_000)
        ));
        // A failed fetch's bundle (older cycle) must not replace a live cycle.
        assert!(!should_swap_nav(
            ("2026-07-09", 80_000),
            ("2026-09-03", 90_000)
        ));
    }

    #[test]
    fn only_a_current_faa_cycle_is_healthy() {
        assert!(nav_health(FAA_SOURCE, "2026-09-03", Some(0), "2026-09-03").is_ok());
    }

    #[test]
    fn a_fallback_source_at_the_current_cycle_is_degraded() {
        let (detail, behind) = nav_health(
            "runtime fetch (squawk)",
            "2026-09-03",
            Some(0),
            "2026-09-03",
        )
        .unwrap_err();
        assert_eq!(behind, Some(0));
        assert!(detail.contains("squawk"), "{detail}");
    }

    #[test]
    fn a_behind_cycle_is_degraded_even_from_faa() {
        // The production incident: the bundle's 2026-07-09 served while 2026-09-03 was current.
        let (detail, behind) =
            nav_health(FAA_SOURCE, "2026-07-09", Some(2), "2026-09-03").unwrap_err();
        assert_eq!(behind, Some(2));
        assert!(
            detail.contains("2 cycle(s) behind current 2026-09-03"),
            "{detail}"
        );
    }

    #[test]
    fn an_unreadable_cycle_is_degraded() {
        assert_eq!(
            nav_health(FAA_SOURCE, "?", None, "2026-09-03")
                .unwrap_err()
                .1,
            None
        );
    }
}

#[cfg(test)]
mod ace_reminder_tests {
    use sqlx::PgPool;

    use super::ace_reminder_scheduler_once;
    use crate::realtime::topic;
    use crate::repos::ace as ace_repo;

    /// A claim coming due for someone with no linked Discord account still nudges connected clients,
    /// so the desktop app can remind them. Keying the nudge on Discord DMs sent meant it never fired
    /// for them at all (VATUSA/OIS#348 review).
    #[sqlx::test]
    async fn a_claim_due_without_discord_still_nudges_clients(pool: PgPool) {
        let user = |name: &'static str| {
            let pool = pool.clone();
            async move {
                sqlx::query_scalar::<_, String>(
                    "insert into identity.users (full_name, display_name) values ($1, $1) returning id",
                )
                .bind(name)
                .fetch_one(&pool)
                .await
                .unwrap()
            }
        };
        let requester = user("Requester").await;
        let claimer = user("No Discord").await;
        sqlx::query(
            "insert into events.event (id, title, start_time, end_time) \
             values (9, 'Fly-In', now() + interval '5 hours', now() + interval '7 hours')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let mut tx = pool.begin().await.unwrap();
        let request = ace_repo::create_request(&mut tx, 9, &requester, Some("ZDC"), None, 1, "")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let mut tx = pool.begin().await.unwrap();
        ace_repo::claim_request(&mut tx, &request, &claimer, "", None, None)
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let (events, mut received) = tokio::sync::broadcast::channel(8);
        ace_reminder_scheduler_once(&pool, &events).await.unwrap();

        let event = received.try_recv().expect("a reminder nudge was published");
        assert_eq!(event.topic, topic::EVENT_REMINDER);
    }
}

#[cfg(test)]
mod capture_scheduler_tests {
    use chrono::{DateTime, Utc};
    use sqlx::PgPool;

    use super::{capture_scheduler_once, snapshot_event_movements};
    use crate::repos::stats as stats_repo;

    const EVENT: i64 = 800;

    /// Yesterday at `hour:minute`, not a fixed calendar date. This pass compares against
    /// `Utc::now()`, and the backfill is deliberately bounded to the last `DELAY_LEG_RETAIN_DAYS`, so
    /// a hard-coded date would silently drift out of both — first out of the retention window, and
    /// eventually the test would be asserting nothing at all.
    fn at(hour: u32, minute: u32) -> DateTime<Utc> {
        (Utc::now() - chrono::Duration::days(1))
            .date_naive()
            .and_hms_opt(hour, minute, 0)
            .expect("a valid time of day")
            .and_utc()
    }

    /// An event that ran 12:00–14:00 with the default ±30 min capture padding, so its capture window
    /// is 11:30–14:30. Both are in the past, so one scheduler pass closes the capture.
    async fn seed(pool: &PgPool, capture_status: &str) {
        sqlx::query(
            "insert into events.event (id, title, start_time, end_time) values ($1, 'Test', $2, $3)",
        )
        .bind(EVENT)
        .bind(at(12, 0))
        .bind(at(14, 0))
        .execute(pool)
        .await
        .unwrap();
        sqlx::query("insert into events.airport_rate (event_id, icao) values ($1, 'KJFK')")
            .bind(EVENT)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query(
            "insert into stats.event_capture (event_id, enabled, pre_minutes, post_minutes) \
             values ($1, true, 30, 30)",
        )
        .bind(EVENT)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into stats.capture (id, event_id, label, start_time, end_time, status) \
             values ('cap-sched', $1, 'Test', $2, $3, $4)",
        )
        .bind(EVENT)
        .bind(at(11, 30))
        .bind((capture_status == "saved").then(|| at(14, 30)))
        .bind(capture_status)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn leg(pool: &PgPool, kind: &str, cid: i32, end: DateTime<Utc>) {
        sqlx::query(
            "insert into stats.flight_leg \
             (kind, airport, callsign, cid, start_time, end_time, duration_sec) \
             values ($1, 'KJFK', $2, $3, $4, $5, 600)",
        )
        .bind(kind)
        .bind(format!("TEST{cid}"))
        .bind(cid)
        .bind(end - chrono::Duration::minutes(10))
        .bind(end)
        .execute(pool)
        .await
        .unwrap();
    }

    /// Seed one leg inside the event and one inside the padding on either side of it.
    async fn three_legs(pool: &PgPool) {
        leg(pool, "departure", 1, at(11, 45)).await; // padding, before the event
        leg(pool, "departure", 2, at(13, 0)).await; // inside the event
        leg(pool, "arrival", 3, at(14, 15)).await; // padding, after the event
    }

    /// A capture that closes on this tick is frozen on this tick, over the **event's** window rather
    /// than the capture's padded one.
    ///
    /// This is the seam the unit tests cannot reach: `stats_window_tests` writes a snapshot by hand
    /// with the window it then asserts, and the `repos::stats` cases are *handed* a window rather than
    /// choosing one. So passing the padded `window_start, window_end` — in scope at the call site —
    /// left all 538 tests green while silently reintroducing #433's AC 4 bug on the frozen path, the
    /// one that serves every event past leg retention (#433 review).
    #[sqlx::test]
    async fn the_scheduler_freezes_over_the_events_window_not_the_padded_capture(pool: PgPool) {
        seed(&pool, "open").await;
        three_legs(&pool).await;

        capture_scheduler_once(&pool).await.unwrap();

        let snap = stats_repo::event_movements_snapshot(&pool, EVENT)
            .await
            .unwrap()
            .expect("closing the capture froze the movements");
        assert_eq!(
            (snap.window_start, snap.window_end),
            (at(12, 0), at(14, 0)),
            "the event's own window, not 11:30–14:30"
        );
        assert_eq!(
            snap.rows[0].arrivals + snap.rows[0].departures,
            1,
            "only the movement inside the event window was frozen"
        );
    }

    /// The other half of the same pass: an event whose capture was *already* saved. It never enters
    /// the close arm again, so if freezing only happened there its counts would compute from legs and
    /// fall to zero at `DELAY_LEG_RETAIN_DAYS` — correct numbers that quietly disappear, which is the
    /// state every event on the board was in before this shipped (#433 review).
    #[sqlx::test]
    async fn an_event_that_closed_before_the_table_existed_is_still_frozen(pool: PgPool) {
        seed(&pool, "saved").await;
        three_legs(&pool).await;
        assert!(
            stats_repo::event_movements_snapshot(&pool, EVENT)
                .await
                .unwrap()
                .is_none(),
            "nothing frozen yet — this is the pre-deploy state"
        );

        capture_scheduler_once(&pool).await.unwrap();

        let snap = stats_repo::event_movements_snapshot(&pool, EVENT)
            .await
            .unwrap()
            .expect("the backfill froze it");
        assert_eq!((snap.window_start, snap.window_end), (at(12, 0), at(14, 0)));
        assert_eq!(snap.rows[0].arrivals + snap.rows[0].departures, 1);

        // Idempotent and self-limiting: the row it just wrote is what stops the event matching.
        assert_eq!(
            stats_repo::events_missing_movement_snapshot(&pool, super::DELAY_LEG_RETAIN_DAYS)
                .await
                .unwrap()
                .len(),
            0
        );
    }

    /// An event whose legs were already pruned recomputes to zero for that reason alone. Freezing
    /// that would make an artifact of retention permanent, because the row is what stops the backfill
    /// matching — so it is deliberately left unfrozen (#433 review).
    #[sqlx::test]
    async fn an_event_with_no_surviving_legs_is_not_frozen_at_zero(pool: PgPool) {
        seed(&pool, "saved").await;
        // The shape of a pruned event: the connections are still there (`stats.flight` is never
        // pruned) so the breakdown is *not* empty — it is a row of zero movements beside a real pilot
        // count, which is exactly what the page renders as "Movements 0 / Pilots 1".
        sqlx::query(
            "insert into stats.flight \
             (session_id, cid, callsign, logon_time, first_seen, last_seen, status, departure, arrival) \
             values (1, 1001, 'TEST1', $1, $1, $2, 'active', 'KJFK', 'KBOS')",
        )
        .bind(at(12, 10))
        .bind(at(13, 40))
        .execute(&pool)
        .await
        .unwrap();
        let rows =
            stats_repo::event_airport_breakdown(&pool, &["KJFK".to_string()], at(12, 0), at(14, 0))
                .await
                .unwrap();
        assert_eq!(rows.len(), 1, "a row exists, it just has no movements");
        assert_eq!(
            (rows[0].arrivals, rows[0].departures, rows[0].unique_pilots),
            (0, 0, 1)
        );

        assert!(
            !snapshot_event_movements(&pool, EVENT, at(12, 0), at(14, 0)).await,
            "it reports that it froze nothing, so the job summary stays honest"
        );
        assert!(
            stats_repo::event_movements_snapshot(&pool, EVENT)
                .await
                .unwrap()
                .is_none(),
            "a zero-movement breakdown must not be frozen — freezing it would make the artifact \
             permanent, since the row is what stops the backfill matching"
        );
    }
}

#[cfg(test)]
mod outbound_job_reaper_tests {
    use sqlx::PgPool;

    use super::{OUTBOUND_JOB_LEASE_TIMEOUT_MINS, outbound_job_reaper_once};

    async fn leased(pool: &PgPool, mins_ago: i64) -> String {
        sqlx::query_scalar::<_, String>(
            "insert into integration.outbound_jobs \
             (job_type, status, attempt_count, last_attempt_at) \
             values ('tmi_publish', 'in_progress', 1, now() - make_interval(mins => $1)) \
             returning id",
        )
        .bind(mins_ago as i32)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn status_of(pool: &PgPool, id: &str) -> String {
        sqlx::query_scalar::<_, String>(
            "select status from integration.outbound_jobs where id = $1",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// The pass applies `OUTBOUND_JOB_LEASE_TIMEOUT_MINS` rather than taking a cutoff, so this is what
    /// pins the constant: widen it and the stranded job stops being recovered, which is the bug back
    /// (#446 review).
    #[sqlx::test]
    async fn the_pass_reaps_past_the_lease_and_leaves_a_fresh_lease_alone(pool: PgPool) {
        let stranded = leased(&pool, OUTBOUND_JOB_LEASE_TIMEOUT_MINS + 5).await;
        let working = leased(&pool, 1).await;

        assert_eq!(
            outbound_job_reaper_once(&pool).await.unwrap(),
            "1 requeued",
            "the summary the admin Jobs page shows must count what it actually did"
        );

        assert_eq!(status_of(&pool, &stranded).await, "pending");
        assert_eq!(
            status_of(&pool, &working).await,
            "in_progress",
            "a job inside its lease is being worked on, not abandoned"
        );
    }

    /// Nothing to do is not a failure — the pass runs every CLEANUP_INTERVAL and almost always finds
    /// nothing.
    #[sqlx::test]
    async fn an_empty_queue_is_not_an_error(pool: PgPool) {
        assert_eq!(outbound_job_reaper_once(&pool).await.unwrap(), "0 requeued");
    }
}

#[cfg(test)]
mod registration_tests {
    //! Every background pass defined here has to actually be started in `lib.rs`, and until now
    //! nothing checked that.
    //!
    //! A job can be written, given a registry entry, unit-tested thoroughly, and simply never
    //! spawned — and then the behaviour it exists for is silently absent while the whole suite
    //! stays green. Deleting `spawn_outbound_job_reaper` from `lib.rs` left 536 passed / 0 failed.
    //! Three cards in a row (#433, #436, #446) were returned for a gap of exactly this shape, so
    //! this asserts the wiring itself rather than any one job: add a `spawn_*` and forget to start
    //! it, and this fails.

    const JOBS_RS: &str = include_str!("jobs.rs");
    const LIB_RS: &str = include_str!("lib.rs");

    /// Drop `//` comments, so prose naming a spawn is not mistaken for a call site — the false
    /// positive a source scan gets wrong first.
    fn without_line_comments(src: &str) -> String {
        src.lines()
            .map(|line| match line.find("//") {
                Some(i) => &line[..i],
                None => line,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn spawn_fn_names(src: &str) -> Vec<String> {
        without_line_comments(src)
            .lines()
            .filter_map(|line| line.trim().strip_prefix("pub fn spawn_"))
            .filter_map(|rest| rest.split(['(', '<']).next())
            .map(|name| format!("spawn_{name}"))
            .collect()
    }

    #[test]
    fn every_background_job_is_started_in_lib() {
        let names = spawn_fn_names(JOBS_RS);

        // Without this the test passes by checking nothing the moment the matcher stops matching,
        // which is the way a source scan rots.
        assert!(
            names.len() >= 10,
            "only found {} spawn fns in jobs.rs, so the matcher has stopped matching: {names:?}",
            names.len()
        );
        assert!(
            names.iter().any(|n| n == "spawn_outbound_job_reaper"),
            "the matcher no longer finds a spawn fn known to exist, so it is broken: {names:?}"
        );

        let lib = without_line_comments(LIB_RS);
        let missing: Vec<&str> = names
            .iter()
            .map(String::as_str)
            .filter(|name| !lib.contains(&format!("jobs::{name}(")))
            .collect();

        assert!(
            missing.is_empty(),
            "defined in jobs.rs but never started in lib.rs, so they silently never run: {missing:?}"
        );
    }
}
