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
    repos::{
        access::{self as access_repo, BASELINE_ROLE},
        auth as auth_repo, users as user_repo,
    },
    state::AppState,
};

const OAUTH_STATE_COOKIE: &str = "ois_oauth_state";
const OAUTH_RETURN_TO_COOKIE: &str = "ois_oauth_return_to";
const SESSION_COOKIE: &str = "ois_session";
const OAUTH_STATE_TTL_SECS: i64 = 10 * 60;
const DEFAULT_LOGIN_REDIRECT: &str = "/api/v1/me";
/// The permission whose scope decides whether a member reads restrictions nationally. The same one
/// `RestrictionAlerts` gates its audience on, so the flag and the gate can't drift (#405).
const TMU_READ_PERMISSION: &str = "tmu.program.read";

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
    security(("session" = ["auth.profile.read"])),
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

    let (user_id, first_sign_in) = bootstrap_login_user(
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

    ensure_user_login_access(pool, &user_id, profile.cid, first_sign_in).await?;

    // Sync VATUSA details and the access their roles map to *before* issuing the session, so a
    // first-ever login is already correct (#548). Bounded and best-effort: a slow or failing VATUSA
    // never fails the login. No-ops when VATUSA_API_KEY is unset.
    crate::feed::vatusa::sync_member_on_login(pool, profile.cid).await;

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
        .max_age(time::Duration::seconds(
            crate::repos::auth::SESSION_TTL_SECS,
        ))
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
    security(("session" = ["auth.sessions.delete"])),
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
    security(("session" = [])),
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
    // `permissions` is a flat name tree with no ARTCC dimension, so a client can't tell a national
    // (DCC) reader from a facility-scoped one. Resolve that one question here (#405).
    let tmu_national = match state.db.as_ref() {
        Some(pool) => access_repo::permission_scope(pool, &user.id, TMU_READ_PERMISSION)
            .await?
            .is_national(),
        None => false,
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
        tmu_national,
    })
}

/// Upserts the identity row + audit actor for a logging-in user, returning
/// `(user_id, first_sign_in)`.
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

    Ok((user.id, user.first_sign_in))
}

/// Reconciles the SERVER_ADMIN role against `OIS_SERVER_ADMIN_CID` on every login, and gives a new
/// (or just-demoted) user the baseline by putting them in the [`BASELINE_ROLE`] group.
async fn ensure_user_login_access(
    pool: &sqlx::PgPool,
    user_id: &str,
    cid: i64,
    first_sign_in: bool,
) -> Result<(), ApiError> {
    reconcile_login_access(
        pool,
        user_id,
        cid,
        first_sign_in,
        &configured_server_admin_cids(),
    )
    .await
}

/// [`ensure_user_login_access`] against an explicit admin list, so a test can configure an admin
/// without `set_var` racing the rest of the suite.
pub(crate) async fn reconcile_login_access(
    pool: &sqlx::PgPool,
    user_id: &str,
    cid: i64,
    first_sign_in: bool,
    server_admin_cids: &[i64],
) -> Result<(), ApiError> {
    if server_admin_cids.contains(&cid) {
        let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
        access_repo::assign_server_admin(&mut tx, user_id).await?;
        tx.commit().await.map_err(|_| ApiError::Internal)?;
        tracing::info!(user_id, cid, "server admin role synced on login");
        return Ok(());
    }

    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    let demoted = access_repo::demote_server_admin(&mut tx, user_id).await?;
    if first_sign_in && !demoted {
        // The baseline now arrives through the `USER` group, not as five direct rows (#544). A first
        // sign-in is not a blank slate any more — an admin may have granted (or denied) a user seeded
        // by the VATUSA pull before they ever signed in (#605), and that must survive it.
        // `System`: OIS grants the baseline group itself, so sync and admins both leave it alone.
        access_repo::set_user_role(
            &mut tx,
            user_id,
            BASELINE_ROLE,
            true,
            access_repo::GrantSource::System,
        )
        .await?;
    }
    tx.commit().await.map_err(|_| ApiError::Internal)?;

    if demoted {
        tracing::info!(
            user_id,
            cid,
            "revoked server admin on login; reset to baseline"
        );
    } else if first_sign_in {
        tracing::info!(user_id, cid, "baseline access seeded for new user");
    }

    Ok(())
}

/// Demotes every server admin whose CID is not in `admin_cids`, each in its own transaction, as
/// sign-in would (#805). Run at startup: `OIS_SERVER_ADMIN_CID` can change only across a restart, so
/// this is the moment a removal takes effect, rather than the removed admin's next sign-in, which a
/// live session or API key never needs. Access resolves per request, so their sessions and keys drop
/// to the baseline with it. An empty list demotes everyone: no CID configured means no server admin.
/// Returns how many were demoted.
pub(crate) async fn demote_unconfigured_server_admins(
    pool: &sqlx::PgPool,
    admin_cids: &[i64],
) -> Result<usize, ApiError> {
    let holders = access_repo::server_admins_not_in(pool, admin_cids).await?;
    let mut demoted = 0;
    for (user_id, cid) in &holders {
        let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
        if access_repo::demote_server_admin(&mut tx, user_id).await? {
            demoted += 1;
            tracing::warn!(
                user_id,
                cid,
                "server admin not in OIS_SERVER_ADMIN_CID; demoted to baseline at startup"
            );
        }
        tx.commit().await.map_err(|_| ApiError::Internal)?;
    }
    if admin_cids.is_empty() && demoted > 0 {
        tracing::error!(
            demoted,
            "OIS_SERVER_ADMIN_CID is empty: every server admin was demoted at startup"
        );
    } else if demoted > 0 {
        tracing::info!(demoted, "startup server admin reconciliation done");
    }
    Ok(demoted)
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use sqlx::PgPool;

    use super::*;
    use crate::scope_test_support::{grant, seed_user, test_state};

    fn current_user(id: &str) -> CurrentUser {
        CurrentUser {
            id: id.to_string(),
            cid: 0,
            email: String::new(),
            display_name: String::new(),
            rating: None,
            primary_role: None,
        }
    }

    /// `/me`'s `tmu_national` is what tells the client whether to scope restriction alerts to the
    /// user's own ARTCCs (#405). It has to come from the *scope* of `tmu.program.read`, not from
    /// `server_admin`: a DCC controller holds it nationally without being a server admin, and
    /// reading the flag off `server_admin` would silently narrow them to their home facility.
    #[sqlx::test]
    async fn tmu_national_follows_the_permission_scope_not_the_admin_flag(pool: PgPool) {
        let national = seed_user(&pool).await;
        grant(&pool, &national, TMU_READ_PERMISSION, None).await;
        let scoped = seed_user(&pool).await;
        grant(&pool, &scoped, TMU_READ_PERMISSION, Some("ZDC")).await;
        let none = seed_user(&pool).await;

        let state = test_state(pool, HashMap::new());

        let body = build_me_body(&state, &current_user(&national))
            .await
            .unwrap();
        assert!(body.tmu_national, "an unscoped grant reads nationally");
        assert!(
            !body.server_admin,
            "and does so without being a server admin — the point of the distinction"
        );

        let body = build_me_body(&state, &current_user(&scoped)).await.unwrap();
        assert!(!body.tmu_national, "a ZDC-scoped grant is not national");

        let body = build_me_body(&state, &current_user(&none)).await.unwrap();
        assert!(!body.tmu_national, "no grant is not national");
    }

    /// AC3 of #544: the baseline arrives as **group membership**, not as per-user rows.
    ///
    /// That is the property that makes the group model worth having — editing `USER`'s permission set
    /// changes every signed-in user's baseline with no backfill. The old path wrote five
    /// `access.user_permissions` rows per user, which is exactly the duplication the epic removes.
    #[sqlx::test]
    async fn a_new_user_gets_the_baseline_from_the_user_group_not_direct_rows(pool: PgPool) {
        let user = seed_user(&pool).await;

        super::ensure_user_login_access(&pool, &user, 9_999_999, true)
            .await
            .unwrap();

        let roles: Vec<String> = sqlx::query_scalar(
            "select role_name from access.user_roles where user_id = $1 and artcc_id is null",
        )
        .bind(&user)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert!(roles.iter().any(|r| r == "USER"), "got roles {roles:?}");

        let direct: i64 =
            sqlx::query_scalar("select count(*) from access.user_permissions where user_id = $1")
                .bind(&user)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(direct, 0, "no permission should be copied onto the user");

        // And the baseline still actually resolves, through the group.
        let effective = access_repo::fetch_effective_permissions(&pool, &user)
            .await
            .unwrap();
        for name in [
            "auth.profile.read",
            "auth.profile.update",
            "auth.sessions.delete",
            "access.self.read",
            "users.directory.read",
        ] {
            assert!(effective.contains_key(name), "baseline missing {name}");
        }
    }

    /// A user seeded by the VATUSA pull (#605) can be found by exact CID and granted — or denied —
    /// access before they ever sign in. Their first sign-in adds the baseline and must keep both.
    #[sqlx::test]
    async fn a_first_sign_in_keeps_access_granted_before_it(pool: PgPool) {
        let user = seed_user(&pool).await;
        grant(&pool, &user, "tmu.program.update", None).await;
        crate::scope_test_support::deny_scoped(&pool, &user, "tmu.ntml.create", None).await;

        super::ensure_user_login_access(&pool, &user, 9_999_999, true)
            .await
            .unwrap();

        let direct: Vec<(String, bool)> = sqlx::query_as(
            "select permission_name, granted from access.user_permissions \
             where user_id = $1 order by permission_name",
        )
        .bind(&user)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            direct,
            [
                ("tmu.ntml.create".to_string(), false),
                ("tmu.program.update".to_string(), true)
            ],
            "the pre-sign-in grant and deny both survive"
        );
        let has_user: bool = sqlx::query_scalar(
            "select exists(select 1 from access.user_roles where user_id = $1 and role_name = 'USER')",
        )
        .bind(&user)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(has_user, "and the baseline still arrives");
    }

    /// A demotion must still clear the ex-admin's own national grants — the reason the wipe survived
    /// the move to a group. Without it a former admin would keep everything they had been granted
    /// directly while appearing to be reset to baseline.
    #[sqlx::test]
    async fn a_demotion_clears_direct_grants_and_leaves_only_the_group(pool: PgPool) {
        let user = seed_user(&pool).await;
        grant(&pool, &user, "tmu.program.update", None).await;
        sqlx::query(
            "insert into access.user_roles (user_id, role_name, source) values ($1, 'SERVER_ADMIN', 'system')",
        )
        .bind(&user)
        .execute(&pool)
        .await
        .unwrap();

        super::ensure_user_login_access(&pool, &user, 9_999_999, false)
            .await
            .unwrap();

        let direct: i64 =
            sqlx::query_scalar("select count(*) from access.user_permissions where user_id = $1")
                .bind(&user)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(direct, 0, "a demotion clears direct national grants");

        let effective = access_repo::fetch_effective_permissions(&pool, &user)
            .await
            .unwrap();
        assert!(
            !effective.contains_key("tmu.program.update"),
            "the demoted admin keeps nothing beyond the baseline"
        );
        assert!(
            effective.contains_key("access.self.read"),
            "but keeps the baseline"
        );
    }

    const MIGRATION_0130: &str = include_str!("../../migrations/0130_retag_system_groups.sql");

    /// The user's group rows as `role:source`, sorted.
    async fn role_rows(pool: &PgPool, user: &str) -> Vec<String> {
        sqlx::query_scalar(
            "select role_name || ':' || source from access.user_roles where user_id = $1 order by 1",
        )
        .bind(user)
        .fetch_all(pool)
        .await
        .unwrap()
    }

    /// Seeds the rows 0098's backfill left a pre-0098 holder with: SERVER_ADMIN and USER, both
    /// `manual`.
    async fn seed_backfilled_admin(pool: &PgPool, user: &str) {
        sqlx::query(
            "insert into access.user_roles (user_id, role_name, source) \
             values ($1, 'SERVER_ADMIN', 'manual'), ($1, 'USER', 'manual')",
        )
        .bind(user)
        .execute(pool)
        .await
        .unwrap();
    }

    /// #805 AC1: an admin configured before 0098 and since removed from `OIS_SERVER_ADMIN_CID` is
    /// demoted at their next sign-in. The backfilled row was `manual`, which `revoke_server_admin` used
    /// to skip; it now removes SERVER_ADMIN of any source, and 0130 has re-tagged the row besides.
    #[sqlx::test]
    async fn a_backfilled_admin_no_longer_configured_is_demoted_at_sign_in(pool: PgPool) {
        let user = seed_user(&pool).await;
        seed_backfilled_admin(&pool, &user).await;
        grant(&pool, &user, "tmu.program.update", None).await;
        sqlx::raw_sql(MIGRATION_0130).execute(&pool).await.unwrap();

        super::reconcile_login_access(&pool, &user, 1_000_001, false, &[1_000_002])
            .await
            .unwrap();

        assert_eq!(
            role_rows(&pool, &user).await,
            ["USER:system"],
            "SERVER_ADMIN is gone and only the baseline is left"
        );
        let direct: i64 =
            sqlx::query_scalar("select count(*) from access.user_permissions where user_id = $1")
                .bind(&user)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            direct, 0,
            "the demotion wipes direct grants, as for any demotion"
        );
    }

    /// #805 AC2, the sign-in half: a backfilled admin who is still configured keeps SERVER_ADMIN,
    /// including one who signed in after 0098 and so holds a `system` twin beside the backfilled row.
    #[sqlx::test]
    async fn a_backfilled_admin_still_configured_keeps_server_admin_at_sign_in(pool: PgPool) {
        let twin = seed_user(&pool).await;
        seed_backfilled_admin(&pool, &twin).await;
        sqlx::query(
            "insert into access.user_roles (user_id, role_name, source) \
             values ($1, 'SERVER_ADMIN', 'system')",
        )
        .bind(&twin)
        .execute(&pool)
        .await
        .unwrap();
        let backfill_only = seed_user(&pool).await;
        seed_backfilled_admin(&pool, &backfill_only).await;
        sqlx::raw_sql(MIGRATION_0130).execute(&pool).await.unwrap();

        let admins = [1_000_001, 1_000_002];
        super::reconcile_login_access(&pool, &twin, 1_000_001, false, &admins)
            .await
            .unwrap();
        super::reconcile_login_access(&pool, &backfill_only, 1_000_002, false, &admins)
            .await
            .unwrap();

        for user in [&twin, &backfill_only] {
            assert_eq!(
                role_rows(&pool, user).await,
                ["SERVER_ADMIN:system", "USER:system"]
            );
        }
    }

    /// A user with a CID, as a real account has.
    async fn user_with_cid(pool: &PgPool, cid: Option<i64>) -> String {
        sqlx::query_scalar(
            "insert into identity.users (cid, full_name, display_name) values ($1, 'U', 'U') \
             returning id",
        )
        .bind(cid)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn hold(pool: &PgPool, user: &str, role: &str, source: &str) {
        sqlx::query(
            "insert into access.user_roles (user_id, role_name, source) values ($1, $2, $3)",
        )
        .bind(user)
        .bind(role)
        .bind(source)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn direct_count(pool: &PgPool, user: &str) -> i64 {
        sqlx::query_scalar("select count(*) from access.user_permissions where user_id = $1")
            .bind(user)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// #805: the startup pass demotes every server admin whose CID is not configured, whatever the
    /// row's source and with no CID at all, exactly as sign-in would: baseline group, no direct
    /// grants. The configured admin, and a user who never held SERVER_ADMIN, are left as they were.
    #[sqlx::test]
    async fn the_startup_pass_demotes_every_unconfigured_server_admin(pool: PgPool) {
        let configured = user_with_cid(&pool, Some(1_000_001)).await;
        hold(&pool, &configured, "SERVER_ADMIN", "system").await;
        let mut former = Vec::new();
        for (cid, source) in [
            (Some(1_000_002), "system"),
            (Some(1_000_003), "manual"),
            (Some(1_000_004), "vatusa"),
            (None, "system"),
        ] {
            let user = user_with_cid(&pool, cid).await;
            hold(&pool, &user, "SERVER_ADMIN", source).await;
            grant(&pool, &user, "tmu.program.update", None).await;
            former.push(user);
        }
        let bystander = user_with_cid(&pool, Some(1_000_005)).await;
        hold(&pool, &bystander, "EC", "manual").await;
        grant(&pool, &bystander, "tmu.program.update", None).await;

        let demoted = super::demote_unconfigured_server_admins(&pool, &[1_000_001])
            .await
            .unwrap();

        assert_eq!(demoted, 4);
        assert_eq!(role_rows(&pool, &configured).await, ["SERVER_ADMIN:system"]);
        for user in &former {
            assert_eq!(role_rows(&pool, user).await, ["USER:system"]);
            assert_eq!(direct_count(&pool, user).await, 0);
        }
        assert_eq!(role_rows(&pool, &bystander).await, ["EC:manual"]);
        assert_eq!(direct_count(&pool, &bystander).await, 1);
        assert_eq!(
            super::demote_unconfigured_server_admins(&pool, &[1_000_001])
                .await
                .unwrap(),
            0,
            "a second start changes nothing"
        );
    }

    /// #805: no CID configured means no server admin, so an empty list demotes everyone.
    #[sqlx::test]
    async fn an_empty_admin_list_demotes_every_server_admin_at_startup(pool: PgPool) {
        let admin = user_with_cid(&pool, Some(1_000_001)).await;
        hold(&pool, &admin, "SERVER_ADMIN", "system").await;

        assert_eq!(
            super::demote_unconfigured_server_admins(&pool, &[])
                .await
                .unwrap(),
            1
        );
        assert_eq!(role_rows(&pool, &admin).await, ["USER:system"]);
    }

    /// Sign-in reconciles against the configured admin list. A source scan, because a test cannot set
    /// `OIS_SERVER_ADMIN_CID` without racing every other test.
    #[test]
    fn sign_in_reconciles_against_the_configured_admin_list() {
        let source = include_str!("auth.rs");
        let wrapper = &source[source.find("async fn ensure_user_login_access").unwrap()..];
        let wrapper = &wrapper[..wrapper.find("\n}\n").unwrap()];
        assert!(
            wrapper.contains("&configured_server_admin_cids()"),
            "ensure_user_login_access must pass the configured list"
        );
    }

    /// `/me` could contradict itself (VATUSA/OIS#543): `tmu_national` came from the scoped resolver,
    /// which never read denies, while `permissions` came from the view, which did. So a national
    /// grant plus a deny reported `tmu_national: true` beside a permission tree that omitted the
    /// very same permission. One resolver means the two cannot disagree.
    #[sqlx::test]
    async fn a_denied_reader_is_neither_national_nor_in_the_permission_tree(pool: PgPool) {
        let user = seed_user(&pool).await;
        // The allow comes from a role: `access.user_permissions` is unique on
        // `(user_id, permission_name, coalesce(artcc_id, ''))`, so a direct allow and a direct deny
        // cannot both exist at national scope. In practice a deny always overrides a role-derived
        // grant, which is exactly the shape #542 creates.
        sqlx::query("insert into access.roles (name) values ('TMU_TEST') on conflict do nothing")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "insert into access.role_permissions (role_name, permission_name) values ('TMU_TEST', $1)",
        )
        .bind(TMU_READ_PERMISSION)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("insert into access.user_roles (user_id, role_name, source) values ($1, 'TMU_TEST', 'manual')")
            .bind(&user)
            .execute(&pool)
            .await
            .unwrap();
        crate::scope_test_support::deny_scoped(&pool, &user, TMU_READ_PERMISSION, None).await;

        let state = test_state(pool, HashMap::new());
        let body = build_me_body(&state, &current_user(&user)).await.unwrap();

        assert!(
            !body.tmu_national,
            "a denied permission cannot still read as national authority"
        );
        let names = access_repo::fetch_user_permission_names(state.db.as_ref().unwrap(), &user)
            .await
            .unwrap();
        assert!(
            !names.iter().any(|n| n == TMU_READ_PERMISSION),
            "and it must be absent from the permission tree too — the two answers must agree"
        );
    }
}
