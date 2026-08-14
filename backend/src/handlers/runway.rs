//! Runway Balancer handlers — the shared per-airport board (arrivals assigned to landing
//! runways + demand bins) and its config, computed live off the feed.

use std::collections::{HashMap, HashSet};

use axum::{
    Json,
    extract::{Extension, Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};

use crate::{
    auth::{
        context::CurrentUser,
        permissions::{FlowRunwayRead, FlowRunwayUpdate},
        require_permission::RequirePermission,
    },
    errors::ApiError,
    feed::runway::{
        self, RunwayArrival, RunwayBoard, RunwayConfigRequest, RunwayEnd, SavedConfigRequest,
        SavedRunwayConfig,
    },
    repos::runway as runway_repo,
    state::AppState,
};

const DEFAULT_WINDOW_MIN: i32 = 90;
/// METAR is refreshed at most this often per airport.
const METAR_TTL_MS: i64 = 10 * 60_000;

/// Latest METAR for `icao`, from the shared cache; fetched server-side (no CORS) when stale.
async fn metar_for(state: &AppState, icao: &str) -> Option<crate::feed::metar::MetarInfo> {
    let now = Utc::now().timestamp_millis();
    if let Ok(cache) = state.metar_cache.lock()
        && let Some((info, at)) = cache.get(icao)
        && now - at < METAR_TTL_MS
    {
        return Some(info.clone());
    }
    let client = reqwest::Client::builder()
        .user_agent("ois-metar/1.0 (+https://vatusa.net)")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap_or_default();
    match crate::feed::metar::fetch_one(&client, icao).await {
        Some(info) => {
            if let Ok(mut cache) = state.metar_cache.lock() {
                cache.insert(icao.to_string(), (info.clone(), now));
            }
            Some(info)
        }
        // On a failed fetch, fall back to whatever (possibly stale) value we have.
        None => state
            .metar_cache
            .lock()
            .ok()
            .and_then(|c| c.get(icao).map(|(i, _)| i.clone())),
    }
}

/// Assemble the full board for `icao`: stored config + runway ends + live arrivals assigned
/// to runways + demand bins.
async fn build_board(state: &AppState, icao: &str) -> Result<RunwayBoard, ApiError> {
    let icao = icao.to_ascii_uppercase();

    // Stored config (shared), or defaults when the airport has never been configured.
    let default = || {
        (
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            DEFAULT_WINDOW_MIN,
            Vec::new(),
        )
    };
    let (active_ends, star_rules, overrides, window_min, custom_ends) = match state.db.as_ref() {
        Some(pool) => match runway_repo::get_config(pool, &icao).await? {
            Some(c) => (
                c.active_ends,
                c.star_rules.0,
                c.overrides.0,
                c.window_min,
                c.custom_ends.0,
            ),
            None => default(),
        },
        None => default(),
    };
    let window_min = window_min as i64;

    // Runway ends from the bundled dataset, plus any manually-added ones.
    let mut ends = state.runways.ends_for(&icao);
    let source = if !ends.is_empty() {
        "built-in".to_string()
    } else if custom_ends.is_empty() {
        "none — add ends manually".to_string()
    } else {
        "manual".to_string()
    };
    let active_set: HashSet<&str> = active_ends.iter().map(String::as_str).collect();
    for e in &mut ends {
        e.active = active_set.contains(e.id.as_str());
    }
    for ce in &custom_ends {
        ends.push(RunwayEnd {
            id: ce.id.clone(),
            hdg: ((ce.hdg % 360) + 360) % 360,
            len: ce.len,
            active: active_set.contains(ce.id.as_str()),
            pair: ce.id.clone(),
        });
    }
    let active_ids: Vec<String> = ends
        .iter()
        .filter(|e| e.active)
        .map(|e| e.id.clone())
        .collect();

    // Live arrivals — clone the snapshot + airports and drop the feed lock before the CPU.
    let (snapshot, airports) = {
        let guard = state.feed.read().await;
        (guard.snapshot.clone(), guard.airports.clone())
    };
    let now = Utc::now();
    let arrivals = match &snapshot {
        Some(snap) => runway::collect_arrivals(
            &icao,
            &snap.data,
            airports.as_ref(),
            state.winds.load_full().as_ref(),
            now,
            window_min,
        ),
        None => Vec::new(),
    };

    // Assign each arrival a runway (override → STAR rule → AUTO).
    let assigned = runway::assign(&arrivals, &active_ids, &star_rules, &overrides);
    let out_arrivals: Vec<RunwayArrival> = arrivals
        .iter()
        .zip(&assigned)
        .map(|(a, (rwy, src))| RunwayArrival {
            cs: a.cs.clone(),
            dep: a.dep.clone(),
            actype: a.actype.clone(),
            star: a.star.clone(),
            eta: DateTime::from_timestamp_millis(a.eta_ms).unwrap_or(now),
            dist_nm: a.dist_nm.round() as i64,
            rwy: rwy.clone(),
            src: src.to_string(),
        })
        .collect();

    let now_ms = now.timestamp_millis();
    let demand_input: Vec<(Option<String>, i64)> = arrivals
        .iter()
        .zip(&assigned)
        .map(|(a, (rwy, _))| (rwy.clone(), a.eta_ms))
        .collect();
    let (demand, bins) = runway::demand_bins(&demand_input, &active_ids, now_ms, window_min);

    let rec_input: Vec<(String, Option<String>, String, i64)> = arrivals
        .iter()
        .zip(&assigned)
        .map(|(a, (rwy, src))| (a.cs.clone(), rwy.clone(), src.to_string(), a.eta_ms))
        .collect();
    let recs = runway::recommendations(&rec_input, &active_ids, now_ms, window_min);
    let metar = metar_for(state, &icao).await;

    Ok(RunwayBoard {
        icao,
        source,
        ends,
        custom_ends,
        star_rules,
        overrides,
        window_min,
        arrivals: out_arrivals,
        demand,
        recs,
        bins,
        metar: metar.as_ref().map(|m| m.raw.clone()),
        flight_category: metar.as_ref().map(|m| m.category.clone()),
        wind: metar.as_ref().and_then(|m| m.wind.clone()),
    })
}

/// The runway-balancer board for one airport (runway config + live assigned arrivals).
#[utoipa::path(
    get,
    path = "/api/v1/flow/runway/{icao}",
    tag = "flow",
    params(("icao" = String, Path, description = "Airport ICAO")),
    responses((status = 200, body = RunwayBoard), (status = 401))
)]
pub async fn get_runway(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowRunwayRead>,
    Path(icao): Path<String>,
) -> Result<Json<RunwayBoard>, ApiError> {
    Ok(Json(build_board(&state, &icao).await?))
}

/// Save the shared runway config (active ends, STAR rules, overrides, window) and return
/// the recomputed board.
#[utoipa::path(
    put,
    path = "/api/v1/flow/runway/{icao}",
    tag = "flow",
    params(("icao" = String, Path, description = "Airport ICAO")),
    request_body = RunwayConfigRequest,
    responses((status = 200, body = RunwayBoard), (status = 401), (status = 503))
)]
pub async fn put_runway(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowRunwayUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path(icao): Path<String>,
    Json(body): Json<RunwayConfigRequest>,
) -> Result<Json<RunwayBoard>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let icao = icao.to_ascii_uppercase();
    let window = body.window_min.unwrap_or(DEFAULT_WINDOW_MIN).clamp(30, 240);
    runway_repo::upsert_config(
        pool,
        &icao,
        &body.active_ends,
        &body.star_rules,
        &body.overrides,
        window,
        &user.id,
        body.custom_ends.as_ref(),
    )
    .await?;
    Ok(Json(build_board(&state, &icao).await?))
}

/// List the named runway configs saved for an airport.
#[utoipa::path(
    get,
    path = "/api/v1/flow/runway/{icao}/configs",
    tag = "flow",
    params(("icao" = String, Path, description = "Airport ICAO")),
    responses((status = 200, body = Vec<SavedRunwayConfig>), (status = 401), (status = 503))
)]
pub async fn list_saved_configs(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowRunwayRead>,
    Path(icao): Path<String>,
) -> Result<Json<Vec<SavedRunwayConfig>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let rows = runway_repo::list_saved(pool, &icao.to_ascii_uppercase()).await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| SavedRunwayConfig {
                name: r.name,
                active_ends: r.payload.0.active_ends,
                star_rules: r.payload.0.star_rules,
            })
            .collect(),
    ))
}

/// Save (or replace) a named runway config for an airport.
#[utoipa::path(
    put,
    path = "/api/v1/flow/runway/{icao}/configs/{name}",
    tag = "flow",
    params(
        ("icao" = String, Path, description = "Airport ICAO"),
        ("name" = String, Path, description = "Config name")
    ),
    request_body = SavedConfigRequest,
    responses((status = 204), (status = 400), (status = 401), (status = 503))
)]
pub async fn save_config(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowRunwayUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Path((icao, name)): Path<(String, String)>,
    Json(body): Json<SavedConfigRequest>,
) -> Result<StatusCode, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let name = name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest);
    }
    runway_repo::upsert_saved(
        pool,
        &icao.to_ascii_uppercase(),
        name,
        &runway_repo::SavedPayload {
            active_ends: body.active_ends,
            star_rules: body.star_rules,
        },
        &user.id,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Delete a named runway config.
#[utoipa::path(
    delete,
    path = "/api/v1/flow/runway/{icao}/configs/{name}",
    tag = "flow",
    params(
        ("icao" = String, Path, description = "Airport ICAO"),
        ("name" = String, Path, description = "Config name")
    ),
    responses((status = 204), (status = 401), (status = 404), (status = 503))
)]
pub async fn delete_config(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowRunwayUpdate>,
    Path((icao, name)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    if runway_repo::delete_saved(pool, &icao.to_ascii_uppercase(), name.trim()).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}
