//! Environment-driven configuration helpers (ported/trimmed from osmium).

use http::{
    HeaderValue, Method,
    header::{self, HeaderName},
};
use tower_http::cors::CorsLayer;

pub fn env_flag_enabled(name: &str) -> bool {
    std::env::var(name)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

pub fn vatsim_dev_mode_enabled() -> bool {
    env_flag_enabled("VATSIM_DEV_MODE")
}

/// Whether session/state cookies are marked `Secure`. Keep false on plain local HTTP.
pub fn cookie_secure() -> bool {
    env_flag_enabled("COOKIE_SECURE")
}

/// CIDs that should hold the SERVER_ADMIN role. Single CID or comma-separated list.
/// Server admin is env-configured only — never grantable through the permissions UI.
pub fn configured_server_admin_cids() -> Vec<i64> {
    let Ok(raw) = std::env::var("OIS_SERVER_ADMIN_CID") else {
        return Vec::new();
    };
    raw.split(',')
        .filter_map(|part| part.trim().parse::<i64>().ok())
        .filter(|cid| *cid > 0)
        .collect()
}

/// Origins trusted for credentialed cross-origin requests. Reused as the allowlist
/// for OAuth `return_to` redirect targets — same trust boundary.
pub fn configured_allowed_origins() -> Vec<String> {
    let Some(raw) = std::env::var("CORS_ALLOWED_ORIGINS")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return Vec::new();
    };

    raw.split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_origin)
        .collect()
}

fn trimmed_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty())
}

/// VATUSA API base, e.g. `https://api.vatusa.net` (no trailing slash). Versioned paths
/// (`/v2/...`, `/v3/...`) are appended by callers.
pub fn vatusa_api_base() -> String {
    trimmed_env("VATUSA_API_BASE").unwrap_or_else(|| "https://api.vatusa.net".to_string())
}

/// The VATUSA API key (`apikey` query param for v2, `x-api-key` header for v3). When unset,
/// VATUSA sync is disabled: sign-in fetch, webhook registration, and reconciliation all no-op.
pub fn vatusa_api_key() -> Option<String> {
    trimmed_env("VATUSA_API_KEY")
}

/// This deployment's public HTTPS base URL, used to register the webhook receiver with VATUSA
/// (e.g. `https://ois.vatusa.net`). Webhook registration is skipped when unset.
pub fn ois_public_url() -> Option<String> {
    trimmed_env("OIS_PUBLIC_URL")
}

pub fn build_cors_layer() -> CorsLayer {
    let layer = CorsLayer::new()
        .allow_credentials(true)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PATCH,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::ACCEPT,
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            HeaderName::from_static("x-requested-with"),
        ]);

    let origins: Vec<HeaderValue> = configured_allowed_origins()
        .into_iter()
        .filter_map(|origin| HeaderValue::from_str(&origin).ok())
        .collect();

    if origins.is_empty() {
        layer
    } else {
        layer.allow_origin(origins)
    }
}

fn normalize_origin(raw: &str) -> String {
    let Ok(url) = reqwest::Url::parse(raw) else {
        return raw.to_string();
    };
    let Some(host) = url.host_str() else {
        return raw.to_string();
    };
    let scheme = url.scheme();
    let Some(port) = url.port_or_known_default() else {
        return raw.to_string();
    };

    let is_default_port = (scheme == "http" && port == 80) || (scheme == "https" && port == 443);
    if is_default_port {
        format!("{scheme}://{host}")
    } else {
        format!("{scheme}://{host}:{port}")
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_origin;

    #[test]
    fn normalizes_default_and_custom_ports() {
        assert_eq!(
            normalize_origin("https://app.example.org:443/path"),
            "https://app.example.org"
        );
        assert_eq!(
            normalize_origin("http://127.0.0.1:5173/login"),
            "http://127.0.0.1:5173"
        );
    }
}
