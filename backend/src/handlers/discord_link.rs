//! Discord account linking — proves a user owns a Discord account and records the mapping in
//! `integration.external_sync_mappings`, so the bot can act on their behalf. No session is created;
//! the flow runs inside the user's existing OIS session (the callback reads the session cookie).

use axum::{
    Json,
    extract::{Extension, Query, State},
    http::StatusCode,
    response::Redirect,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::{
    auth::{
        context::CurrentUser,
        discord::{DiscordOAuthConfig, exchange_code_for_token, fetch_identity},
    },
    config::{cookie_secure, ois_public_url},
    errors::ApiError,
    handlers::auth::validate_return_to,
    models::DiscordLinkBody,
    repos::integration as integration_repo,
    state::AppState,
};

const LINK_STATE_COOKIE: &str = "ois_discord_link_state";
const LINK_RETURN_COOKIE: &str = "ois_discord_link_return";
const STATE_TTL_SECS: i64 = 10 * 60;

fn pool(state: &AppState) -> Result<&sqlx::PgPool, ApiError> {
    state.db.as_ref().ok_or(ApiError::ServiceUnavailable)
}

#[derive(Deserialize)]
pub struct LinkStartQuery {
    return_to: Option<String>,
}

#[derive(Deserialize)]
pub struct LinkCallbackQuery {
    code: Option<String>,
    state: Option<String>,
}

#[utoipa::path(
    get, path = "/api/v1/me/discord", tag = "integration",
    responses((status = 200, body = DiscordLinkBody), (status = 401))
)]
pub async fn get_my_discord(
    State(state): State<AppState>,
    Extension(current_user): Extension<Option<CurrentUser>>,
) -> Result<Json<DiscordLinkBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let link = integration_repo::get_discord_link(pool(&state)?, &user.id).await?;
    Ok(Json(match link {
        Some((discord_id, meta)) => DiscordLinkBody {
            linked: true,
            username: meta
                .get("username")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
            discord_id: Some(discord_id),
        },
        None => DiscordLinkBody {
            linked: false,
            discord_id: None,
            username: None,
        },
    }))
}

/// Kick off the OAuth dance: set a state cookie, remember where to land, redirect to Discord.
pub async fn start_discord_link(
    Extension(current_user): Extension<Option<CurrentUser>>,
    jar: CookieJar,
    Query(query): Query<LinkStartQuery>,
) -> Result<(CookieJar, Redirect), ApiError> {
    current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let config = DiscordOAuthConfig::from_env().ok_or(ApiError::ServiceUnavailable)?;

    let oauth_state = Uuid::new_v4().to_string();
    let authorize_url = config.authorization_url(&oauth_state)?;

    let mut jar = jar.add(link_cookie(LINK_STATE_COOKIE, oauth_state));
    if let Some(return_to) = query
        .return_to
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        // Origin-checked against the CORS allowlist; a bad value just means "use the default landing".
        if let Ok(valid) = validate_return_to(return_to) {
            jar = jar.add(link_cookie(LINK_RETURN_COOKIE, valid));
        }
    }
    Ok((jar, Redirect::temporary(&authorize_url)))
}

/// Discord redirects the browser here. We're still inside the user's OIS session (session cookie),
/// so the mapping is recorded against them. Always lands back on a page, never a raw error.
pub async fn discord_link_callback(
    State(state): State<AppState>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    jar: CookieJar,
    Query(query): Query<LinkCallbackQuery>,
) -> (CookieJar, Redirect) {
    let landing = landing_url(&jar);
    let result = complete_link(&state, current_user.as_ref(), &jar, &query).await;
    let cleared = jar
        .remove(Cookie::from(LINK_STATE_COOKIE))
        .remove(Cookie::from(LINK_RETURN_COOKIE));

    let target = match result {
        Ok(()) => append_flag(&landing, "discord=linked"),
        Err(_) => append_flag(&landing, "discord=error"),
    };
    (cleared, Redirect::to(&target))
}

async fn complete_link(
    state: &AppState,
    current_user: Option<&CurrentUser>,
    jar: &CookieJar,
    query: &LinkCallbackQuery,
) -> Result<(), ApiError> {
    let user = current_user.ok_or(ApiError::Unauthorized)?;
    let config = DiscordOAuthConfig::from_env().ok_or(ApiError::ServiceUnavailable)?;

    let code = query
        .code
        .as_deref()
        .filter(|c| !c.is_empty())
        .ok_or(ApiError::BadRequest)?;
    let callback_state = query.state.as_deref().unwrap_or_default();
    let cookie_state = jar
        .get(LINK_STATE_COOKIE)
        .map(|c| c.value().to_string())
        .unwrap_or_default();
    if cookie_state.is_empty() || cookie_state != callback_state {
        tracing::warn!("discord link callback state mismatch");
        return Err(ApiError::BadRequest);
    }

    let token = exchange_code_for_token(&config, code).await?;
    let identity = fetch_identity(&token).await?;
    let metadata = json!({
        "username": identity.global_name.clone().unwrap_or_else(|| identity.username.clone()),
        "handle": identity.username,
    });
    integration_repo::upsert_discord_link(pool(state)?, &user.id, &identity.id, &metadata).await?;
    Ok(())
}

#[utoipa::path(
    delete, path = "/api/v1/me/discord", tag = "integration",
    responses((status = 204), (status = 401), (status = 404))
)]
pub async fn unlink_discord(
    State(state): State<AppState>,
    Extension(current_user): Extension<Option<CurrentUser>>,
) -> Result<StatusCode, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    if integration_repo::delete_discord_link(pool(&state)?, &user.id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

fn link_cookie(name: &'static str, value: String) -> Cookie<'static> {
    Cookie::build((name, value))
        .http_only(true)
        .secure(cookie_secure())
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(time::Duration::seconds(STATE_TTL_SECS))
        .build()
}

/// Where to send the browser after the callback: the remembered return_to, else the public /profile,
/// else a relative /profile.
fn landing_url(jar: &CookieJar) -> String {
    if let Some(c) = jar.get(LINK_RETURN_COOKIE) {
        return c.value().to_string();
    }
    ois_public_url()
        .map(|base| format!("{}/profile", base.trim_end_matches('/')))
        .unwrap_or_else(|| "/profile".to_string())
}

fn append_flag(url: &str, flag: &str) -> String {
    let sep = if url.contains('?') { '&' } else { '?' };
    format!("{url}{sep}{flag}")
}
