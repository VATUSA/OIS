//! Public advisories — read-only, no authentication. Pilot-facing views of the
//! currently-active TMIs and FCAs. Handlers deliberately omit `RequirePermission`
//! so they're reachable while signed out (see handlers/facilities.rs for the same
//! pattern), and only ever return active/published/enabled rows.

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

    // Enrich the metered airports with live inbound demand (next 60 min) so pilots
    // see current pressure vs the AAR, not just that a program exists.
    for g in &mut gdps {
        let demand = flow_for(&state, pool, &g.airport).await?.demand_60min as i64;
        g.demand_60min = demand;
        g.over_capacity = g.aar > 0 && demand > g.aar as i64;
    }
    for p in &mut programs {
        let demand = flow_for(&state, pool, &p.icao).await?.demand_60min as i64;
        p.demand_60min = demand;
        p.over_capacity = p.aar > 0 && demand > p.aar as i64;
    }

    Ok(Json(PublicBoard {
        ground_stops,
        gdps,
        restrictions,
        programs,
        as_of: Utc::now(),
    }))
}
