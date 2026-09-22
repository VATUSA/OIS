use axum::{
    extract::{Request, State},
    middleware::Next,
    response::Response,
};

use crate::{
    auth::{
        acl::{
            PermissionPath, fetch_api_key_access, fetch_service_account_access, fetch_user_access,
        },
        context::{CurrentApiKey, CurrentServiceAccount, CurrentUser, SessionToken},
    },
    errors::ApiError,
    repos::access as access_repo,
    state::AppState,
};

const SESSION_COOKIE: &str = "ois_session";

/// User API keys carry this bearer-token prefix, and service accounts the one below, so the two
/// bearer kinds are told apart without a speculative lookup against both tables.
const API_KEY_TOKEN_PREFIX: &str = "ois_pat_";
const SERVICE_ACCOUNT_TOKEN_PREFIX: &str = "ois_sa_";

#[derive(Debug, PartialEq, Eq)]
enum BearerKind {
    ApiKey,
    ServiceAccount,
}

/// Which resolver a bearer token belongs to, by its prefix. `None` for anything else — the
/// `METRICS_TOKEN` Prometheus presents, or a stranger's junk header — which can match neither table,
/// so it gets no principal and costs no query (VATUSA/OIS#384). Every service-account token is minted
/// with its prefix (`handlers::service_accounts::generate_token`), so none is lost by this.
fn bearer_kind(token: &str) -> Option<BearerKind> {
    if token.starts_with(API_KEY_TOKEN_PREFIX) {
        Some(BearerKind::ApiKey)
    } else if token.starts_with(SERVICE_ACCOUNT_TOKEN_PREFIX) {
        Some(BearerKind::ServiceAccount)
    } else {
        None
    }
}

/// Resolves the current user (session cookie) and/or service account (bearer token)
/// and stashes them in request extensions for downstream extractors/handlers.
pub async fn resolve_current_user(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let session_token = parse_cookie(request.headers().get(http::header::COOKIE), SESSION_COOKIE);
    let bearer_token = parse_bearer_token(request.headers().get(http::header::AUTHORIZATION));

    let current_user =
        if let (Some(pool), Some(token)) = (state.db.as_ref(), session_token.as_deref()) {
            access_repo::find_current_user_by_session_token(pool, token)
                .await
                .ok()
                .flatten()
        } else {
            None
        };

    // A bearer token is either a user API key (`ois_pat_…`) or a service account (`ois_sa_…`); the
    // prefix routes it to the right resolver so at most one table is queried, and none for neither.
    let client_ip = client_ip(request.headers());
    let bearer = bearer_token
        .as_deref()
        .and_then(|t| bearer_kind(t).map(|kind| (t, kind)));
    let (current_service_account, current_api_key) = match (state.db.as_ref(), bearer) {
        (Some(pool), Some((token, BearerKind::ApiKey))) => {
            let key = access_repo::find_current_api_key_by_bearer_token(
                pool,
                token,
                client_ip.as_deref(),
            )
            .await
            .ok()
            .flatten();
            (None, key)
        }
        (Some(pool), Some((token, BearerKind::ServiceAccount))) => {
            let sa = access_repo::find_current_service_account_by_bearer_token(pool, token)
                .await
                .ok()
                .flatten();
            (sa, None)
        }
        _ => (None, None),
    };

    request.extensions_mut().insert(current_user);
    request.extensions_mut().insert(current_service_account);
    request.extensions_mut().insert(current_api_key);
    request.extensions_mut().insert(SessionToken(session_token));
    request.extensions_mut().insert(bearer_token);

    next.run(request).await
}

/// The client IP (via the shared extractor), parsed and re-serialized so only a valid address is ever
/// bound to the `inet` column — a malformed header yields `None` rather than failing the key lookup.
fn client_ip(headers: &http::HeaderMap) -> Option<String> {
    crate::repos::audit::client_ip(headers)?
        .parse::<std::net::IpAddr>()
        .ok()
        .map(|ip| ip.to_string())
}

/// Coarse-grained permission check for a user or service account. Data-dependent
/// authorization (ownership, ARTCC scope) is still done explicitly in handlers.
pub async fn ensure_permission(
    state: &AppState,
    current_user: Option<&CurrentUser>,
    current_service_account: Option<&CurrentServiceAccount>,
    current_api_key: Option<&CurrentApiKey>,
    permission: PermissionPath,
) -> Result<(), ApiError> {
    if let Some(user) = current_user {
        let (_, permissions) = fetch_user_access(state.db.as_ref(), &user.id).await?;
        return if permissions.contains(&permission) {
            Ok(())
        } else {
            Err(ApiError::Unauthorized)
        };
    }

    if let Some(service_account) = current_service_account {
        let (_, permissions) =
            fetch_service_account_access(state.db.as_ref(), &service_account.id).await?;
        return if permissions.contains(&permission) {
            Ok(())
        } else {
            Err(ApiError::Unauthorized)
        };
    }

    // An API key holds a permission only if its owner still does (capped set) — see `fetch_api_key_access`.
    if let Some(api_key) = current_api_key {
        let (_, permissions) = fetch_api_key_access(state.db.as_ref(), api_key).await?;
        return if permissions.contains(&permission) {
            Ok(())
        } else {
            Err(ApiError::Unauthorized)
        };
    }

    Err(ApiError::Unauthorized)
}

fn parse_cookie(cookie_header: Option<&http::HeaderValue>, cookie_name: &str) -> Option<String> {
    let header_value = cookie_header?.to_str().ok()?;
    for raw_cookie in header_value.split(';') {
        let mut parts = raw_cookie.trim().splitn(2, '=');
        let name = parts.next()?.trim();
        let value = parts.next()?.trim();
        if name == cookie_name {
            return Some(value.to_string());
        }
    }
    None
}

fn parse_bearer_token(auth_header: Option<&http::HeaderValue>) -> Option<String> {
    let token = auth_header?.to_str().ok()?.trim().strip_prefix("Bearer ")?;
    let token = token.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{BearerKind, bearer_kind, parse_bearer_token, parse_cookie};

    #[test]
    fn routes_bearer_tokens_by_prefix() {
        assert_eq!(bearer_kind("ois_pat_abc123"), Some(BearerKind::ApiKey));
        assert_eq!(
            bearer_kind("ois_sa_abc123"),
            Some(BearerKind::ServiceAccount)
        );
    }

    /// Neither prefix means neither table can match, so no lookup is spent on it — the Prometheus
    /// `METRICS_TOKEN` on every scrape used to cost a service-account SELECT (VATUSA/OIS#384).
    #[test]
    fn a_bearer_with_neither_prefix_goes_to_no_resolver() {
        for token in [
            "s3cret-metrics-token",
            "ois_other_abc",
            "OIS_SA_upper",
            "sa_abc",
            "",
        ] {
            assert_eq!(bearer_kind(token), None, "{token:?}");
        }
    }

    #[test]
    fn parses_bearer_token() {
        let value = http::HeaderValue::from_static("Bearer token-123");
        assert_eq!(
            parse_bearer_token(Some(&value)).as_deref(),
            Some("token-123")
        );
        let basic = http::HeaderValue::from_static("Basic abc");
        assert!(parse_bearer_token(Some(&basic)).is_none());
    }

    #[test]
    fn parses_named_cookie() {
        let value = http::HeaderValue::from_static("foo=1; ois_session=abc; bar=2");
        assert_eq!(
            parse_cookie(Some(&value), "ois_session").as_deref(),
            Some("abc")
        );
    }
}
