//! TMU handlers — Traffic Management Initiatives (TMIs).

use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::{
    auth::{
        context::{CurrentApiKey, CurrentUser},
        permissions::{
            TmuAdvCreate, TmuAdvPublish, TmuAdvRead, TmuAdvUpdate, TmuGroundStopCreate,
            TmuGroundStopDelete, TmuGroundStopPublish, TmuGroundStopRead, TmuProgramDelete,
            TmuProgramRead, TmuProgramUpdate, TmuTmiCreate, TmuTmiDelete, TmuTmiPublish,
            TmuTmiRead, TmuTmiUpdate,
        },
        principal::Principal,
        require_permission::RequirePermission,
    },
    errors::ApiError,
    handlers::restriction_artcc,
    models::{
        AdvisoryBody, CreateAdvisoryRequest, CreateGroundStopRequest, CreateTmiRequest, GateRule,
        GroundStopBody, ProgramBody, PublishGroundStopRequest, TmiBody, UpdateAdvisoryRequest,
        UpdateTmiRequest, UpsertProgramRequest,
    },
    repos::{integration as integration_repo, tmu as tmu_repo},
    state::AppState,
};
use serde_json::json;

/// Logical channel name (mapped to a snowflake in the Discord config) where NTML rows are posted.
///
/// Separate from `tmu-advisories` (#436): NTML is a chronological log of restrictions, and real
/// vATCSCC advisories are a different artifact entirely. Sharing one channel made the name a
/// misnomer the moment either grew.
pub(crate) const NTML_CHANNEL: &str = "tmu-ntml";

/// Records that a TMI never reached Discord because the logical channel resolves to nothing.
///
/// `channel_id` answering `None` means "don't post", and that is a legitimate state — a deployment
/// with no Discord config, or a guild that has not mapped this channel. It is also exactly what a
/// typo, a removed mapping or a renamed constant looks like, and the caller still returns 200 either
/// way. One line here is the difference between a diagnosable gap and a channel nobody notices has
/// gone quiet (#436 review).
fn skipped_post(channel: &str, what: &str, tmi_id: &str) {
    tracing::warn!(
        channel,
        tmi = tmi_id,
        "tmu: no Discord channel mapped for `{channel}`; TMI {what} not posted"
    );
}

/// The `tmi_publish` job payload: the assembled NTML row plus what the bot needs to post it.
///
/// Shared because there are three paths that post one — a TMI published directly, one materialized
/// from an event package (`handlers/events.rs`), and the corrected row an edit posts (#453) — and
/// they previously built this object separately. A corrected row must be assembled exactly like the
/// original; the two drifting is how the channel ends up showing a line the row never had. The bot
/// stays a dumb renderer: everything about NTML's shape is decided here (#436).
pub(crate) fn tmi_publish_job(channel_id: &str, tmi: &TmiBody) -> serde_json::Value {
    json!({
        "channel_id": channel_id,
        "tmi_id": tmi.id,
        // Stamped now, because this is the moment it is being logged.
        "ntml": crate::tmi::ntml_line(
            chrono::Utc::now(),
            &tmi.restriction,
            Some(tmi.start_time),
            tmi.stop_time,
            Some(&tmi.requesting),
            Some(&tmi.providing),
        ),
    })
}

#[derive(Debug, Default, Deserialize)]
pub struct TmiListQuery {
    status: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    facility: Option<String>,
    /// Active-during range (RFC 3339); a TMI matches when its window overlaps `[from, to]`.
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
}

/// Trim to a non-empty owned value, else `None`.
fn clean(v: Option<String>) -> Option<String> {
    v.as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

impl TmiListQuery {
    fn into_filters(self) -> tmu_repo::TmiFilters {
        tmu_repo::TmiFilters {
            status: clean(self.status),
            kind: clean(self.kind),
            facility: clean(self.facility),
            from: self.from,
            to: self.to,
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/tmu/tmis",
    tag = "tmu",
    params(
        ("status" = Option<String>, Query, description = "Filter by status (draft|published|expired|cancelled)"),
        ("type" = Option<String>, Query, description = "Filter by structured restriction kind (MIT, MINIT, STOP, …); excludes raw-typed TMIs"),
        ("facility" = Option<String>, Query, description = "Filter to TMIs where this facility is requesting or providing"),
        ("from" = Option<String>, Query, description = "Active-during range start (RFC 3339)"),
        ("to" = Option<String>, Query, description = "Active-during range end (RFC 3339)"),
    ),
    responses((status = 200, body = Vec<TmiBody>), (status = 401))
)]
pub async fn list_tmis(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuTmiRead>,
    Query(query): Query<TmiListQuery>,
) -> Result<Json<Vec<TmiBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let mut tmis = tmu_repo::list_tmis(pool, &query.into_filters()).await?;
    restriction_artcc::stamp_tmis(&*state.facilities.read().await, &mut tmis);
    Ok(Json(tmis))
}

#[utoipa::path(
    post,
    path = "/api/v1/tmu/tmis",
    tag = "tmu",
    request_body = CreateTmiRequest,
    responses((status = 200, body = TmiBody), (status = 400), (status = 401))
)]
pub async fn create_tmi(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuTmiCreate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Json(mut payload): Json<CreateTmiRequest>,
) -> Result<Json<TmiBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    payload.requesting = payload.requesting.trim().to_ascii_uppercase();
    payload.providing = payload.providing.trim().to_ascii_uppercase();
    // A structured (form-built) TMI derives its raw line from the fields; a raw TMI uses the text.
    if let Some(s) = &payload.structured {
        if s.element.trim().is_empty() || s.kind.trim().is_empty() {
            return Err(ApiError::BadRequest);
        }
        payload.restriction = crate::tmi::encode(s);
    }
    payload.restriction = payload.restriction.trim().to_string();
    if payload.requesting.is_empty()
        || payload.providing.is_empty()
        || payload.restriction.is_empty()
    {
        return Err(ApiError::BadRequest);
    }

    let id = tmu_repo::create_tmi(pool, &payload, &user.id).await?;
    let mut tmi = tmu_repo::get_tmi(pool, &id)
        .await?
        .ok_or(ApiError::Internal)?;
    restriction_artcc::stamp_tmi(&*state.facilities.read().await, &mut tmi);
    Ok(Json(tmi))
}

#[utoipa::path(
    patch,
    path = "/api/v1/tmu/tmis/{id}",
    tag = "tmu",
    params(("id" = String, Path, description = "TMI id")),
    request_body = UpdateTmiRequest,
    responses((status = 200, body = TmiBody), (status = 400), (status = 401), (status = 404))
)]
pub async fn update_tmi(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuTmiUpdate>,
    Path(id): Path<String>,
    Json(mut payload): Json<UpdateTmiRequest>,
) -> Result<Json<TmiBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if let Some(requesting) = &payload.requesting {
        payload.requesting = Some(requesting.trim().to_ascii_uppercase());
    }
    if let Some(providing) = &payload.providing {
        payload.providing = Some(providing.trim().to_ascii_uppercase());
    }
    // Same rule as `create_tmi`: a structured edit derives its raw line from the fields, so the two
    // cannot disagree. Without this the row kept the new text beside the old breakdown, and "View
    // structured" answered with a restriction that was no longer in force (#452).
    if let Some(structured) = &payload.structured {
        if structured.element.trim().is_empty() || structured.kind.trim().is_empty() {
            return Err(ApiError::BadRequest);
        }
        payload.restriction = Some(crate::tmi::encode(structured));
    }
    // Resolved before the tx, like `publish_tmi`: no channel configured just means "don't post".
    let channel = integration_repo::channel_id(pool, NTML_CHANNEL, None).await?;
    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    let edit = tmu_repo::update_tmi(&mut tx, &id, &payload)
        .await?
        .ok_or(ApiError::NotFound)?;
    let mut tmi = edit.tmi;

    // An edit to a TMI that is already in the channel posts a **revised row** rather than editing the
    // original message. NTML is a chronological log and the later line for an element supersedes the
    // earlier one, which is also the shape a cancellation takes — and editing in place would silently
    // rewrite what a controller has already read. The row carries no "REVISED" marker on purpose: an
    // unmarked later line is how NTML reads, and marking it would have to happen bot-side.
    //
    // Only `published` posts. A `draft` was never in the channel, and `cancelled`/`expired` have had
    // their say — re-posting either would put a dead restriction back at the bottom of the log.
    //
    // And only when the edit actually changed the line. `rows_affected` cannot tell: a COALESCE update
    // setting every column to its current value still affects the row, so an empty `PATCH {}` — a "save"
    // with nothing altered, or a retried request — used to queue a second identical row. In a log whose
    // premise is that a later line supersedes the earlier one, a duplicate reads as a re-issue, and is
    // indistinguishable from one (#453 review).
    //
    // Enqueued in the same tx as the edit, for the reason `publish_tmi` does it: an edit that commits
    // without its job is this bug again, and a job without its edit posts a line the row never had.
    if tmi.status == "published" && edit.line_changed {
        if let Some(channel_id) = &channel {
            let job = tmi_publish_job(channel_id, &tmi);
            integration_repo::enqueue_job(&mut tx, "tmi_publish", &job, Some("tmi"), Some(&tmi.id))
                .await?;
        }
    }
    tx.commit().await.map_err(|_| ApiError::Internal)?;

    restriction_artcc::stamp_tmi(&*state.facilities.read().await, &mut tmi);
    Ok(Json(tmi))
}

#[utoipa::path(
    post,
    path = "/api/v1/tmu/tmis/{id}/publish",
    tag = "tmu",
    params(("id" = String, Path, description = "TMI id")),
    responses((status = 200, body = TmiBody), (status = 401), (status = 409))
)]
pub async fn publish_tmi(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuTmiPublish>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<String>,
) -> Result<Json<TmiBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    // Resolve the target channel before the tx; no config just means "don't post" (skip enqueue).
    // Unscoped: a TMI has requesting/providing ARTCCs but no single "issuing facility", and the TMU
    // channel may be network-wide rather than per-facility (#194).
    let channel = integration_repo::channel_id(pool, NTML_CHANNEL, None).await?;
    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    let mut tmi = tmu_repo::publish_tmi(&mut tx, &id, &user.id)
        .await?
        .ok_or(ApiError::Conflict)?; // not a draft (or absent)
    if let Some(channel_id) = channel {
        // Enqueued in the same tx as the publish: the NTML row can't post without the TMI going live.
        let job = tmi_publish_job(&channel_id, &tmi);
        integration_repo::enqueue_job(&mut tx, "tmi_publish", &job, Some("tmi"), Some(&tmi.id))
            .await?;
    } else {
        // Not an error — a deployment with no Discord config is a real state — but it is
        // indistinguishable from a mapping that is missing by accident, and the publish still
        // answers 200. Without this the only symptom is a channel that quietly went silent (#436
        // review); renaming the logical channel made that case likely across every install at once.
        skipped_post(NTML_CHANNEL, "publish", &tmi.id);
    }
    tx.commit().await.map_err(|_| ApiError::Internal)?;

    restriction_artcc::stamp_tmi(&*state.facilities.read().await, &mut tmi);
    Ok(Json(tmi))
}

#[utoipa::path(
    post,
    path = "/api/v1/tmu/tmis/{id}/cancel",
    tag = "tmu",
    params(("id" = String, Path, description = "TMI id")),
    responses((status = 200, body = TmiBody), (status = 401), (status = 409))
)]
pub async fn cancel_tmi(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuTmiPublish>,
    Path(id): Path<String>,
) -> Result<Json<TmiBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    // The channel the publish actually posted to, not a fresh resolution (#436 review). A cancel row
    // only corrects anything if it lands beside the row it corrects, and re-resolving does not get
    // there: this path passes `None` while `activate_package` passes the event's facility, so
    // `resolve_scoped_id` can answer with a different guild for the same logical name — publishing an
    // event-package TMI to ZNY's channel and cancelling it in VATUSA's. Falling back to a fresh
    // resolution covers a TMI with no publish job at all (never published, or published before this
    // shipped); no config still just means "don't post".
    let channel = match integration_repo::published_channel_for_tmi(pool, &id).await? {
        Some(posted_to) => Some(posted_to),
        None => integration_repo::channel_id(pool, NTML_CHANNEL, None).await?,
    };
    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    if !tmu_repo::cancel_tmi(&mut *tx, &id).await? {
        return Err(ApiError::Conflict);
    }
    let mut tmi = tmu_repo::get_tmi(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if let Some(channel_id) = channel {
        // A cancel is its own NTML row rather than an edit of the original (#436): the channel is a
        // chronological log, and the original entry did happen. Enqueued in the same tx as the
        // cancel, so the post can't exist for a TMI that is still live.
        let job = json!({
            "channel_id": channel_id,
            "tmi_id": tmi.id,
            "ntml": crate::tmi::ntml_cancel_line(
                chrono::Utc::now(),
                &tmi.restriction,
                Some(&tmi.requesting),
                Some(&tmi.providing),
            ),
        });
        integration_repo::enqueue_job(&mut tx, "tmi_cancel", &job, Some("tmi"), Some(&tmi.id))
            .await?;
    } else {
        skipped_post(NTML_CHANNEL, "cancel", &tmi.id);
    }
    tx.commit().await.map_err(|_| ApiError::Internal)?;
    restriction_artcc::stamp_tmi(&*state.facilities.read().await, &mut tmi);
    Ok(Json(tmi))
}

#[utoipa::path(
    delete,
    path = "/api/v1/tmu/tmis/{id}",
    tag = "tmu",
    params(("id" = String, Path, description = "TMI id")),
    responses((status = 204), (status = 401), (status = 404))
)]
pub async fn delete_tmi(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuTmiDelete>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if !tmu_repo::delete_tmi(pool, &id).await? {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

// --- rate programs ---

/// Uppercase alphanumerics only; keep 3–4 char ICAOs, else reject.
fn normalize_icao(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase();
    (cleaned.len() >= 3 && cleaned.len() <= 4).then_some(cleaned)
}

fn clean_alnum(raw: &str, min: usize, max: usize) -> Option<String> {
    let s: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase();
    (s.len() >= min && s.len() <= max).then_some(s)
}

/// Normalize a program payload the way vatflow's `normRate` does: clamp/validate the
/// airport-wide fields (400 on a bad AAR), drop malformed gates/exclusions, and enforce
/// that miles-in-trail overrides minutes-in-trail. Returns the normalized gate list.
fn normalize_program(payload: &mut UpsertProgramRequest) -> Result<Vec<GateRule>, ApiError> {
    if !(1..=200).contains(&payload.aar) {
        return Err(ApiError::BadRequest);
    }
    payload.trail = payload.trail.clamp(0, 60);
    payload.mit = payload.mit.clamp(0, 300);
    if payload.mit > 0 {
        payload.trail = 0; // MIT overrides minutes-in-trail at the airport level.
    }

    let gates: Vec<GateRule> = payload
        .gates
        .iter()
        .filter_map(|g| {
            let name = clean_alnum(&g.name, 1, 8)?;
            let mit = g.mit.clamp(0, 300);
            let trail = if mit > 0 { 0 } else { g.trail.clamp(0, 60) };
            Some(GateRule { name, trail, mit })
        })
        .take(10)
        .collect();

    payload.exclude_wake = payload
        .exclude_wake
        .iter()
        .map(|w| w.trim().to_ascii_uppercase())
        .filter(|w| matches!(w.as_str(), "L" | "M" | "H" | "J"))
        .collect();
    payload.exclude_types = payload
        .exclude_types
        .iter()
        .filter_map(|t| clean_alnum(t, 2, 4))
        .collect();

    Ok(gates)
}

#[utoipa::path(
    get,
    path = "/api/v1/tmu/programs",
    tag = "tmu",
    responses((status = 200, body = Vec<ProgramBody>), (status = 401))
)]
pub async fn list_programs(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuProgramRead>,
) -> Result<Json<Vec<ProgramBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let mut programs = tmu_repo::list_programs(pool).await?;
    restriction_artcc::stamp_programs(&*state.facilities.read().await, &mut programs);
    Ok(Json(programs))
}

#[utoipa::path(
    put,
    path = "/api/v1/tmu/programs/{icao}",
    tag = "tmu",
    params(("icao" = String, Path, description = "Airport ICAO")),
    request_body = UpsertProgramRequest,
    responses((status = 200, body = ProgramBody), (status = 400), (status = 401))
)]
pub async fn upsert_program(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuProgramUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(icao): Path<String>,
    Json(mut payload): Json<UpsertProgramRequest>,
) -> Result<Json<ProgramBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    let icao = normalize_icao(&icao).ok_or(ApiError::BadRequest)?;
    let gates = normalize_program(&mut payload)?;

    tmu_repo::upsert_program(pool, &icao, &payload, &gates, &user.id).await?;
    let mut program = tmu_repo::get_program(pool, &icao)
        .await?
        .ok_or(ApiError::Internal)?;
    restriction_artcc::stamp_program(&*state.facilities.read().await, &mut program);
    Ok(Json(program))
}

#[utoipa::path(
    delete,
    path = "/api/v1/tmu/programs/{icao}",
    tag = "tmu",
    params(("icao" = String, Path, description = "Airport ICAO")),
    responses((status = 204), (status = 401), (status = 404))
)]
pub async fn delete_program(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuProgramDelete>,
    Path(icao): Path<String>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let icao = normalize_icao(&icao).ok_or(ApiError::BadRequest)?;
    if !tmu_repo::delete_program(pool, &icao).await? {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

// --- ground stops ---

/// Normalize the scope: uppercase ARTCC/FIR codes, single-spaced. Empty = field-wide.
fn normalize_scope(raw: Option<&str>) -> String {
    raw.unwrap_or("")
        .split_whitespace()
        .map(|c| c.to_ascii_uppercase())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Normalize an "until" clock time to canonical `HHMM`. Blank -> None (until further
/// notice); anything present but not a valid HHMM -> Err (400).
fn normalize_until(raw: Option<&str>) -> Result<Option<String>, ApiError> {
    let digits: String = raw
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        return Ok(None);
    }
    let padded = if digits.len() == 3 {
        format!("0{digits}")
    } else {
        digits
    };
    if padded.len() != 4 {
        return Err(ApiError::BadRequest);
    }
    let hh: u32 = padded[0..2].parse().map_err(|_| ApiError::BadRequest)?;
    let mm: u32 = padded[2..4].parse().map_err(|_| ApiError::BadRequest)?;
    if hh >= 24 || mm >= 60 {
        return Err(ApiError::BadRequest);
    }
    Ok(Some(padded))
}

#[utoipa::path(
    get,
    path = "/api/v1/tmu/ground-stops",
    tag = "tmu",
    responses((status = 200, body = Vec<GroundStopBody>), (status = 401))
)]
pub async fn list_ground_stops(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuGroundStopRead>,
) -> Result<Json<Vec<GroundStopBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let mut stops = tmu_repo::list_ground_stops(pool).await?;
    restriction_artcc::stamp_ground_stops(&*state.facilities.read().await, &mut stops);
    Ok(Json(stops))
}

#[utoipa::path(
    post,
    path = "/api/v1/tmu/ground-stops",
    tag = "tmu",
    request_body = CreateGroundStopRequest,
    responses((status = 200, body = GroundStopBody), (status = 400), (status = 401))
)]
pub async fn create_ground_stop(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuGroundStopCreate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Json(mut payload): Json<CreateGroundStopRequest>,
) -> Result<Json<GroundStopBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    payload.airport = payload
        .airport
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase();
    if payload.airport.len() < 3 || payload.airport.len() > 4 {
        return Err(ApiError::BadRequest);
    }
    let scope = normalize_scope(payload.scope.as_deref());
    let until = normalize_until(payload.until.as_deref())?;

    let id =
        tmu_repo::create_ground_stop(pool, &payload, &scope, until.as_deref(), &user.id).await?;
    let mut gs = tmu_repo::get_ground_stop(pool, &id)
        .await?
        .ok_or(ApiError::Internal)?;
    restriction_artcc::stamp_ground_stop(&*state.facilities.read().await, &mut gs);
    Ok(Json(gs))
}

#[utoipa::path(
    post,
    path = "/api/v1/tmu/ground-stops/{id}/publish",
    tag = "tmu",
    params(("id" = String, Path, description = "Ground stop id")),
    request_body(
        content = Option<PublishGroundStopRequest>,
        description = "Optional editorial fields for the advisory generated on publish (#508). A ground \
                       stop has no delay data of its own, so its delay triplets are author-supplied."
    ),
    responses((status = 200, body = GroundStopBody), (status = 401), (status = 409))
)]
pub async fn publish_ground_stop(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuGroundStopPublish>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<String>,
    editorial: Option<Json<PublishGroundStopRequest>>,
) -> Result<Json<GroundStopBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let editorial = editorial.map(|Json(e)| e).unwrap_or_default();
    let now = Utc::now();

    // One transaction for the publish and the generated advisory (#508): an advisory must not exist for
    // a stop that did not publish, or the reverse. Simpler than the GDP path -- a ground stop has no
    // slot table, so there is no feed work to keep outside the transaction.
    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;
    if !tmu_repo::publish_ground_stop(&mut *tx, &id, &user.id).await? {
        return Err(ApiError::Conflict); // not a draft (or absent)
    }
    let mut gs = tmu_repo::get_ground_stop(&mut *tx, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    // A stop starts when it is issued -- there is no start column. The end comes from Postgres's own
    // `tmu.ground_stop_until_ts`, the same resolver the cleanup job expires the stop with, so the
    // document cannot state an end the system does not enforce.
    let until = tmu_repo::ground_stop_until_instant(&mut tx, &id).await?;
    let doc = crate::advisory::ground_stop_advisory_from(&gs, (now, until), &editorial, now);
    let req = CreateAdvisoryRequest {
        facility: "DCC".to_string(),
        kind: crate::models::ADVISORY_KIND_GROUND_STOP.to_string(),
        body: String::new(),
        structured: Some(serde_json::to_value(&doc).map_err(|_| ApiError::Internal)?),
        decoded: None,
    };
    tmu_repo::create_advisory_tx(
        &mut tx,
        &req,
        &user.id,
        Some(tmu_repo::AdvisoryProgram::GroundStop(&id)),
    )
    .await?;
    tx.commit().await.map_err(|_| ApiError::Internal)?;

    restriction_artcc::stamp_ground_stop(&*state.facilities.read().await, &mut gs);
    Ok(Json(gs))
}

#[utoipa::path(
    post,
    path = "/api/v1/tmu/ground-stops/{id}/cancel",
    tag = "tmu",
    params(("id" = String, Path, description = "Ground stop id")),
    responses((status = 200, body = GroundStopBody), (status = 401), (status = 409))
)]
pub async fn cancel_ground_stop(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuGroundStopPublish>,
    Path(id): Path<String>,
) -> Result<Json<GroundStopBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if !tmu_repo::cancel_ground_stop(pool, &id).await? {
        return Err(ApiError::Conflict);
    }
    let mut gs = tmu_repo::get_ground_stop(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    restriction_artcc::stamp_ground_stop(&*state.facilities.read().await, &mut gs);
    Ok(Json(gs))
}

#[utoipa::path(
    delete,
    path = "/api/v1/tmu/ground-stops/{id}",
    tag = "tmu",
    params(("id" = String, Path, description = "Ground stop id")),
    responses((status = 204), (status = 401), (status = 404))
)]
pub async fn delete_ground_stop(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuGroundStopDelete>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if !tmu_repo::delete_ground_stop(pool, &id).await? {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    /// #436 review: renaming the logical channel without moving the existing row would have made
    /// `channel_id` answer `None` for every guild that already had one — and `None` means "don't
    /// post", silently, with the publish still returning 200. The migration repoints it, mirroring
    /// `0041_stats_perm_rename.sql`.
    ///
    /// `#[sqlx::test]` applies every migration to a fresh database, so seeding the *old* name here
    /// and finding the new one is only possible if the rename is idempotent — which it must be,
    /// since it runs on databases that never had the old row either.
    #[sqlx::test]
    async fn the_ntml_channel_mapping_survives_the_rename(pool: sqlx::PgPool) {
        let config = sqlx::query_scalar::<_, String>(
            "insert into integration.discord_configs (name, guild_id) values ('g', '1') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        // A guild mapped under the pre-#436 name, as every existing deployment is.
        sqlx::query(
            "insert into integration.discord_channels (config_id, name, channel_id) \
             values ($1, 'tmu-advisories', '999')",
        )
        .bind(&config)
        .execute(&pool)
        .await
        .unwrap();

        // Re-run the rename the migration performs; on a real deploy the migration has already run
        // before this row existed, so applying it here is what reproduces the upgrade order.
        sqlx::query(include_str!(
            "../../migrations/0083_tmu_ntml_channel_rename.sql"
        ))
        .execute(&pool)
        .await
        .unwrap();

        let mapped = crate::repos::integration::channel_id(&pool, NTML_CHANNEL, None)
            .await
            .unwrap();
        assert_eq!(
            mapped,
            Some("999".to_string()),
            "an existing mapping must keep posting without an admin remapping it"
        );
    }

    use super::*;
    use crate::scope_test_support;

    /// Seed a guild with `tmu-ntml` mapped to `channel`, optionally scoped to one ARTCC, and return
    /// its config id.
    async fn guild(
        pool: &sqlx::PgPool,
        name: &str,
        channel: &str,
        sort_order: i32,
        artcc: Option<&str>,
    ) -> String {
        let config = sqlx::query_scalar::<_, String>(
            "insert into integration.discord_configs (name, guild_id, sort_order) \
             values ($1, $2, $3) returning id",
        )
        .bind(name)
        .bind(format!("g-{name}"))
        .bind(sort_order)
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into integration.discord_channels (config_id, name, channel_id) \
             values ($1, 'tmu-ntml', $2)",
        )
        .bind(&config)
        .bind(channel)
        .execute(pool)
        .await
        .unwrap();
        if let Some(artcc) = artcc {
            sqlx::query(
                "insert into integration.discord_config_facilities (config_id, artcc_id) \
                 values ($1, $2)",
            )
            .bind(&config)
            .bind(artcc)
            .execute(pool)
            .await
            .unwrap();
        }
        config
    }

    /// A TMI row, published, with a `tmi_publish` job recording the channel it went to.
    async fn published_tmi(pool: &sqlx::PgPool, id: &str, posted_to: Option<&str>) {
        sqlx::query(
            "insert into tmu.tmis (id, requesting, providing, restriction, status, start_time) \
             values ($1, 'N90', 'ZNY', 'JFK arrivals via CAMRN 20MIT', 'published', now())",
        )
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
        if let Some(channel) = posted_to {
            sqlx::query(
                "insert into integration.outbound_jobs \
                 (job_type, payload, subject_type, subject_id) \
                 values ('tmi_publish', $1::jsonb, 'tmi', $2)",
            )
            .bind(json!({ "channel_id": channel, "tmi_id": id, "ntml": "row" }).to_string())
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
        }
    }

    async fn cancel_jobs(pool: &sqlx::PgPool, tmi_id: &str) -> Vec<(String, String)> {
        sqlx::query_as::<_, (String, String)>(
            "select payload->>'channel_id', payload->>'ntml' \
             from integration.outbound_jobs \
             where job_type = 'tmi_cancel' and subject_id = $1",
        )
        .bind(tmi_id)
        .fetch_all(pool)
        .await
        .unwrap()
    }

    /// Drive the real cancel route: `resolve_current_user`, `RequirePermission<TmuTmiPublish>` and the
    /// handler all on the path (`scope_test_support::send`, VATUSA/OIS#364). The enqueue is a database
    /// side effect, so it is observable afterwards even though `send` only returns the status.
    async fn cancel_through_the_router(
        pool: &sqlx::PgPool,
        tmi_id: &str,
        with_permission: bool,
    ) -> http::StatusCode {
        let user = scope_test_support::seed_user(pool).await;
        if with_permission {
            scope_test_support::grant(pool, &user, "tmu.tmi.publish", None).await;
        }
        let cookie = scope_test_support::session_cookie(pool, &user).await;
        let state = scope_test_support::test_state(pool.clone(), Default::default());
        scope_test_support::send(
            &state,
            http::Method::POST,
            &format!("/api/v1/tmu/tmis/{tmi_id}/cancel"),
            &cookie,
            None,
        )
        .await
    }

    /// AC 3's first half. Nothing covered the enqueue itself: deleting it, or changing the job type
    /// the bot dispatches on, left all 534 tests green while cancellations silently never reached
    /// Discord — which is the bug #436 exists to fix (#436 review).
    #[sqlx::test]
    async fn cancelling_a_tmi_enqueues_a_cancel_post(pool: sqlx::PgPool) {
        guild(&pool, "VATUSA", "CH-VATUSA", 1, None).await;
        published_tmi(&pool, "tmi-1", Some("CH-VATUSA")).await;

        assert_eq!(
            cancel_through_the_router(&pool, "tmi-1", true).await,
            http::StatusCode::OK
        );

        let jobs = cancel_jobs(&pool, "tmi-1").await;
        assert_eq!(jobs.len(), 1, "exactly one tmi_cancel job");
        assert_eq!(jobs[0].0, "CH-VATUSA");
        assert!(
            jobs[0].1.contains("CANCEL TMI") && jobs[0].1.contains("N90:ZNY"),
            "the assembled cancel row, not a placeholder: {}",
            jobs[0].1
        );
    }

    /// AC 3's "**the same** channel". `publish_tmi` resolves the channel unscoped while
    /// `activate_package` resolves it with the event's facility, so with two guilds the same logical
    /// name answers differently — an event-package TMI published to ZNY's channel would have had its
    /// cancellation posted to VATUSA's, leaving the restriction uncorrected where anyone could see it
    /// (#436 review). Proven in SQL before the fix.
    #[sqlx::test]
    async fn a_cancel_posts_to_the_channel_the_publish_used(pool: sqlx::PgPool) {
        // VATUSA sorts first, so an unscoped resolution picks it; ZNY is where the publish went.
        guild(&pool, "VATUSA", "CH-VATUSA", 1, None).await;
        guild(&pool, "ZNY", "CH-ZNY", 2, Some("ZNY")).await;
        published_tmi(&pool, "tmi-2", Some("CH-ZNY")).await;

        cancel_through_the_router(&pool, "tmi-2", true).await;

        assert_eq!(
            cancel_jobs(&pool, "tmi-2").await[0].0,
            "CH-ZNY",
            "the cancel must land beside the row it corrects, not in whichever guild sorts first"
        );
    }

    /// A TMI with no publish job — never published, or published before this shipped — still cancels,
    /// falling back to a fresh resolution rather than posting nowhere.
    #[sqlx::test]
    async fn a_cancel_without_a_publish_job_falls_back_to_the_mapped_channel(pool: sqlx::PgPool) {
        guild(&pool, "VATUSA", "CH-VATUSA", 1, None).await;
        published_tmi(&pool, "tmi-3", None).await;

        cancel_through_the_router(&pool, "tmi-3", true).await;

        assert_eq!(cancel_jobs(&pool, "tmi-3").await[0].0, "CH-VATUSA");
    }

    /// The route is gated: without `tmu.tmi.publish` the cancel is refused and nothing is enqueued, so
    /// dropping the extractor cannot pass unnoticed.
    #[sqlx::test]
    async fn cancelling_without_the_permission_is_refused_and_enqueues_nothing(pool: sqlx::PgPool) {
        guild(&pool, "VATUSA", "CH-VATUSA", 1, None).await;
        published_tmi(&pool, "tmi-4", Some("CH-VATUSA")).await;

        assert_eq!(
            cancel_through_the_router(&pool, "tmi-4", false).await,
            http::StatusCode::UNAUTHORIZED
        );
        assert!(cancel_jobs(&pool, "tmi-4").await.is_empty());
    }

    /// The `ntml` key is a contract with `ois-discord`, which reads exactly that key and now hard-errors
    /// without it. Nothing referenced `tmi_publish_job` but its two call sites, so either side could be
    /// renamed silently (#436 review).
    #[test]
    fn the_publish_payload_carries_the_assembled_row_under_ntml() {
        let now = chrono::Utc::now();
        let tmi = TmiBody {
            id: "tmi-9".into(),
            requesting: "N90".into(),
            providing: "ZNY".into(),
            requesting_artcc: None,
            providing_artcc: None,
            restriction: "JFK arrivals via CAMRN 20MIT".into(),
            start_time: now,
            stop_time: None,
            status: "published".into(),
            published_at: Some(now),
            created_at: now,
            author: None,
            structured: None,
            decoded: None,
        };

        let job = tmi_publish_job("CH-1", &tmi);

        assert_eq!(job["channel_id"], "CH-1");
        assert_eq!(job["tmi_id"], "tmi-9");
        let ntml = job["ntml"]
            .as_str()
            .expect("the bot reads `ntml` and nothing else");
        assert!(ntml.contains("JFK arrivals via CAMRN 20MIT"), "{ntml}");
        assert!(ntml.ends_with("N90:ZNY"), "{ntml}");
    }

    /// A guild that already has both names must not fail the migration: `(config_id, name)` is
    /// unique, so a bare `update` would collide and take the whole deploy's migration down with it.
    /// The existing `tmu-ntml` row is the one that already wins, so it is left alone.
    /// (Caught by mutation: removing the guard left the test above green.)
    #[sqlx::test]
    async fn a_guild_holding_both_channel_names_does_not_break_the_rename(pool: sqlx::PgPool) {
        let config = sqlx::query_scalar::<_, String>(
            "insert into integration.discord_configs (name, guild_id) values ('g', '1') returning id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        for (name, id) in [("tmu-advisories", "111"), ("tmu-ntml", "222")] {
            sqlx::query(
                "insert into integration.discord_channels (config_id, name, channel_id) \
                 values ($1, $2, $3)",
            )
            .bind(&config)
            .bind(name)
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        }

        sqlx::query(include_str!(
            "../../migrations/0083_tmu_ntml_channel_rename.sql"
        ))
        .execute(&pool)
        .await
        .expect("the rename must not collide with an existing tmu-ntml row");

        assert_eq!(
            crate::repos::integration::channel_id(&pool, NTML_CHANNEL, None)
                .await
                .unwrap(),
            Some("222".to_string()),
            "the channel already mapped as tmu-ntml stays the one that wins"
        );
    }

    use std::collections::HashMap;

    use sqlx::PgPool;

    use crate::scope_test_support::{grant, seed_user, send, session_cookie, test_state};

    fn new_advisory() -> serde_json::Value {
        serde_json::json!({"facility": "DCC", "kind": "reroute", "body": "vATCSCC ADVZY"})
    }

    async fn seed_draft(pool: &PgPool) -> String {
        seed_draft_for(pool, "DCC").await
    }

    /// A draft belonging to `facility`.
    ///
    /// The scope tests need a real ARTCC because `access.user_permissions.artcc_id` is a foreign key
    /// to `org.facilities`, so a grant simply cannot be scoped to something absent from it — see
    /// `a_dcc_advisory_is_reachable_only_by_a_national_grant`.
    async fn seed_draft_for(pool: &PgPool, facility: &str) -> String {
        let author = seed_user(pool).await;
        crate::repos::tmu::create_advisory(
            pool,
            &CreateAdvisoryRequest {
                facility: facility.into(),
                kind: "reroute".into(),
                body: "vATCSCC ADVZY".into(),
                structured: None,
                decoded: None,
            },
            &author,
        )
        .await
        .unwrap()
    }

    /// A signed-in caller holding no permissions, and the state to send through.
    ///
    /// Deliberately two steps rather than one refused-then-granted helper: the interesting assertion
    /// is that the *refused* call changed nothing, and that has to be checked before the granted call
    /// runs — a helper that did both first would let the granted call mask it (which it did, on the
    /// first run of these tests).
    async fn caller(pool: &PgPool) -> (crate::state::AppState, String, String) {
        let state = test_state(pool.clone(), HashMap::new());
        let user = seed_user(pool).await;
        let cookie = session_cookie(pool, &user).await;
        (state, user, cookie)
    }

    async fn status_of(pool: &PgPool, id: &str) -> Option<String> {
        crate::repos::tmu::get_advisory(pool, id)
            .await
            .unwrap()
            .map(|a| a.status)
    }

    /// #457 review: `RequirePermission` answers "holds it", not "holds it here". An advisory
    /// carries the issuing facility in its identity, so a grant scoped to one ARTCC must not reach
    /// another's document — otherwise a ZDC controller could cancel ZNY's published advisory.
    #[sqlx::test]
    async fn a_facility_scoped_grant_cannot_touch_another_facilitys_advisory(pool: PgPool) {
        let id = seed_draft_for(&pool, "ZDC").await;
        let (state, user, cookie) = caller(&pool).await;
        grant(&pool, &user, "tmu.adv.publish", Some("ZNY")).await;
        grant(&pool, &user, "tmu.adv.update", Some("ZNY")).await;

        for (method, uri) in [
            (
                http::Method::POST,
                format!("/api/v1/tmu/advisories/{id}/cancel"),
            ),
            (
                http::Method::POST,
                format!("/api/v1/tmu/advisories/{id}/publish"),
            ),
            (http::Method::DELETE, format!("/api/v1/tmu/advisories/{id}")),
        ] {
            assert_eq!(
                send(&state, method.clone(), &uri, &cookie, None).await,
                http::StatusCode::FORBIDDEN,
                "{method} {uri} must be refused for another facility's advisory"
            );
        }
        assert_eq!(
            status_of(&pool, &id).await.as_deref(),
            Some("draft"),
            "a refused call must leave the advisory untouched"
        );
    }

    /// The same grant scoped to the advisory's *own* facility is allowed — the check must gate on
    /// the facility, not simply refuse every scoped grant.
    #[sqlx::test]
    async fn a_grant_scoped_to_the_advisorys_own_facility_is_allowed(pool: PgPool) {
        let id = seed_draft_for(&pool, "ZDC").await;
        let (state, user, cookie) = caller(&pool).await;
        grant(&pool, &user, "tmu.adv.publish", Some("ZDC")).await;

        assert_eq!(
            send(
                &state,
                http::Method::POST,
                &format!("/api/v1/tmu/advisories/{id}/publish"),
                &cookie,
                None,
            )
            .await,
            http::StatusCode::OK
        );
        assert_eq!(status_of(&pool, &id).await.as_deref(), Some("published"));
    }

    /// A DCC advisory is reachable only by a **national** grant, and that falls out of the schema
    /// rather than being a rule anyone wrote: `tmu.advisories.facility` is deliberately not a
    /// foreign key (0085 — the DCC is not an ARTCC in `org.facilities`), while
    /// `access.user_permissions.artcc_id` *is* one. So no grant can be scoped to `DCC` in the first
    /// place, and only an unscoped holder can act on its documents (#457 review).
    ///
    /// That is the right outcome — DCC advisories are national by nature — but it is worth pinning,
    /// because it means facility scoping silently does not apply to a whole class of advisory.
    #[sqlx::test]
    async fn a_dcc_advisory_is_reachable_only_by_a_national_grant(pool: PgPool) {
        let id = seed_draft_for(&pool, "DCC").await;
        let (state, user, cookie) = caller(&pool).await;
        let uri = format!("/api/v1/tmu/advisories/{id}/publish");

        // A grant scoped to a real ARTCC does not reach it.
        grant(&pool, &user, "tmu.adv.publish", Some("ZDC")).await;
        assert_eq!(
            send(&state, http::Method::POST, &uri, &cookie, None).await,
            http::StatusCode::FORBIDDEN
        );

        // A national one does.
        grant(&pool, &user, "tmu.adv.publish", None).await;
        assert_eq!(
            send(&state, http::Method::POST, &uri, &cookie, None).await,
            http::StatusCode::OK
        );
    }

    /// Creating is scoped on the facility in the *body*, which is the only place it exists yet —
    /// so this is what stops a ZDC controller issuing an advisory in another facility's name.
    #[sqlx::test]
    async fn creating_for_another_facility_is_refused(pool: PgPool) {
        let (state, user, cookie) = caller(&pool).await;
        grant(&pool, &user, "tmu.adv.create", Some("ZNY")).await;

        assert_eq!(
            send(
                &state,
                http::Method::POST,
                "/api/v1/tmu/advisories",
                &cookie,
                Some(new_advisory()), // facility DCC
            )
            .await,
            http::StatusCode::FORBIDDEN
        );
    }

    /// AC 1's third verb. `cancel_advisory` shares `TmuAdvPublish` with publish, so publish's test
    /// covered the permission but not *this route's* use of it: dropping the extractor here left 21
    /// passed / 0 failed (#457 review).
    #[sqlx::test]
    async fn cancelling_an_advisory_requires_the_publish_permission(pool: PgPool) {
        let id = seed_draft(&pool).await;
        let (state, user, cookie) = caller(&pool).await;
        let uri = format!("/api/v1/tmu/advisories/{id}/cancel");

        let refused = send(&state, http::Method::POST, &uri, &cookie, None).await;
        assert_eq!(refused, http::StatusCode::UNAUTHORIZED);
        assert_eq!(
            status_of(&pool, &id).await.as_deref(),
            Some("draft"),
            "a refused cancel must not have changed the status"
        );

        grant(&pool, &user, "tmu.adv.publish", None).await;
        let allowed = send(&state, http::Method::POST, &uri, &cookie, None).await;
        assert_eq!(allowed, http::StatusCode::OK);
        assert_eq!(status_of(&pool, &id).await.as_deref(), Some("cancelled"));
    }

    /// The remaining two gated routes, for the same reason: each one's extractor should be removable
    /// only at the cost of a red test (#457 review).
    #[sqlx::test]
    async fn editing_an_advisory_requires_the_update_permission(pool: PgPool) {
        let id = seed_draft(&pool).await;
        let (state, user, cookie) = caller(&pool).await;
        let uri = format!("/api/v1/tmu/advisories/{id}");
        let edit = || Some(serde_json::json!({"body": "vATCSCC ADVZY 001 amended"}));
        let body_now = async || {
            crate::repos::tmu::get_advisory(&pool, &id)
                .await
                .unwrap()
                .unwrap()
                .body
        };

        let refused = send(&state, http::Method::PATCH, &uri, &cookie, edit()).await;
        assert_eq!(refused, http::StatusCode::UNAUTHORIZED);
        assert_eq!(
            body_now().await,
            "vATCSCC ADVZY",
            "a refused edit must not have rewritten the document"
        );

        grant(&pool, &user, "tmu.adv.update", None).await;
        let allowed = send(&state, http::Method::PATCH, &uri, &cookie, edit()).await;
        assert_eq!(allowed, http::StatusCode::OK);
        assert_eq!(body_now().await, "vATCSCC ADVZY 001 amended");
    }

    #[sqlx::test]
    async fn abandoning_an_advisory_requires_the_update_permission(pool: PgPool) {
        let id = seed_draft(&pool).await;
        let (state, user, cookie) = caller(&pool).await;
        let uri = format!("/api/v1/tmu/advisories/{id}");

        let refused = send(&state, http::Method::DELETE, &uri, &cookie, None).await;
        assert_eq!(refused, http::StatusCode::UNAUTHORIZED);
        assert!(
            status_of(&pool, &id).await.is_some(),
            "a refused delete must not have removed it"
        );

        grant(&pool, &user, "tmu.adv.update", None).await;
        let allowed = send(&state, http::Method::DELETE, &uri, &cookie, None).await;
        assert_eq!(allowed, http::StatusCode::NO_CONTENT);
        assert!(status_of(&pool, &id).await.is_none());
    }

    /// #457 AC1. `tmu.adv.*` were seeded in 0008_tmu.sql and dead ever since, because nothing could
    /// gate on them. These go through the real router so `RequirePermission` is on the tested path —
    /// it holds a private field, so a handler cannot be called directly to check its gate.
    ///
    /// A missing permission is **401** here, not 403: `ensure_permission` answers `Unauthorized` for
    /// every absent permission, and 403 is reserved for a wrong facility scope.
    #[sqlx::test]
    async fn creating_an_advisory_requires_the_create_permission(pool: PgPool) {
        let state = test_state(pool.clone(), HashMap::new());
        let user = seed_user(&pool).await;
        let cookie = session_cookie(&pool, &user).await;

        let refused = send(
            &state,
            http::Method::POST,
            "/api/v1/tmu/advisories",
            &cookie,
            Some(new_advisory()),
        )
        .await;
        assert_eq!(refused, http::StatusCode::UNAUTHORIZED);
        assert!(
            crate::repos::tmu::list_advisories(&pool)
                .await
                .unwrap()
                .is_empty(),
            "a refused create must not have written anything"
        );

        grant(&pool, &user, "tmu.adv.create", None).await;
        let allowed = send(
            &state,
            http::Method::POST,
            "/api/v1/tmu/advisories",
            &cookie,
            Some(new_advisory()),
        )
        .await;
        assert_eq!(allowed, http::StatusCode::OK);
        assert_eq!(
            crate::repos::tmu::list_advisories(&pool)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    async fn draft_ground_stop(pool: &PgPool, author: &str, until: Option<&str>) -> String {
        tmu_repo::create_ground_stop(
            pool,
            &CreateGroundStopRequest {
                airport: "KDFW".into(),
                scope: Some("ZHU ZME".into()),
                until: until.map(str::to_string),
            },
            "ZHU ZME",
            until,
            author,
        )
        .await
        .unwrap()
    }

    async fn advisories_for_ground_stop(pool: &PgPool, id: &str) -> Vec<(String, String, String)> {
        sqlx::query_as::<_, (String, String, String)>(
            "select id, status, body from tmu.advisories where ground_stop_id = $1 order by number",
        )
        .bind(id)
        .fetch_all(pool)
        .await
        .unwrap()
    }

    /// AC1/AC3 for a ground stop — publishing generates the document, and it carries the program's own
    /// airport rather than a constant.
    #[sqlx::test]
    async fn publishing_a_ground_stop_generates_its_advisory(pool: PgPool) {
        let state = test_state(pool.clone(), HashMap::new());
        let user = seed_user(&pool).await;
        let cookie = session_cookie(&pool, &user).await;
        grant(&pool, &user, "tmu.groundstop.publish", None).await;
        let id = draft_ground_stop(&pool, &user, Some("1630")).await;

        let status = send(
            &state,
            http::Method::POST,
            &format!("/api/v1/tmu/ground-stops/{id}/publish"),
            &cookie,
            None,
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);

        let advisories = advisories_for_ground_stop(&pool, &id).await;
        assert_eq!(advisories.len(), 1);
        let body = &advisories[0].2;
        assert!(body.contains("CDM GROUND STOP"), "got:\n{body}");
        assert!(
            body.contains("CTL ELEMENT: DFW"),
            "KDFW should print as DFW; got:\n{body}"
        );
        assert!(
            body.contains("1630Z"),
            "the stated end should appear; got:\n{body}"
        );
    }

    /// A ground stop with no stated end runs until further notice. Printing an invented end time would
    /// commit the document to something nobody agreed to.
    #[sqlx::test]
    async fn an_open_ended_ground_stop_renders_ufn(pool: PgPool) {
        let state = test_state(pool.clone(), HashMap::new());
        let user = seed_user(&pool).await;
        let cookie = session_cookie(&pool, &user).await;
        grant(&pool, &user, "tmu.groundstop.publish", None).await;
        let id = draft_ground_stop(&pool, &user, None).await;

        send(
            &state,
            http::Method::POST,
            &format!("/api/v1/tmu/ground-stops/{id}/publish"),
            &cookie,
            None,
        )
        .await;

        let body = advisories_for_ground_stop(&pool, &id).await.remove(0).2;
        assert!(
            body.contains("UFN"),
            "an open-ended stop should say UFN; got:\n{body}"
        );
    }

    /// The document's end must be the instant the system will actually expire the stop at.
    ///
    /// `tmu.ground_stop_until_ts` resolves a bare HHMM relative to `created_at`, so a stop created at
    /// 1500Z with `until` 1430 ends *tomorrow* at 1430 — not today, and not relative to whenever it was
    /// published. Resolving that rule a second time in Rust is how the advisory would come to state an
    /// end nothing enforces, so this pins the document to the database's own answer.
    #[sqlx::test]
    async fn the_stated_end_matches_what_the_system_will_expire(pool: PgPool) {
        let state = test_state(pool.clone(), HashMap::new());
        let user = seed_user(&pool).await;
        let cookie = session_cookie(&pool, &user).await;
        grant(&pool, &user, "tmu.groundstop.publish", None).await;
        let id = draft_ground_stop(&pool, &user, Some("1430")).await;
        // Backdate by whole days, not hours. With a few hours' offset the two readings usually
        // coincide, so the test passed against a now-relative resolver — it did, until this was
        // checked. Three days back puts the system's answer unambiguously in the past, days away from
        // whatever "the next 1430 from now" would be, whatever time the suite happens to run at.
        sqlx::query(
            "update tmu.ground_stops set created_at = now() - interval '3 days' where id = $1",
        )
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();

        send(
            &state,
            http::Method::POST,
            &format!("/api/v1/tmu/ground-stops/{id}/publish"),
            &cookie,
            None,
        )
        .await;

        let expected: Option<chrono::DateTime<Utc>> = sqlx::query_scalar(
            "select tmu.ground_stop_until_ts(created_at, until) from tmu.ground_stops where id = $1",
        )
        .bind(&id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let expected = expected.expect("a stated until resolves to an instant");
        let body = advisories_for_ground_stop(&pool, &id).await.remove(0).2;
        let stamp = expected.format("%d/%H%MZ").to_string();
        assert!(
            body.contains(&stamp),
            "the document should carry the system's own resolved end {stamp}; got:\n{body}"
        );
    }

    /// A ground stop has no delay data anywhere in the system, so the triplets are author-supplied —
    /// this is the test that they actually arrive.
    #[sqlx::test]
    async fn ground_stop_delay_triplets_come_from_the_request(pool: PgPool) {
        let state = test_state(pool.clone(), HashMap::new());
        let user = seed_user(&pool).await;
        let cookie = session_cookie(&pool, &user).await;
        grant(&pool, &user, "tmu.groundstop.publish", None).await;
        let id = draft_ground_stop(&pool, &user, Some("1630")).await;

        send(
            &state,
            http::Method::POST,
            &format!("/api/v1/tmu/ground-stops/{id}/publish"),
            &cookie,
            Some(serde_json::json!({
                "current_delays": "1240/414/81",
                "probability_of_extension": "MEDIUM",
            })),
        )
        .await;

        let body = advisories_for_ground_stop(&pool, &id).await.remove(0).2;
        assert!(body.contains("1240/414/81"), "got:\n{body}");
        assert!(
            body.contains("PROBABILITY OF EXTENSION: MEDIUM"),
            "got:\n{body}"
        );
    }

    /// AC1's other direction for a ground stop: a refused publish writes nothing.
    #[sqlx::test]
    async fn a_conflicting_ground_stop_publish_generates_no_advisory(pool: PgPool) {
        let state = test_state(pool.clone(), HashMap::new());
        let user = seed_user(&pool).await;
        let cookie = session_cookie(&pool, &user).await;
        grant(&pool, &user, "tmu.groundstop.publish", None).await;
        let id = draft_ground_stop(&pool, &user, Some("1630")).await;
        let uri = format!("/api/v1/tmu/ground-stops/{id}/publish");

        assert_eq!(
            send(&state, http::Method::POST, &uri, &cookie, None).await,
            http::StatusCode::OK
        );
        assert_eq!(
            send(&state, http::Method::POST, &uri, &cookie, None).await,
            http::StatusCode::CONFLICT
        );

        assert_eq!(advisories_for_ground_stop(&pool, &id).await.len(), 1);
    }

    /// Publishing is a separate permission from creating: drafting a document and issuing it are
    /// different levels of trust, which is why 0008_tmu.sql seeded them separately.
    #[sqlx::test]
    async fn publishing_an_advisory_requires_the_publish_permission(pool: PgPool) {
        let state = test_state(pool.clone(), HashMap::new());
        let id = seed_draft(&pool).await;

        let user = seed_user(&pool).await;
        let cookie = session_cookie(&pool, &user).await;
        let uri = format!("/api/v1/tmu/advisories/{id}/publish");

        // Holding create is not holding publish.
        grant(&pool, &user, "tmu.adv.create", None).await;
        assert_eq!(
            send(&state, http::Method::POST, &uri, &cookie, None).await,
            http::StatusCode::UNAUTHORIZED
        );
        let still_draft = crate::repos::tmu::get_advisory(&pool, &id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(still_draft.status, "draft");

        grant(&pool, &user, "tmu.adv.publish", None).await;
        assert_eq!(
            send(&state, http::Method::POST, &uri, &cookie, None).await,
            http::StatusCode::OK
        );
        let published = crate::repos::tmu::get_advisory(&pool, &id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(published.status, "published");
        assert!(published.published_at.is_some());
    }

    /// Reading is gated too — advisories are not public until #459 posts them.
    #[sqlx::test]
    async fn listing_advisories_requires_the_read_permission(pool: PgPool) {
        let state = test_state(pool.clone(), HashMap::new());
        let user = seed_user(&pool).await;
        let cookie = session_cookie(&pool, &user).await;

        assert_eq!(
            send(
                &state,
                http::Method::GET,
                "/api/v1/tmu/advisories",
                &cookie,
                None
            )
            .await,
            http::StatusCode::UNAUTHORIZED
        );

        grant(&pool, &user, "tmu.adv.read", None).await;
        assert_eq!(
            send(
                &state,
                http::Method::GET,
                "/api/v1/tmu/advisories",
                &cookie,
                None
            )
            .await,
            http::StatusCode::OK
        );
    }

    /// #452: the handler, not the repo, is what derives a structured edit's raw line — mirroring
    /// `create_tmi`. A repo-level test cannot see that, because it hands the repo both fields; this
    /// sends **only** `structured`, so the restriction can only change if the handler derived it.
    #[sqlx::test]
    async fn a_structured_patch_derives_the_restriction(pool: PgPool) {
        let state = test_state(pool.clone(), HashMap::new());
        let author = seed_user(&pool).await;

        let id = crate::repos::tmu::create_tmi(
            &pool,
            &CreateTmiRequest {
                requesting: "ZDC".into(),
                providing: "ZNY".into(),
                restriction: "JFK arrivals via CAMRN 20MIT".into(),
                structured: None,
                start_time: None,
                stop_time: None,
            },
            &author,
        )
        .await
        .unwrap();

        let editor = seed_user(&pool).await;
        grant(&pool, &editor, "tmu.tmi.update", None).await;
        let cookie = session_cookie(&pool, &editor).await;

        let status = send(
            &state,
            http::Method::PATCH,
            &format!("/api/v1/tmu/tmis/{id}"),
            &cookie,
            Some(serde_json::json!({
                "structured": {
                    "element": "JFK",
                    "direction": "arrivals",
                    "kind": "MIT",
                    "via": "CAMRN",
                    "value": 30
                }
            })),
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);

        let after = crate::repos::tmu::get_tmi(&pool, &id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            after.restriction, "JFK arrivals via CAMRN 30MIT",
            "the raw line must be re-derived from the fields, not left at the old text"
        );
        assert_eq!(after.structured.unwrap().0.value, Some(30));
    }

    /// The same validation `create_tmi` applies: a structured payload with no element is not a
    /// restriction, and must not silently overwrite the stored one with an empty line.
    #[sqlx::test]
    async fn a_structured_patch_with_no_element_is_refused(pool: PgPool) {
        let state = test_state(pool.clone(), HashMap::new());
        let author = seed_user(&pool).await;
        let id = crate::repos::tmu::create_tmi(
            &pool,
            &CreateTmiRequest {
                requesting: "ZDC".into(),
                providing: "ZNY".into(),
                restriction: "JFK arrivals via CAMRN 20MIT".into(),
                structured: None,
                start_time: None,
                stop_time: None,
            },
            &author,
        )
        .await
        .unwrap();

        let editor = seed_user(&pool).await;
        grant(&pool, &editor, "tmu.tmi.update", None).await;
        let cookie = session_cookie(&pool, &editor).await;

        let status = send(
            &state,
            http::Method::PATCH,
            &format!("/api/v1/tmu/tmis/{id}"),
            &cookie,
            Some(serde_json::json!({
                "structured": {"element": "  ", "direction": "arrivals", "kind": "MIT"}
            })),
        )
        .await;
        assert_eq!(status, http::StatusCode::BAD_REQUEST);

        let after = crate::repos::tmu::get_tmi(&pool, &id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.restriction, "JFK arrivals via CAMRN 20MIT");
    }

    #[test]
    fn icao_normalization() {
        assert_eq!(normalize_icao("kjfk").as_deref(), Some("KJFK"));
        assert_eq!(normalize_icao("k jfk!").as_deref(), Some("KJFK"));
        assert_eq!(normalize_icao("zzz").as_deref(), Some("ZZZ"));
        assert_eq!(normalize_icao("xx"), None); // too short
        assert_eq!(normalize_icao("toolong"), None); // too long
    }

    #[test]
    fn clean_alnum_bounds() {
        assert_eq!(clean_alnum("c172", 2, 4).as_deref(), Some("C172"));
        assert_eq!(clean_alnum("pa-28", 2, 4).as_deref(), Some("PA28"));
        assert_eq!(clean_alnum("x", 2, 4), None);
        assert_eq!(clean_alnum("toolong", 2, 4), None);
    }

    #[test]
    fn tmi_list_query_into_filters_trims_and_drops_blanks() {
        let q = TmiListQuery {
            status: Some("  published ".into()),
            kind: Some("MIT".into()),
            facility: Some("   ".into()), // blank → None
            from: None,
            to: None,
        };
        let f = q.into_filters();
        assert_eq!(f.status.as_deref(), Some("published"));
        assert_eq!(f.kind.as_deref(), Some("MIT"));
        assert_eq!(f.facility, None);

        // An empty query yields no constraints.
        let f = TmiListQuery::default().into_filters();
        assert!(
            f.status.is_none()
                && f.kind.is_none()
                && f.facility.is_none()
                && f.from.is_none()
                && f.to.is_none()
        );
    }

    #[test]
    fn scope_normalization() {
        assert_eq!(normalize_scope(Some("ztl  zjx")), "ZTL ZJX");
        assert_eq!(normalize_scope(Some("  zdc ")), "ZDC");
        assert_eq!(normalize_scope(None), "");
    }

    #[test]
    fn until_normalization() {
        assert_eq!(
            normalize_until(Some("200z")).unwrap().as_deref(),
            Some("0200")
        );
        assert_eq!(
            normalize_until(Some("1430")).unwrap().as_deref(),
            Some("1430")
        );
        assert_eq!(normalize_until(Some("")).unwrap(), None);
        assert_eq!(normalize_until(None).unwrap(), None);
        assert!(normalize_until(Some("9999")).is_err()); // hour 99
        assert!(normalize_until(Some("2460")).is_err()); // hour 24
        assert!(normalize_until(Some("1275")).is_err()); // minute 75
    }
}

// --- advisories (ADVZY documents, #457) ---

#[utoipa::path(
    get, path = "/api/v1/tmu/advisories", tag = "tmu",
    responses((status = 200, body = Vec<AdvisoryBody>), (status = 401))
)]
pub async fn list_advisories(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuAdvRead>,
) -> Result<Json<Vec<AdvisoryBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    Ok(Json(tmu_repo::list_advisories(pool).await?))
}

#[utoipa::path(
    get, path = "/api/v1/tmu/advisories/{id}", tag = "tmu",
    params(("id" = String, Path)),
    responses((status = 200, body = AdvisoryBody), (status = 401), (status = 404))
)]
pub async fn get_advisory(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuAdvRead>,
    Path(id): Path<String>,
) -> Result<Json<AdvisoryBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    tmu_repo::get_advisory(pool, &id)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

/// Creates a draft, allocating its advisory number immediately so the author can see what they will
/// issue under (#457).
/// The permission whose ARTCC scope decides who may act on a facility's advisory.
///
/// `tmu.adv.update` and `tmu.adv.publish` are graded separately everywhere else, but the *scope*
/// question is the same one for both: is this principal allowed to act for this facility at all.
/// Checking the permission the caller was already gated on keeps the two answers from diverging.
async fn require_advisory_scope(
    state: &AppState,
    principal: &Principal,
    permission: &str,
    facility: &str,
) -> Result<(), ApiError> {
    if principal
        .permission_scope(state, permission)
        .await?
        .allows(Some(facility))
    {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

/// The facility an existing advisory belongs to, or 404.
///
/// Read before every mutation so the scope check is against the row's own facility rather than
/// anything the caller sent — the body cannot move a document into a facility the caller can reach
/// (#457 review).
async fn advisory_facility(pool: &sqlx::PgPool, id: &str) -> Result<String, ApiError> {
    tmu_repo::get_advisory(pool, id)
        .await?
        .map(|a| a.facility)
        .ok_or(ApiError::NotFound)
}

#[utoipa::path(
    post, path = "/api/v1/tmu/advisories", tag = "tmu",
    request_body = CreateAdvisoryRequest,
    responses((status = 200, body = AdvisoryBody), (status = 400), (status = 401))
)]
pub async fn create_advisory(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuAdvCreate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Json(payload): Json<CreateAdvisoryRequest>,
) -> Result<Json<AdvisoryBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if payload.facility.trim().is_empty()
        || payload.kind.trim().is_empty()
        || payload.body.trim().is_empty()
    {
        return Err(ApiError::BadRequest);
    }
    // On create the facility comes from the body, so this is what stops a ZDC controller issuing
    // an advisory in ZNY's name (#457 review). A national grant allows any.
    require_advisory_scope(
        &state,
        &principal,
        "tmu.adv.create",
        &payload.facility.trim().to_ascii_uppercase(),
    )
    .await?;
    let id = tmu_repo::create_advisory(pool, &payload, &user.id).await?;
    tmu_repo::get_advisory(pool, &id)
        .await?
        .map(Json)
        .ok_or(ApiError::Internal)
}

#[utoipa::path(
    patch, path = "/api/v1/tmu/advisories/{id}", tag = "tmu",
    params(("id" = String, Path)), request_body = UpdateAdvisoryRequest,
    responses((status = 200, body = AdvisoryBody), (status = 401), (status = 404))
)]
pub async fn update_advisory(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuAdvUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path(id): Path<String>,
    Json(payload): Json<UpdateAdvisoryRequest>,
) -> Result<Json<AdvisoryBody>, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let facility = advisory_facility(pool, &id).await?;
    require_advisory_scope(&state, &principal, "tmu.adv.update", &facility).await?;
    if !tmu_repo::update_advisory(pool, &id, &payload).await? {
        return Err(ApiError::NotFound);
    }
    tmu_repo::get_advisory(pool, &id)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

#[utoipa::path(
    post, path = "/api/v1/tmu/advisories/{id}/publish", tag = "tmu",
    params(("id" = String, Path)),
    responses((status = 200, body = AdvisoryBody), (status = 401), (status = 409))
)]
pub async fn publish_advisory(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuAdvPublish>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path(id): Path<String>,
) -> Result<Json<AdvisoryBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let facility = advisory_facility(pool, &id).await?;
    require_advisory_scope(&state, &principal, "tmu.adv.publish", &facility).await?;
    if !tmu_repo::publish_advisory(pool, &id, &user.id).await? {
        return Err(ApiError::Conflict);
    }
    tmu_repo::get_advisory(pool, &id)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

#[utoipa::path(
    post, path = "/api/v1/tmu/advisories/{id}/cancel", tag = "tmu",
    params(("id" = String, Path)),
    responses((status = 200, body = AdvisoryBody), (status = 401), (status = 409))
)]
pub async fn cancel_advisory(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuAdvPublish>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path(id): Path<String>,
) -> Result<Json<AdvisoryBody>, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let facility = advisory_facility(pool, &id).await?;
    require_advisory_scope(&state, &principal, "tmu.adv.publish", &facility).await?;
    if !tmu_repo::cancel_advisory(pool, &id).await? {
        return Err(ApiError::Conflict);
    }
    tmu_repo::get_advisory(pool, &id)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

/// Abandons a draft. `409` when it is not a draft — a published advisory is a document that went out,
/// and a cancelled one is a record of that; neither is deleted. Any draft can be abandoned, whether or
/// not its number is the top of the sequence; abandoning an older one simply leaves a gap (see
/// `repos::tmu::delete_advisory`).
#[utoipa::path(
    delete, path = "/api/v1/tmu/advisories/{id}", tag = "tmu",
    params(("id" = String, Path)),
    responses((status = 204), (status = 401), (status = 409))
)]
pub async fn delete_advisory(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuAdvUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let facility = advisory_facility(pool, &id).await?;
    require_advisory_scope(&state, &principal, "tmu.adv.update", &facility).await?;
    if tmu_repo::delete_advisory(pool, &id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::Conflict)
    }
}

/// VATUSA/OIS#453 (and its duplicate #466): editing a **published** TMI has to correct the channel.
///
/// Driven through the real router rather than by calling the handler, because
/// `RequirePermission<TmuTmiUpdate>` holds a private field and cannot be constructed here — and
/// because a test that called the repo directly would stay green if the handler stopped enqueuing,
/// which is the entire defect. `send` returns only the status, so the assertions that matter read the
/// enqueued rows back out of the same pool.
#[cfg(test)]
mod repost_tests {
    use sqlx::PgPool;

    use super::NTML_CHANNEL;
    use crate::scope_test_support::{self, grant, send, session_cookie, test_state};

    /// Map `NTML_CHANNEL` to a channel, the way `repos::integration`'s own tests do.
    async fn map_tmu_channel(pool: &PgPool) {
        let config_id = sqlx::query_scalar::<_, String>(
            "insert into integration.discord_configs (name, guild_id, sort_order) \
             values ('test-guild', 'test-guild-snowflake', 0) returning id",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into integration.discord_channels (config_id, name, channel_id) \
             values ($1, $2, '1234567890')",
        )
        .bind(&config_id)
        .bind(NTML_CHANNEL)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_tmi(pool: &PgPool, status: &str) -> String {
        sqlx::query_scalar::<_, String>(
            "insert into tmu.tmis \
             (requesting, providing, restriction, status, published_at) \
             values ('N90', 'ZNY', 'JFK arrivals via CAMRN 20MIT', $1, \
                     case when $1 = 'draft' then null else now() end) \
             returning id",
        )
        .bind(status)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// The assembled NTML line of every `tmi_publish` job queued for this TMI, newest last.
    ///
    /// #436 replaced the payload's loose `requesting`/`providing`/`restriction` fields with the single
    /// `ntml` row the bot posts verbatim, so the restriction is now a substring of the line rather than
    /// a column of its own.
    async fn queued_ntml_lines(pool: &PgPool, tmi_id: &str) -> Vec<String> {
        sqlx::query_scalar::<_, String>(
            "select payload->>'ntml' from integration.outbound_jobs \
             where job_type = 'tmi_publish' and subject_type = 'tmi' and subject_id = $1 \
             order by created_at",
        )
        .bind(tmi_id)
        .fetch_all(pool)
        .await
        .unwrap()
    }

    /// PATCH a TMI through the router with an arbitrary body, as a user holding `tmu.tmi.update`.
    async fn patch(pool: PgPool, tmi_id: &str, body: serde_json::Value) -> http::StatusCode {
        let user = scope_test_support::seed_user(&pool).await;
        grant(&pool, &user, "tmu.tmi.update", None).await;
        let cookie = session_cookie(&pool, &user).await;
        let state = test_state(pool, std::collections::HashMap::new());
        send(
            &state,
            http::Method::PATCH,
            &format!("/api/v1/tmu/tmis/{tmi_id}"),
            &cookie,
            Some(body),
        )
        .await
    }

    /// Edit a TMI through the router as a user holding `tmu.tmi.update`.
    async fn patch_restriction(pool: PgPool, tmi_id: &str, restriction: &str) -> http::StatusCode {
        let user = scope_test_support::seed_user(&pool).await;
        grant(&pool, &user, "tmu.tmi.update", None).await;
        let cookie = session_cookie(&pool, &user).await;
        let state = test_state(pool, std::collections::HashMap::new());
        send(
            &state,
            http::Method::PATCH,
            &format!("/api/v1/tmu/tmis/{tmi_id}"),
            &cookie,
            Some(serde_json::json!({ "restriction": restriction })),
        )
        .await
    }

    #[sqlx::test]
    async fn editing_a_published_tmi_enqueues_a_revised_row(pool: PgPool) {
        map_tmu_channel(&pool).await;
        let id = seed_tmi(&pool, "published").await;

        let status = patch_restriction(pool.clone(), &id, "JFK arrivals via CAMRN 30MIT").await;
        assert_eq!(status, http::StatusCode::OK);

        // The new text, not merely "a job exists" — a job carrying the pre-edit line would leave the
        // channel just as wrong while looking like the fix worked.
        let queued = queued_ntml_lines(&pool, &id).await;
        assert_eq!(
            queued.len(),
            1,
            "an edit to a published TMI must queue exactly one revised row, carrying the new line"
        );
        assert!(
            queued[0].contains("JFK arrivals via CAMRN 30MIT"),
            "the queued row must carry the expected restriction, got {:?}",
            queued[0]
        );
    }

    #[sqlx::test]
    async fn editing_a_draft_enqueues_nothing(pool: PgPool) {
        map_tmu_channel(&pool).await;
        let id = seed_tmi(&pool, "draft").await;

        let status = patch_restriction(pool.clone(), &id, "JFK arrivals via CAMRN 30MIT").await;
        assert_eq!(status, http::StatusCode::OK);

        assert!(
            queued_ntml_lines(&pool, &id).await.is_empty(),
            "a draft was never posted, so editing it must not put anything in the channel"
        );
    }

    #[sqlx::test]
    async fn editing_a_cancelled_tmi_enqueues_nothing(pool: PgPool) {
        map_tmu_channel(&pool).await;
        let id = seed_tmi(&pool, "cancelled").await;

        let status = patch_restriction(pool.clone(), &id, "JFK arrivals via CAMRN 30MIT").await;
        assert_eq!(status, http::StatusCode::OK);

        assert!(
            queued_ntml_lines(&pool, &id).await.is_empty(),
            "a cancelled restriction is dead; re-posting it would revive it at the bottom of the log"
        );
    }

    /// #453 review: an edit that changed nothing must not post. `rows_affected` cannot tell — a COALESCE
    /// update setting every column to its current value still affects the row — so an empty `PATCH {}`
    /// queued a second identical line. In a log whose premise is that a later line supersedes the earlier
    /// one, that reads as a re-issue and is indistinguishable from one.
    ///
    /// Both shapes are pinned because they arrive by different routes: an empty body leaves every field
    /// `None`, while a resent value passes COALESCE a value equal to the column's.
    #[sqlx::test]
    async fn an_edit_that_changes_nothing_enqueues_nothing(pool: PgPool) {
        map_tmu_channel(&pool).await;
        let id = seed_tmi(&pool, "published").await;

        assert_eq!(
            patch(pool.clone(), &id, serde_json::json!({})).await,
            http::StatusCode::OK,
            "a no-op edit is not an error, it just has nothing to say to the channel"
        );
        assert!(
            queued_ntml_lines(&pool, &id).await.is_empty(),
            "an empty PATCH must not queue a row"
        );

        assert_eq!(
            patch(
                pool.clone(),
                &id,
                serde_json::json!({ "restriction": "JFK arrivals via CAMRN 20MIT" })
            )
            .await,
            http::StatusCode::OK
        );
        assert!(
            queued_ntml_lines(&pool, &id).await.is_empty(),
            "resending the line the row already has must not queue a duplicate either"
        );
    }

    /// The other half of the same guard: the valid window *is* part of the posted row, so a `stop_time`-only
    /// edit has to post even though the restriction text is untouched. A guard written as "was a restriction
    /// supplied" would silently drop this (#453 review).
    #[sqlx::test]
    async fn editing_only_the_valid_window_still_enqueues(pool: PgPool) {
        map_tmu_channel(&pool).await;
        let id = seed_tmi(&pool, "published").await;

        let status = patch(
            pool.clone(),
            &id,
            serde_json::json!({ "stop_time": "2026-06-01T23:15:00Z" }),
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);

        let queued = queued_ntml_lines(&pool, &id).await;
        assert_eq!(
            queued.len(),
            1,
            "the window changed, so the channel needs the corrected row — carrying the same text"
        );
        assert!(
            queued[0].contains("JFK arrivals via CAMRN 20MIT"),
            "the queued row must carry the expected restriction, got {:?}",
            queued[0]
        );
    }

    /// No channel mapped means "don't post" everywhere else in this file, and an edit is no different
    /// — in particular it must not fail the edit.
    #[sqlx::test]
    async fn an_edit_with_no_channel_mapped_enqueues_nothing(pool: PgPool) {
        let id = seed_tmi(&pool, "published").await;

        let status = patch_restriction(pool.clone(), &id, "JFK arrivals via CAMRN 30MIT").await;
        assert_eq!(status, http::StatusCode::OK);

        assert!(queued_ntml_lines(&pool, &id).await.is_empty());
    }
}
