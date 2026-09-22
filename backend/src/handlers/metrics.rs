//! `GET /metrics` — Prometheus text exposition (#382).
//!
//! Deliberately **outside the OpenAPI spec and the typed client**: this is a scrape target for
//! Prometheus, not a JSON API the SPA consumes, so there is no `#[utoipa::path]` here, no entry in
//! `openapi.rs`, and no client to regenerate.
//!
//! It gets **no port of its own**: `docker-compose.observability.yml` scrapes it in-network at
//! `backend:3000/metrics`, so nothing new is exposed on the host. But it does share the API's
//! listener, so anyone who can reach the API can reach this — including through a public
//! `api.<domain>` proxy. Set `METRICS_TOKEN` (or block the path at the proxy) wherever that
//! matters; see `docs/deploy.md`.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};

use crate::{metrics, state::AppState};

/// Prometheus text exposition format version served by `metrics-exporter-prometheus`.
const EXPOSITION_CONTENT_TYPE: &str = "text/plain; version=0.0.4";

pub async fn metrics(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let configured = std::env::var("METRICS_TOKEN").ok();
    let presented = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    if !metrics::scrape_authorized(configured.as_deref(), presented) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    // The one aggregate that isn't already in memory. A failed query degrades the scrape to the
    // in-memory series rather than failing it — Prometheus losing every metric because Postgres
    // blipped is strictly worse than losing six of them.
    let domain = match state.db.as_ref() {
        Some(pool) => match crate::repos::metrics::domain_counts(pool).await {
            Ok(counts) => Some(counts),
            Err(_) => {
                tracing::warn!("metrics: domain aggregate failed; serving in-memory series only");
                None
            }
        },
        None => None,
    };

    metrics::observe(&state, domain).await;

    (
        [(header::CONTENT_TYPE, EXPOSITION_CONTENT_TYPE)],
        state.metrics.render(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    fn set_token(value: &str) {
        // SAFETY: `METRICS_TOKEN` is process-wide and this is the only test that touches it —
        // see the single-test note on `the_endpoint_renders_exposition_and_honours_the_token`.
        unsafe { std::env::set_var("METRICS_TOKEN", value) };
    }

    fn clear_token() {
        // SAFETY: as above.
        unsafe { std::env::remove_var("METRICS_TOKEN") };
    }

    async fn scrape(headers: HeaderMap) -> (StatusCode, String, Option<String>) {
        let state = AppState::without_db();
        let response = metrics(State(state), headers).await;
        let status = response.status();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            String::from_utf8(body.to_vec()).unwrap(),
            content_type,
        )
    }

    /// AC1 (valid exposition, key series present) and AC4 (the token guard) in **one** test.
    ///
    /// They share a body deliberately: `METRICS_TOKEN` is process-wide and the handler reads it at
    /// call time, so two tests asserting different values of it would race under the default
    /// multi-threaded test harness. One body makes the ordering explicit and guarantees the var is
    /// cleared again at the end.
    #[tokio::test]
    async fn the_endpoint_renders_exposition_and_honours_the_token() {
        clear_token();
        let (status, body, content_type) = scrape(HeaderMap::new()).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(content_type.as_deref(), Some(EXPOSITION_CONTENT_TYPE));

        // Runtime, then domain — the issue asks for both, and asserting only one would let half
        // the exporter be deleted silently.
        for name in [
            "ois_build_info",
            "ois_uptime_seconds",
            "ois_nav_data_age_seconds",
            "ois_winds_data_age_seconds",
            "ois_data_refresh_in_flight",
            "ois_feed_healthy",
            "ois_feed_pilots",
        ] {
            assert!(body.contains(name), "missing series {name} in:\n{body}");
        }

        assert!(
            parses_as_exposition(&body),
            "body is not valid Prometheus exposition:\n{body}"
        );

        // --- AC4: with a token configured, the guard is mandatory. Each case below turns red if
        // `scrape_authorized` is dropped from the handler.
        set_token("s3cret");

        let (status, ..) = scrape(HeaderMap::new()).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "no header must be refused"
        );

        let mut wrong = HeaderMap::new();
        wrong.insert(header::AUTHORIZATION, "Bearer nope".parse().unwrap());
        let (status, ..) = scrape(wrong).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "wrong token must be refused"
        );

        let mut right = HeaderMap::new();
        right.insert(header::AUTHORIZATION, "Bearer s3cret".parse().unwrap());
        let (status, body, _) = scrape(right).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "the right token must be let through"
        );
        assert!(body.contains("ois_build_info"));

        clear_token();
    }

    /// A deliberately small exposition check: every non-blank, non-comment line must be
    /// `name{labels} value [timestamp]` with a parseable value. Enough to catch a malformed
    /// render; not a reimplementation of the parser.
    fn parses_as_exposition(body: &str) -> bool {
        let mut samples = 0;
        for line in body.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((name, rest)) = line.split_once(' ') else {
                return false;
            };
            if name.is_empty() {
                return false;
            }
            let value = rest.split_whitespace().next().unwrap_or("");
            if value.parse::<f64>().is_err() {
                return false;
            }
            samples += 1;
        }
        samples > 0
    }
}
