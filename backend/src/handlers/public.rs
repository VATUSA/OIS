//! Public advisories — read-only, no authentication. Pilot-facing views of the
//! currently-active TMIs and FCAs. Handlers deliberately omit `RequirePermission`
//! so they're reachable while signed out (see handlers/facilities.rs for the same
//! pattern), and only ever return active/published/enabled rows.

use axum::{Json, extract::State};
use chrono::Utc;

use crate::{errors::ApiError, models::PublicBoard, repos::public as repo, state::AppState};

#[utoipa::path(
    get,
    path = "/api/v1/public/board",
    tag = "public",
    responses((status = 200, body = PublicBoard))
)]
pub async fn get_board(State(state): State<AppState>) -> Result<Json<PublicBoard>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let (ground_stops, gdps, restrictions, programs) = tokio::try_join!(
        repo::active_ground_stops(pool),
        repo::active_gdps(pool),
        repo::active_restrictions(pool),
        repo::active_programs(pool),
    )?;
    Ok(Json(PublicBoard {
        ground_stops,
        gdps,
        restrictions,
        programs,
        as_of: Utc::now(),
    }))
}
