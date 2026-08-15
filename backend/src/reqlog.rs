//! Developer-facing request logging. Emits one readable, level-coded line per HTTP request
//! (INFO for 2xx/3xx, WARN for 4xx, ERROR for 5xx) so failures stand out at a glance. Each
//! line carries the method, path (with real ids inline), status, latency, the actor (user CID
//! + name, or service account), the client IP when present, and a short sequential request id
//! that is also echoed in the `x-request-id` response header for correlation.
//!
//! Runs inside `resolve_current_user` (so the actor is known) and outside the audit layer (so
//! its latency reflects the whole app-level request). Toggle verbosity with `RUST_LOG`
//! (e.g. `RUST_LOG=ois_backend::reqlog=warn` to see only failures).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use axum::{extract::Request, middleware::Next, response::Response};
use http::{HeaderValue, Method};

use crate::auth::context::{CurrentServiceAccount, CurrentUser};

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
    "anon".to_string()
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
