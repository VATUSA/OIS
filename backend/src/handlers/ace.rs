//! ACE support (event-scoped): controllers open coverage requests on an event; people claim slots
//! (one per person) with notes + an availability window inside the event; the ACE team works the queue
//! and keeps a national roster. Create/claim/release enqueue Discord jobs (`ace_request_post` /
//! `ace_request_notify`) in the same tx as the state change; enqueue is skipped when no channel is set.

use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::{
    auth::{
        context::CurrentUser,
        permissions::{AceRequestsClaim, AceRequestsCreate, AceRequestsDecide, EventsPlanRead},
        require_permission::RequirePermission,
    },
    errors::ApiError,
    models::{
        AceRequestBody, ClaimAceRequest, CreateAceRequestRequest, DecideAceRequestRequest,
        EventBody,
    },
    repos::{ace as ace_repo, events as events_repo, integration as integration_repo},
    state::AppState,
};
use serde_json::json;

/// Logical channel name (mapped to a snowflake in the Discord config) where ACE requests are posted.
pub(crate) const ACE_CHANNEL: &str = "aceteam-requests";

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

/// An availability window is valid only when it sits inside the event and start precedes end.
fn validate_window(
    start: Option<DateTime<Utc>>,
    end: Option<DateTime<Utc>>,
    event_start: DateTime<Utc>,
    event_end: DateTime<Utc>,
) -> Result<(), ApiError> {
    if let (Some(s), Some(e)) = (start, end)
        && s >= e
    {
        return Err(ApiError::BadRequest);
    }
    for t in [start, end].into_iter().flatten() {
        if t < event_start || t > event_end {
            return Err(ApiError::BadRequest);
        }
    }
    Ok(())
}

/// Parse a Zulu `HHMM` (or `HH:MM`) into the event window: the time on the event's start date, rolled
/// to the next day if it falls before the window start (a window that crosses midnight). `None` if
/// blank/unparseable or outside `[event_start, event_end]`.
pub(crate) fn parse_hhmm_in_window(
    raw: Option<&str>,
    event_start: DateTime<Utc>,
    event_end: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    use chrono::{Duration, NaiveTime, TimeZone};
    let s = raw?.trim().replace(':', "");
    if s.len() != 4 || !s.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (hh, mm): (u32, u32) = (s[0..2].parse().ok()?, s[2..4].parse().ok()?);
    let time = NaiveTime::from_hms_opt(hh, mm, 0)?;
    let mut cand = Utc.from_utc_datetime(&event_start.date_naive().and_time(time));
    if cand < event_start {
        cand += Duration::days(1);
    }
    (cand >= event_start && cand <= event_end).then_some(cand)
}

/// Zulu HHMM slots across the event window (30-min steps, ≤25 options), plus a `2300–0300z` label —
/// used to build the Discord claim time-selectors so users pick instead of typing.
pub(crate) fn event_time_options(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> (String, Vec<String>) {
    let fmt = |d: DateTime<Utc>| d.format("%H%M").to_string();
    let label = format!("{}–{}z", fmt(start), fmt(end));
    let total_min = (end - start).num_minutes().max(0);
    // Prefer 15-min slots; coarsen if the window would exceed Discord's 25-option select cap.
    // ≤25 options means total_min/step + 1 ≤ 25, i.e. total_min/step < 25.
    let step = [15, 30, 60]
        .into_iter()
        .find(|s| total_min / s < 25)
        .unwrap_or(60);
    let mut opts = Vec::new();
    let mut t = start;
    while t <= end && opts.len() < 25 {
        opts.push(fmt(t));
        t += chrono::Duration::minutes(step);
    }
    if opts.last().map(String::as_str) != Some(fmt(end).as_str()) && opts.len() < 25 {
        opts.push(fmt(end));
    }
    (label, opts)
}

/// Build + enqueue an `ace_request_notify` job in `tx` reflecting the request's current claims, so the
/// bot re-renders the embed ("X/N claimed" + claimers; keeps the button while slots remain). No-op if
/// nothing was posted to Discord (no message id) or no channel is configured.
pub(crate) async fn enqueue_notify(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    p: &sqlx::PgPool,
    request_id: &str,
    slots: i32,
    claims_count: i64,
) -> Result<(), ApiError> {
    let message_id =
        integration_repo::succeeded_job_result(p, "ace_request", request_id, "ace_request_post")
            .await?
            .and_then(|r| {
                r.get("message_id")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
            });
    let (Some(message_id), Some(channel_id)) = (
        message_id,
        integration_repo::channel_id(p, ACE_CHANNEL).await?,
    ) else {
        return Ok(());
    };
    let claims = ace_repo::claims_for(tx, request_id).await?;
    let claimers: Vec<_> = claims
        .iter()
        .map(|c| {
            json!({
                "name": c.display_name,
                "notes": c.notes,
                "start_time": c.start_time,
                "end_time": c.end_time,
            })
        })
        .collect();
    // The request's static fields (unchanged by claiming) let the bot re-render the whole embed; read
    // them from the committed row + the event.
    let request = ace_repo::get_request(p, request_id).await?;
    let (artcc, position, details, event_title) = match request {
        Some(r) => {
            let title = events_repo::get(p, r.event_id)
                .await?
                .map(|e| e.title)
                .unwrap_or_default();
            (r.artcc_id, r.position, r.details, title)
        }
        None => (None, None, String::new(), String::new()),
    };
    let job = json!({
        "channel_id": channel_id,
        "message_id": message_id,
        "request_id": request_id,
        "slots": slots,
        "claims_count": claims_count,
        "filled": claims_count >= slots as i64,
        "artcc_id": artcc,
        "position": position,
        "details": details,
        "event_title": event_title,
        "claimers": claimers,
    });
    integration_repo::enqueue_job(
        tx,
        "ace_request_notify",
        &job,
        Some("ace_request"),
        Some(request_id),
    )
    .await?;
    Ok(())
}

#[derive(Deserialize)]
pub struct RequestsQuery {
    status: Option<String>,
}

#[utoipa::path(
    get, path = "/api/v1/events/{id}/ace", tag = "ace",
    params(("id" = i64, Path), ("status" = Option<String>, Query, description = "Filter by status")),
    responses((status = 200, body = Vec<AceRequestBody>), (status = 401))
)]
pub async fn list_requests(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
    Path(event_id): Path<i64>,
    Query(q): Query<RequestsQuery>,
) -> Result<Json<Vec<AceRequestBody>>, ApiError> {
    let status = clean(q.status);
    Ok(Json(
        ace_repo::list_requests(pool(&state)?, event_id, status.as_deref()).await?,
    ))
}

#[utoipa::path(
    post, path = "/api/v1/events/{id}/ace", tag = "ace",
    params(("id" = i64, Path)), request_body = CreateAceRequestRequest,
    responses((status = 200, body = AceRequestBody), (status = 400), (status = 401), (status = 404))
)]
pub async fn create_request(
    State(state): State<AppState>,
    _permission: RequirePermission<AceRequestsCreate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(event_id): Path<i64>,
    Json(payload): Json<CreateAceRequestRequest>,
) -> Result<Json<AceRequestBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let p = pool(&state)?;
    let details = payload.details.trim();
    if details.is_empty() || details.len() > 4000 || !(1..=99).contains(&payload.slots) {
        return Err(ApiError::BadRequest);
    }
    let event = events_repo::get(p, event_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let artcc = clean(payload.artcc_id).map(|a| a.to_ascii_uppercase());
    let position = clean(payload.position);

    let channel = integration_repo::channel_id(p, ACE_CHANNEL).await?;
    let mut tx = p.begin().await.map_err(|_| ApiError::Internal)?;
    let id = create_one(
        &mut tx,
        &event,
        channel.as_deref(),
        &user.id,
        &user.display_name,
        artcc.as_deref(),
        position.as_deref(),
        payload.slots,
        details,
    )
    .await?;
    tx.commit().await.map_err(|_| ApiError::Internal)?;

    ace_repo::get_request(p, &id)
        .await?
        .map(Json)
        .ok_or(ApiError::Internal)
}

/// Insert one ACE request in the caller's tx and enqueue its `ace_request_post` Discord job (when a
/// channel is configured). Returns the new request id. Shared by the single-create handler and the
/// Tier-1 generator so auto-generated requests behave exactly like hand-created ones.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn create_one(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event: &EventBody,
    channel: Option<&str>,
    requester_id: &str,
    requester_name: &str,
    artcc: Option<&str>,
    position: Option<&str>,
    slots: i32,
    details: &str,
) -> Result<String, ApiError> {
    let id = ace_repo::create_request(tx, event.id, requester_id, artcc, position, slots, details)
        .await?;
    if let Some(channel_id) = channel {
        let job = json!({
            "channel_id": channel_id,
            "request_id": id,
            "requested_by_name": requester_name,
            "artcc_id": artcc,
            "position": position,
            "slots": slots,
            "details": details,
            "event_title": event.title,
            "event_start": event.start_time,
            "event_end": event.end_time,
        });
        integration_repo::enqueue_job(tx, "ace_request_post", &job, Some("ace_request"), Some(&id))
            .await?;
    }
    Ok(id)
}

#[utoipa::path(
    delete, path = "/api/v1/events/{id}/ace/{req}", tag = "ace",
    params(("id" = i64, Path), ("req" = String, Path)),
    responses((status = 204), (status = 401), (status = 404))
)]
pub async fn delete_request(
    State(state): State<AppState>,
    _permission: RequirePermission<AceRequestsDecide>,
    Path((_event_id, req)): Path<(i64, String)>,
) -> Result<StatusCode, ApiError> {
    if ace_repo::delete_request(pool(&state)?, &req).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

#[utoipa::path(
    post, path = "/api/v1/events/{id}/ace/{req}/claim", tag = "ace",
    params(("id" = i64, Path), ("req" = String, Path)), request_body = ClaimAceRequest,
    responses((status = 200, body = AceRequestBody), (status = 400), (status = 401), (status = 404), (status = 409))
)]
pub async fn claim_request(
    State(state): State<AppState>,
    _permission: RequirePermission<AceRequestsClaim>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path((event_id, req)): Path<(i64, String)>,
    Json(payload): Json<ClaimAceRequest>,
) -> Result<Json<AceRequestBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let p = pool(&state)?;

    let request = ace_repo::get_request(p, &req)
        .await?
        .ok_or(ApiError::NotFound)?;
    if request.event_id != event_id {
        return Err(ApiError::NotFound);
    }
    let event = events_repo::get(p, event_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    validate_window(
        payload.start_time,
        payload.end_time,
        event.start_time,
        event.end_time,
    )?;
    let notes = clean(payload.notes).unwrap_or_default();

    let mut tx = p.begin().await.map_err(|_| ApiError::Internal)?;
    let (slots, count) = ace_repo::claim_request(
        &mut tx,
        &req,
        &user.id,
        &notes,
        payload.start_time,
        payload.end_time,
    )
    .await?;
    enqueue_notify(&mut tx, p, &req, slots, count).await?;
    tx.commit().await.map_err(|_| ApiError::Internal)?;

    ace_repo::get_request(p, &req)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

#[utoipa::path(
    delete, path = "/api/v1/events/{id}/ace/{req}/claim", tag = "ace",
    params(("id" = i64, Path), ("req" = String, Path)),
    responses((status = 200, body = AceRequestBody), (status = 401), (status = 404))
)]
pub async fn release_claim(
    State(state): State<AppState>,
    _permission: RequirePermission<AceRequestsClaim>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path((_event_id, req)): Path<(i64, String)>,
) -> Result<Json<AceRequestBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let p = pool(&state)?;

    let mut tx = p.begin().await.map_err(|_| ApiError::Internal)?;
    let (slots, count) = ace_repo::release_claim(&mut tx, &req, &user.id).await?;
    enqueue_notify(&mut tx, p, &req, slots, count).await?;
    tx.commit().await.map_err(|_| ApiError::Internal)?;

    ace_repo::get_request(p, &req)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

#[utoipa::path(
    post, path = "/api/v1/events/{id}/ace/{req}/decide", tag = "ace",
    params(("id" = i64, Path), ("req" = String, Path)), request_body = DecideAceRequestRequest,
    responses((status = 200, body = AceRequestBody), (status = 400), (status = 401), (status = 404), (status = 409))
)]
pub async fn decide_request(
    State(state): State<AppState>,
    _permission: RequirePermission<AceRequestsDecide>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path((_event_id, req)): Path<(i64, String)>,
    Json(payload): Json<DecideAceRequestRequest>,
) -> Result<Json<AceRequestBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let p = pool(&state)?;
    if !matches!(payload.outcome.as_str(), "completed" | "cancelled") {
        return Err(ApiError::BadRequest);
    }
    ace_repo::decide_request(p, &req, &user.id, &payload.outcome).await?;
    ace_repo::get_request(p, &req)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}
