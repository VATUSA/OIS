//! Prometheus exposition for the backend (#382).
//!
//! Three parts:
//!
//! * [`track_http`] — an Axum layer that counts every request and times it, labelled by the
//!   *route template* and an allowlisted *method* rather than by anything the caller chooses, so
//!   neither ids nor invented HTTP verbs can fan the label set out. This is the only metric
//!   written continuously.
//! * [`spawn_upkeep`] — drains the recorder on a timer. Not optional: the observability stack is
//!   opt-in, so the default deployment never scrapes, and an unscraped recorder retains every
//!   latency sample it was ever given.
//! * [`observe`] — everything else. The process already knows its own state (the feed snapshot,
//!   the freshness stamps, the job registry, the DB pool), so rather than maintaining a parallel
//!   copy on every write we simply project that state into gauges at scrape time. Reads are
//!   cheap and non-blocking; nothing here queries the database (the one aggregate that needs it
//!   is fetched by the handler and passed in as [`DomainCounts`]).
//!
//! The registry itself is process-wide because `metrics`' facade macros write to a global
//! recorder. [`handle`] installs it exactly once and hands back a render handle, which
//! `AppState` carries so the handler reads it from state rather than reaching for a global.

use std::sync::OnceLock;
use std::sync::atomic::Ordering;
use std::time::Instant;

use axum::{
    extract::MatchedPath, extract::Request, http::Method, middleware::Next, response::Response,
};
use metrics::{counter, gauge, histogram};
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};

use crate::state::AppState;

/// Latency buckets for `http_request_duration_seconds`, 5ms → 10s. Chosen to straddle what this
/// API actually does: cache-backed flow reads land in the first few buckets, DB-backed admin
/// reads in the middle, and the upstream-fetching refresh endpoints in the tail.
const LATENCY_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

/// Label used when a request never matched a route (404s, scanner traffic). Without this the raw
/// path would become the label and a scanner could mint unbounded series.
const UNMATCHED_ROUTE: &str = "unmatched";

/// Label used for any method outside [`method_label`]'s allowlist.
const OTHER_METHOD: &str = "other";

/// How often [`spawn_upkeep`] drains the recorder when nothing is scraping it.
const UPKEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
static STARTED: OnceLock<Instant> = OnceLock::new();

/// Install the process-wide Prometheus recorder (idempotent) and return its render handle.
///
/// Installation failure is deliberately ignored rather than panicking: the only way it fails is
/// another recorder already being registered, and a process that cannot export metrics should
/// still serve traffic. The returned handle always renders whatever *this* recorder holds.
///
/// Upkeep is **not** optional here — see [`spawn_upkeep`]. An earlier version of this comment
/// argued a background task was unnecessary because `render()` drains histograms on every scrape.
/// That only holds if something scrapes: the observability stack is opt-in, so in the default
/// deployment `GET /metrics` is never called, nothing drains, and every recorded sample is retained
/// for the life of the process.
pub fn handle() -> PrometheusHandle {
    HANDLE
        .get_or_init(|| {
            let recorder = PrometheusBuilder::new()
                .set_buckets_for_metric(
                    Matcher::Full("http_request_duration_seconds".to_string()),
                    LATENCY_BUCKETS,
                )
                .expect("latency buckets are a non-empty constant")
                .build_recorder();
            let handle = recorder.handle();
            if metrics::set_global_recorder(recorder).is_err() {
                tracing::warn!("a metrics recorder was already installed; /metrics may be empty");
            }
            STARTED.get_or_init(Instant::now);
            handle
        })
        .clone()
}

/// Drain the recorder on a timer, independently of whether anyone scrapes.
///
/// `track_http` records a latency sample per request, and those samples sit in the recorder's
/// buckets until something drains them into distributions. Only `render()` (a scrape) and
/// `run_upkeep()` do that. Because the observability stack is opt-in, the default deployment never
/// scrapes — so without this task memory grows linearly with request count and is never released.
/// It also covers the half-configured case `docs/deploy.md` warns about: with `METRICS_TOKEN` set
/// but Prometheus not yet given the credentials file, the handler answers 401 *before* it renders,
/// so a scraping Prometheus still drains nothing.
///
/// A minute is far finer than needed to bound the footprint and costs nothing when idle.
pub fn spawn_upkeep(handle: PrometheusHandle) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(UPKEEP_INTERVAL);
        // The first tick fires immediately; skip it so startup does no redundant work.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            handle.run_upkeep();
            // A drain leaves no other trace, so the loop counts itself. Without this the one task
            // standing between the default deployment and unbounded growth would be invisible —
            // and untestable. Alert on it going flat.
            counter!("ois_metrics_upkeep_total").increment(1);
        }
    });
}

/// Coarse method label, restricted to a fixed allowlist.
///
/// The same cardinality guarantee as [`UNMATCHED_ROUTE`], for the other label. HTTP methods are
/// *tokens*, not a closed set — hyper accepts any token as an extension method, and axum still
/// matches the path before the `MethodRouter` rejects it, so `curl -X ZZZQ1 /health` would otherwise
/// mint `method="ZZZQ1"` on a real route template (a counter plus a whole latency histogram). No
/// `idle_timeout` is configured, so those series would live for the life of the process. Anything
/// unrecognised therefore collapses onto one label.
fn method_label(method: &Method) -> &'static str {
    match *method {
        Method::GET => "GET",
        Method::POST => "POST",
        Method::PUT => "PUT",
        Method::PATCH => "PATCH",
        Method::DELETE => "DELETE",
        Method::HEAD => "HEAD",
        Method::OPTIONS => "OPTIONS",
        Method::TRACE => "TRACE",
        Method::CONNECT => "CONNECT",
        _ => OTHER_METHOD,
    }
}

/// Coarse status label. A class rather than the exact code: `status="5xx"` is what alerts fire on,
/// and it keeps the series count at five per route/method instead of one per status code.
fn status_class(status: u16) -> &'static str {
    match status {
        100..=199 => "1xx",
        200..=299 => "2xx",
        300..=399 => "3xx",
        400..=499 => "4xx",
        _ => "5xx",
    }
}

/// Count + time every request, labelled by route template, method and status class (AC2).
///
/// Sits outside `resolve_current_user` and inside CORS, so the recorded latency covers auth
/// resolution, the audit layer and the handler — i.e. everything the client actually waits for.
pub async fn track_http(request: Request, next: Next) -> Response {
    // `MatchedPath` is the registered template ("/api/v1/flow/fca/{id}"), not the concrete path,
    // so ids never reach the label set.
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| UNMATCHED_ROUTE.to_string());
    let method = method_label(request.method());

    let started = Instant::now();
    let response = next.run(request).await;
    let elapsed = started.elapsed().as_secs_f64();
    let status = status_class(response.status().as_u16());

    counter!(
        "http_requests_total",
        "route" => route.clone(),
        "method" => method,
        "status" => status,
    )
    .increment(1);
    histogram!(
        "http_request_duration_seconds",
        "route" => route,
        "method" => method,
    )
    .record(elapsed);

    response
}

/// Whether a scrape may proceed (AC4).
///
/// No token configured (`AppState::metrics_token`, from `METRICS_TOKEN`) means the endpoint is
/// open, which is the intended default: the observability stack scrapes it in-network and
/// publishes no port for it. It does still ride the API's own listener, so set a token wherever
/// the API is publicly proxied. Once a token *is* configured it is mandatory, and only an exact
/// `Bearer <token>` passes.
pub fn scrape_authorized(expected: Option<&str>, authorization: Option<&str>) -> bool {
    let Some(expected) = expected.map(str::trim).filter(|t| !t.is_empty()) else {
        return true;
    };
    authorization
        .and_then(|h| h.strip_prefix("Bearer "))
        .is_some_and(|presented| constant_time_eq(presented.as_bytes(), expected.as_bytes()))
}

/// Compare two secrets without an early return.
///
/// `==` on `&str` short-circuits at the first differing byte, which leaks a matching-prefix timing
/// signal (and the length) for the one credential this module owns. The accumulate-then-compare
/// form below always touches every byte of `expected`. Length still differs in cost — unavoidable
/// without hashing — so the length check is folded into the accumulator rather than returned early.
fn constant_time_eq(presented: &[u8], expected: &[u8]) -> bool {
    // A bool, not the XOR of the two lengths: `len_a ^ len_b` narrowed to a byte accumulator would
    // read as "equal" whenever the lengths happened to differ by a multiple of 256.
    let mut diff = u32::from(presented.len() != expected.len());
    for (i, e) in expected.iter().enumerate() {
        // The loop runs over `expected`, so its length depends only on the configured token and
        // never on what the caller sent. A short `presented` pads with 0 rather than returning.
        let p = presented.get(i).copied().unwrap_or(0);
        diff |= u32::from(p ^ *e);
    }
    diff == 0
}

/// The one aggregate that cannot be answered from memory — see `repos::metrics`.
#[derive(Debug, Default, Clone, Copy, sqlx::FromRow)]
pub struct DomainCounts {
    pub active_tmis: i64,
    pub active_ground_stops: i64,
    pub active_gdps: i64,
    pub active_programs: i64,
    pub enabled_fcas: i64,
    pub gdp_delay_minutes: i64,
}

/// Project the live process state into gauges, immediately before rendering.
///
/// `domain` is `None` when there is no database or the aggregate failed; the DB-derived gauges are
/// then left at their previous values rather than being zeroed, so a blip reads as "stale" rather
/// than as "every TMI was just cancelled".
pub async fn observe(state: &AppState, domain: Option<DomainCounts>) {
    gauge!("ois_build_info", "version" => crate::VERSION).set(1.0);
    let uptime = STARTED
        .get()
        .map(|s| s.elapsed().as_secs_f64())
        .unwrap_or(0.0);
    gauge!("ois_uptime_seconds").set(uptime);

    observe_db_pool(state);
    observe_jobs(state);
    observe_freshness(state);
    observe_feed(state).await;

    // Registered every scrape so the series exists from the first one: a panel reading "No data"
    // is indistinguishable from a healthy zero, and this is how a permanently-broken aggregate
    // (which otherwise just freezes the six gauges at plausible values) becomes visible.
    counter!("ois_metrics_domain_aggregate_failures_total").increment(0);
    // Registered here too, not only from inside the drain loop. The loop creates it on its first
    // tick, so a task that never spawned — the very failure "alert on it going flat" is meant to
    // catch — left the series absent instead of flat, and an alert on an absent series never fires.
    //
    // Not unit-tested: the recorder is process-wide, so `the_upkeep_task_drains_on_its_timer_…`
    // creates this series too and an "is it absent?" assertion would depend on test order. Checked
    // instead against a running backend, scraped inside the first minute.
    counter!("ois_metrics_upkeep_total").increment(0);

    if let Some(d) = domain {
        gauge!("ois_active_tmis").set(d.active_tmis as f64);
        gauge!("ois_active_ground_stops").set(d.active_ground_stops as f64);
        gauge!("ois_active_gdps").set(d.active_gdps as f64);
        gauge!("ois_active_programs").set(d.active_programs as f64);
        gauge!("ois_fca_enabled").set(d.enabled_fcas as f64);
        // Not `_total`: that suffix is reserved for counters, and this is a *current* sum of
        // assigned delay across live GDPs, which falls as programs end.
        gauge!("ois_gdp_delay_minutes").set(d.gdp_delay_minutes as f64);
    }
}

fn observe_db_pool(state: &AppState) {
    let Some(pool) = state.db.as_ref() else {
        return;
    };
    let size = pool.size();
    let idle = pool.num_idle();
    gauge!("ois_db_pool_size").set(size as f64);
    gauge!("ois_db_pool_connections", "state" => "idle").set(idle as f64);
    // `size` counts every connection the pool owns, idle or checked out, so in-use is the
    // difference. Saturated via `saturating_sub` because the two reads are not atomic together.
    gauge!("ois_db_pool_connections", "state" => "in_use")
        .set((size as usize).saturating_sub(idle) as f64);
}

fn observe_jobs(state: &AppState) {
    for job in state.jobs.snapshot() {
        let name = job.name.clone();
        // `last_finished_ms == 0` means "never finished" (see `JobRegistry`), not "finished at the
        // epoch". Emitting the zero made the dashboard's `time() - ois_job_last_run_timestamp_seconds`
        // panel read ~57 years for every job that had not ticked since boot — which, for a daily
        // job like compaction, is most of the day after every restart, and trips any
        // "last run older than N" alert on all of them at once. Leaving the series out says
        // "unknown", which is what Prometheus takes an absent sample to mean.
        if job.last_finished_ms > 0 {
            gauge!("ois_job_last_run_timestamp_seconds", "job" => name.clone())
                .set(job.last_finished_ms as f64 / 1000.0);
        }
        // Again not `_total` — this is a gauge read off the registry, not a counter we own.
        gauge!("ois_job_runs", "job" => name.clone()).set(job.runs as f64);
        // `last_ok` is `None` until the job has finished once; report that as a failure so a job
        // that has never completed is as visible to an alert as one that completed badly.
        gauge!("ois_job_last_success", "job" => name.clone()).set(if job.last_ok == Some(true) {
            1.0
        } else {
            0.0
        });
        gauge!("ois_job_running", "job" => name.clone()).set(if job.running { 1.0 } else { 0.0 });
        // Only meaningful once a run has both started and finished.
        if job.last_finished_ms > job.last_started_ms {
            gauge!("ois_job_last_duration_seconds", "job" => name)
                .set((job.last_finished_ms - job.last_started_ms) as f64 / 1000.0);
        }
    }
}

fn observe_freshness(state: &AppState) {
    let now_ms = chrono::Utc::now().timestamp_millis();
    for (name, stamp) in [
        ("nav", &state.nav_refreshed),
        ("winds", &state.winds_refreshed),
    ] {
        let refreshed_ms = stamp.load(Ordering::Relaxed);
        gauge!(format!("ois_{name}_refreshed_timestamp_seconds")).set(refreshed_ms as f64 / 1000.0);
        // 0 = never fetched at runtime, which makes the age the full epoch age — enormous, and so
        // it trips any staleness threshold. That is the wanted behaviour: "never refreshed" *is*
        // stale, and it needs no sentinel value to say so.
        gauge!(format!("ois_{name}_data_age_seconds")).set((now_ms - refreshed_ms) as f64 / 1000.0);
    }
    gauge!("ois_data_refresh_in_flight").set(
        if state.data_refresh_in_flight.load(Ordering::Relaxed) {
            1.0
        } else {
            0.0
        },
    );
}

async fn observe_feed(state: &AppState) {
    let status = state.feed.read().await.status.clone();
    gauge!("ois_feed_healthy").set(if status.healthy { 1.0 } else { 0.0 });
    gauge!("ois_feed_pilots").set(status.pilots as f64);
    gauge!("ois_feed_prefiles").set(status.prefiles as f64);
    gauge!("ois_feed_airports_loaded").set(status.airports_loaded as f64);
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::{Router, body::Body, http::Request as HttpRequest, routing::get};
    use tower::ServiceExt;

    /// AC2 — a request is labelled by its **route template**, never by the concrete path.
    ///
    /// This is the cardinality guarantee the whole layer exists for: `/probe/KDCA` and
    /// `/probe/KJFK` must collapse onto one series. `MatchedPath` is opaque and only routing can
    /// produce it, so this drives a real `Router` rather than hand-building a request — deleting
    /// the `MatchedPath` lookup makes the assertions below fail with the raw path instead.
    #[tokio::test]
    async fn requests_are_labelled_by_route_template_not_raw_path() {
        let handle = handle();
        let app = Router::new()
            .route("/probe/{icao}", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(track_http));

        for icao in ["KDCA", "KJFK", "KLAX"] {
            let request = HttpRequest::builder()
                .uri(format!("/probe/{icao}"))
                .body(Body::empty())
                .unwrap();
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), 200);
        }

        let rendered = handle.render();
        let counted: Vec<&str> = rendered
            .lines()
            .filter(|l| l.starts_with("http_requests_total") && l.contains("/probe/"))
            .collect();

        // Three requests to three distinct paths, but exactly one series.
        assert_eq!(
            counted.len(),
            1,
            "expected one series for the route template, got {counted:#?}"
        );
        let series = counted[0];
        assert!(
            series.contains(r#"route="/probe/{icao}""#),
            "not labelled by template: {series}"
        );
        assert!(
            series.contains(r#"method="GET""#),
            "method missing: {series}"
        );
        assert!(
            series.contains(r#"status="2xx""#),
            "status class missing: {series}"
        );
        assert!(
            series.ends_with(" 3"),
            "all three requests should land on the one series: {series}"
        );
        // The concrete paths must appear nowhere in the exposition.
        for icao in ["KDCA", "KJFK", "KLAX"] {
            assert!(
                !rendered.contains(&format!("/probe/{icao}")),
                "raw path /probe/{icao} leaked into the label set"
            );
        }

        // And the latency histogram carries the same template.
        assert!(
            rendered
                .lines()
                .any(|l| l.starts_with("http_request_duration_seconds")
                    && l.contains(r#"route="/probe/{icao}""#)),
            "no latency histogram for the route template"
        );
    }

    /// The leak fix (#382 review): the drain task must keep running without anyone scraping.
    ///
    /// **What this can and cannot prove.** `run_upkeep()` has no observable effect through the
    /// exporter's API — a drained and an undrained recorder render identically — so no test can
    /// tell that call from a no-op. What is observable is the timer, which is the realistic
    /// regression (someone drops the task, or the loop exits after one pass), so that is what is
    /// asserted here; `run_upkeep_preserves_the_samples_it_drains` covers the other half, that
    /// draining does not cost us the data. Deleting the `spawn`, or the loop, hangs this test.
    #[tokio::test(start_paused = true)]
    async fn the_upkeep_task_drains_on_its_timer_with_no_scrape() {
        let handle = handle();
        spawn_upkeep(handle.clone());

        fn upkeep_count(rendered: &str) -> u64 {
            rendered
                .lines()
                .find(|l| l.starts_with("ois_metrics_upkeep_total"))
                .and_then(|l| l.rsplit_once(' '))
                .and_then(|(_, v)| v.parse().ok())
                .unwrap_or(0)
        }

        async fn advance(intervals: u32) {
            for _ in 0..intervals {
                tokio::time::advance(UPKEEP_INTERVAL).await;
                tokio::task::yield_now().await;
            }
        }

        // Nothing is scraped here; only the timer drives the task.
        let before = upkeep_count(&handle.render());
        advance(4).await;
        let first = upkeep_count(&handle.render());
        advance(4).await;
        let second = upkeep_count(&handle.render());

        assert!(
            first > before,
            "upkeep never ran on its timer: {before} -> {first}"
        );
        // A repeating timer, not a single drain at startup — the leak returns if the loop exits.
        assert!(
            second > first,
            "upkeep ran once but did not keep running: {first} -> {second}"
        );
    }

    /// Draining must move samples into distributions, not discard them.
    ///
    /// The whole leak fix rests on calling `run_upkeep()` on a timer; if that dropped what it
    /// drained, a default deployment would trade a memory leak for silently wrong latency numbers
    /// the moment anyone did scrape. Records a known number of samples, drains *without* rendering,
    /// and asserts the subsequent render still accounts for every one.
    #[test]
    fn run_upkeep_preserves_the_samples_it_drains() {
        let handle = handle();
        let _ = handle.render(); // start from a known state

        for i in 0..7 {
            histogram!("upkeep_drain_probe").record(i as f64 * 0.001);
        }
        // Drain via upkeep only — no render in between.
        handle.run_upkeep();

        let rendered = handle.render();
        let count = rendered
            .lines()
            .find(|l| l.starts_with("upkeep_drain_probe_count"))
            .and_then(|l| l.rsplit_once(' '))
            .and_then(|(_, v)| v.parse::<u64>().ok());
        assert_eq!(
            count,
            Some(7),
            "upkeep lost samples it drained:\n{rendered}"
        );
    }

    /// The mirror of the route-template test, for the *method* label.
    ///
    /// HTTP methods are tokens, not an enum, so an unauthenticated caller can invent as many as it
    /// likes. Without the allowlist each one mints a counter series *and* a full latency histogram
    /// on a real route template, and nothing ever expires them. Labelling by
    /// `request.method().as_str()` makes this fail with one series per bogus method.
    #[tokio::test]
    async fn invented_methods_collapse_onto_one_label() {
        let handle = handle();
        let app = Router::new()
            .route("/methodprobe", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(track_http));

        for i in 0..25 {
            let request = HttpRequest::builder()
                .method(format!("BOGUS{i}").as_str())
                .uri("/methodprobe")
                .body(Body::empty())
                .unwrap();
            app.clone().oneshot(request).await.unwrap();
        }

        let rendered = handle.render();
        let series: Vec<&str> = rendered
            .lines()
            .filter(|l| l.starts_with("http_requests_total") && l.contains("/methodprobe"))
            .collect();

        assert_eq!(
            series.len(),
            1,
            "25 invented methods must collapse onto one series, got {series:#?}"
        );
        assert!(
            series[0].contains(r#"method="other""#),
            "not collapsed onto the catch-all: {}",
            series[0]
        );
        // No invented token may reach the exposition, on either the counter or the histogram.
        for i in 0..25 {
            assert!(
                !rendered.contains(&format!("BOGUS{i}")),
                "invented method BOGUS{i} leaked into the label set"
            );
        }
        // Real methods keep their own identity — the allowlist must not flatten everything.
        assert_eq!(method_label(&Method::GET), "GET");
        assert_eq!(method_label(&Method::DELETE), "DELETE");
    }

    /// A job that has never finished must report *no* last-run time, not the epoch.
    ///
    /// `JobRegistry` uses `last_finished_ms == 0` for "never". Publishing that zero made the
    /// dashboard's `time() - ois_job_last_run_timestamp_seconds` panel read ~57 years for every job
    /// that had not run since boot, and tripped any "last run older than N" alert on all of them at
    /// once after each restart.
    #[tokio::test]
    async fn a_job_that_has_never_finished_reports_no_last_run_time() {
        let state = crate::state::AppState::without_db();
        state
            .jobs
            .register("qa_never_ran", "never finished", Some(86_400), false);
        state
            .jobs
            .register("qa_has_run", "finished once", Some(60), false);
        state.jobs.begin("qa_has_run");
        state.jobs.finish("qa_has_run", true, "ok");

        let handle = handle();
        observe(&state, None).await;
        let rendered = handle.render();

        let line_for = |job: &str| {
            rendered
                .lines()
                .find(|l| l.starts_with("ois_job_last_run_timestamp_seconds") && l.contains(job))
                .map(str::to_string)
        };

        assert_eq!(
            line_for("qa_never_ran"),
            None,
            "a job that never finished must be absent, not zero:\n{rendered}"
        );
        let ran = line_for("qa_has_run").expect("a finished job must report its last run");
        let seconds: f64 = ran.rsplit_once(' ').unwrap().1.parse().unwrap();
        // Seconds since the epoch, i.e. actually now-ish — not 0.
        assert!(
            seconds > 1_700_000_000.0,
            "implausible last-run time: {ran}"
        );
    }

    /// AC1/AC3 for the half that comes from the database.
    ///
    /// `observe` is what turns a [`DomainCounts`] row into gauges, and the committed Grafana
    /// dashboard binds to these exact names — renaming one ships as a silently empty panel. Asserts
    /// the value too, so a gauge wired to the wrong field is caught as well as a typo.
    #[tokio::test]
    async fn domain_counts_reach_the_exposition_under_their_dashboard_names() {
        let state = crate::state::AppState::without_db();
        let handle = handle();

        observe(
            &state,
            Some(DomainCounts {
                active_tmis: 7,
                active_ground_stops: 3,
                active_gdps: 2,
                active_programs: 5,
                enabled_fcas: 11,
                gdp_delay_minutes: 137,
            }),
        )
        .await;

        let rendered = handle.render();
        for (name, value) in [
            ("ois_active_tmis", 7),
            ("ois_active_ground_stops", 3),
            ("ois_active_gdps", 2),
            ("ois_active_programs", 5),
            ("ois_fca_enabled", 11),
            ("ois_gdp_delay_minutes", 137),
        ] {
            let line = rendered
                .lines()
                .find(|l| l.split_whitespace().next() == Some(name))
                .unwrap_or_else(|| panic!("domain series {name} missing from:\n{rendered}"));
            assert_eq!(
                line,
                format!("{name} {value}"),
                "{name} carries the wrong value"
            );
        }
    }

    #[test]
    fn status_codes_collapse_to_their_class() {
        assert_eq!(status_class(200), "2xx");
        assert_eq!(status_class(204), "2xx");
        assert_eq!(status_class(302), "3xx");
        assert_eq!(status_class(401), "4xx");
        assert_eq!(status_class(404), "4xx");
        assert_eq!(status_class(500), "5xx");
        assert_eq!(status_class(503), "5xx");
    }

    #[test]
    fn an_unconfigured_token_leaves_the_endpoint_open() {
        assert!(scrape_authorized(None, None));
        // An empty/whitespace value is treated as unset rather than as a token nobody can present,
        // so `METRICS_TOKEN=` in a .env does not silently lock Prometheus out.
        assert!(scrape_authorized(Some(""), None));
        assert!(scrape_authorized(Some("   "), None));
    }

    #[test]
    fn a_configured_token_is_required_and_must_match_exactly() {
        assert!(scrape_authorized(Some("s3cret"), Some("Bearer s3cret")));

        assert!(!scrape_authorized(Some("s3cret"), None));
        assert!(!scrape_authorized(Some("s3cret"), Some("")));
        assert!(!scrape_authorized(Some("s3cret"), Some("Bearer wrong")));
        // The scheme is not optional, and it is case-sensitive.
        assert!(!scrape_authorized(Some("s3cret"), Some("s3cret")));
        assert!(!scrape_authorized(Some("s3cret"), Some("bearer s3cret")));
        // A prefix match must not pass.
        assert!(!scrape_authorized(Some("s3cret"), Some("Bearer s3cretX")));
        assert!(!scrape_authorized(Some("s3cret"), Some("Bearer s3cre")));
        // Regression on the constant-time compare: the length difference must not be accumulated
        // in a byte, or a token exactly 256 longer than the real one would compare equal.
        let padded = format!("Bearer s3cret{}", "X".repeat(256));
        assert!(!scrape_authorized(Some("s3cret"), Some(&padded)));
        let long = "s".repeat(300);
        assert!(!scrape_authorized(Some(&long), Some("Bearer s")));
        assert!(scrape_authorized(
            Some(&long),
            Some(&format!("Bearer {long}"))
        ));
    }
}
