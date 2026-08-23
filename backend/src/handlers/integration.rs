//! Discord integration endpoints: the outbound-job queue the bot drains (lease/ack, gated
//! `integration.jobs.update`), and the guild config the operators edit (`discord.config.*`).

use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;

use crate::{
    auth::{
        context::CurrentUser,
        permissions::{DiscordConfigRead, DiscordConfigUpdate, IntegrationJobsUpdate},
        require_permission::RequirePermission,
    },
    errors::ApiError,
    models::{
        AceRequestBody, AckJobRequest, DiscordAceClaimRequest, DiscordConfigBody, DiscordLinkBody,
        OutboundJobBody, UpsertDiscordConfigRequest,
    },
    repos::{ace as ace_repo, events as events_repo, integration as integration_repo},
    state::AppState,
};

fn pool(state: &AppState) -> Result<&sqlx::PgPool, ApiError> {
    state.db.as_ref().ok_or(ApiError::ServiceUnavailable)
}

#[derive(Deserialize)]
pub struct LeaseQuery {
    /// Max jobs to lease (default 10, clamped 1–100).
    limit: Option<i64>,
}

#[utoipa::path(
    post, path = "/api/v1/integration/jobs/lease", tag = "integration",
    params(("limit" = Option<i64>, Query, description = "Max jobs (default 10)")),
    responses((status = 200, body = Vec<OutboundJobBody>), (status = 401))
)]
pub async fn lease_jobs(
    State(state): State<AppState>,
    _permission: RequirePermission<IntegrationJobsUpdate>,
    Query(q): Query<LeaseQuery>,
) -> Result<Json<Vec<OutboundJobBody>>, ApiError> {
    let limit = q.limit.unwrap_or(10);
    Ok(Json(
        integration_repo::lease_jobs(pool(&state)?, limit).await?,
    ))
}

#[utoipa::path(
    post, path = "/api/v1/integration/jobs/{id}/ack", tag = "integration",
    params(("id" = String, Path)), request_body = AckJobRequest,
    responses((status = 204), (status = 401), (status = 404))
)]
pub async fn ack_job(
    State(state): State<AppState>,
    _permission: RequirePermission<IntegrationJobsUpdate>,
    Path(id): Path<String>,
    Json(payload): Json<AckJobRequest>,
) -> Result<StatusCode, ApiError> {
    let ok = integration_repo::ack_job(
        pool(&state)?,
        &id,
        payload.success,
        payload.result.as_ref(),
        payload.error.as_deref(),
    )
    .await?;
    if ok {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

// --- current user's Discord link (read-only; sourced from VATUSA) ---

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
        Some((discord_id, _meta)) => DiscordLinkBody {
            linked: true,
            discord_id: Some(discord_id),
            // Username isn't provided by VATUSA — only the id.
            username: None,
        },
        None => DiscordLinkBody {
            linked: false,
            discord_id: None,
            username: None,
        },
    }))
}

// --- interaction callbacks (bot acts on behalf of the linked user) ---

#[utoipa::path(
    post, path = "/api/v1/integration/discord/ace/{id}/claim", tag = "integration",
    params(("id" = String, Path)), request_body = DiscordAceClaimRequest,
    responses(
        (status = 200, body = AceRequestBody), (status = 401),
        (status = 403, description = "Discord account not linked to an OIS user"),
        (status = 404), (status = 409)
    )
)]
pub async fn discord_ace_claim(
    State(state): State<AppState>,
    _permission: RequirePermission<IntegrationJobsUpdate>,
    Path(id): Path<String>,
    Json(payload): Json<DiscordAceClaimRequest>,
) -> Result<Json<AceRequestBody>, ApiError> {
    let p = pool(&state)?;
    // The clicking Discord user must have linked their OIS account — that's who the claim belongs to.
    let user_id = integration_repo::find_user_by_discord_id(p, &payload.discord_user_id)
        .await?
        .ok_or(ApiError::Forbidden)?;

    // Parse the modal's Zulu HHMM against the request's event window (the bot has no per-message state).
    let request = ace_repo::get_request(p, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let event = events_repo::get(p, request.event_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let start = crate::handlers::ace::parse_hhmm_in_window(
        payload.start_hhmm.as_deref(),
        event.start_time,
        event.end_time,
    );
    let end = crate::handlers::ace::parse_hhmm_in_window(
        payload.end_hhmm.as_deref(),
        event.start_time,
        event.end_time,
    );
    let notes = payload.notes.as_deref().unwrap_or("").trim().to_string();

    let mut tx = p.begin().await.map_err(|_| ApiError::Internal)?;
    let (slots, count) =
        ace_repo::claim_request(&mut tx, &id, &user_id, &notes, start, end).await?;
    crate::handlers::ace::enqueue_notify(&mut tx, p, &id, slots, count).await?;
    tx.commit().await.map_err(|_| ApiError::Internal)?;

    ace_repo::get_request(p, &id)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

// --- guild config ---

/// Empty config shown before anything's been saved.
fn empty_config() -> DiscordConfigBody {
    DiscordConfigBody {
        id: None,
        name: String::new(),
        guild_id: String::new(),
        channels: Vec::new(),
        roles: Vec::new(),
        categories: Vec::new(),
    }
}

#[utoipa::path(
    get, path = "/api/v1/integration/discord", tag = "integration",
    responses((status = 200, body = DiscordConfigBody), (status = 401))
)]
pub async fn get_discord_config(
    State(state): State<AppState>,
    _permission: RequirePermission<DiscordConfigRead>,
) -> Result<Json<DiscordConfigBody>, ApiError> {
    Ok(Json(
        integration_repo::get_config(pool(&state)?)
            .await?
            .unwrap_or_else(empty_config),
    ))
}

#[utoipa::path(
    put, path = "/api/v1/integration/discord", tag = "integration",
    request_body = UpsertDiscordConfigRequest,
    responses((status = 200, body = DiscordConfigBody), (status = 400), (status = 401))
)]
pub async fn put_discord_config(
    State(state): State<AppState>,
    _permission: RequirePermission<DiscordConfigUpdate>,
    Json(payload): Json<UpsertDiscordConfigRequest>,
) -> Result<Json<DiscordConfigBody>, ApiError> {
    let p = pool(&state)?;
    if payload.name.trim().is_empty() || payload.guild_id.trim().is_empty() {
        return Err(ApiError::BadRequest);
    }
    integration_repo::upsert_config(p, &payload).await?;
    Ok(Json(
        integration_repo::get_config(p)
            .await?
            .unwrap_or_else(empty_config),
    ))
}
