//! Live VATSIM feed: a background poller keeps an in-memory snapshot of network
//! traffic (plus a cached airport-coordinate database) that the TMU flow endpoints
//! read to meter arrivals against rate programs.

pub mod airports;
pub mod airspace;
mod cifp;
pub mod coverage;
pub mod delays;
pub mod departure_runway;
pub mod events;
pub mod facilities;
pub mod fca;
pub mod flow;
pub mod forecast;
pub mod gdp;
pub mod metar;
pub mod nav;
mod nav_dns;
pub mod nav_source;
pub mod neighbors;
pub mod predict;
pub mod runway;
pub mod runway_db;
pub mod stats;
pub mod taxi;
pub mod taxi_estimate;
pub mod taxi_observations;
pub mod tracon;
pub mod trajectory;
pub mod vatsim;
pub mod vatusa;
pub mod winds;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::RwLock;

use airports::AirportDb;
use vatsim::VatsimData;

/// Poll ~10s after the source's last known `update_timestamp` — VATSIM refreshes ~15s, so this
/// lands just after the next publish instead of averaging half a fixed interval either side of it.
/// Left as-is rather than re-tuned: a one-off live sample against the real feed (see #92) showed
/// steady-state capture lag of ~23-25s, not the ~10-12s this targets — but that's a single
/// measurement, not a tuned constant. The `tracing::debug!` added in #92 is the way to gather real
/// production numbers before changing this.
const REFRESH_BUFFER_SECS: u64 = 10;
/// Fast retry cadence used when the refresh-buffer target has already passed (the fetch was late,
/// `update_timestamp` didn't parse, this is the very first tick, or a failure streak is in
/// progress) — catches the source's next publish quickly rather than waiting out a full interval.
const FAST_POLL_SECS: u64 = 2;
/// 30 fast-polls (~60s) of an unchanged `update_timestamp` after the refresh-buffer target has
/// passed means the source is presumed frozen (a stale-but-200-OK response), not mid-catch-up —
/// well past the ~23-25s publish latency observed live against the real feed (see #92), so normal
/// catch-up near the buffer boundary won't false-trigger this.
const MAX_CONSECUTIVE_STALE_POLLS: u32 = 30;
/// Backoff cadence once `MAX_CONSECUTIVE_STALE_POLLS` is hit — a 15x cut from the 2s fast-poll
/// while still checking back often enough to recover quickly once the source actually advances.
const STALE_POLL_BACKOFF_SECS: u64 = 30;
/// A single failed fetch must not flip `healthy` — transient connect/timeout blips on
/// data.vatsim.net are normal and self-heal on the next tick. Only N in a row means the feed is
/// actually behind.
const MAX_CONSECUTIVE_FAILURES: u32 = 3;

pub struct Snapshot {
    pub fetched_at: DateTime<Utc>,
    pub source_timestamp: String,
    pub data: VatsimData,
}

impl Snapshot {
    /// Wrap a `VatsimData` in a bare snapshot — for the historical replay (reconstructed data)
    /// and empty-feed fallbacks, where `fetched_at` / `source_timestamp` carry no meaning.
    pub fn of(data: VatsimData) -> Self {
        Self {
            fetched_at: Utc::now(),
            source_timestamp: String::new(),
            data,
        }
    }
}

#[derive(Clone, Default)]
pub struct FeedStatus {
    pub healthy: bool,
    pub last_ok: Option<DateTime<Utc>>,
    pub source_timestamp: Option<String>,
    pub last_error: Option<String>,
    pub pilots: usize,
    pub prefiles: usize,
    pub airports_loaded: usize,
}

#[derive(Default)]
pub struct FeedInner {
    /// Behind `Arc` so read handlers can clone it and drop the feed lock before doing the
    /// heavy per-request CPU (route resolution / metering), instead of holding the read
    /// guard across it and stalling the poller's writes.
    pub snapshot: Option<Arc<Snapshot>>,
    pub airports: Arc<AirportDb>,
    /// IATA → ICAO, for resolving US-style ATC callsign prefixes (`SFO_TWR` → `KSFO`).
    pub iata: Arc<airports::IataMap>,
    pub status: FeedStatus,
    /// Departures currently being timed (callsign -> session).
    pub taxi_sessions: HashMap<String, taxi::TaxiSession>,
    /// Completed taxi samples per airport (rolling 3h).
    pub taxi_samples: HashMap<String, Vec<taxi::TaxiSample>>,
}

/// Shared, cheaply-cloneable handle to the feed state.
pub type FeedState = Arc<RwLock<FeedInner>>;

pub fn new_state() -> FeedState {
    Arc::new(RwLock::new(FeedInner::default()))
}

/// Spawn the background poller. Safe to call once at startup; it phase-locks to the source's own
/// refresh cadence (see `next_poll_delay`). The airport coordinate database is loaded separately
/// by `jobs::spawn_airports_refresh` (#216), not by this poller.
/// `events` is the realtime hub: the poller publishes [`realtime::topic::FEED_TICK`] once per
/// upstream publish (#648), so live screens refetch on new data instead of on their own timers.
pub fn spawn_poller(state: FeedState, events: crate::realtime::Events) {
    tokio::spawn(async move { poller(state, events).await });
}

/// After a successful fetch with the source's `update_timestamp` (`None` before the first ever
/// fetch), when to poll again: shortly after the source is expected to have refreshed
/// (`update_timestamp + REFRESH_BUFFER_SECS`), or immediately at the fast-poll cadence if that
/// target has already passed — this fetch was itself already late, `update_timestamp` failed to
/// parse, there's no prior timestamp yet, or a failure streak is in progress (the caller leaves
/// `last_source_ts` unchanged on failure, so this naturally retries fast during an outage instead
/// of waiting out a full interval). Once `consecutive_stale_polls` reaches
/// `MAX_CONSECUTIVE_STALE_POLLS` — the source has kept responding `Ok` but its own timestamp hasn't
/// moved — back off to `STALE_POLL_BACKOFF_SECS` instead of fast-polling forever (see #92).
fn next_poll_delay(
    last_source_ts: Option<DateTime<Utc>>,
    consecutive_stale_polls: u32,
    now: DateTime<Utc>,
) -> Duration {
    let fast = Duration::from_secs(FAST_POLL_SECS);
    let Some(ts) = last_source_ts else {
        return Duration::ZERO;
    };
    let target = ts + chrono::Duration::seconds(REFRESH_BUFFER_SECS as i64);
    if target > now {
        (target - now).to_std().unwrap_or(fast)
    } else if consecutive_stale_polls >= MAX_CONSECUTIVE_STALE_POLLS {
        Duration::from_secs(STALE_POLL_BACKOFF_SECS)
    } else {
        fast
    }
}

/// Whether a successful fetch's parsed timestamp should extend the stale-poll streak: either it
/// didn't parse at all, or it parsed to the same value already on record — neither is new
/// information, so both count the same toward `MAX_CONSECUTIVE_STALE_POLLS` (see #92). A parse
/// failure must count as stale too, not reset the streak — otherwise a persistently malformed
/// `update_timestamp` (fetch `Ok`, field unparseable every time) can sustain the 2s fast-poll
/// forever off a `last_source_ts` that's frozen at its last known-good value, exactly the failure
/// mode this streak exists to bound.
fn is_stale_poll(parsed_ts: Option<DateTime<Utc>>, last_source_ts: Option<DateTime<Utc>>) -> bool {
    parsed_ts.is_none() || parsed_ts == last_source_ts
}

/// Whether a successful fetch is a new upstream publish, worth a [`crate::realtime::topic::FEED_TICK`]
/// (#648). Exactly the complement of [`is_stale_poll`], judged against `last_source_ts` **before**
/// it advances — so "new data" means the same thing to the tick as to the back-off logic, and a
/// stale or unparseable timestamp ticks no client.
fn ticks(parsed_ts: Option<DateTime<Utc>>, last_source_ts: Option<DateTime<Utc>>) -> bool {
    !is_stale_poll(parsed_ts, last_source_ts)
}

/// Whether `consecutive_failures` should flip `healthy` false — a single transient error must not;
/// only `MAX_CONSECUTIVE_FAILURES` in a row means the feed is actually behind.
fn should_mark_unhealthy(consecutive_failures: u32) -> bool {
    consecutive_failures >= MAX_CONSECUTIVE_FAILURES
}

/// One successful fetch: track staleness, install the snapshot, and — only if the source published
/// new data — tell live clients (#648). Split out of [`poller`] so the tick is tested where it is
/// wired, not just as a rule.
async fn apply_fetch(
    state: &FeedState,
    events: &crate::realtime::Events,
    data: VatsimData,
    now: DateTime<Utc>,
    delay: Duration,
    last_source_ts: &mut Option<DateTime<Utc>>,
    consecutive_stale_polls: &mut u32,
) {
    let pilots = data.pilots.len();
    let prefiles = data.prefiles.len();
    let source_timestamp = data.general.update_timestamp.clone();
    let parsed_ts = DateTime::parse_from_rfc3339(&source_timestamp)
        .map(|dt| dt.with_timezone(&Utc))
        .ok();
    let tick = ticks(parsed_ts, *last_source_ts);
    *consecutive_stale_polls = if is_stale_poll(parsed_ts, *last_source_ts) {
        *consecutive_stale_polls + 1
    } else {
        0
    };
    if *consecutive_stale_polls == MAX_CONSECUTIVE_STALE_POLLS {
        tracing::warn!(
            consecutive_stale_polls,
            "feed: source timestamp hasn't advanced in a while, backing off"
        );
    }
    tracing::debug!(
        prior_poll_delay_secs = delay.as_secs(),
        source_timestamp = %source_timestamp,
        snapshot_age_secs = parsed_ts.map(|ts| (now - ts).num_seconds()),
        "feed: poll succeeded"
    );
    *last_source_ts = parsed_ts.or(*last_source_ts);
    let mut guard = state.write().await;
    guard.status.healthy = true;
    guard.status.last_ok = Some(now);
    guard.status.source_timestamp = Some(source_timestamp.clone());
    guard.status.last_error = None;
    guard.status.pilots = pilots;
    guard.status.prefiles = prefiles;
    // Advance the taxi state machine before the data is moved into the snapshot.
    let FeedInner {
        taxi_sessions,
        taxi_samples,
        airports,
        ..
    } = &mut *guard;
    taxi::process(taxi_sessions, taxi_samples, airports, &data, now);
    guard.snapshot = Some(Arc::new(Snapshot {
        fetched_at: now,
        source_timestamp,
        data,
    }));
    drop(guard);

    // After the snapshot is in place and the lock released, so a client refetching on the tick reads
    // the new data. One in-process hub: a second replica would need #649 first.
    if tick {
        let _ = events.send(crate::realtime::WsEvent {
            topic: crate::realtime::topic::FEED_TICK.to_string(),
        });
    }
}

async fn poller(state: FeedState, events: crate::realtime::Events) {
    let client = match reqwest::Client::builder()
        .user_agent("ois-backend/0.1 (+https://vatusa.net)")
        .timeout(Duration::from_secs(20))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "feed poller: failed to build HTTP client");
            return;
        }
    };

    // The airport coordinate database is loaded by its own retrying job
    // (`jobs::spawn_airports_refresh`, #216) — a one-shot fetch here left `airports` permanently
    // empty on any transient boot failure, with no retry.

    let mut last_source_ts: Option<DateTime<Utc>> = None;
    let mut consecutive_failures: u32 = 0;
    let mut consecutive_stale_polls: u32 = 0;
    loop {
        let delay = next_poll_delay(last_source_ts, consecutive_stale_polls, Utc::now());
        tokio::time::sleep(delay).await;
        match vatsim::fetch(&client).await {
            Ok(data) => {
                consecutive_failures = 0;
                apply_fetch(
                    &state,
                    &events,
                    data,
                    Utc::now(),
                    delay,
                    &mut last_source_ts,
                    &mut consecutive_stale_polls,
                )
                .await;
            }
            Err(e) => {
                consecutive_failures += 1;
                // An outright failure isn't a "stale but successful" read — reset so a failure
                // streak always retries at the fast cadence (below), even right after a stale
                // freeze had already backed off to STALE_POLL_BACKOFF_SECS.
                consecutive_stale_polls = 0;
                tracing::warn!(error = %e, consecutive_failures, "feed: vatsim fetch failed");
                // last_source_ts is left as-is: next_poll_delay sees its buffer target already
                // passed and retries at the fast cadence instead of waiting out a full interval.
                if should_mark_unhealthy(consecutive_failures) {
                    let mut guard = state.write().await;
                    guard.status.healthy = false;
                    guard.status.last_error = Some(e.to_string());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- #648: the feed tells live clients when it has new data ------------------------------------

    fn ts(s: &str) -> Option<DateTime<Utc>> {
        Some(DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc))
    }

    #[test]
    fn a_new_upstream_publish_ticks_and_nothing_else_does() {
        let earlier = ts("2026-10-04T00:00:00Z");
        let later = ts("2026-10-04T00:00:15Z");
        assert!(ticks(later, earlier), "advanced");
        assert!(ticks(earlier, None), "the very first fetch is new data");
        assert!(!ticks(earlier, earlier), "unchanged is stale");
        assert!(!ticks(None, earlier), "unparseable is stale");
        assert!(!ticks(None, None), "unparseable first fetch is stale");
        // An older timestamp is not `==` the last one, so it counts as new — as the back-off logic
        // already treats it. The tick follows that rule rather than inventing a second one.
        assert_eq!(ticks(earlier, later), !is_stale_poll(earlier, later));
    }

    fn fetched(update_timestamp: &str) -> VatsimData {
        VatsimData {
            general: vatsim::General {
                update_timestamp: update_timestamp.to_string(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Through the real success path: one tick per new publish, none for a repeat or a garbage
    /// timestamp — and by the time a client hears the tick, the new snapshot is what it will read.
    #[tokio::test]
    async fn the_poller_publishes_one_tick_per_new_publish_after_installing_it() {
        let state = new_state();
        let (events, mut rx) = tokio::sync::broadcast::channel(16);
        let (mut last, mut stale) = (None, 0u32);

        apply_fetch(
            &state,
            &events,
            fetched("2026-10-04T00:00:00Z"),
            Utc::now(),
            Duration::ZERO,
            &mut last,
            &mut stale,
        )
        .await;
        let tick = rx.try_recv().expect("the first publish ticks");
        assert_eq!(tick.topic, crate::realtime::topic::FEED_TICK);
        assert_eq!(
            state
                .read()
                .await
                .snapshot
                .as_ref()
                .unwrap()
                .source_timestamp,
            "2026-10-04T00:00:00Z",
            "the snapshot a client refetches is already the new one"
        );

        apply_fetch(
            &state,
            &events,
            fetched("2026-10-04T00:00:00Z"),
            Utc::now(),
            Duration::ZERO,
            &mut last,
            &mut stale,
        )
        .await;
        assert!(
            rx.try_recv().is_err(),
            "a repeat of the same publish ticks nobody"
        );

        apply_fetch(
            &state,
            &events,
            fetched("not a timestamp"),
            Utc::now(),
            Duration::ZERO,
            &mut last,
            &mut stale,
        )
        .await;
        assert!(
            rx.try_recv().is_err(),
            "an unparseable timestamp ticks nobody"
        );

        apply_fetch(
            &state,
            &events,
            fetched("2026-10-04T00:00:15Z"),
            Utc::now(),
            Duration::ZERO,
            &mut last,
            &mut stale,
        )
        .await;
        assert!(rx.try_recv().is_ok(), "the next publish ticks again");
        assert!(rx.try_recv().is_err(), "exactly once");
    }

    #[test]
    fn first_tick_polls_immediately() {
        assert_eq!(next_poll_delay(None, 0, Utc::now()), Duration::ZERO);
    }

    #[test]
    fn waits_until_the_refresh_buffer_when_the_source_just_updated() {
        let now = Utc::now();
        let delay = next_poll_delay(Some(now), 0, now);
        // Buffer hasn't elapsed yet: wait roughly the remaining buffer, not the fast cadence.
        assert_eq!(delay, Duration::from_secs(REFRESH_BUFFER_SECS));
    }

    #[test]
    fn waits_less_as_the_buffer_elapses() {
        let now = Utc::now();
        let ts = now - chrono::Duration::seconds(4);
        let delay = next_poll_delay(Some(ts), 0, now);
        assert_eq!(delay, Duration::from_secs(REFRESH_BUFFER_SECS - 4));
    }

    #[test]
    fn fast_polls_once_the_buffer_target_has_passed() {
        let now = Utc::now();
        // A timestamp from well before now: the buffer target is already behind us.
        let stale = now - chrono::Duration::seconds(REFRESH_BUFFER_SECS as i64 + 30);
        assert_eq!(
            next_poll_delay(Some(stale), 0, now),
            Duration::from_secs(FAST_POLL_SECS)
        );
    }

    #[test]
    fn backs_off_once_the_fast_poll_streak_is_stale_for_too_long() {
        let now = Utc::now();
        let stale = now - chrono::Duration::seconds(REFRESH_BUFFER_SECS as i64 + 30);
        // Still within the normal catch-up window: fast-poll as usual.
        assert_eq!(
            next_poll_delay(Some(stale), MAX_CONSECUTIVE_STALE_POLLS - 1, now),
            Duration::from_secs(FAST_POLL_SECS)
        );
        // The source has kept responding Ok with this same stale timestamp for too long: a
        // frozen-but-successful feed can't sustain fast-poll forever (see #92).
        assert_eq!(
            next_poll_delay(Some(stale), MAX_CONSECUTIVE_STALE_POLLS, now),
            Duration::from_secs(STALE_POLL_BACKOFF_SECS)
        );
    }

    #[test]
    fn unchanged_timestamp_is_stale() {
        let ts = Utc::now();
        assert!(is_stale_poll(Some(ts), Some(ts)));
    }

    #[test]
    fn advanced_timestamp_is_not_stale() {
        let now = Utc::now();
        let later = now + chrono::Duration::seconds(15);
        assert!(!is_stale_poll(Some(later), Some(now)));
    }

    #[test]
    fn unparseable_timestamp_is_stale_even_with_a_prior_good_one() {
        // The bug this closes: a persistently malformed `update_timestamp` (fetch `Ok`, field
        // unparseable every time) must extend the streak, not reset it — otherwise it can sustain
        // the fast-poll forever off a `last_source_ts` frozen at its last known-good value, same as
        // an unchanged-but-parseable timestamp would.
        let last_good = Utc::now();
        assert!(is_stale_poll(None, Some(last_good)));
    }

    #[test]
    fn first_ever_fetch_is_not_stale() {
        let ts = Utc::now();
        assert!(!is_stale_poll(Some(ts), None));
    }

    #[test]
    fn a_single_failure_does_not_mark_unhealthy() {
        assert!(!should_mark_unhealthy(1));
        assert!(!should_mark_unhealthy(MAX_CONSECUTIVE_FAILURES - 1));
    }

    #[test]
    fn n_consecutive_failures_marks_unhealthy() {
        assert!(should_mark_unhealthy(MAX_CONSECUTIVE_FAILURES));
        assert!(should_mark_unhealthy(MAX_CONSECUTIVE_FAILURES + 1));
    }
}
