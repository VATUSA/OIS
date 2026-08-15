//! Public advisories — read-only, no authentication. Pilot-facing views of the
//! currently-active TMIs and FCAs. Handlers deliberately omit `RequirePermission`
//! so they're reachable while signed out (see handlers/facilities.rs for the same
//! pattern), and only ever return active/published/enabled rows.

use std::collections::{HashMap, HashSet};

use axum::{Json, extract::State};
use chrono::Utc;

use crate::{
    errors::ApiError, handlers::feed::flow_for, models::PublicBoard, repos::public as repo,
    state::AppState,
};

#[utoipa::path(
    get,
    path = "/api/v1/public/board",
    tag = "public",
    responses((status = 200, body = PublicBoard))
)]
pub async fn get_board(State(state): State<AppState>) -> Result<Json<PublicBoard>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let (ground_stops, mut gdps, restrictions, mut programs) = tokio::try_join!(
        repo::active_ground_stops(pool),
        repo::active_gdps(pool),
        repo::active_restrictions(pool),
        repo::active_programs(pool),
    )?;

    // Enrich the metered airports with live inbound demand (next 60 min) so pilots see current
    // pressure vs the AAR. Compute each unique airport's demand once, concurrently.
    let airports: HashSet<String> = gdps
        .iter()
        .map(|g| g.airport.clone())
        .chain(programs.iter().map(|p| p.icao.clone()))
        .collect();
    let mut tasks = tokio::task::JoinSet::new();
    for airport in airports {
        let state = state.clone();
        let pool = pool.clone();
        tasks.spawn(async move {
            let demand = flow_for(&state, &pool, &airport)
                .await
                .map(|f| f.demand_60min as i64)
                .unwrap_or(0);
            (airport, demand)
        });
    }
    let mut demand: HashMap<String, i64> = HashMap::new();
    while let Some(res) = tasks.join_next().await {
        if let Ok((airport, d)) = res {
            demand.insert(airport, d);
        }
    }
    for g in &mut gdps {
        let d = demand.get(&g.airport).copied().unwrap_or(0);
        g.demand_60min = d;
        g.over_capacity = g.aar > 0 && d > g.aar as i64;
    }
    for p in &mut programs {
        let d = demand.get(&p.icao).copied().unwrap_or(0);
        p.demand_60min = d;
        p.over_capacity = p.aar > 0 && d > p.aar as i64;
    }

    Ok(Json(PublicBoard {
        ground_stops,
        gdps,
        restrictions,
        programs,
        as_of: Utc::now(),
    }))
}
