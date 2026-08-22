//! ACE support: controllers open coverage requests; the ACE team works the queue and keeps a roster.
//! Create/claim enqueue Discord jobs (`ace_request_post` / `ace_request_notify`) in the same tx as the
//! state change; the bot performs them. Enqueue is skipped when no Discord channel is configured.

use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;

use crate::{
    auth::{
        context::CurrentUser,
        permissions::{
            AceRequestsClaim, AceRequestsCreate, AceRequestsDecide, AceRequestsRead, AceTeamRead,
            AceTeamUpdate,
        },
        require_permission::RequirePermission,
    },
    errors::ApiError,
    models::{
        AceRequestBody, AceTeamMemberBody, CreateAceRequestRequest, DecideAceRequestRequest,
        UpsertAceTeamMemberRequest,
    },
    repos::{access as access_repo, ace as ace_repo, integration as integration_repo},
    state::AppState,
};
use serde_json::json;

/// Logical channel name (mapped to a snowflake in the Discord config) where ACE requests are posted.
const ACE_CHANNEL: &str = "aceteam-requests";

fn pool(state: &AppState) -> Result<&sqlx::PgPool, ApiError> {
    state.db.as_ref().ok_or(ApiError::ServiceUnavailable)
}

/// Trim to a non-empty value, or `None`.
fn clean(value: Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned)
}

#[derive(Deserialize)]
pub struct RequestsQuery {
    status: Option<String>,
    artcc_id: Option<String>,
}

#[utoipa::path(
    get, path = "/api/v1/ace/requests", tag = "ace",
    params(
        ("status" = Option<String>, Query, description = "Filter by status"),
        ("artcc_id" = Option<String>, Query, description = "Filter by ARTCC")
    ),
    responses((status = 200, body = Vec<AceRequestBody>), (status = 401))
)]
pub async fn list_requests(
    State(state): State<AppState>,
    _permission: RequirePermission<AceRequestsRead>,
    Query(q): Query<RequestsQuery>,
) -> Result<Json<Vec<AceRequestBody>>, ApiError> {
    let p = pool(&state)?;
    let status = clean(q.status);
    let artcc = clean(q.artcc_id).map(|a| a.to_ascii_uppercase());
    Ok(Json(
        ace_repo::list_requests(p, status.as_deref(), artcc.as_deref()).await?,
    ))
}

#[utoipa::path(
    get, path = "/api/v1/ace/requests/{id}", tag = "ace",
    params(("id" = String, Path)),
    responses((status = 200, body = AceRequestBody), (status = 401), (status = 404))
)]
pub async fn get_request(
    State(state): State<AppState>,
    _permission: RequirePermission<AceRequestsRead>,
    Path(id): Path<String>,
) -> Result<Json<AceRequestBody>, ApiError> {
    ace_repo::get_request(pool(&state)?, &id)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

#[utoipa::path(
    post, path = "/api/v1/ace/requests", tag = "ace",
    request_body = CreateAceRequestRequest,
    responses((status = 200, body = AceRequestBody), (status = 400), (status = 401))
)]
pub async fn create_request(
    State(state): State<AppState>,
    _permission: RequirePermission<AceRequestsCreate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Json(payload): Json<CreateAceRequestRequest>,
) -> Result<Json<AceRequestBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let p = pool(&state)?;
    let details = payload.details.trim();
    if details.is_empty() || details.len() > 4000 {
        return Err(ApiError::BadRequest);
    }
    let artcc = clean(payload.artcc_id).map(|a| a.to_ascii_uppercase());
    let position = clean(payload.position);

    // Resolve the target channel before the tx; a missing config just means "don't post" (skip).
    let channel = integration_repo::channel_id(p, ACE_CHANNEL).await?;
    let mut tx = p.begin().await.map_err(|_| ApiError::Internal)?;
    let id = ace_repo::create_request(
        &mut tx,
        &user.id,
        artcc.as_deref(),
        position.as_deref(),
        payload.requested_for,
        details,
    )
    .await?;
    if let Some(channel_id) = channel {
        // Enqueued in the same tx: no request without its post-job, no post-job without the request.
        let job = json!({
            "channel_id": channel_id,
            "request_id": id,
            "requested_by_cid": user.cid,
            "requested_by_name": user.display_name,
            "artcc_id": artcc,
            "position": position,
            "details": details,
        });
        integration_repo::enqueue_job(
            &mut tx,
            "ace_request_post",
            &job,
            Some("ace_request"),
            Some(&id),
        )
        .await?;
    }
    tx.commit().await.map_err(|_| ApiError::Internal)?;

    ace_repo::get_request(p, &id)
        .await?
        .map(Json)
        .ok_or(ApiError::Internal)
}

#[utoipa::path(
    post, path = "/api/v1/ace/requests/{id}/claim", tag = "ace",
    params(("id" = String, Path)),
    responses((status = 200, body = AceRequestBody), (status = 401), (status = 404), (status = 409))
)]
pub async fn claim_request(
    State(state): State<AppState>,
    _permission: RequirePermission<AceRequestsClaim>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<String>,
) -> Result<Json<AceRequestBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let p = pool(&state)?;

    // Recover the id of the message the bot posted for this request (if any) so the notify job can
    // edit that embed. Read outside the tx — the value is immutable once the post-job succeeded.
    let posted = integration_repo::succeeded_job_result(p, "ace_request", &id, "ace_request_post")
        .await?
        .and_then(|r| {
            r.get("message_id")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        });

    let mut tx = p.begin().await.map_err(|_| ApiError::Internal)?;
    let artcc = ace_repo::claim_request(&mut tx, &id, &user.id).await?;
    if let (Some(message_id), Some(channel_id)) =
        (posted, integration_repo::channel_id(p, ACE_CHANNEL).await?)
    {
        let job = json!({
            "channel_id": channel_id,
            "message_id": message_id,
            "request_id": id,
            "artcc_id": artcc,
            "claimed_by_cid": user.cid,
            "claimed_by_name": user.display_name,
        });
        integration_repo::enqueue_job(
            &mut tx,
            "ace_request_notify",
            &job,
            Some("ace_request"),
            Some(&id),
        )
        .await?;
    }
    tx.commit().await.map_err(|_| ApiError::Internal)?;

    ace_repo::get_request(p, &id)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

#[utoipa::path(
    post, path = "/api/v1/ace/requests/{id}/decide", tag = "ace",
    params(("id" = String, Path)), request_body = DecideAceRequestRequest,
    responses((status = 200, body = AceRequestBody), (status = 400), (status = 401), (status = 404), (status = 409))
)]
pub async fn decide_request(
    State(state): State<AppState>,
    _permission: RequirePermission<AceRequestsDecide>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<String>,
    Json(payload): Json<DecideAceRequestRequest>,
) -> Result<Json<AceRequestBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let p = pool(&state)?;
    if !matches!(payload.outcome.as_str(), "completed" | "cancelled") {
        return Err(ApiError::BadRequest);
    }
    ace_repo::decide_request(p, &id, &user.id, &payload.outcome).await?;
    ace_repo::get_request(p, &id)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

// --- team roster ---

#[derive(Deserialize)]
pub struct TeamQuery {
    /// Include soft-removed (inactive) members.
    all: Option<bool>,
}

#[utoipa::path(
    get, path = "/api/v1/ace/team", tag = "ace",
    params(("all" = Option<bool>, Query, description = "Include inactive members")),
    responses((status = 200, body = Vec<AceTeamMemberBody>), (status = 401))
)]
pub async fn list_team(
    State(state): State<AppState>,
    _permission: RequirePermission<AceTeamRead>,
    Query(q): Query<TeamQuery>,
) -> Result<Json<Vec<AceTeamMemberBody>>, ApiError> {
    Ok(Json(
        ace_repo::list_team(pool(&state)?, !q.all.unwrap_or(false)).await?,
    ))
}

#[utoipa::path(
    put, path = "/api/v1/ace/team", tag = "ace",
    request_body = UpsertAceTeamMemberRequest,
    responses((status = 200, body = Vec<AceTeamMemberBody>), (status = 400), (status = 401))
)]
pub async fn upsert_team_member(
    State(state): State<AppState>,
    _permission: RequirePermission<AceTeamUpdate>,
    Json(payload): Json<UpsertAceTeamMemberRequest>,
) -> Result<Json<Vec<AceTeamMemberBody>>, ApiError> {
    let p = pool(&state)?;
    let user_id = access_repo::find_user_id_by_cid(p, payload.cid)
        .await?
        .ok_or(ApiError::BadRequest)?; // the CID must be a known OIS user
    let role = clean(payload.role);
    let artcc = clean(payload.artcc_id).map(|a| a.to_ascii_uppercase());
    ace_repo::upsert_team_member(
        p,
        &user_id,
        role.as_deref(),
        artcc.as_deref(),
        payload.active.unwrap_or(true),
    )
    .await?;
    Ok(Json(ace_repo::list_team(p, false).await?))
}

#[utoipa::path(
    delete, path = "/api/v1/ace/team/{cid}", tag = "ace",
    params(("cid" = i64, Path, description = "Member's VATSIM CID")),
    responses((status = 204), (status = 401), (status = 404))
)]
pub async fn remove_team_member(
    State(state): State<AppState>,
    _permission: RequirePermission<AceTeamUpdate>,
    Path(cid): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let p = pool(&state)?;
    let user_id = access_repo::find_user_id_by_cid(p, cid)
        .await?
        .ok_or(ApiError::NotFound)?;
    if ace_repo::remove_team_member(p, &user_id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}
