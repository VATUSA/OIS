//! Flow handlers — FCA CRUD + a lightweight live-traffic feed for the FCA map.

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
        permissions::{FlowFcaDelete, FlowFcaRead, FlowFcaUpdate},
        require_permission::RequirePermission,
    },
    errors::ApiError,
    feed::{airports::AirportDb, fca, nav::NavData, vatsim::FlightPlan, vatsim::VatsimData},
    models::{
        AircraftRoute, FcaBody, FcaFlight, ReleaseRequest, ReorderRequest, TrafficAircraft,
        UpsertFcaRequest,
    },
    repos::flow as flow_repo,
    state::AppState,
};

/// Frozen releases keyed by callsign: (cta_ms, edct_ms).
type ReleaseMap = HashMap<String, (i64, i64)>;

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
    path = "/api/v1/flow/counts",
    tag = "flow",
    responses((status = 200, body = std::collections::HashMap<String, i64>), (status = 401))
)]
pub async fn fca_counts(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaRead>,
) -> Result<Json<HashMap<String, i64>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let fcas = flow_repo::list_fcas(pool).await?;
    let mut counts: HashMap<String, i64> = fcas.iter().map(|f| (f.id.clone(), 0)).collect();
    let active: Vec<&FcaBody> = fcas
        .iter()
        .filter(|f| f.enabled && f.points.0.len() >= 2)
        .collect();
    if active.is_empty() {
        return Ok(Json(counts));
    }

    let guard = state.feed.read().await;
    let Some(snap) = guard.snapshot.as_ref() else {
        return Ok(Json(counts));
    };
    let airports = &guard.airports;
    let nav = state.nav.as_ref();

    // Resolve each aircraft's route once, then test it against every active FCA.
    let mut tally = |fp: &FlightPlan, lat: f64, lon: f64, hdg: i64, gs: i64, alt: i64| {
        let airborne = gs >= 50;
        let Some(path) = fca::route_path(
            nav,
            airports,
            &fp.departure,
            &fp.arrival,
            &fp.route,
            lat,
            lon,
            hdg,
            gs,
        ) else {
            return;
        };
        for f in &active {
            if !passes_filters(f, fp, alt, airborne) {
                continue;
            }
            if fca::crosses(&path, &f.points.0, airborne, lat, lon, hdg).is_some() {
                *counts.get_mut(&f.id).unwrap() += 1;
            }
        }
    };

    for p in &snap.data.pilots {
        if let Some(fp) = &p.flight_plan {
            tally(
                fp,
                p.latitude,
                p.longitude,
                p.heading,
                p.groundspeed,
                p.altitude,
            );
        }
    }
    for pf in &snap.data.prefiles {
        if let Some(fp) = &pf.flight_plan {
            tally(fp, 0.0, 0.0, 0, 0, 0);
        }
    }
    Ok(Json(counts))
}

#[utoipa::path(
    get,
    path = "/api/v1/flow/aircraft/{callsign}/route",
    tag = "flow",
    params(("callsign" = String, Path, description = "Aircraft callsign")),
    responses((status = 200, body = AircraftRoute), (status = 401), (status = 404))
)]
pub async fn aircraft_route(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaRead>,
    Path(callsign): Path<String>,
) -> Result<Json<AircraftRoute>, ApiError> {
    let cs = callsign.to_ascii_uppercase();
    let guard = state.feed.read().await;
    let snap = guard
        .snapshot
        .as_ref()
        .ok_or(ApiError::ServiceUnavailable)?;
    let fp = snap
        .data
        .pilots
        .iter()
        .find(|p| p.callsign.eq_ignore_ascii_case(&cs))
        .and_then(|p| p.flight_plan.as_ref())
        .or_else(|| {
            snap.data
                .prefiles
                .iter()
                .find(|pf| pf.callsign.eq_ignore_ascii_case(&cs))
                .and_then(|pf| pf.flight_plan.as_ref())
        })
        .ok_or(ApiError::NotFound)?;
    let points = fca::full_route(
        state.nav.as_ref(),
        &guard.airports,
        &fp.departure,
        &fp.arrival,
        &fp.route,
    );
    Ok(Json(AircraftRoute {
        callsign: cs,
        points,
    }))
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

/// Parse an "HHMM"/"HHMMz" Zulu clock into an epoch-ms near `now` (±12h).
fn parse_hhmm_z(s: &str, now: DateTime<Utc>) -> Option<i64> {
    let s = s.trim().trim_end_matches(['z', 'Z']);
    if s.len() != 4 || !s.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let h: u32 = s[0..2].parse().ok()?;
    let m: u32 = s[2..4].parse().ok()?;
    let naive = now.date_naive().and_hms_opt(h, m, 0)?;
    let mut t = naive.and_utc().timestamp_millis();
    let now_ms = now.timestamp_millis();
    if t < now_ms - 12 * 3600 * 1000 {
        t += 24 * 3600 * 1000;
    } else if t > now_ms + 12 * 3600 * 1000 {
        t -= 24 * 3600 * 1000;
    }
    Some(t)
}

fn fca_flight(
    callsign: &str,
    fp: &FlightPlan,
    status: &str,
    lat: f64,
    lon: f64,
    cross: &fca::FcaCrossing,
    eta: Option<DateTime<Utc>>,
    gs: i64,
    alt: i64,
    hdg: i64,
    rel: Option<&(i64, i64)>,
) -> FcaFlight {
    FcaFlight {
        callsign: callsign.to_string(),
        dep: fp.departure.clone(),
        arr: fp.arrival.clone(),
        aircraft_type: fp.aircraft_short.clone(),
        status: status.to_string(),
        lat,
        lon,
        cross_lat: cross.lat,
        cross_lon: cross.lon,
        distance_nm: cross.along_nm.round() as i64,
        eta,
        cross_time: None,
        delay_min: 0,
        seq: 0,
        edct: rel.and_then(|(_, e)| DateTime::from_timestamp_millis(*e)),
        released: rel.is_some(),
        groundspeed: gs,
        altitude: alt,
        heading: hdg,
    }
}

/// Build the crossing candidates for an FCA from a live snapshot (no metering yet).
fn build_candidates(
    fca: &FcaBody,
    data: &VatsimData,
    airports: &AirportDb,
    nav: &NavData,
    releases: &ReleaseMap,
    now: DateTime<Utc>,
) -> (Vec<FcaFlight>, Vec<fca::MeterInput>) {
    let pts = fca.points.0.clone();
    let mut flights = Vec::new();
    let mut metas = Vec::new();

    for p in &data.pilots {
        let Some(fp) = &p.flight_plan else { continue };
        let airborne = p.groundspeed >= 50;
        if !passes_filters(fca, fp, p.altitude, airborne) {
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
        let rel = releases.get(&p.callsign);
        metas.push(fca::MeterInput {
            eta_ms: eta.map(|e| e.timestamp_millis()).unwrap_or(0),
            airborne,
            cross_speed: if airborne { p.groundspeed as f64 } else { tas },
            frozen_ms: rel.map(|(cta, _)| *cta),
        });
        flights.push(fca_flight(
            &p.callsign,
            fp,
            if airborne { "airborne" } else { "ground" },
            p.latitude,
            p.longitude,
            &cross,
            eta,
            p.groundspeed,
            p.altitude,
            p.heading,
            rel,
        ));
    }

    for pf in &data.prefiles {
        let Some(fp) = &pf.flight_plan else { continue };
        if !passes_filters(fca, fp, 0, false) {
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
        let rel = releases.get(&pf.callsign);
        metas.push(fca::MeterInput {
            eta_ms: eta.map(|e| e.timestamp_millis()).unwrap_or(0),
            airborne: false,
            cross_speed: fp.cruise_tas.parse::<f64>().unwrap_or(0.0),
            frozen_ms: rel.map(|(cta, _)| *cta),
        });
        flights.push(fca_flight(
            &pf.callsign,
            fp,
            "proposed",
            dep_lat,
            dep_lon,
            &cross,
            eta,
            0,
            0,
            0,
            rel,
        ));
    }

    (flights, metas)
}

/// Meter the candidates (auto, or the FCA's manual order) and finalize sequence/delay.
fn finalize(
    fca: &FcaBody,
    mut flights: Vec<FcaFlight>,
    metas: &[fca::MeterInput],
) -> Vec<FcaFlight> {
    let order: Option<Vec<usize>> = (fca.manual_seq && !fca.manual_order.is_empty()).then(|| {
        fca.manual_order
            .iter()
            .filter_map(|cs| flights.iter().position(|f| &f.callsign == cs))
            .collect()
    });
    let metered = fca::meter(metas, &fca.mode, fca.rate, fca.mit, order.as_deref());
    for (f, m) in flights.iter_mut().zip(&metered) {
        f.cross_time = DateTime::from_timestamp_millis(m.sched_ms);
        f.delay_min = (m.delay_sec + 30) / 60;
        f.seq = m.seq;
    }
    flights.sort_by_key(|f| f.seq);
    flights
}

async fn load_releases(pool: &sqlx::PgPool, id: &str) -> Result<ReleaseMap, ApiError> {
    Ok(flow_repo::list_releases(pool, id)
        .await?
        .into_iter()
        .map(|(c, cta, edct)| (c, (cta, edct)))
        .collect())
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
    if fca.points.0.len() < 2 {
        return Ok(Json(Vec::new()));
    }
    let releases = load_releases(pool, &id).await?;
    let now = Utc::now();

    let built = {
        let guard = state.feed.read().await;
        guard.snapshot.as_ref().map(|snap| {
            build_candidates(
                &fca,
                &snap.data,
                &guard.airports,
                state.nav.as_ref(),
                &releases,
                now,
            )
        })
    };
    let Some((flights, metas)) = built else {
        return Ok(Json(Vec::new()));
    };
    Ok(Json(finalize(&fca, flights, &metas)))
}

#[utoipa::path(
    post,
    path = "/api/v1/flow/fcas/{id}/release/{callsign}",
    tag = "flow",
    params(
        ("id" = String, Path, description = "FCA id"),
        ("callsign" = String, Path, description = "Aircraft callsign")
    ),
    request_body = ReleaseRequest,
    responses((status = 200, body = Vec<FcaFlight>), (status = 400), (status = 401), (status = 404))
)]
pub async fn mark_release(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path((id, callsign)): Path<(String, String)>,
    Json(payload): Json<ReleaseRequest>,
) -> Result<Json<Vec<FcaFlight>>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let fca = flow_repo::get_fca(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let callsign = callsign.to_ascii_uppercase();
    let now = Utc::now();
    let now_ms = now.timestamp_millis();
    let releases = load_releases(pool, &id).await?;

    let built = {
        let guard = state.feed.read().await;
        guard.snapshot.as_ref().map(|snap| {
            build_candidates(
                &fca,
                &snap.data,
                &guard.airports,
                state.nav.as_ref(),
                &releases,
                now,
            )
        })
    };
    let Some((mut flights, mut metas)) = built else {
        return Err(ApiError::ServiceUnavailable);
    };
    let ti = flights
        .iter()
        .position(|f| f.callsign == callsign)
        .ok_or(ApiError::NotFound)?; // not currently crossing
    let eta_ms = metas[ti].eta_ms;

    // Committed = every other pinned crossing (airborne ETA or an existing frozen CTA).
    let committed: Vec<i64> = metas
        .iter()
        .enumerate()
        .filter(|(j, _)| *j != ti)
        .filter_map(|(_, m)| {
            if m.airborne {
                Some(m.eta_ms)
            } else {
                m.frozen_ms
            }
        })
        .collect();
    let sep_ms = if fca.mode == "mit" {
        ((fca.mit as f64 / metas[ti].cross_speed.max(60.0)) * 3600.0 * 1000.0) as i64
    } else if fca.rate > 0 {
        (3600.0 / fca.rate as f64 * 1000.0) as i64
    } else {
        0
    };

    let cta = match payload.ready.as_deref().filter(|s| !s.trim().is_empty()) {
        // SET: pin the crossing so wheels-up lands on the requested time.
        Some(ready) => parse_hhmm_z(ready, now).ok_or(ApiError::BadRequest)? + (eta_ms - now_ms),
        // RDY: earliest metered slot.
        None => fca::earliest_slot(eta_ms, &committed, sep_ms),
    };
    let edct = cta - (eta_ms - now_ms);
    flow_repo::upsert_release(pool, &id, &callsign, cta, edct, &user.id).await?;

    // Reflect the new release and re-meter without another snapshot read.
    metas[ti].frozen_ms = Some(cta);
    flights[ti].released = true;
    flights[ti].edct = DateTime::from_timestamp_millis(edct);
    Ok(Json(finalize(&fca, flights, &metas)))
}

#[utoipa::path(
    delete,
    path = "/api/v1/flow/fcas/{id}/release/{callsign}",
    tag = "flow",
    params(
        ("id" = String, Path, description = "FCA id"),
        ("callsign" = String, Path, description = "Aircraft callsign")
    ),
    responses((status = 200, body = Vec<FcaFlight>), (status = 401), (status = 404))
)]
pub async fn clear_release(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaUpdate>,
    Path((id, callsign)): Path<(String, String)>,
) -> Result<Json<Vec<FcaFlight>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let fca = flow_repo::get_fca(pool, &id)
        .await?
        .ok_or(ApiError::NotFound)?;
    flow_repo::delete_release(pool, &id, &callsign.to_ascii_uppercase()).await?;
    let releases = load_releases(pool, &id).await?;
    let now = Utc::now();

    let built = {
        let guard = state.feed.read().await;
        guard.snapshot.as_ref().map(|snap| {
            build_candidates(
                &fca,
                &snap.data,
                &guard.airports,
                state.nav.as_ref(),
                &releases,
                now,
            )
        })
    };
    let Some((flights, metas)) = built else {
        return Ok(Json(Vec::new()));
    };
    Ok(Json(finalize(&fca, flights, &metas)))
}

#[utoipa::path(
    put,
    path = "/api/v1/flow/fcas/{id}/order",
    tag = "flow",
    params(("id" = String, Path, description = "FCA id")),
    request_body = ReorderRequest,
    responses((status = 204), (status = 401), (status = 404))
)]
pub async fn reorder_fca(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(id): Path<String>,
    Json(payload): Json<ReorderRequest>,
) -> Result<StatusCode, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    // Empty order clears manual mode (back to auto).
    let manual = !payload.order.is_empty();
    if !flow_repo::set_manual_order(pool, &id, &payload.order, manual, &user.id).await? {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}
