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
        AceRequestBody, AckJobRequest, DiscordAceClaimRequest, DiscordAceInfoBody,
        DiscordAvailabilityRequest, DiscordAvailabilityResult, DiscordConfigBody, DiscordLinkBody,
        EventThreadTemplateBody, OutboundJobBody, PushGuildSnapshotRequest,
        UpsertDiscordConfigRequest, UpsertEventThreadTemplateRequest,
    },
    repos::{
        access as access_repo, ace as ace_repo, availability as availability_repo,
        events as events_repo, integration as integration_repo,
    },
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
    get, path = "/api/v1/integration/discord/ace/{id}", tag = "integration",
    params(("id" = String, Path)),
    responses((status = 200, body = DiscordAceInfoBody), (status = 401), (status = 404))
)]
pub async fn discord_ace_info(
    State(state): State<AppState>,
    _permission: RequirePermission<IntegrationJobsUpdate>,
    Path(id): Path<String>,
) -> Result<Json<DiscordAceInfoBody>, ApiError> {
    let p = pool(&state)?;
    let request = ace_repo::get_request(p, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let event = events_repo::get(p, request.event_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let (window_label, time_options) =
        crate::handlers::ace::event_time_options(event.start_time, event.end_time);
    Ok(Json(DiscordAceInfoBody {
        event_title: event.title,
        window_label,
        slots: request.slots,
        claims_count: request.claims_count,
        time_options,
    }))
}

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

/// Availability button (🟢/🟡/🔴) on a DCC event thread. The bot relays the click; we resolve the
/// Discord user to their linked OIS account and record the response — but only if they actually hold
/// `events.availability.update` (NTMOs / DCC staff). Refusals come back as `ok=false` (never an error
/// status) so the bot can explain why to the user.
#[utoipa::path(
    post, path = "/api/v1/integration/discord/availability/{id}", tag = "integration",
    params(("id" = i64, Path)), request_body = DiscordAvailabilityRequest,
    responses((status = 200, body = DiscordAvailabilityResult), (status = 401), (status = 404))
)]
pub async fn discord_availability(
    State(state): State<AppState>,
    _permission: RequirePermission<IntegrationJobsUpdate>,
    Path(id): Path<i64>,
    Json(payload): Json<DiscordAvailabilityRequest>,
) -> Result<Json<DiscordAvailabilityResult>, ApiError> {
    let p = pool(&state)?;

    let refuse = |reason: &str| {
        Ok(Json(DiscordAvailabilityResult {
            ok: false,
            reason: Some(reason.to_string()),
            display_name: None,
            status: None,
        }))
    };

    // Only the three known button values are accepted.
    if !matches!(
        payload.status.as_str(),
        "available" | "partial" | "unavailable"
    ) {
        return refuse("invalid");
    }

    // The event must still exist (threads can outlive pruned events).
    events_repo::get(p, id).await?.ok_or(ApiError::NotFound)?;

    // Resolve the presser to an OIS user (VATUSA-linked Discord); unlinked users can't respond.
    let Some(user_id) =
        integration_repo::find_user_by_discord_id(p, &payload.discord_user_id).await?
    else {
        return refuse("unlinked");
    };

    // Gate on the resolved user's effective permissions — NTMO / DCC staff by default.
    let perms = access_repo::fetch_user_permission_names(p, &user_id).await?;
    if !perms
        .iter()
        .any(|perm| perm == "events.availability.update")
    {
        return refuse("forbidden");
    }

    availability_repo::set_availability(p, id, &user_id, &payload.status).await?;
    state.publish(crate::realtime::topic::EVENT_AVAILABILITY);
    let display_name = access_repo::user_display_name(p, &user_id).await?;
    Ok(Json(DiscordAvailabilityResult {
        ok: true,
        reason: None,
        display_name,
        status: Some(payload.status),
    }))
}

// --- guild config ---

#[utoipa::path(
    get, path = "/api/v1/integration/discord", tag = "integration",
    responses((status = 200, body = DiscordConfigBody), (status = 401))
)]
pub async fn get_discord_config(
    State(state): State<AppState>,
    _permission: RequirePermission<DiscordConfigRead>,
) -> Result<Json<DiscordConfigBody>, ApiError> {
    Ok(Json(integration_repo::get_config(pool(&state)?).await?))
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
    for g in &payload.guilds {
        if g.name.trim().is_empty() || g.guild_id.trim().is_empty() {
            return Err(ApiError::BadRequest);
        }
    }
    integration_repo::upsert_config(p, &payload).await?;
    Ok(Json(integration_repo::get_config(p).await?))
}

#[utoipa::path(
    get, path = "/api/v1/integration/discord/thread-template", tag = "integration",
    responses((status = 200, body = EventThreadTemplateBody), (status = 401))
)]
pub async fn get_event_thread_template(
    State(state): State<AppState>,
    _permission: RequirePermission<DiscordConfigRead>,
) -> Result<Json<EventThreadTemplateBody>, ApiError> {
    let body = integration_repo::get_event_thread_template(pool(&state)?).await?;
    Ok(Json(EventThreadTemplateBody { body }))
}

#[utoipa::path(
    put, path = "/api/v1/integration/discord/thread-template", tag = "integration",
    request_body = UpsertEventThreadTemplateRequest,
    responses((status = 200, body = EventThreadTemplateBody), (status = 400), (status = 401))
)]
pub async fn put_event_thread_template(
    State(state): State<AppState>,
    _permission: RequirePermission<DiscordConfigUpdate>,
    Json(payload): Json<UpsertEventThreadTemplateRequest>,
) -> Result<Json<EventThreadTemplateBody>, ApiError> {
    if payload.body.trim().is_empty() {
        return Err(ApiError::BadRequest);
    }
    let body =
        integration_repo::set_event_thread_template(pool(&state)?, payload.body.trim()).await?;
    Ok(Json(EventThreadTemplateBody { body }))
}

/// The bot pushes the guilds it's in (channels + roles) so the editor can offer dropdowns. Gated by
/// the bot's `integration.jobs.update` (a human admin never calls this).
#[utoipa::path(
    post, path = "/api/v1/integration/discord/guilds/snapshot", tag = "integration",
    request_body = PushGuildSnapshotRequest,
    responses((status = 204), (status = 401))
)]
pub async fn push_guild_snapshot(
    State(state): State<AppState>,
    _permission: RequirePermission<IntegrationJobsUpdate>,
    Json(payload): Json<PushGuildSnapshotRequest>,
) -> Result<StatusCode, ApiError> {
    integration_repo::replace_guild_snapshots(pool(&state)?, &payload.guilds).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Ask the bot to re-pull the guild snapshot (the admin's "Refresh from Discord" button). Enqueues a
/// `guild_snapshot` job the bot handles by pushing a fresh snapshot. Gated by `discord.config.update`.
#[utoipa::path(
    post, path = "/api/v1/integration/discord/refresh", tag = "integration",
    responses((status = 202), (status = 401))
)]
pub async fn refresh_guild_snapshot(
    State(state): State<AppState>,
    _permission: RequirePermission<DiscordConfigUpdate>,
) -> Result<StatusCode, ApiError> {
    let p = pool(&state)?;
    let mut tx = p.begin().await.map_err(|_| ApiError::Internal)?;
    integration_repo::enqueue_job(
        &mut tx,
        "guild_snapshot",
        &serde_json::json!({}),
        None,
        None,
    )
    .await?;
    tx.commit().await.map_err(|_| ApiError::Internal)?;
    Ok(StatusCode::ACCEPTED)
}
