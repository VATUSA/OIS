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
    let presented = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    if !metrics::scrape_authorized(state.metrics_token.as_deref(), presented) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    // The one aggregate that isn't already in memory. A failed query degrades the scrape to the
    // in-memory series rather than failing it — Prometheus losing every metric because Postgres
    // blipped is strictly worse than losing six of them.
    let domain = match state.db.as_ref() {
        Some(pool) => match crate::repos::metrics::domain_counts(pool).await {
            Ok(counts) => Some(counts),
            Err(_) => {
                // The gauges keep their last values, which look perfectly plausible — so the
                // failure has to be counted, or a permanently-broken aggregate shows up only as a
                // WARN nobody is watching. See `metrics::observe`.
                ::metrics::counter!("ois_metrics_domain_aggregate_failures_total").increment(1);
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

    /// A state whose only interesting property is the configured scrape token. Building the state
    /// is how a test sets it — the handler no longer reads the process environment, so nothing here
    /// mutates global state and these tests can run concurrently with the rest of the suite.
    fn state_with_token(token: Option<&str>) -> AppState {
        let mut state = AppState::without_db();
        state.metrics_token = token.map(str::to_string);
        state
    }

    async fn scrape(state: AppState, headers: HeaderMap) -> (StatusCode, String, Option<String>) {
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

    fn bearer(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, value.parse().unwrap());
        headers
    }

    /// AC1 — the endpoint serves valid exposition carrying the runtime series.
    ///
    /// The domain (database-derived) series are asserted in `metrics::tests`, which can feed
    /// `observe` a known `DomainCounts` without a pool; here `without_db()` means there is none.
    #[tokio::test]
    async fn the_endpoint_renders_valid_exposition() {
        let (status, body, content_type) = scrape(state_with_token(None), HeaderMap::new()).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(content_type.as_deref(), Some(EXPOSITION_CONTENT_TYPE));

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
    }

    /// AC4 — with a token configured the guard is mandatory. Each case turns red if
    /// `scrape_authorized` is dropped from the handler.
    #[tokio::test]
    async fn a_configured_token_gates_the_endpoint() {
        let state = || state_with_token(Some("s3cret"));

        let (status, ..) = scrape(state(), HeaderMap::new()).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "no header must be refused"
        );

        let (status, ..) = scrape(state(), bearer("Bearer nope")).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "wrong token must be refused"
        );

        let (status, body, _) = scrape(state(), bearer("Bearer s3cret")).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "the right token must be let through"
        );
        assert!(body.contains("ois_build_info"));
    }

    /// A blank token is not a token: `METRICS_TOKEN=` in a `.env` must leave the endpoint open
    /// rather than lock the scraper out with a value nobody can present.
    #[tokio::test]
    async fn a_blank_token_leaves_the_endpoint_open() {
        let (status, ..) = scrape(state_with_token(Some("   ")), HeaderMap::new()).await;
        assert_eq!(status, StatusCode::OK);
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
            // Split from the right: the value is the last token, and a label value may legally
            // contain a space (a build version, say), which a left split would mangle.
            let Some((name, value)) = line.rsplit_once(' ') else {
                return false;
            };
            if name.is_empty() || value.parse::<f64>().is_err() {
                return false;
            }
            samples += 1;
        }
        samples > 0
    }
}
