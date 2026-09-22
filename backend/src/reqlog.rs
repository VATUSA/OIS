//! Developer-facing request logging. Emits one readable, level-coded line per HTTP request
//! (INFO for 2xx/3xx, WARN for 4xx, ERROR for 5xx) so failures stand out at a glance. Each
//! line carries the method, path (with real ids inline), status, latency, the actor (user CID
//! and name, or service account), the client IP when present, and a short sequential request
//! id that is also echoed in the `x-request-id` response header for correlation.
//!
//! Runs inside `resolve_current_user` (so the actor is known) and outside the audit layer (so
//! its latency reflects the whole app-level request). Toggle verbosity with `RUST_LOG`
//! (e.g. `RUST_LOG=ois_backend::reqlog=warn` to see only failures).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use axum::{extract::Request, middleware::Next, response::Response};
use http::{HeaderValue, Method};

use crate::auth::context::{CurrentApiKey, CurrentServiceAccount, CurrentUser};

/// Monotonic per-process request counter — short and readable (`req-42`), no uuid dependency.
static REQUEST_SEQ: AtomicU64 = AtomicU64::new(1);

/// Human label for who made the request: a user (CID + name), a service account, or anon.
fn actor_label(request: &Request) -> String {
    if let Some(user) = request
        .extensions()
        .get::<Option<CurrentUser>>()
        .and_then(Option::as_ref)
    {
        return format!("user={} ({})", user.cid, user.display_name);
    }
    if let Some(svc) = request
        .extensions()
        .get::<Option<CurrentServiceAccount>>()
        .and_then(Option::as_ref)
    {
        return format!("svc={}", svc.name);
    }
    if let Some(key) = request
        .extensions()
        .get::<Option<CurrentApiKey>>()
        .and_then(Option::as_ref)
    {
        return format!("apikey={} ({})", key.prefix, key.name);
    }
    "anon".to_string()
}

/// Paths polled on a fixed interval forever, whose successful requests are pure log noise.
/// Prometheus scrapes `/metrics` every 15s by default — thousands of identical INFO lines a day,
/// burying everything else.
const POLLED_PATHS: &[&str] = &["/metrics"];

/// Whether a finished request earns a log line.
///
/// A *failed* poll is still worth one — that is how a misconfigured `METRICS_TOKEN` or a broken
/// scrape surfaces at all — so only the 2xx/3xx case on a polled path is suppressed.
fn should_log(path: &str, status: u16) -> bool {
    status >= 400 || !POLLED_PATHS.contains(&path)
}

pub async fn log_requests(request: Request, next: Next) -> Response {
    // CORS preflight is noise — pass it through unlogged.
    if request.method() == Method::OPTIONS {
        return next.run(request).await;
    }

    let req_id = REQUEST_SEQ.fetch_add(1, Ordering::Relaxed);
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let query = request.uri().query().map(str::to_string);
    let actor = actor_label(&request);
    let ip = crate::repos::audit::client_ip(request.headers());

    let started = Instant::now();
    let mut response = next.run(request).await;
    let ms = started.elapsed().as_millis();
    let status = response.status().as_u16();

    // Echo the request id so it can be correlated with a client-side error report.
    if let Ok(value) = HeaderValue::from_str(&format!("req-{req_id}")) {
        response.headers_mut().insert("x-request-id", value);
    }

    // Checked before the line is built rather than after it: a path polled every 15s forever should
    // not pay to format a string that is immediately dropped. (`actor_label` above still runs — it
    // needs the request, which `next.run` consumes — so this trims the formatting, not everything.)
    if !should_log(&path, status) {
        return response;
    }

    // Build a single readable line; append query + ip only when present to avoid noise.
    let mut line = format!("{method} {path} -> {status} in {ms}ms · {actor}");
    if let Some(q) = &query {
        line.push_str(&format!(" ?{q}"));
    }
    if let Some(ip) = &ip {
        line.push_str(&format!(" · {ip}"));
    }

    if status >= 500 {
        tracing::error!(req = req_id, status, latency_ms = ms as u64, "{line}");
    } else if status >= 400 {
        tracing::warn!(req = req_id, status, latency_ms = ms as u64, "{line}");
    } else {
        tracing::info!(req = req_id, status, latency_ms = ms as u64, "{line}");
    }

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_successful_scrape_is_suppressed_but_a_failing_one_is_not() {
        // The whole point: Prometheus polling every 15s must not fill the log.
        assert!(!should_log("/metrics", 200));
        assert!(!should_log("/metrics", 304));
        // ...but a scrape that is being refused has to be visible, or a misconfigured
        // METRICS_TOKEN looks exactly like a healthy deployment from the logs.
        assert!(should_log("/metrics", 401));
        assert!(should_log("/metrics", 500));
    }

    /// The rule above is a pure function; this drives the **middleware** that is supposed to obey
    /// it. Without it the whole suppression can be deleted from `log_requests` in silence — the
    /// predicate keeps passing its own tests while every scrape logs again.
    ///
    /// Counting events is enough: one line per logged request, none for a suppressed one.
    #[tokio::test]
    async fn the_middleware_actually_obeys_the_rule() {
        use axum::{Router, body::Body, http::Request as HttpRequest, routing::get};
        use std::sync::Arc;
        use std::sync::atomic::AtomicUsize;
        use tower::ServiceExt;
        use tracing::instrument::WithSubscriber;

        /// Counts emitted events and ignores everything else.
        struct Counting(Arc<AtomicUsize>);
        impl tracing::Subscriber for Counting {
            fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
                true
            }
            fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::Id {
                tracing::Id::from_u64(1)
            }
            fn record(&self, _: &tracing::Id, _: &tracing::span::Record<'_>) {}
            fn record_follows_from(&self, _: &tracing::Id, _: &tracing::Id) {}
            fn event(&self, _: &tracing::Event<'_>) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
            fn enter(&self, _: &tracing::Id) {}
            fn exit(&self, _: &tracing::Id) {}
        }

        async fn lines_for(path: &'static str, status: axum::http::StatusCode) -> usize {
            let count = Arc::new(AtomicUsize::new(0));
            let app = Router::new()
                .route(path, get(move || async move { status }))
                .layer(axum::middleware::from_fn(log_requests));
            let request = HttpRequest::builder()
                .uri(path)
                .body(Body::empty())
                .unwrap();
            async move {
                app.oneshot(request).await.unwrap();
            }
            .with_subscriber(Counting(Arc::clone(&count)))
            .await;
            count.load(Ordering::Relaxed)
        }

        // The point of the feature: a healthy scrape every 15s must leave no trace.
        assert_eq!(
            lines_for("/metrics", axum::http::StatusCode::OK).await,
            0,
            "a successful scrape must not be logged"
        );
        // ...while a refused one must, or a misconfigured METRICS_TOKEN is invisible.
        assert_eq!(
            lines_for("/metrics", axum::http::StatusCode::UNAUTHORIZED).await,
            1,
            "a refused scrape must still be logged"
        );
        // And nothing else is affected.
        assert_eq!(
            lines_for("/health", axum::http::StatusCode::OK).await,
            1,
            "ordinary requests must still be logged"
        );
    }

    #[test]
    fn ordinary_requests_are_always_logged() {
        assert!(should_log("/health", 200));
        assert!(should_log("/api/v1/me", 200));
        // Not a prefix match — a real route must not be silenced by sharing a stem.
        assert!(should_log("/metrics/extra", 200));
        assert!(should_log("/api/v1/metrics", 200));
    }
}
