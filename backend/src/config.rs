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

/// The raw value of `name`, for the `parse_*` functions below. Set but not UTF-8 comes back as its
/// lossy form so it parses as malformed rather than reading as unset.
fn raw_env(name: &str) -> Option<String> {
    raw_from(std::env::var(name))
}

fn raw_from(value: Result<String, std::env::VarError>) -> Option<String> {
    match value {
        Ok(raw) => Some(raw),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(raw)) => Some(raw.to_string_lossy().into_owned()),
    }
}

/// A whole number above zero from a raw env value (#923). Absent or blank is `Ok(None)`, so the caller
/// applies its default; zero or anything that is not a whole number is an `Err` naming `name`.
pub fn parse_nonzero_u32(name: &str, raw: Option<String>) -> Result<Option<NonZeroU32>, String> {
    let Some(value) = raw.as_deref().map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    value
        .parse::<u32>()
        .ok()
        .and_then(NonZeroU32::new)
        .map(Some)
        .ok_or_else(|| format!("{name} must be a whole number above 0, got {value:?}"))
}

/// `parse_nonzero_u32`, falling back to `default` on a bad value with one ERROR naming the variable.
/// A typo keeps the default rather than switching a limit off or locking everyone out.
fn nonzero_u32_or_default(name: &str, raw: Option<String>, default: u32) -> NonZeroU32 {
    let fallback = NonZeroU32::new(default).unwrap_or(NonZeroU32::MIN);
    match parse_nonzero_u32(name, raw) {
        Ok(value) => value.unwrap_or(fallback),
        Err(problem) => {
            tracing::error!(variable = name, default, "{problem}; using the default");
            fallback
        }
    }
}

/// Requests per minute allowed to one rate-limit bucket (#588), read from `name`
/// (`RATE_LIMIT_CREDENTIAL_PER_MIN`, `RATE_LIMIT_USER_PER_MIN`, `RATE_LIMIT_ANON_PER_MIN`). Unset or
/// blank is `default`; zero or malformed is `default` and logs an ERROR.
pub fn rate_limit_per_min(name: &str, default: u32) -> NonZeroU32 {
    nonzero_u32_or_default(name, raw_env(name), default)
}

/// The database pool's `max_connections` (`DATABASE_MAX_CONNECTIONS`, default 20). Unset or blank is
/// the default; zero or malformed is the default and logs an ERROR.
pub fn database_max_connections() -> u32 {
    nonzero_u32_or_default(
        "DATABASE_MAX_CONNECTIONS",
        raw_env("DATABASE_MAX_CONNECTIONS"),
        20,
    )
    .get()
}

/// Production's Cloudflare → Traefik chain.
pub const DEFAULT_TRUSTED_PROXY_HOPS: usize = 2;

/// How many reverse proxies we run in front of the backend (`TRUSTED_PROXY_HOPS`). Each appends one
/// `X-Forwarded-For` entry, so the client's address is this many from the right; anything further left
/// came from the client and can be forged (#588).
///
/// Absent or blank is `DEFAULT_TRUSTED_PROXY_HOPS`. A deployment behind a single proxy (the compose
/// stack behind one Caddy/nginx) must set 1, or every caller is keyed on the proxy's address. Zero or
/// a value that is not a whole number is an `Err`: startup refuses it (#923), since falling back to 2
/// behind one proxy would take the client IP from the part of the header the client wrote.
pub fn parse_trusted_proxy_hops(raw: Option<String>) -> Result<usize, String> {
    let Some(value) = raw.as_deref().map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(DEFAULT_TRUSTED_PROXY_HOPS);
    };
    match value.parse::<usize>() {
        Ok(0) => Err("TRUSTED_PROXY_HOPS must be at least 1, got 0".to_string()),
        Ok(hops) => Ok(hops),
        Err(_) => Err(format!(
            "TRUSTED_PROXY_HOPS must be a whole number of proxies, got {value:?}"
        )),
    }
}

/// `TRUSTED_PROXY_HOPS` from the environment; see `parse_trusted_proxy_hops`. Read once, by `run()`.
pub fn trusted_proxy_hops() -> Result<usize, String> {
    parse_trusted_proxy_hops(raw_env("TRUSTED_PROXY_HOPS"))
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
    use super::{
        DEFAULT_TRUSTED_PROXY_HOPS, nonzero_u32_or_default, normalize_origin, parse_nonzero_u32,
        parse_server_admin_cids, parse_trusted_proxy_hops, raw_from, server_admin_cids_from,
    };
    use std::num::NonZeroU32;
    use std::sync::{Arc, Mutex};

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

    /// The four variables `parse_nonzero_u32` serves (#923).
    const NONZERO_VARIABLES: [&str; 4] = [
        "RATE_LIMIT_CREDENTIAL_PER_MIN",
        "RATE_LIMIT_USER_PER_MIN",
        "RATE_LIMIT_ANON_PER_MIN",
        "DATABASE_MAX_CONNECTIONS",
    ];

    fn some(raw: &str) -> Option<String> {
        Some(raw.to_string())
    }

    /// Set but not UTF-8 is malformed, so it must not read as unset and take the default.
    #[test]
    fn a_raw_value_that_is_not_utf8_is_malformed_not_unset() {
        use std::env::VarError;
        assert_eq!(raw_from(Err(VarError::NotPresent)), None);
        assert_eq!(raw_from(Ok("1".to_string())), some("1"));
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            let raw = raw_from(Err(VarError::NotUnicode(std::ffi::OsString::from_vec(
                vec![b'1', 0xff],
            ))));
            assert!(parse_trusted_proxy_hops(raw.clone()).is_err(), "{raw:?}");
            assert!(
                parse_nonzero_u32("RATE_LIMIT_USER_PER_MIN", raw.clone()).is_err(),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn trusted_proxy_hops_absent_or_blank_is_the_default_two() {
        for raw in [None, some(""), some("   ")] {
            assert_eq!(parse_trusted_proxy_hops(raw.clone()), Ok(2), "{raw:?}");
        }
        assert_eq!(DEFAULT_TRUSTED_PROXY_HOPS, 2);
    }

    #[test]
    fn trusted_proxy_hops_parses_a_whole_number_either_side_of_two() {
        for (raw, hops) in [("1", 1), ("3", 3), (" 2 ", 2)] {
            assert_eq!(parse_trusted_proxy_hops(some(raw)), Ok(hops), "{raw:?}");
        }
    }

    /// Zero would key every caller on the address of the proxy nearest the backend.
    #[test]
    fn trusted_proxy_hops_refuses_zero_naming_the_variable() {
        assert_eq!(
            parse_trusted_proxy_hops(some("0")),
            Err("TRUSTED_PROXY_HOPS must be at least 1, got 0".to_string())
        );
    }

    /// A set but unparseable value is refused, never read as the default. `2/` used to have its
    /// slash stripped and read as 2.
    #[test]
    fn trusted_proxy_hops_refuses_a_malformed_value_naming_it() {
        for raw in ["1x", "-1", "2/", "1.5", "18446744073709551616"] {
            assert_eq!(
                parse_trusted_proxy_hops(some(raw)),
                Err(format!(
                    "TRUSTED_PROXY_HOPS must be a whole number of proxies, got \"{raw}\""
                )),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn nonzero_u32_absent_or_blank_is_unset_for_every_variable() {
        for name in NONZERO_VARIABLES {
            for raw in [None, some(""), some("   ")] {
                assert_eq!(
                    parse_nonzero_u32(name, raw.clone()),
                    Ok(None),
                    "{name} {raw:?}"
                );
            }
        }
    }

    #[test]
    fn nonzero_u32_parses_a_positive_value_for_every_variable() {
        for name in NONZERO_VARIABLES {
            for (raw, value) in [
                ("1", 1),
                ("450", 450),
                (" 45 ", 45),
                ("4294967295", u32::MAX),
            ] {
                assert_eq!(
                    parse_nonzero_u32(name, some(raw)),
                    Ok(NonZeroU32::new(value)),
                    "{name} {raw:?}"
                );
            }
        }
    }

    #[test]
    fn nonzero_u32_refuses_zero_or_malformed_naming_every_variable() {
        for name in NONZERO_VARIABLES {
            for raw in ["0", "abc", "-5", "300/", "1.5", "4294967296"] {
                assert_eq!(
                    parse_nonzero_u32(name, some(raw)),
                    Err(format!(
                        "{name} must be a whole number above 0, got \"{raw}\""
                    )),
                    "{name} {raw:?}"
                );
            }
        }
    }

    /// Runs `f` under a subscriber that records every event, and returns the recorded lines.
    fn captured_logs(f: impl FnOnce()) -> Vec<String> {
        #[derive(Clone, Default)]
        struct Capture(Arc<Mutex<Vec<u8>>>);
        impl std::io::Write for Capture {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().expect("capture lock").extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let capture = Capture::default();
        let writer = capture.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, f);
        let bytes = capture.0.lock().expect("capture lock").clone();
        String::from_utf8(bytes)
            .expect("utf-8 log output")
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// A typo must not switch limiting off: it keeps the default and says so once (#923).
    #[test]
    fn a_malformed_limit_logs_one_error_and_keeps_its_default() {
        for name in NONZERO_VARIABLES {
            for raw in ["abc", "0"] {
                let mut value = None;
                let lines =
                    captured_logs(|| value = Some(nonzero_u32_or_default(name, some(raw), 37)));
                assert_eq!(value.map(NonZeroU32::get), Some(37), "{name} {raw:?}");
                assert_eq!(lines.len(), 1, "{name} {raw:?}: {lines:?}");
                assert!(lines[0].contains("ERROR"), "{name} {raw:?}: {lines:?}");
                assert!(
                    lines[0].contains(&format!("variable=\"{name}\"")),
                    "{name}: {lines:?}"
                );
                for other in NONZERO_VARIABLES.iter().filter(|other| **other != name) {
                    assert!(
                        !lines[0].contains(other),
                        "{name} line names {other}: {lines:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn an_absent_or_blank_limit_logs_nothing_and_keeps_its_default() {
        for name in NONZERO_VARIABLES {
            for raw in [None, some(""), some("  ")] {
                let mut value = None;
                let lines =
                    captured_logs(|| value = Some(nonzero_u32_or_default(name, raw.clone(), 37)));
                assert_eq!(value.map(NonZeroU32::get), Some(37), "{name} {raw:?}");
                assert!(lines.is_empty(), "{name} {raw:?}: {lines:?}");
            }
        }
    }

    #[test]
    fn a_valid_limit_is_used_and_logs_nothing() {
        for name in NONZERO_VARIABLES {
            let mut value = None;
            let lines =
                captured_logs(|| value = Some(nonzero_u32_or_default(name, some("450"), 37)));
            assert_eq!(value.map(NonZeroU32::get), Some(450), "{name}");
            assert!(lines.is_empty(), "{name}: {lines:?}");
        }
    }
}
