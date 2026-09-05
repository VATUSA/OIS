//! Ground Delay Program handlers — the program lifecycle (draft → published → cancelled) and
//! the live board (Ration-By-Schedule control times joined to current traffic), computed off
//! the feed. Control times are frozen into `tmu.gdp_slot` at publish so issued EDCTs hold.

use std::collections::HashMap;

use axum::{
    Json,
    extract::{Extension, Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Duration, Utc};

use crate::{
    auth::{
        context::CurrentUser,
        permissions::{TmuGdpCreate, TmuGdpDelete, TmuGdpPublish, TmuGdpRead},
        require_permission::RequirePermission,
    },
    errors::ApiError,
    feed::facilities,
    feed::flow,
    feed::gdp::{self, GdpBoard, GdpFlightView},
    models::{AarStep, CreateGdpRequest, GdpBody, UpdateGdpRequest},
    repos::gdp as gdp_repo,
    state::AppState,
};

/// Parse a required HHMM Zulu clock string to minutes-past-midnight (0–1439).
fn hhmm_to_min(raw: &str) -> Option<i64> {
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    let padded = if digits.len() == 3 {
        format!("0{digits}")
    } else {
        digits
    };
    if padded.len() != 4 {
        return None;
    }
    let hh: i64 = padded[0..2].parse().ok()?;
    let mm: i64 = padded[2..4].parse().ok()?;
    if hh >= 24 || mm >= 60 {
        return None;
    }
    Some(hh * 60 + mm)
}

/// Normalize a required HHMM field to canonical `HHMM`, or 400.
fn norm_hhmm(raw: &str) -> Result<String, ApiError> {
    let m = hhmm_to_min(raw).ok_or(ApiError::BadRequest)?;
    Ok(format!("{:02}{:02}", m / 60, m % 60))
}

/// Normalize a departure scope: uppercase ARTCC codes, single-spaced. Empty = all departures.
fn normalize_scope(raw: Option<&str>) -> String {
    raw.unwrap_or("")
        .split_whitespace()
        .map(|c| c.to_ascii_uppercase())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Resolve the program's HHMM window to concrete timestamps: start = the occurrence of
/// `start` nearest to `now` (±12h), end = the first occurrence of `end` strictly after start.
fn resolve_window(now: DateTime<Utc>, start: &str, end: &str) -> Option<(i64, i64)> {
    let sm = hhmm_to_min(start)?;
    let em = hhmm_to_min(end)?;
    let midnight = now.date_naive().and_hms_opt(0, 0, 0)?.and_utc();
    let today_start = midnight + Duration::minutes(sm);
    // Pick the start occurrence (yesterday/today/tomorrow) closest to now.
    let start_ts = [-1i64, 0, 1]
        .into_iter()
        .map(|d| today_start + Duration::days(d))
        .min_by_key(|c| (*c - now).num_seconds().abs())?;
    let end_midnight = start_ts.date_naive().and_hms_opt(0, 0, 0)?.and_utc();
    let mut end_ts = end_midnight + Duration::minutes(em);
    while end_ts <= start_ts {
        end_ts += Duration::days(1);
    }
    Some((start_ts.timestamp_millis(), end_ts.timestamp_millis()))
}

/// Resolve an AAR-step's HHMM to a concrete instant inside the resolved window (the first
/// occurrence at/after the window start). Steps outside the window are dropped.
fn resolve_step_ms(win_start_ms: i64, win_end_ms: i64, hhmm: &str) -> Option<i64> {
    let m = hhmm_to_min(hhmm)?;
    let ws = DateTime::from_timestamp_millis(win_start_ms)?;
    let base = ws.date_naive().and_hms_opt(0, 0, 0)?.and_utc();
    let mut cand = base + Duration::minutes(m);
    while cand.timestamp_millis() < win_start_ms {
        cand += Duration::days(1);
    }
    (cand.timestamp_millis() <= win_end_ms).then_some(cand.timestamp_millis())
}

/// The program's (possibly time-varying) rate schedule over its resolved window.
fn rate_schedule(gdp: &GdpBody, win_start: i64, win_end: i64) -> gdp::RateSchedule {
    let steps: Vec<(i64, i32)> = gdp
        .aar_steps
        .0
        .iter()
        .filter_map(|s| resolve_step_ms(win_start, win_end, &s.start_time).map(|ms| (ms, s.aar)))
        .collect();
    gdp::RateSchedule::new(gdp.aar, win_start, &steps)
}

/// Validate AAR steps: each a valid HHMM + AAR in 1..=200 that lands inside the program
/// window (a step outside the window would be silently inert). Returns canonical HHMM steps.
fn validate_steps(
    steps: &[AarStep],
    win_start: i64,
    win_end: i64,
) -> Result<Vec<AarStep>, ApiError> {
    steps
        .iter()
        .map(|s| {
            if !(1..=200).contains(&s.aar) {
                return Err(ApiError::BadRequest);
            }
            let hhmm = norm_hhmm(&s.start_time)?;
            if resolve_step_ms(win_start, win_end, &hhmm).is_none() {
                return Err(ApiError::BadRequest); // outside the window → rejected
            }
            Ok(AarStep {
                start_time: hhmm,
                aar: s.aar,
            })
        })
        .collect()
}

/// Project the live feed into GDP inbounds for `icao` (raw classification + ETA + ETD +
/// departure ARTCC, no metering). Airborne/ground/proposed only — arrived flights dropped.
async fn live_inbounds(state: &AppState, icao: &str, now: DateTime<Utc>) -> Vec<gdp::Inbound> {
    let (snapshot, airports) = {
        let guard = state.feed.read().await;
        (guard.snapshot.clone(), guard.airports.clone())
    };
    let Some(snap) = snapshot else {
        return Vec::new();
    };
    let flow = flow::compute(
        icao,
        None, // no metering — we want the raw arrival picture
        &snap.data,
        airports.as_ref(),
        state.winds.load_full().as_ref(),
        state.aircraft_profiles.load_full().as_ref(),
        &HashMap::new(),
        now,
    );
    // Resolve each origin field's owning ARTCC once, memoized across shared departures.
    let map = state.facilities.read().await;
    let mut artcc_of: HashMap<String, Option<String>> = HashMap::new();
    flow.flights
        .into_iter()
        .filter(|f| f.status != "arrived")
        .filter_map(|f| {
            let dep_artcc = artcc_of
                .entry(f.dep.clone())
                .or_insert_with(|| facilities::artcc_for_airport(&map, &f.dep.to_ascii_uppercase()))
                .clone();
            Some(gdp::Inbound {
                cs: f.callsign,
                dep: f.dep,
                dep_artcc,
                status: f.status,
                eta_ms: f.eta?.timestamp_millis(),
                etd_ms: f.etd.map(|e| e.timestamp_millis()),
            })
        })
        .collect()
}

fn ms(v: i64, fallback: DateTime<Utc>) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(v).unwrap_or(fallback)
}

/// Fresh RBS off the current feed for `gdp` — no frozen overrides applied.
async fn fresh_assignments(
    state: &AppState,
    gdp: &GdpBody,
    now: DateTime<Utc>,
) -> Result<(i64, i64, Vec<gdp::Assignment>), ApiError> {
    let (win_start, win_end) =
        resolve_window(now, &gdp.start_time, &gdp.end_time).ok_or(ApiError::Internal)?;
    let inbounds = live_inbounds(state, &gdp.airport.to_ascii_uppercase(), now).await;
    let scope: Vec<String> = gdp.scope.split_whitespace().map(String::from).collect();
    let rates = rate_schedule(gdp, win_start, win_end);
    let assignments = gdp::ration_by_schedule(
        inbounds,
        &rates,
        win_start,
        win_end,
        gdp.exempt_airborne,
        &scope,
        gdp.max_enroute_min,
    );
    Ok((win_start, win_end, assignments))
}

/// Freeze control times: run fresh RBS off the current feed with the program's current
/// params and persist the controlled slots (replacing any existing ones). Used on publish
/// and when revising a published program.
async fn freeze_slots(
    state: &AppState,
    pool: &sqlx::PgPool,
    gdp: &GdpBody,
    now: DateTime<Utc>,
) -> Result<(), ApiError> {
    let (_s, _e, assignments) = fresh_assignments(state, gdp, now).await?;
    let slots: Vec<gdp_repo::GdpSlotRow> = assignments
        .iter()
        .filter(|a| a.controlled)
        .map(|a| gdp_repo::GdpSlotRow {
            callsign: a.cs.clone(),
            dep: a.dep.clone(),
            original_eta: ms(a.original_eta_ms, now),
            cta: ms(a.cta_ms, now),
            edct: a.edct_ms.map(|v| ms(v, now)),
            delay_min: a.delay_min as i32,
        })
        .collect();
    gdp_repo::replace_slots(pool, &gdp.id, &slots).await
}

/// Run RBS off the current feed for `gdp`, applying frozen control times when published.
async fn assign_live(
    state: &AppState,
    pool: &sqlx::PgPool,
    gdp: &GdpBody,
    now: DateTime<Utc>,
) -> Result<(i64, i64, Vec<gdp::Assignment>), ApiError> {
    let (win_start, win_end, mut assignments) = fresh_assignments(state, gdp, now).await?;
    // Freeze: once published, matched flights hold their persisted control times.
    if gdp.status == "published" {
        let frozen = gdp_repo::list_slots(pool, &gdp.id).await?;
        let by_cs: HashMap<&str, &gdp_repo::GdpSlotRow> =
            frozen.iter().map(|s| (s.callsign.as_str(), s)).collect();
        for a in &mut assignments {
            if let Some(s) = by_cs.get(a.cs.as_str()) {
                a.cta_ms = s.cta.timestamp_millis();
                a.edct_ms = s.edct.map(|e| e.timestamp_millis());
                a.delay_min = s.delay_min as i64;
                a.controlled = true;
                a.frozen = true;
                a.exempt_reason = None;
            }
        }
    }
    Ok((win_start, win_end, assignments))
}

/// Assemble the board for `gdp` off the live feed.
async fn build_board(
    state: &AppState,
    pool: &sqlx::PgPool,
    gdp: &GdpBody,
) -> Result<GdpBoard, ApiError> {
    let now = Utc::now();
    let (win_start, win_end, assignments) = assign_live(state, pool, gdp, now).await?;
    let rates = rate_schedule(gdp, win_start, win_end);
    let demand = gdp::demand_bins(&assignments, win_start, win_end, &rates);
    let stats = gdp::program_stats(&assignments);

    let mut flights = Vec::new();
    let mut exempt = Vec::new();
    for a in assignments {
        let view = GdpFlightView {
            cs: a.cs,
            dep: a.dep,
            status: a.status,
            eta: ms(a.original_eta_ms, now),
            cta: ms(a.cta_ms, now),
            edct: a.edct_ms.map(|v| ms(v, now)),
            delay_min: a.delay_min,
            controlled: a.controlled,
            frozen: a.frozen,
            exempt_reason: a.exempt_reason,
        };
        if view.controlled {
            flights.push(view);
        } else {
            exempt.push(view);
        }
    }
    flights.sort_by_key(|f| f.cta);
    exempt.sort_by_key(|e| e.eta);

    Ok(GdpBoard {
        id: gdp.id.clone(),
        airport: gdp.airport.clone(),
        aar: gdp.aar,
        scope: gdp.scope.clone(),
        status: gdp.status.clone(),
        start_time: gdp.start_time.clone(),
        end_time: gdp.end_time.clone(),
        window_start: ms(win_start, now),
        window_end: ms(win_end, now),
        max_enroute_min: gdp.max_enroute_min,
        exempt_airborne: gdp.exempt_airborne,
        aar_steps: gdp.aar_steps.0.clone(),
        published: gdp.status == "published",
        flights,
        exempt,
        demand,
        stats,
    })
}

#[utoipa::path(
    get,
    path = "/api/v1/tmu/gdp",
    tag = "tmu",
    responses((status = 200, body = Vec<GdpBody>), (status = 401), (status = 503))
)]
pub async fn list_gdps(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuGdpRead>,
) -> Result<Json<Vec<GdpBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    Ok(Json(gdp_repo::list_gdps(pool).await?))
}

#[utoipa::path(
    post,
    path = "/api/v1/tmu/gdp",
    tag = "tmu",
    request_body = CreateGdpRequest,
    responses((status = 200, body = GdpBody), (status = 400), (status = 401), (status = 503))
)]
pub async fn create_gdp(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuGdpCreate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Json(payload): Json<CreateGdpRequest>,
) -> Result<Json<GdpBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    let airport: String = payload
        .airport
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase();
    if airport.len() < 3 || airport.len() > 4 {
        return Err(ApiError::BadRequest);
    }
    if !(1..=200).contains(&payload.aar) {
        return Err(ApiError::BadRequest);
    }
    let start = norm_hhmm(&payload.start_time)?;
    let end = norm_hhmm(&payload.end_time)?;
    let scope = normalize_scope(payload.scope.as_deref());
    let max_enroute = payload.max_enroute_min.filter(|m| *m > 0);
    let (ws, we) = resolve_window(Utc::now(), &start, &end).ok_or(ApiError::BadRequest)?;
    let steps = validate_steps(&payload.aar_steps, ws, we)?;

    let id = gdp_repo::create_gdp(
        pool,
        &airport,
        payload.aar,
        &scope,
        &start,
        &end,
        max_enroute,
        payload.exempt_airborne,
        &steps,
        &user.id,
    )
    .await?;
    let gdp = gdp_repo::get_gdp(pool, &id)
        .await?
        .ok_or(ApiError::Internal)?;
    Ok(Json(gdp))
}

/// Revise a GDP — change the AAR, window, tier, scope, or airborne policy. On a published
/// program this re-rations off the live feed and re-freezes control times (EDCTs may shift);
/// on a draft it just updates the parameters. Airport is immutable.
#[utoipa::path(
    put,
    path = "/api/v1/tmu/gdp/{id}",
    tag = "tmu",
    params(("id" = String, Path, description = "GDP id")),
    request_body = UpdateGdpRequest,
    responses((status = 200, body = GdpBoard), (status = 400), (status = 401), (status = 404), (status = 409), (status = 503))
)]
pub async fn revise_gdp(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuGdpCreate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<String>,
    Json(payload): Json<UpdateGdpRequest>,
) -> Result<Json<GdpBoard>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;

    if !(1..=200).contains(&payload.aar) {
        return Err(ApiError::BadRequest);
    }
    let start = norm_hhmm(&payload.start_time)?;
    let end = norm_hhmm(&payload.end_time)?;
    let scope = normalize_scope(payload.scope.as_deref());
    let max_enroute = payload.max_enroute_min.filter(|m| *m > 0);
    let (ws, we) = resolve_window(Utc::now(), &start, &end).ok_or(ApiError::BadRequest)?;
    let steps = validate_steps(&payload.aar_steps, ws, we)?;

    // 404 if absent, 409 if terminal (expired/cancelled — nothing to revise).
    if gdp_repo::get_gdp(pool, &id).await?.is_none() {
        return Err(ApiError::NotFound);
    }
    if !gdp_repo::update_gdp(
        pool,
        &id,
        payload.aar,
        &scope,
        &start,
        &end,
        max_enroute,
        payload.exempt_airborne,
        &steps,
        &user.id,
    )
    .await?
    {
        return Err(ApiError::Conflict); // present but terminal
    }

    let gdp = gdp_repo::get_gdp(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    // A live program re-rations + re-freezes with the new parameters.
    if gdp.status == "published" {
        freeze_slots(&state, pool, &gdp, Utc::now()).await?;
    }
    Ok(Json(build_board(&state, pool, &gdp).await?))
}

#[utoipa::path(
    get,
    path = "/api/v1/tmu/gdp/{id}/board",
    tag = "tmu",
    params(("id" = String, Path, description = "GDP id")),
    responses((status = 200, body = GdpBoard), (status = 401), (status = 404), (status = 503))
)]
pub async fn get_gdp_board(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuGdpRead>,
    Path(id): Path<String>,
) -> Result<Json<GdpBoard>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let gdp = gdp_repo::get_gdp(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(build_board(&state, pool, &gdp).await?))
}

#[utoipa::path(
    post,
    path = "/api/v1/tmu/gdp/{id}/publish",
    tag = "tmu",
    params(("id" = String, Path, description = "GDP id")),
    responses((status = 200, body = GdpBoard), (status = 401), (status = 409), (status = 503))
)]
pub async fn publish_gdp(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuGdpPublish>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<String>,
) -> Result<Json<GdpBoard>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if !gdp_repo::publish_gdp(pool, &id, &user.id).await? {
        return Err(ApiError::Conflict); // not a draft (or absent)
    }
    let gdp = gdp_repo::get_gdp(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;

    // Freeze control times off the current feed so issued EDCTs hold.
    freeze_slots(&state, pool, &gdp, Utc::now()).await?;

    Ok(Json(build_board(&state, pool, &gdp).await?))
}

#[utoipa::path(
    post,
    path = "/api/v1/tmu/gdp/{id}/cancel",
    tag = "tmu",
    params(("id" = String, Path, description = "GDP id")),
    responses((status = 200, body = GdpBody), (status = 401), (status = 409), (status = 503))
)]
pub async fn cancel_gdp(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuGdpPublish>,
    Path(id): Path<String>,
) -> Result<Json<GdpBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if !gdp_repo::cancel_gdp(pool, &id).await? {
        return Err(ApiError::Conflict);
    }
    let gdp = gdp_repo::get_gdp(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(gdp))
}

#[utoipa::path(
    delete,
    path = "/api/v1/tmu/gdp/{id}",
    tag = "tmu",
    params(("id" = String, Path, description = "GDP id")),
    responses((status = 204), (status = 401), (status = 404), (status = 503))
)]
pub async fn delete_gdp(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuGdpDelete>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if !gdp_repo::delete_gdp(pool, &id).await? {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Lock a controlled flight's current (advisory) control time into a frozen slot. Used to
/// pin a pop-up that appeared after publish so its EDCT stops drifting.
#[utoipa::path(
    post,
    path = "/api/v1/tmu/gdp/{id}/slots/{callsign}",
    tag = "tmu",
    params(
        ("id" = String, Path, description = "GDP id"),
        ("callsign" = String, Path, description = "Flight callsign")
    ),
    responses((status = 200, body = GdpBoard), (status = 401), (status = 404), (status = 409), (status = 503))
)]
pub async fn lock_slot(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuGdpPublish>,
    Path((id, callsign)): Path<(String, String)>,
) -> Result<Json<GdpBoard>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let gdp = gdp_repo::get_gdp(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if gdp.status != "published" {
        return Err(ApiError::Conflict); // only a live program has slots to lock
    }
    let now = Utc::now();
    let callsign = callsign.to_ascii_uppercase();
    // Freeze the flight's current advisory assignment (fresh RBS, no existing overrides).
    let (_s, _e, assignments) = fresh_assignments(&state, &gdp, now).await?;
    let a = assignments
        .iter()
        .find(|a| a.controlled && a.cs.eq_ignore_ascii_case(&callsign))
        .ok_or(ApiError::NotFound)?; // not an eligible controlled flight
    gdp_repo::upsert_slot(
        pool,
        &gdp.id,
        &gdp_repo::GdpSlotRow {
            callsign: a.cs.clone(),
            dep: a.dep.clone(),
            original_eta: ms(a.original_eta_ms, now),
            cta: ms(a.cta_ms, now),
            edct: a.edct_ms.map(|v| ms(v, now)),
            delay_min: a.delay_min as i32,
        },
    )
    .await?;
    Ok(Json(build_board(&state, pool, &gdp).await?))
}

/// Unlock (remove) a frozen slot — the flight reverts to a live advisory control time.
#[utoipa::path(
    delete,
    path = "/api/v1/tmu/gdp/{id}/slots/{callsign}",
    tag = "tmu",
    params(
        ("id" = String, Path, description = "GDP id"),
        ("callsign" = String, Path, description = "Flight callsign")
    ),
    responses((status = 200, body = GdpBoard), (status = 401), (status = 404), (status = 503))
)]
pub async fn unlock_slot(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuGdpPublish>,
    Path((id, callsign)): Path<(String, String)>,
) -> Result<Json<GdpBoard>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let gdp = gdp_repo::get_gdp(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if !gdp_repo::delete_slot(pool, &gdp.id, &callsign.to_ascii_uppercase()).await? {
        return Err(ApiError::NotFound);
    }
    Ok(Json(build_board(&state, pool, &gdp).await?))
}

/// Compress the program: reclaim capacity freed by departed/cancelled flights by pulling each
/// frozen slot to its current fresh-RBS time — earlier only, never later than already issued.
/// Frozen flights that have left the arrival picture are dropped.
#[utoipa::path(
    post,
    path = "/api/v1/tmu/gdp/{id}/compress",
    tag = "tmu",
    params(("id" = String, Path, description = "GDP id")),
    responses((status = 200, body = GdpBoard), (status = 401), (status = 404), (status = 409), (status = 503))
)]
pub async fn compress_gdp(
    State(state): State<AppState>,
    _permission: RequirePermission<TmuGdpPublish>,
    Path(id): Path<String>,
) -> Result<Json<GdpBoard>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let gdp = gdp_repo::get_gdp(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if gdp.status != "published" {
        return Err(ApiError::Conflict);
    }
    let now = Utc::now();
    let (_s, _e, fresh) = fresh_assignments(&state, &gdp, now).await?;
    let frozen = gdp_repo::list_slots(pool, &gdp.id).await?;
    let frozen_cta: HashMap<&str, i64> = frozen
        .iter()
        .map(|s| (s.callsign.as_str(), s.cta.timestamp_millis()))
        .collect();

    // Keep only flights that are both currently controllable and previously frozen; pull each
    // to its compressed time. Frozen flights no longer inbound simply fall away.
    let mut new_slots = Vec::new();
    for a in &fresh {
        if !a.controlled {
            continue;
        }
        let Some(&frozen_cta_ms) = frozen_cta.get(a.cs.as_str()) else {
            continue;
        };
        let (cta_ms, edct_ms, delay_min) = gdp::compress_slot(a, frozen_cta_ms);
        new_slots.push(gdp_repo::GdpSlotRow {
            callsign: a.cs.clone(),
            dep: a.dep.clone(),
            original_eta: ms(a.original_eta_ms, now),
            cta: ms(cta_ms, now),
            edct: edct_ms.map(|v| ms(v, now)),
            delay_min: delay_min as i32,
        });
    }
    gdp_repo::replace_slots(pool, &gdp.id, &new_slots).await?;
    Ok(Json(build_board(&state, pool, &gdp).await?))
}
