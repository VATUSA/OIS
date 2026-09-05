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
    config::{configured_allowed_origins, configured_server_admin_cids, cookie_secure},
    errors::ApiError,
    models::MeBody,
    repos::{access as access_repo, auth as auth_repo, users as user_repo},
    state::AppState,
};

const OAUTH_STATE_COOKIE: &str = "ois_oauth_state";
const OAUTH_RETURN_TO_COOKIE: &str = "ois_oauth_return_to";
const SESSION_COOKIE: &str = "ois_session";
const OAUTH_STATE_TTL_SECS: i64 = 10 * 60;
const SESSION_TTL_SECS: i64 = 60 * 60 * 24 * 30;
const DEFAULT_LOGIN_REDIRECT: &str = "/api/v1/me";

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

    let redirect_target = jar
        .get(OAUTH_RETURN_TO_COOKIE)
        .map(|cookie| cookie.value().to_string())
        .filter(|value| !value.is_empty())
        .and_then(|value| validate_return_to(&value).ok())
        .unwrap_or_else(|| DEFAULT_LOGIN_REDIRECT.to_string());

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

/// Validates a `return_to` target: absolute http(s) whose origin is in
/// `CORS_ALLOWED_ORIGINS`. Prevents the login flow becoming an open redirect.
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
    if !configured_allowed_origins()
        .iter()
        .any(|allowed| allowed == &origin)
    {
        tracing::warn!(origin, "return_to origin not in CORS_ALLOWED_ORIGINS");
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
