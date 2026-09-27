//! Stamps the owning ARTCC onto the restriction bodies, so a client can tell a ground stop at its
//! own centre from one three time zones away (#405).
//!
//! The ARTCC is derived, never stored: it's resolved live from the feed's facility map at request
//! time, the same choice `airport_configs::annotate_and_filter` makes — a stored snapshot goes stale
//! after a facility realignment. The map lives in `AppState` behind an `RwLock`, so these functions
//! take the already-read map and stay pure, which is also what makes them testable without a DB.
//!
//! The lookups want uppercase ids, which is what the write handlers store — every create/update
//! path, including event activation, normalizes the airport, icao and TMI facilities before insert.
//! A row that still resolves to nothing stamps `None`, which the client treats as *in* scope for
//! everyone rather than for no one: scoping narrows an audience that used to be the whole country,
//! and for a ground stop a missed alert is worse than an extra one. See `inRestrictionScope` in
//! `web/src/lib/restriction-scope.ts` — don't make this fail closed without changing that too.

use crate::{
    feed::facilities::{FacilityMap, artcc_for_airport, artcc_for_facility},
    models::{GdpBody, GroundStopBody, ProgramBody, TmiBody},
};

pub fn stamp_ground_stop(map: &FacilityMap, row: &mut GroundStopBody) {
    row.artcc = artcc_for_airport(map, &row.airport);
}

pub fn stamp_ground_stops(map: &FacilityMap, rows: &mut [GroundStopBody]) {
    for row in rows {
        stamp_ground_stop(map, row);
    }
}

pub fn stamp_gdp(map: &FacilityMap, row: &mut GdpBody) {
    row.artcc = artcc_for_airport(map, &row.airport);
}

pub fn stamp_gdps(map: &FacilityMap, rows: &mut [GdpBody]) {
    for row in rows {
        stamp_gdp(map, row);
    }
}

pub fn stamp_program(map: &FacilityMap, row: &mut ProgramBody) {
    row.artcc = artcc_for_airport(map, &row.icao);
}

pub fn stamp_programs(map: &FacilityMap, rows: &mut [ProgramBody]) {
    for row in rows {
        stamp_program(map, row);
    }
}

/// Both sides, because a centre cares about a TMI whether it's the one asking or the one providing.
pub fn stamp_tmi(map: &FacilityMap, row: &mut TmiBody) {
    row.requesting_artcc = artcc_for_facility(map, &row.requesting);
    row.providing_artcc = artcc_for_facility(map, &row.providing);
}

pub fn stamp_tmis(map: &FacilityMap, rows: &mut [TmiBody]) {
    for row in rows {
        stamp_tmi(map, row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::facilities::Facility;

    fn facility(kind: &str, airports: &[&str]) -> Facility {
        Facility {
            kind: kind.to_string(),
            airports: airports.iter().map(|a| a.to_string()).collect(),
        }
    }

    fn map() -> FacilityMap {
        FacilityMap::from([
            ("ZDC".to_string(), facility("artcc", &["KDCA", "KIAD"])),
            ("ZLA".to_string(), facility("artcc", &["KLAX"])),
            ("PCT".to_string(), facility("tracon", &["KDCA", "KIAD"])),
        ])
    }

    fn ground_stop(airport: &str) -> GroundStopBody {
        GroundStopBody {
            id: "gs1".to_string(),
            airport: airport.to_string(),
            artcc: None,
            scope: String::new(),
            until: None,
            status: "published".to_string(),
            published_at: None,
            updated_at: chrono::Utc::now(),
            updated_by: None,
        }
    }

    fn tmi(requesting: &str, providing: &str) -> TmiBody {
        TmiBody {
            id: "t1".to_string(),
            requesting: requesting.to_string(),
            providing: providing.to_string(),
            requesting_artcc: None,
            providing_artcc: None,
            restriction: "20 MIT".to_string(),
            start_time: chrono::Utc::now(),
            stop_time: None,
            status: "published".to_string(),
            published_at: None,
            created_at: chrono::Utc::now(),
            author: None,
            structured: None,
            decoded: None,
        }
    }

    #[test]
    fn stamps_a_ground_stops_owning_artcc() {
        let mut rows = vec![ground_stop("KDCA"), ground_stop("KLAX")];
        stamp_ground_stops(&map(), &mut rows);
        assert_eq!(rows[0].artcc.as_deref(), Some("ZDC"));
        assert_eq!(rows[1].artcc.as_deref(), Some("ZLA"));
    }

    fn program(icao: &str) -> ProgramBody {
        ProgramBody {
            icao: icao.to_string(),
            artcc: None,
            aar: 40,
            trail: 0,
            mit: 0,
            gates: sqlx::types::Json(Vec::new()),
            exclude_wake: Vec::new(),
            exclude_types: Vec::new(),
            jets_only: false,
            active_until: None,
            updated_at: chrono::Utc::now(),
            updated_by: None,
        }
    }

    /// Programs are the one restriction keyed on `icao` rather than `airport`, so they get their own
    /// case — nothing pinned that they were stamped at all (VATUSA/OIS#405 review).
    #[test]
    fn stamps_a_programs_owning_artcc() {
        let mut rows = vec![program("KDCA"), program("LAX")];
        stamp_programs(&map(), &mut rows);
        assert_eq!(rows[0].artcc.as_deref(), Some("ZDC"));
        assert_eq!(
            rows[1].artcc.as_deref(),
            Some("ZLA"),
            "a 3-letter id resolves as its ICAO"
        );
    }

    #[test]
    fn leaves_an_unknown_airport_unresolved() {
        let mut rows = vec![ground_stop("XXXX")];
        stamp_ground_stops(&map(), &mut rows);
        assert_eq!(rows[0].artcc, None);
    }

    /// The field arrives from the DB unset and is only ever filled here — a real round trip, because
    /// `#[sqlx(default)]` is what lets a column-less field ride along on these hand-written SELECTs,
    /// and a SELECT that started returning an `artcc` column would silently take this over.
    #[sqlx::test]
    async fn the_column_less_field_survives_a_real_round_trip(pool: sqlx::PgPool) {
        let user = crate::scope_test_support::seed_user(&pool).await;
        let id = crate::repos::tmu::create_ground_stop(
            &pool,
            &crate::models::CreateGroundStopRequest {
                airport: "KDCA".to_string(),
                scope: None,
                until: None,
            },
            "",
            None,
            &user,
        )
        .await
        .unwrap();

        let mut rows = crate::repos::tmu::list_ground_stops(&pool).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].artcc, None, "the DB knows no ARTCC");

        stamp_ground_stops(&map(), &mut rows);
        assert_eq!(rows[0].artcc.as_deref(), Some("ZDC"));
        assert_eq!(rows[0].id, id);
    }

    #[test]
    fn stamps_both_sides_of_a_tmi_and_resolves_a_tracon_to_its_centre() {
        let mut rows = vec![tmi("PCT", "ZDC"), tmi("ZLA", "XXXX")];
        stamp_tmis(&map(), &mut rows);
        // A TRACON resolves through its member airports to the centre above it.
        assert_eq!(rows[0].requesting_artcc.as_deref(), Some("ZDC"));
        assert_eq!(rows[0].providing_artcc.as_deref(), Some("ZDC"));
        assert_eq!(rows[1].requesting_artcc.as_deref(), Some("ZLA"));
        assert_eq!(rows[1].providing_artcc, None);
    }
}
