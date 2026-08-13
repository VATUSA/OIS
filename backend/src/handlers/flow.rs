//! Flow handlers — FCA CRUD + a lightweight live-traffic feed for the FCA map.

use axum::{
    Json,
    extract::{Extension, Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Duration, Utc};

use crate::{
    auth::{
        context::CurrentUser,
        permissions::{FlowFcaDelete, FlowFcaRead, FlowFcaUpdate},
        require_permission::RequirePermission,
    },
    errors::ApiError,
    feed::{fca, vatsim::FlightPlan},
    models::{FcaBody, FcaFlight, TrafficAircraft, UpsertFcaRequest},
    repos::flow as flow_repo,
    state::AppState,
};

/// Airport-code match, tolerant of a leading `K` (KJFK ~ JFK).
fn airport_match(filter: &str, code: &str) -> bool {
    let (f, c) = (filter.to_ascii_uppercase(), code.to_ascii_uppercase());
    f == c || c.strip_prefix('K') == Some(f.as_str()) || f.strip_prefix('K') == Some(c.as_str())
}

/// Whether the filed route mentions `fix` as a token (revision digit tolerant).
fn route_has_fix(route: &str, fix: &str) -> bool {
    let fix = fix.to_ascii_uppercase();
    route.split_whitespace().any(|tok| {
        let t = tok.split('/').next().unwrap_or("").to_ascii_uppercase();
        t == fix || t.trim_end_matches(|c: char| c.is_ascii_digit()) == fix
    })
}

/// Membership filters (dest / origin / fix / altitude). Altitude is checked only for
/// airborne aircraft (current alt); scope (ARTCC polygon) is not yet enforced.
fn passes_filters(fca: &FcaBody, fp: &FlightPlan, alt: i64, airborne: bool) -> bool {
    if !fca.dests.is_empty() && !fca.dests.iter().any(|d| airport_match(d, &fp.arrival)) {
        return false;
    }
    if !fca.origins.is_empty() && !fca.origins.iter().any(|o| airport_match(o, &fp.departure)) {
        return false;
    }
    if !fca.fixes.is_empty() && !fca.fixes.iter().any(|f| route_has_fix(&fp.route, f)) {
        return false;
    }
    if airborne {
        if let Some(min) = fca.min_fl {
            if alt < min as i64 * 100 {
                return false;
            }
        }
        if let Some(max) = fca.max_fl {
            if alt > max as i64 * 100 {
                return false;
            }
        }
    }
    true
}

fn eta_to_crossing(
    airborne: bool,
    along_nm: f64,
    gs: i64,
    cruise_tas: &str,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let minutes = if airborne {
        along_nm / (gs.max(100) as f64) * 60.0
    } else {
        let tas = cruise_tas.parse::<f64>().unwrap_or(0.0).max(120.0);
        along_nm / tas * 60.0 + 12.0
    };
    Some(now + Duration::seconds((minutes * 60.0) as i64))
}

fn validate_fca(req: &UpsertFcaRequest) -> Result<(), ApiError> {
    if req.name.trim().is_empty() || req.points.len() < 2 {
        return Err(ApiError::BadRequest);
    }
    if let Some(m) = &req.mode {
        if m != "rate" && m != "mit" {
            return Err(ApiError::BadRequest);
        }
    }
    Ok(())
}

#[utoipa::path(
    get,
    path = "/api/v1/flow/fcas",
    tag = "flow",
    responses((status = 200, body = Vec<FcaBody>), (status = 401))
)]
pub async fn list_fcas(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaRead>,
) -> Result<Json<Vec<FcaBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    Ok(Json(flow_repo::list_fcas(pool).await?))
}

#[utoipa::path(
    post,
    path = "/api/v1/flow/fcas",
    tag = "flow",
    request_body = UpsertFcaRequest,
    responses((status = 200, body = FcaBody), (status = 400), (status = 401))
)]
pub async fn create_fca(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Json(payload): Json<UpsertFcaRequest>,
) -> Result<Json<FcaBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    validate_fca(&payload)?;
    let id = flow_repo::create_fca(pool, &payload, &user.id).await?;
    flow_repo::get_fca(pool, &id)
        .await?
        .map(Json)
        .ok_or(ApiError::Internal)
}

#[utoipa::path(
    put,
    path = "/api/v1/flow/fcas/{id}",
    tag = "flow",
    params(("id" = String, Path, description = "FCA id")),
    request_body = UpsertFcaRequest,
    responses((status = 200, body = FcaBody), (status = 400), (status = 401), (status = 404))
)]
pub async fn update_fca(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<String>,
    Json(payload): Json<UpsertFcaRequest>,
) -> Result<Json<FcaBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    validate_fca(&payload)?;
    if !flow_repo::update_fca(pool, &id, &payload, &user.id).await? {
        return Err(ApiError::NotFound);
    }
    flow_repo::get_fca(pool, &id)
        .await?
        .map(Json)
        .ok_or(ApiError::Internal)
}

#[utoipa::path(
    delete,
    path = "/api/v1/flow/fcas/{id}",
    tag = "flow",
    params(("id" = String, Path, description = "FCA id")),
    responses((status = 204), (status = 401), (status = 404))
)]
pub async fn delete_fca(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaDelete>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if flow_repo::delete_fca(pool, &id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/flow/traffic",
    tag = "flow",
    responses((status = 200, body = Vec<TrafficAircraft>), (status = 401))
)]
pub async fn list_traffic(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaRead>,
) -> Json<Vec<TrafficAircraft>> {
    let guard = state.feed.read().await;
    let aircraft = guard
        .snapshot
        .as_ref()
        .map(|snap| {
            snap.data
                .pilots
                .iter()
                .filter(|p| p.latitude != 0.0 || p.longitude != 0.0)
                .map(|p| {
                    let fp = p.flight_plan.as_ref();
                    TrafficAircraft {
                        callsign: p.callsign.clone(),
                        lat: p.latitude,
                        lon: p.longitude,
                        heading: p.heading,
                        gs: p.groundspeed,
                        alt: p.altitude,
                        dep: fp.map(|f| f.departure.clone()).unwrap_or_default(),
                        arr: fp.map(|f| f.arrival.clone()).unwrap_or_default(),
                        actype: fp.map(|f| f.aircraft_short.clone()).unwrap_or_default(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    Json(aircraft)
}

#[utoipa::path(
    get,
    path = "/api/v1/flow/fcas/{id}/traffic",
    tag = "flow",
    params(("id" = String, Path, description = "FCA id")),
    responses((status = 200, body = Vec<FcaFlight>), (status = 401), (status = 404))
)]
pub async fn fca_traffic(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaRead>,
    Path(id): Path<String>,
) -> Result<Json<Vec<FcaFlight>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let fca = flow_repo::get_fca(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let pts = fca.points.0.clone();
    if pts.len() < 2 {
        return Ok(Json(Vec::new()));
    }

    let now = Utc::now();
    let guard = state.feed.read().await;
    let Some(snap) = guard.snapshot.as_ref() else {
        return Ok(Json(Vec::new()));
    };
    let airports = &guard.airports;
    let nav = state.nav.as_ref();
    let mut out: Vec<FcaFlight> = Vec::new();
    let mut metas: Vec<fca::MeterInput> = Vec::new();

    // Connected pilots — airborne or on the ground.
    for p in &snap.data.pilots {
        let Some(fp) = &p.flight_plan else { continue };
        let airborne = p.groundspeed >= 50;
        if !passes_filters(&fca, fp, p.altitude, airborne) {
            continue;
        }
        let Some(cross) = fca::crossing_for(
            &pts,
            nav,
            airports,
            &fp.departure,
            &fp.arrival,
            &fp.route,
            p.latitude,
            p.longitude,
            p.heading,
            p.groundspeed,
        ) else {
            continue;
        };
        let eta = eta_to_crossing(airborne, cross.along_nm, p.groundspeed, &fp.cruise_tas, now);
        let tas = fp.cruise_tas.parse::<f64>().unwrap_or(0.0);
        metas.push(fca::MeterInput {
            eta_ms: eta.map(|e| e.timestamp_millis()).unwrap_or(0),
            airborne,
            cross_speed: if airborne { p.groundspeed as f64 } else { tas },
        });
        out.push(FcaFlight {
            callsign: p.callsign.clone(),
            dep: fp.departure.clone(),
            arr: fp.arrival.clone(),
            aircraft_type: fp.aircraft_short.clone(),
            status: if airborne { "airborne" } else { "ground" }.to_string(),
            lat: p.latitude,
            lon: p.longitude,
            cross_lat: cross.lat,
            cross_lon: cross.lon,
            distance_nm: cross.along_nm.round() as i64,
            eta,
            cross_time: None,
            delay_min: 0,
            seq: 0,
            groundspeed: p.groundspeed,
            altitude: p.altitude,
            heading: p.heading,
        });
    }

    // Prefiles — not yet connected; treated as proposed departures from their field.
    for pf in &snap.data.prefiles {
        let Some(fp) = &pf.flight_plan else { continue };
        if !passes_filters(&fca, fp, 0, false) {
            continue;
        }
        let Some(cross) = fca::crossing_for(
            &pts,
            nav,
            airports,
            &fp.departure,
            &fp.arrival,
            &fp.route,
            0.0,
            0.0,
            0,
            0,
        ) else {
            continue;
        };
        let (dep_lat, dep_lon) = airports
            .get(&fp.departure.to_ascii_uppercase())
            .copied()
            .unwrap_or((0.0, 0.0));
        let eta = eta_to_crossing(false, cross.along_nm, 0, &fp.cruise_tas, now);
        metas.push(fca::MeterInput {
            eta_ms: eta.map(|e| e.timestamp_millis()).unwrap_or(0),
            airborne: false,
            cross_speed: fp.cruise_tas.parse::<f64>().unwrap_or(0.0),
        });
        out.push(FcaFlight {
            callsign: pf.callsign.clone(),
            dep: fp.departure.clone(),
            arr: fp.arrival.clone(),
            aircraft_type: fp.aircraft_short.clone(),
            status: "proposed".to_string(),
            lat: dep_lat,
            lon: dep_lon,
            cross_lat: cross.lat,
            cross_lon: cross.lon,
            distance_nm: cross.along_nm.round() as i64,
            eta,
            cross_time: None,
            delay_min: 0,
            seq: 0,
            groundspeed: 0,
            altitude: 0,
            heading: 0,
        });
    }

    // Sequence the crossing traffic (airborne priority; ground floats into gaps).
    let metered = fca::meter(&metas, &fca.mode, fca.rate, fca.mit);
    for (f, m) in out.iter_mut().zip(&metered) {
        f.cross_time = DateTime::from_timestamp_millis(m.sched_ms);
        f.delay_min = (m.delay_sec + 30) / 60;
        f.seq = m.seq;
    }
    out.sort_by_key(|f| f.seq);
    Ok(Json(out))
}
