use axum::{
    Json,
    extract::{Extension, Query, State},
    http::{HeaderMap, StatusCode},
    response::Redirect,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use reqwest::Url;
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    auth::{
        acl::{fetch_user_access, is_server_admin, permission_tree_from_paths},
        context::{CurrentUser, SessionToken},
        permissions::{AuthProfileRead, AuthSessionsDelete},
        require_permission::RequirePermission,
        vatsim::{VatsimOAuthConfig, exchange_code_for_token, fetch_profile},
    },
    config::{configured_return_to_origins, configured_server_admin_cids, cookie_secure},
    errors::ApiError,
    models::{DesktopExchangeRequest, DesktopSessionBody, MeBody},
    repos::{access as access_repo, auth as auth_repo, users as user_repo},
    state::AppState,
};

const OAUTH_STATE_COOKIE: &str = "ois_oauth_state";
const OAUTH_RETURN_TO_COOKIE: &str = "ois_oauth_return_to";
const SESSION_COOKIE: &str = "ois_session";
const OAUTH_STATE_TTL_SECS: i64 = 10 * 60;
const SESSION_TTL_SECS: i64 = 60 * 60 * 24 * 30;
const DEFAULT_LOGIN_REDIRECT: &str = "/api/v1/me";

/// Set alongside the OAuth state when the desktop app starts the flow, so the callback knows to
/// also mint a one-time code. A cookie rather than a round-trip through VATSIM's `state` because
/// that is where the rest of this flow already keeps its per-attempt context.
const OAUTH_DESKTOP_COOKIE: &str = "ois_oauth_desktop";

/// Query parameters the callback appends to the desktop app's loopback redirect.
const DESKTOP_CODE_PARAM: &str = "code";
const DESKTOP_STATE_PARAM: &str = "state";

/// Upper bound on the nonce we will echo back, so a hostile `desktop_state` can't be used to build
/// an unbounded redirect URL.
const DESKTOP_STATE_MAX_LEN: usize = 128;

/// Marks a session token as belonging to the desktop app; `auth::middleware` dispatches on it.
const DESKTOP_SESSION_TOKEN_PREFIX: &str = "ois_dsk_";

/// Baseline self-service permissions every non-SERVER_ADMIN user is entitled to.
/// Seeded on first login; every name here must exist in `access.permissions`.
const BASELINE_SELF_SERVICE_PERMISSIONS: &[&str] = &[
    "auth.profile.read",
    "auth.profile.update",
    "auth.sessions.delete",
    "access.self.read",
    "users.directory.read",
];

#[derive(Deserialize)]
pub struct LoginQuery {
    return_to: Option<String>,
    /// Set by the desktop app. Makes the callback hand back a one-time code as well as the usual
    /// cookie, which the app trades for a keychain-stored session token (#346).
    desktop: Option<bool>,
    /// The desktop app's per-attempt nonce, echoed back on the loopback redirect so the app can
    /// tell its own callback from one injected by anything else that can reach its port.
    desktop_state: Option<String>,
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/v1/me",
    tag = "auth",
    responses((status = 200, body = MeBody), (status = 401))
)]
pub async fn me(
    State(state): State<AppState>,
    _permission: RequirePermission<AuthProfileRead>,
    Extension(current_user): Extension<Option<CurrentUser>>,
) -> Result<Json<MeBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    Ok(Json(build_me_body(&state, user).await?))
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/vatsim/login",
    tag = "auth",
    params(("return_to" = Option<String>, Query, description = "Post-login redirect (must be an allowed origin)")),
    responses((status = 307, description = "Redirect to VATSIM OAuth"))
)]
pub async fn vatsim_login(
    jar: CookieJar,
    headers: HeaderMap,
    Query(query): Query<LoginQuery>,
) -> Result<(CookieJar, Redirect), ApiError> {
    let config = VatsimOAuthConfig::from_env()?;
    validate_oauth_login_origin(&headers, &config)?;

    let oauth_state = Uuid::new_v4().to_string();
    let authorize_url = config.authorization_url(&oauth_state)?;

    let state_cookie = Cookie::build((OAUTH_STATE_COOKIE, oauth_state))
        .http_only(true)
        .secure(cookie_secure())
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(time::Duration::seconds(OAUTH_STATE_TTL_SECS))
        .build();

    let mut jar = jar.add(state_cookie);

    if let Some(raw_return_to) = query
        .return_to
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let return_to = validate_return_to(raw_return_to)?;
        let return_to_cookie = Cookie::build((OAUTH_RETURN_TO_COOKIE, return_to))
            .http_only(true)
            .secure(cookie_secure())
            .same_site(SameSite::Lax)
            .path("/")
            .max_age(time::Duration::seconds(OAUTH_STATE_TTL_SECS))
            .build();
        jar = jar.add(return_to_cookie);
    }

    if query.desktop.unwrap_or(false) {
        // The cookie carries the app's nonce so the callback can echo it back. Empty when the app
        // sent none — such a flow still works, it simply cannot be verified by the app.
        let desktop_state = query
            .desktop_state
            .as_deref()
            .map(str::trim)
            .filter(|state| !state.is_empty() && state.len() <= DESKTOP_STATE_MAX_LEN)
            .filter(|state| state.chars().all(|c| c.is_ascii_alphanumeric()))
            .unwrap_or_default()
            .to_string();

        let desktop_cookie = Cookie::build((OAUTH_DESKTOP_COOKIE, desktop_state))
            .http_only(true)
            .secure(cookie_secure())
            .same_site(SameSite::Lax)
            .path("/")
            .max_age(time::Duration::seconds(OAUTH_STATE_TTL_SECS))
            .build();
        jar = jar.add(desktop_cookie);
    }

    Ok((jar, Redirect::temporary(&authorize_url)))
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/vatsim/callback",
    tag = "auth",
    responses((status = 302, description = "Login complete; redirect to return_to or /me"), (status = 400))
)]
pub async fn vatsim_callback(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(query): Query<CallbackQuery>,
) -> Result<(CookieJar, Redirect), ApiError> {
    let code = query
        .code
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(ApiError::BadRequest)?;

    let callback_state = query
        .state
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(ApiError::BadRequest)?;

    let Some(cookie_state) = jar.get(OAUTH_STATE_COOKIE).map(|cookie| cookie.value()) else {
        tracing::warn!("oauth callback missing state cookie");
        return Err(ApiError::OAuthStateCookieMissing);
    };
    if cookie_state != callback_state {
        tracing::warn!("oauth callback state mismatch");
        return Err(ApiError::OAuthStateMismatch);
    }

    let config = VatsimOAuthConfig::from_env()?;
    let access_token = exchange_code_for_token(&config, code).await?;
    let profile = fetch_profile(&config, &access_token).await?;

    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    let (user_id, was_new_user) = bootstrap_login_user(
        pool,
        profile.cid,
        &profile.email,
        &profile.display_name,
        profile.rating.as_deref(),
    )
    .await?;

    tracing::info!(
        cid = profile.cid,
        user_id = user_id.as_str(),
        "oauth user sync completed"
    );

    ensure_user_login_access(pool, &user_id, profile.cid, was_new_user).await?;

    // Enrich with VATUSA member details in the background (best-effort — login never waits on,
    // nor fails because of, VATUSA availability). No-ops when VATUSA_API_KEY is unset.
    crate::feed::vatusa::spawn_member_sync(pool.clone(), profile.cid);

    let session_token = Uuid::new_v4().to_string();
    auth_repo::insert_session(pool, &session_token, &user_id).await?;

    let mut redirect_target = jar
        .get(OAUTH_RETURN_TO_COOKIE)
        .map(|cookie| cookie.value().to_string())
        .filter(|value| !value.is_empty())
        .and_then(|value| validate_return_to(&value).ok())
        .unwrap_or_else(|| DEFAULT_LOGIN_REDIRECT.to_string());

    // The desktop app is blocked on a loopback listener waiting for this redirect. Hand it a
    // one-time code rather than the session token itself: a token in the URL would persist in
    // browser history and in any referer, whereas the code is useless once exchanged.
    // Mint ONLY when the target is the app's loopback listener. Otherwise a flow whose `return_to`
    // cookie went missing, or whose origin is no longer allowlisted, would fall back to
    // DEFAULT_LOGIN_REDIRECT and send the browser to `/api/v1/me?code=<live credential>` — putting
    // the credential in the address bar, history and any referer, which is the exact thing handing
    // back a code instead of a token is supposed to avoid. It also stops `desktop=true` dropping a
    // live code into the web app's URL for any other allowlisted origin.
    if let Some(desktop_cookie) = jar.get(OAUTH_DESKTOP_COOKIE)
        && is_loopback_redirect(&redirect_target)
    {
        let code = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        auth_repo::insert_desktop_auth_code(pool, &access_repo::sha256_hex(&code), &user_id)
            .await?;
        redirect_target = append_query_param(&redirect_target, DESKTOP_CODE_PARAM, &code);

        let state = desktop_cookie.value();
        if !state.is_empty() {
            redirect_target = append_query_param(&redirect_target, DESKTOP_STATE_PARAM, state);
        }
    }

    let clear_desktop = Cookie::build((OAUTH_DESKTOP_COOKIE, ""))
        .path("/")
        .max_age(time::Duration::seconds(0))
        .build();
    let clear_state = Cookie::build((OAUTH_STATE_COOKIE, ""))
        .path("/")
        .max_age(time::Duration::seconds(0))
        .build();
    let clear_return_to = Cookie::build((OAUTH_RETURN_TO_COOKIE, ""))
        .path("/")
        .max_age(time::Duration::seconds(0))
        .build();
    let session_cookie = Cookie::build((SESSION_COOKIE, session_token))
        .http_only(true)
        .secure(cookie_secure())
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(time::Duration::seconds(SESSION_TTL_SECS))
        .build();

    Ok((
        jar.remove(clear_state)
            .remove(clear_return_to)
            .remove(clear_desktop)
            .add(session_cookie),
        Redirect::to(&redirect_target),
    ))
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/logout",
    tag = "auth",
    responses((status = 204, description = "Session revoked"), (status = 401))
)]
pub async fn logout(
    State(state): State<AppState>,
    _permission: RequirePermission<AuthSessionsDelete>,
    Extension(SessionToken(session_token)): Extension<SessionToken>,
    jar: CookieJar,
) -> Result<(CookieJar, StatusCode), ApiError> {
    if let (Some(pool), Some(token)) = (state.db.as_ref(), session_token.as_deref()) {
        auth_repo::delete_session(pool, token).await?;
    }

    let session_cookie = Cookie::build((SESSION_COOKIE, ""))
        .path("/")
        .max_age(time::Duration::seconds(0))
        .build();

    Ok((jar.remove(session_cookie), StatusCode::NO_CONTENT))
}

/// Appends a query parameter to an already-validated absolute URL, preserving whatever query the
/// caller's `return_to` already carried.
fn append_query_param(url: &str, key: &str, value: &str) -> String {
    match Url::parse(url) {
        Ok(mut parsed) => {
            parsed.query_pairs_mut().append_pair(key, value);
            parsed.to_string()
        }
        // `validate_return_to` already proved this parses; the fallback only exists so a future
        // caller passing a bare path still produces something usable rather than panicking.
        Err(_) => {
            let separator = if url.contains('?') { '&' } else { '?' };
            format!("{url}{separator}{key}={value}")
        }
    }
}

/// Whether a redirect target is the desktop app's loopback listener — the only place a one-time
/// code may be handed to.
fn is_loopback_redirect(url: &str) -> bool {
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };
    if parsed.scheme() != "http" {
        return false;
    }
    matches!(
        parsed.host_str(),
        Some("127.0.0.1") | Some("localhost") | Some("::1")
    )
}

/// Mints a fresh desktop session token. The prefix is what `auth::middleware` dispatches on.
fn new_desktop_session_token() -> String {
    format!(
        "{DESKTOP_SESSION_TOKEN_PREFIX}{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    )
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/desktop/exchange",
    tag = "auth",
    request_body = DesktopExchangeRequest,
    responses(
        (status = 200, body = DesktopSessionBody, description = "A desktop session token"),
        (status = 401, description = "Unknown, expired, or already-used code")
    )
)]
/// Trades the one-time code from the OAuth callback for a desktop session token.
///
/// Public, like the OAuth callback itself — the code *is* the credential, and it is single-use and
/// short-lived. Unknown, expired and already-consumed codes are all reported identically so a probe
/// learns nothing from which it hit.
pub async fn desktop_exchange(
    State(state): State<AppState>,
    Json(body): Json<DesktopExchangeRequest>,
) -> Result<Json<DesktopSessionBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    let code = body.code.trim();
    if code.is_empty() {
        return Err(ApiError::Unauthorized);
    }

    let user_id = auth_repo::consume_desktop_auth_code(pool, &access_repo::sha256_hex(code))
        .await?
        .ok_or(ApiError::Unauthorized)?;

    let token = new_desktop_session_token();
    let expires_at = auth_repo::insert_desktop_session(pool, &token, &user_id).await?;

    Ok(Json(DesktopSessionBody { token, expires_at }))
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/desktop/refresh",
    tag = "auth",
    responses(
        (status = 200, body = DesktopSessionBody, description = "A rotated desktop session token"),
        (status = 401, description = "Not a live desktop session")
    )
)]
/// Rotates the caller's desktop session, returning a new token and extending the expiry.
///
/// Authenticated by the token being rotated — no permission gate, because holding a live desktop
/// session is the whole claim being made. Rotation means a token that leaked stops working as soon
/// as the app next refreshes.
///
/// Only `kind = 'desktop'` rows rotate: a stolen browser cookie cannot be traded up for a
/// long-lived keychain credential.
pub async fn desktop_refresh(
    State(state): State<AppState>,
    Extension(SessionToken(session_token)): Extension<SessionToken>,
) -> Result<Json<DesktopSessionBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let old_token = session_token.ok_or(ApiError::Unauthorized)?;

    let token = new_desktop_session_token();
    let expires_at = auth_repo::rotate_desktop_session(pool, &old_token, &token)
        .await?
        .ok_or(ApiError::Unauthorized)?;

    Ok(Json(DesktopSessionBody { token, expires_at }))
}

async fn build_me_body(state: &AppState, user: &CurrentUser) -> Result<MeBody, ApiError> {
    let (roles, permissions) = fetch_user_access(state.db.as_ref(), &user.id).await?;
    let vatusa = match state.db.as_ref() {
        Some(pool) => crate::repos::vatusa::fetch_profile(pool, user.cid).await?,
        None => None,
    };
    Ok(MeBody {
        id: user.id.clone(),
        cid: user.cid,
        email: user.email.clone(),
        display_name: user.display_name.clone(),
        rating: user.rating.clone(),
        server_admin: is_server_admin(&roles),
        role_names: roles,
        permissions: permission_tree_from_paths(&permissions),
        vatusa,
    })
}

/// Upserts the identity row + audit actor for a logging-in user, returning
/// `(user_id, was_new_user)`.
async fn bootstrap_login_user(
    pool: &sqlx::PgPool,
    cid: i64,
    email: &str,
    display_name: &str,
    rating: Option<&str>,
) -> Result<(String, bool), ApiError> {
    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;

    let user = user_repo::upsert_login_user(
        &mut tx,
        &Uuid::new_v4().to_string(),
        cid,
        email,
        display_name,
        display_name,
        rating,
    )
    .await?;

    access_repo::ensure_user_actor(&mut tx, &user.id, display_name).await?;

    tx.commit().await.map_err(|_| ApiError::Internal)?;

    Ok((user.id, user.was_new_user))
}

/// Reconciles the SERVER_ADMIN role against `OIS_SERVER_ADMIN_CID` on every login and
/// seeds baseline self-service permissions for new (or just-demoted) users.
async fn ensure_user_login_access(
    pool: &sqlx::PgPool,
    user_id: &str,
    cid: i64,
    was_new_user: bool,
) -> Result<(), ApiError> {
    if configured_server_admin_cids().contains(&cid) {
        let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
        access_repo::assign_server_admin(&mut tx, user_id).await?;
        tx.commit().await.map_err(|_| ApiError::Internal)?;
        tracing::info!(user_id, cid, "server admin role synced on login");
        return Ok(());
    }

    // Demotion + baseline seed share one transaction so they commit or roll back
    // together — a former admin holds no other access, so a crash between a bare
    // revoke and the seed could lock the account out.
    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    let demoted = access_repo::revoke_server_admin(&mut tx, user_id).await?;

    if was_new_user || demoted {
        let baseline: Vec<String> = BASELINE_SELF_SERVICE_PERMISSIONS
            .iter()
            .map(|permission| permission.to_string())
            .collect();
        access_repo::replace_user_permissions(&mut tx, user_id, &baseline).await?;
    }

    tx.commit().await.map_err(|_| ApiError::Internal)?;

    if demoted {
        tracing::info!(
            user_id,
            cid,
            "revoked server admin on login; reset to baseline"
        );
    } else if was_new_user {
        tracing::info!(user_id, cid, "baseline access seeded for new user");
    }

    Ok(())
}

/// Validates a `return_to` target: absolute http(s) whose origin is allowlisted for redirects
/// (`CORS_ALLOWED_ORIGINS` plus `OAUTH_RETURN_TO_ORIGINS`). Prevents the login flow becoming an
/// open redirect. Note the redirect-only list deliberately does not grant CORS.
fn validate_return_to(raw: &str) -> Result<String, ApiError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ApiError::BadRequest);
    }
    let parsed = Url::parse(trimmed).map_err(|_| ApiError::BadRequest)?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(ApiError::BadRequest);
    }
    let origin = url_origin(trimmed).ok_or(ApiError::BadRequest)?;
    if !configured_return_to_origins()
        .iter()
        .any(|allowed| allowed == &origin)
    {
        tracing::warn!(
            origin,
            "return_to origin is not an allowlisted redirect target"
        );
        return Err(ApiError::BadRequest);
    }
    Ok(trimmed.to_string())
}

fn validate_oauth_login_origin(
    headers: &HeaderMap,
    config: &VatsimOAuthConfig,
) -> Result<(), ApiError> {
    let expected_origin = url_origin(&config.redirect_uri).ok_or(ApiError::Internal)?;
    let Some(request_origin) = request_origin(headers) else {
        tracing::warn!(
            expected_origin,
            "oauth login request missing host/origin headers"
        );
        return Err(ApiError::OAuthLoginOriginMismatch);
    };
    if request_origin != expected_origin {
        tracing::warn!(
            expected_origin,
            request_origin,
            "oauth login origin mismatch"
        );
        return Err(ApiError::OAuthLoginOriginMismatch);
    }
    Ok(())
}

fn request_origin(headers: &HeaderMap) -> Option<String> {
    if let Some(origin) = header_value(headers, "origin") {
        return Some(origin);
    }
    let host =
        header_value(headers, "x-forwarded-host").or_else(|| header_value(headers, "host"))?;
    let proto = header_value(headers, "x-forwarded-proto").unwrap_or_else(|| "http".to_string());
    Some(format!("{proto}://{host}"))
}

fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn url_origin(raw: &str) -> Option<String> {
    let url = Url::parse(raw).ok()?;
    let host = url.host_str()?;
    let scheme = url.scheme();
    let port = url.port_or_known_default()?;
    let is_default_port = (scheme == "http" && port == 80) || (scheme == "https" && port == 443);
    if is_default_port {
        Some(format!("{scheme}://{host}"))
    } else {
        Some(format!("{scheme}://{host}:{port}"))
    }
}

#[cfg(test)]
mod desktop_redirect_tests {
    use super::*;

    /// A one-time code may only ever be handed to the desktop app's loopback listener. If the
    /// `return_to` cookie is missing or its origin is no longer allowlisted, `redirect_target`
    /// falls back to DEFAULT_LOGIN_REDIRECT — and appending a live credential to that would send
    /// the browser to `/api/v1/me?code=…`, putting it in the address bar and history.
    #[test]
    fn only_a_loopback_target_may_carry_a_code() {
        assert!(is_loopback_redirect("http://127.0.0.1:8765/callback"));
        assert!(is_loopback_redirect("http://localhost:8765/callback"));

        assert!(
            !is_loopback_redirect(DEFAULT_LOGIN_REDIRECT),
            "the fallback target must never be given a code"
        );
        assert!(!is_loopback_redirect("https://ois.vatusa.net/"));
        assert!(
            !is_loopback_redirect("https://127.0.0.1:8765/callback"),
            "the loopback listener is plain http; https there is not it"
        );
        assert!(!is_loopback_redirect("not a url"));
    }

    #[test]
    fn appending_the_code_to_the_fallback_would_expose_it() {
        // Documents exactly what the gate above prevents.
        assert_eq!(
            append_query_param(DEFAULT_LOGIN_REDIRECT, DESKTOP_CODE_PARAM, "LIVE"),
            "/api/v1/me?code=LIVE"
        );
    }
}
