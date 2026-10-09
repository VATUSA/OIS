//! Environment-driven configuration helpers (ported/trimmed from osmium).

use http::{
    HeaderValue, Method,
    header::{self, HeaderName},
};
use std::num::NonZeroU32;
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

/// `OIS_SERVER_ADMIN_CID`, parsed: a single CID or a comma-separated list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerAdminCids {
    /// The CIDs that hold SERVER_ADMIN.
    pub cids: Vec<i64>,
    /// Every non-blank part that is not a positive integer, as written. A demotion pass demotes no
    /// one while there is any (#805): a typo would otherwise strip the admin it meant to keep.
    pub rejected: Vec<String>,
}

/// Parses an `OIS_SERVER_ADMIN_CID` value. Blank parts (`"1,,2"`, a trailing comma) are skipped.
pub fn parse_server_admin_cids(raw: &str) -> ServerAdminCids {
    let mut list = ServerAdminCids::default();
    for part in raw
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        match part.parse::<i64>() {
            Ok(cid) if cid > 0 => list.cids.push(cid),
            _ => list.rejected.push(part.to_string()),
        }
    }
    list
}

/// `OIS_SERVER_ADMIN_CID` from the environment. Server admin is env-configured only: the list alone
/// grants and removes it, never the permissions UI.
pub fn server_admin_cids() -> ServerAdminCids {
    server_admin_cids_from(std::env::var("OIS_SERVER_ADMIN_CID"))
}

/// Unset is an empty list. A value that is set but not valid UTF-8 is one rejected part, not an empty
/// list: it is malformed, so a demotion pass demotes no one rather than everyone.
fn server_admin_cids_from(value: Result<String, std::env::VarError>) -> ServerAdminCids {
    match value {
        Ok(raw) => parse_server_admin_cids(&raw),
        Err(std::env::VarError::NotPresent) => ServerAdminCids::default(),
        Err(std::env::VarError::NotUnicode(raw)) => ServerAdminCids {
            cids: Vec::new(),
            rejected: vec![raw.to_string_lossy().into_owned()],
        },
    }
}

/// Origins that are valid OAuth `return_to` targets but must NOT be granted credentialed CORS.
///
/// The desktop app's loopback listener is the case this exists for: the backend has to be willing
/// to redirect a browser to `http://127.0.0.1:8765/callback`, but that port is only bound while
/// sign-in is in progress, so granting it CORS would let anything else that binds it serve a page
/// making credentialed API calls with the user's cookie. Two different trust questions, two lists.
pub fn configured_return_to_only_origins() -> Vec<String> {
    parse_origin_list("OAUTH_RETURN_TO_ORIGINS")
}

/// Every origin acceptable as an OAuth `return_to` target: the CORS origins plus the
/// redirect-only ones above.
pub fn configured_return_to_origins() -> Vec<String> {
    let mut origins = configured_allowed_origins();
    origins.extend(configured_return_to_only_origins());
    origins
}

fn parse_origin_list(var: &str) -> Vec<String> {
    let Some(raw) = std::env::var(var)
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

/// Origins trusted for credentialed cross-origin requests.
///
/// This is the CORS allowlist only. `return_to` targets are [`configured_return_to_origins`],
/// which is this list plus the redirect-only entries.
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

/// Requests per minute allowed to one rate-limit bucket (#588), read from `name`
/// (`RATE_LIMIT_CREDENTIAL_PER_MIN`, `RATE_LIMIT_USER_PER_MIN`, `RATE_LIMIT_ANON_PER_MIN`). Unset,
/// unparsable or zero falls back to `default` — a typo must not switch limiting off or lock everyone out.
pub fn rate_limit_per_min(name: &str, default: u32) -> NonZeroU32 {
    trimmed_env(name)
        .and_then(|v| v.parse::<u32>().ok())
        .and_then(NonZeroU32::new)
        .or(NonZeroU32::new(default))
        .unwrap_or(NonZeroU32::MIN)
}

/// How many reverse proxies we run in front of the backend (`TRUSTED_PROXY_HOPS`, minimum 1). Each
/// appends one `X-Forwarded-For` entry, so the client's address is this many from the right; anything
/// further left came from the client and can be forged (#588).
///
/// The default, 2, is production's Cloudflare → Traefik chain. A deployment behind a single proxy (the
/// compose stack behind one Caddy/nginx) must set 1, or every caller is keyed on the proxy's address.
pub fn trusted_proxy_hops() -> usize {
    trimmed_env("TRUSTED_PROXY_HOPS")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(2)
        .max(1)
}

/// The key that encrypts stored secrets OIS must read back (today only the VATUSA webhook secret,
/// #605): 32 bytes, base64-encoded, in `OIS_SECRET_KEY`. `None` when unset or malformed — callers
/// treat that as "feature off" and log it, never panic, matching how `vatusa_api_key()` disables sync.
pub fn ois_secret_key() -> Option<[u8; 32]> {
    use base64::Engine;
    let raw = trimmed_env("OIS_SECRET_KEY")?;
    match base64::engine::general_purpose::STANDARD
        .decode(raw)
        .ok()
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
    {
        Some(key) => Some(key),
        None => {
            tracing::warn!("OIS_SECRET_KEY is set but is not 32 base64-encoded bytes; ignoring it");
            None
        }
    }
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
        ])
        // The web app and desktop are cross-origin, so without this they could not read how much of
        // their allowance is left or how long to back off after a 429 (#588).
        .expose_headers([
            header::RETRY_AFTER,
            crate::rate_limit::LIMIT_HEADER,
            crate::rate_limit::REMAINING_HEADER,
            crate::rate_limit::RESET_HEADER,
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
    use super::{normalize_origin, parse_server_admin_cids, server_admin_cids_from};

    /// Unset is no admin; set but not UTF-8 is malformed, so it must not read as unset (#805).
    #[test]
    fn server_admin_cids_tell_unset_from_unreadable() {
        use std::env::VarError;
        assert_eq!(
            server_admin_cids_from(Err(VarError::NotPresent)),
            Default::default()
        );
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            let raw = std::ffi::OsString::from_vec(vec![b'1', 0xff]);
            let list = server_admin_cids_from(Err(VarError::NotUnicode(raw)));
            assert!(list.cids.is_empty() && list.rejected.len() == 1, "{list:?}");
        }
        assert_eq!(server_admin_cids_from(Ok("1, 2".to_string())).cids, [1, 2]);
    }

    /// What `OIS_SERVER_ADMIN_CID` parses to (#805). Sign-in grants the CIDs, and nothing demotes
    /// anyone while any part is rejected, so a typo must land in `rejected`, not vanish.
    #[test]
    fn server_admin_cids_parse_and_reject_by_part() {
        let parsed = |raw: &str| {
            let list = parse_server_admin_cids(raw);
            (list.cids, list.rejected)
        };
        let none: Vec<String> = Vec::new();
        assert_eq!(parsed(""), (vec![], none.clone()));
        assert_eq!(
            parsed(" 1234567 , 7654321 ,"),
            (vec![1234567, 7654321], none.clone())
        );
        assert_eq!(parsed("1,,+2"), (vec![1, 2], none.clone()));
        assert_eq!(
            parsed("1234567;7654321"),
            (vec![], vec!["1234567;7654321".to_string()])
        );
        assert_eq!(
            parsed("1234567, 765432l, 0, -5, 1 2"),
            (
                vec![1234567],
                vec!["765432l", "0", "-5", "1 2"]
                    .into_iter()
                    .map(String::from)
                    .collect()
            )
        );
    }

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

    /// The Tauri webview's origin is not an http host: `tauri://localhost` on macOS/Linux and
    /// `http://tauri.localhost` on Windows. Both ship in `.env.example`'s `CORS_ALLOWED_ORIGINS`,
    /// and the desktop app depends on them surviving normalisation verbatim — a `tauri://` scheme
    /// has no known default port, so it must fall through to the raw value rather than be dropped.
    /// Tightening this to http(s)-only silently blocks every desktop API call, which presents as
    /// the server being down.
    #[test]
    fn passes_the_desktop_webview_origins_through_untouched() {
        assert_eq!(normalize_origin("tauri://localhost"), "tauri://localhost");
        assert_eq!(
            normalize_origin("http://tauri.localhost"),
            "http://tauri.localhost"
        );
    }
}
