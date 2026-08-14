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
    feed::flow,
    feed::gdp::{self, GdpBoard, GdpFlightView},
    models::{CreateGdpRequest, GdpBody},
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

/// Project the live feed into GDP inbounds for `icao` (raw classification + ETA + ETD, no
/// metering). Airborne/ground/proposed only — already-arrived flights are dropped.
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
        &HashMap::new(),
        now,
    );
    flow.flights
        .into_iter()
        .filter(|f| f.status != "arrived")
        .filter_map(|f| {
            Some(gdp::Inbound {
                cs: f.callsign,
                dep: f.dep,
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

/// Run RBS off the current feed for `gdp`, applying frozen control times when published.
async fn assign_live(
    state: &AppState,
    pool: &sqlx::PgPool,
    gdp: &GdpBody,
    now: DateTime<Utc>,
) -> Result<(i64, i64, Vec<gdp::Assignment>), ApiError> {
    let (win_start, win_end) =
        resolve_window(now, &gdp.start_time, &gdp.end_time).ok_or(ApiError::Internal)?;
    let inbounds = live_inbounds(state, &gdp.airport.to_ascii_uppercase(), now).await;
    let mut assignments = gdp::ration_by_schedule(
        inbounds,
        gdp.aar,
        win_start,
        win_end,
        gdp.exempt_airborne,
        gdp.max_enroute_min,
    );
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
    let demand = gdp::demand_bins(&assignments, win_start, win_end, gdp.aar);
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
    flights.sort_by(|a, b| a.cta.cmp(&b.cta));
    exempt.sort_by(|a, b| a.eta.cmp(&b.eta));

    Ok(GdpBoard {
        id: gdp.id.clone(),
        airport: gdp.airport.clone(),
        aar: gdp.aar,
        status: gdp.status.clone(),
        start_time: gdp.start_time.clone(),
        end_time: gdp.end_time.clone(),
        window_start: ms(win_start, now),
        window_end: ms(win_end, now),
        max_enroute_min: gdp.max_enroute_min,
        exempt_airborne: gdp.exempt_airborne,
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
    let max_enroute = payload.max_enroute_min.filter(|m| *m > 0);

    let id = gdp_repo::create_gdp(
        pool,
        &airport,
        payload.aar,
        &start,
        &end,
        max_enroute,
        payload.exempt_airborne,
        &user.id,
    )
    .await?;
    let gdp = gdp_repo::get_gdp(pool, &id)
        .await?
        .ok_or(ApiError::Internal)?;
    Ok(Json(gdp))
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

    // Freeze control times: run RBS off the current feed and persist the controlled slots.
    let now = Utc::now();
    let (_s, _e, assignments) = assign_live(&state, pool, &gdp, now).await?;
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
    gdp_repo::replace_slots(pool, &gdp.id, &slots).await?;

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
