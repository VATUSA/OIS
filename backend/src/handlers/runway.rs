//! Runway Balancer handlers — the shared per-airport board (arrivals assigned to landing
//! runways + demand bins) and its config, computed live off the feed.

use std::collections::{HashMap, HashSet};

use axum::{
    Json,
    extract::{Extension, Path, State},
};
use chrono::{DateTime, Utc};

use crate::{
    auth::{
        context::CurrentUser,
        permissions::{FlowRunwayRead, FlowRunwayUpdate},
        require_permission::RequirePermission,
    },
    errors::ApiError,
    feed::runway::{self, RunwayArrival, RunwayBoard, RunwayConfigRequest},
    repos::runway as runway_repo,
    state::AppState,
};

const DEFAULT_WINDOW_MIN: i32 = 90;

/// Assemble the full board for `icao`: stored config + runway ends + live arrivals assigned
/// to runways + demand bins.
async fn build_board(state: &AppState, icao: &str) -> Result<RunwayBoard, ApiError> {
    let icao = icao.to_ascii_uppercase();

    // Stored config (shared), or defaults when the airport has never been configured.
    let (active_ends, star_rules, overrides, window_min) = match state.db.as_ref() {
        Some(pool) => match runway_repo::get_config(pool, &icao).await? {
            Some(c) => (c.active_ends, c.star_rules.0, c.overrides.0, c.window_min),
            None => (
                Vec::new(),
                HashMap::new(),
                HashMap::new(),
                DEFAULT_WINDOW_MIN,
            ),
        },
        None => (
            Vec::new(),
            HashMap::new(),
            HashMap::new(),
            DEFAULT_WINDOW_MIN,
        ),
    };
    let window_min = window_min as i64;

    // Runway ends from the bundled dataset; flag the configured-active ones.
    let mut ends = state.runways.ends_for(&icao);
    let source = if ends.is_empty() {
        "none — add ends manually".to_string()
    } else {
        "built-in".to_string()
    };
    let active_set: HashSet<&str> = active_ends.iter().map(String::as_str).collect();
    for e in &mut ends {
        e.active = active_set.contains(e.id.as_str());
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

    Ok(RunwayBoard {
        icao,
        source,
        ends,
        star_rules,
        overrides,
        window_min,
        arrivals: out_arrivals,
        demand,
        recs,
        bins,
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
    )
    .await?;
    Ok(Json(build_board(&state, &icao).await?))
}
