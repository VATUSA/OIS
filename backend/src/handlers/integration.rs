//! Discord integration endpoints: the outbound-job queue the bot drains (lease/ack, gated
//! `integration.jobs.update`), and the guild config the operators edit (`discord.config.*`).

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;

use crate::{
    auth::{
        permissions::{DiscordConfigRead, DiscordConfigUpdate, IntegrationJobsUpdate},
        require_permission::RequirePermission,
    },
    errors::ApiError,
    models::{AckJobRequest, DiscordConfigBody, OutboundJobBody, UpsertDiscordConfigRequest},
    repos::integration as integration_repo,
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
