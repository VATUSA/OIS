//! Prometheus exposition for the backend (#382).
//!
//! Two halves:
//!
//! * [`track_http`] — an Axum layer that counts every request and times it, labelled by the
//!   *route template* rather than the raw path, so ids and callsigns can never fan the label set
//!   out. This is the only metric written continuously.
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

use axum::{extract::MatchedPath, extract::Request, middleware::Next, response::Response};
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

static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
static STARTED: OnceLock<Instant> = OnceLock::new();

/// Install the process-wide Prometheus recorder (idempotent) and return its render handle.
///
/// Installation failure is deliberately ignored rather than panicking: the only way it fails is
/// another recorder already being registered, and a process that cannot export metrics should
/// still serve traffic. The returned handle always renders whatever *this* recorder holds.
///
/// No periodic upkeep task is spawned, and none is needed here. The exporter asks callers to run
/// upkeep themselves, but it does two things: draining histograms into distributions — which
/// `render()` already does on every scrape — and expiring idle metrics, which only happens when an
/// `idle_timeout` is configured. We configure none (every series here is bounded and long-lived),
/// so a background task would have nothing left to do.
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
    let method = request.method().as_str().to_string();

    let started = Instant::now();
    let response = next.run(request).await;
    let elapsed = started.elapsed().as_secs_f64();
    let status = status_class(response.status().as_u16());

    counter!(
        "http_requests_total",
        "route" => route.clone(),
        "method" => method.clone(),
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
/// No `METRICS_TOKEN` configured means the endpoint is open, which is the intended default: the
/// observability stack scrapes it in-network and publishes no port for it. It does still ride the
/// API's own listener, so set a token wherever the API is publicly proxied. Once a token *is*
/// configured it is mandatory, and only an exact `Bearer <token>` passes.
pub fn scrape_authorized(expected: Option<&str>, authorization: Option<&str>) -> bool {
    let Some(expected) = expected.map(str::trim).filter(|t| !t.is_empty()) else {
        return true;
    };
    authorization
        .and_then(|h| h.strip_prefix("Bearer "))
        .is_some_and(|presented| presented == expected)
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
        gauge!("ois_job_last_run_timestamp_seconds", "job" => name.clone())
            .set(job.last_finished_ms as f64 / 1000.0);
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
    }
}
