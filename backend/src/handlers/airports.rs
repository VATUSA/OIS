//! Airport coordinates (#540).
//!
//! The positions have always been in memory — `feed::airports::AirportDb`, loaded once from the
//! public mwgg/Airports dataset to compute distance-to-destination for ETAs — but **no route
//! returned them**. The only airport coordinates a client could see were the *staffed* fields on the
//! ATC board, so anything needing to point a map at an arbitrary airport had nothing to ask.
//!
//! That is why the surface viewer fitted to whatever geometry happened to be loaded instead of to
//! the airport: with no geometry it could not fit at all, and the FAA extract covers only 185 fields.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
};
use serde::Serialize;
use utoipa::ToSchema;

use crate::{errors::ApiError, feed::airports::AirportDb, state::AppState};

/// One airport's position.
#[derive(Debug, Serialize, ToSchema)]
pub struct AirportPositionBody {
    /// Uppercase ICAO, echoed so a caller can key a cache on the response alone.
    pub icao: String,
    pub lat: f64,
    pub lon: f64,
    /// Field elevation, feet MSL.
    pub elevation_ft: f64,
}

/// Look `icao` up in the loaded database.
///
/// Split from the handler so the lookup has tests without an `AppState`: the handler is three lines
/// of plumbing around this, and `AppState` needs a feed, a pool and a dozen caches to construct.
fn position(db: &AirportDb, icao: &str) -> Option<AirportPositionBody> {
    let icao = icao.trim().to_ascii_uppercase();
    db.get(&icao).map(|a| AirportPositionBody {
        icao,
        lat: a.lat,
        lon: a.lon,
        elevation_ft: a.elevation_ft,
    })
}

/// An airport's coordinates, for pointing a map at it.
///
/// Public, under `/api/v1/public/`, for the same reason the desktop-download redirect is: these are
/// reference coordinates, the surface viewer needs no permission to open, and a GET with no
/// `RequirePermission` reads as an oversight anywhere else in `router.rs`.
///
/// One ICAO rather than the whole map. The caller needs the airport it is already showing, and
/// returning every entry to centre a map is the same mistake as shipping the 1.1 MB stand extract to
/// the browser.
///
/// `503` when the feed has not loaded the database yet — distinct from `404`, which means the
/// dataset genuinely has no such airport, so a client can tell "try again" from "wrong code".
#[utoipa::path(
    get,
    path = "/api/v1/public/airports/{icao}",
    tag = "airports",
    params(("icao" = String, Path, description = "ICAO identifier, case-insensitive")),
    responses(
        (status = 200, body = AirportPositionBody),
        (status = 404, description = "No such airport in the dataset"),
        (status = 503, description = "The airport database has not loaded yet")
    )
)]
pub async fn get_airport(
    State(state): State<AppState>,
    Path(icao): Path<String>,
) -> Result<Json<AirportPositionBody>, ApiError> {
    // Clone the Arc and drop the guard before doing anything else, as `handlers::atc` does — the
    // lookup itself must not run while the feed lock is held.
    let db: Arc<AirportDb> = {
        let guard = state.feed.read().await;
        guard.airports.clone()
    };
    if db.is_empty() {
        return Err(ApiError::ServiceUnavailable);
    }
    position(&db, &icao).map(Json).ok_or(ApiError::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::airports::Airport;

    fn db() -> AirportDb {
        AirportDb::from([
            ("KDCA".to_string(), Airport::at(38.8512, -77.0402)),
            ("KSFO".to_string(), Airport::at(37.6188, -122.3750)),
        ])
    }

    #[test]
    fn finds_an_airport_and_echoes_its_icao() {
        let found = position(&db(), "KDCA").expect("KDCA is in the fixture");

        assert_eq!(found.icao, "KDCA");
        assert_eq!(found.lat, 38.8512);
        assert_eq!(found.lon, -77.0402);
    }

    /// A client may pass whatever the URL carried. The dataset is keyed on uppercase ICAO, so a
    /// lowercase code must not read as "no such airport" — that would send the caller looking for a
    /// missing dataset entry instead of a working map.
    #[test]
    fn looks_up_case_insensitively_and_ignores_surrounding_space() {
        for given in ["kdca", "KdCa", " KDCA "] {
            let found = position(&db(), given).unwrap_or_else(|| panic!("{given} should resolve"));
            assert_eq!(found.icao, "KDCA", "{given}");
        }
    }

    /// Distinct from the 503 the handler returns for an unloaded database: this is a real answer.
    #[test]
    fn an_unknown_code_is_not_found() {
        assert!(position(&db(), "ZZZZ").is_none());
        assert!(position(&db(), "").is_none());
    }
}
