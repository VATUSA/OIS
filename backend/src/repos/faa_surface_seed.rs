//! Idempotently seeds `flow.airport_ramp_area` / `flow.airport_taxiway` from the bundled FAA
//! Aerodrome Mapping extract produced by `bin/faa_surface_importer.rs` (#230), tagging every row
//! `source = 'faa'`. Touches only `source='faa'` rows on re-runs — `manual`/`crc` rows for the
//! same airport are left untouched. One documented exception: a pre-existing `source='osm'` row
//! (from migration `0067`'s KDCA seed) is retired for any ICAO the FAA extract also covers, since
//! FAA data supersedes it — confirmed decision, see VATUSA/OIS#231. No gate rows are written: the
//! FAA AM layer set has no gate/parking-stand layer (see #230).
//!
//! Deliberately not folded into `repos::airport_surface` — that module is per-request CRUD for the
//! map editor; this is a bulk load from bundled, compiled-in data, closer in spirit to
//! `feed::runway_db`'s `include_str!` pattern than to a handler-facing repo.

use std::collections::HashMap;

use serde::Deserialize;
use sqlx::PgPool;

use crate::errors::ApiError;

#[derive(Debug, Deserialize)]
struct ExtractAirport {
    #[serde(default)]
    taxiways: Vec<ExtractTaxiway>,
    #[serde(default)]
    ramps: Vec<ExtractRamp>,
}

#[derive(Debug, Deserialize)]
struct ExtractTaxiway {
    name: String,
    points: Vec<[f64; 2]>,
}

#[derive(Debug, Deserialize)]
struct ExtractRamp {
    name: String,
    kind: String,
    rings: Vec<Vec<[f64; 2]>>,
}

fn load_bundled_extract() -> HashMap<String, ExtractAirport> {
    serde_json::from_str(include_str!("../../data/faa_surface.json"))
        .expect("bundled backend/data/faa_surface.json should parse")
}

#[derive(Debug, Default)]
pub struct SeedSummary {
    pub taxiways_inserted: usize,
    pub ramps_inserted: usize,
    pub osm_taxiways_retired: usize,
    pub osm_ramps_retired: usize,
}

impl std::fmt::Display for SeedSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "seeded {} taxiways + {} ramps (retired {} osm taxiways, {} osm ramps)",
            self.taxiways_inserted,
            self.ramps_inserted,
            self.osm_taxiways_retired,
            self.osm_ramps_retired
        )
    }
}

/// Re-seeds every `source='faa'` row from the bundled extract in one transaction. Safe to call
/// repeatedly (each call fully replaces the prior `faa` rows, so nothing duplicates).
pub async fn seed(pool: &PgPool) -> Result<SeedSummary, ApiError> {
    let extract = load_bundled_extract();
    let icaos: Vec<&str> = extract.keys().map(String::as_str).collect();

    let mut tx = pool.begin().await.map_err(|_| ApiError::Internal)?;

    sqlx::query("delete from flow.airport_taxiway where source = 'faa'")
        .execute(&mut *tx)
        .await
        .map_err(|_| ApiError::Internal)?;
    sqlx::query("delete from flow.airport_ramp_area where source = 'faa'")
        .execute(&mut *tx)
        .await
        .map_err(|_| ApiError::Internal)?;

    let osm_taxiways_retired =
        sqlx::query("delete from flow.airport_taxiway where source = 'osm' and icao = any($1)")
            .bind(&icaos)
            .execute(&mut *tx)
            .await
            .map_err(|_| ApiError::Internal)?
            .rows_affected() as usize;
    let osm_ramps_retired =
        sqlx::query("delete from flow.airport_ramp_area where source = 'osm' and icao = any($1)")
            .bind(&icaos)
            .execute(&mut *tx)
            .await
            .map_err(|_| ApiError::Internal)?
            .rows_affected() as usize;

    let mut taxiways_inserted = 0usize;
    let mut ramps_inserted = 0usize;
    for (icao, airport) in &extract {
        for t in &airport.taxiways {
            sqlx::query(
                "insert into flow.airport_taxiway (icao, name, points, source) \
                 values ($1, $2, $3, 'faa')",
            )
            .bind(icao)
            .bind(&t.name)
            .bind(sqlx::types::Json(&t.points))
            .execute(&mut *tx)
            .await
            .map_err(|_| ApiError::Internal)?;
            taxiways_inserted += 1;
        }
        for r in &airport.ramps {
            sqlx::query(
                "insert into flow.airport_ramp_area (icao, name, kind, rings, source) \
                 values ($1, $2, $3, $4, 'faa')",
            )
            .bind(icao)
            .bind(&r.name)
            .bind(&r.kind)
            .bind(sqlx::types::Json(&r.rings))
            .execute(&mut *tx)
            .await
            .map_err(|_| ApiError::Internal)?;
            ramps_inserted += 1;
        }
    }

    tx.commit().await.map_err(|_| ApiError::Internal)?;

    Ok(SeedSummary {
        taxiways_inserted,
        ramps_inserted,
        osm_taxiways_retired,
        osm_ramps_retired,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_extract_parses() {
        let extract = load_bundled_extract();
        assert!(!extract.is_empty());
        let kdca = &extract["KDCA"];
        assert!(!kdca.taxiways.is_empty());
        assert!(!kdca.ramps.is_empty());
    }

    #[test]
    fn a_small_extract_shape_deserializes() {
        let json = serde_json::json!({
            "KTST": {
                "taxiways": [{ "name": "A", "points": [[38.85, -77.04], [38.86, -77.05]] }],
                "ramps": [{ "name": "Ramp 1", "kind": "apron", "rings": [[[38.85, -77.04]]] }]
            }
        });
        let extract: HashMap<String, ExtractAirport> = serde_json::from_value(json).unwrap();
        let ktst = &extract["KTST"];
        assert_eq!(ktst.taxiways[0].name, "A");
        assert_eq!(ktst.ramps[0].kind, "apron");
    }

    #[sqlx::test]
    async fn seed_inserts_rows_from_the_bundled_extract(pool: sqlx::PgPool) {
        let summary = seed(&pool).await.unwrap();
        assert_eq!(summary.taxiways_inserted, 20069);
        assert_eq!(summary.ramps_inserted, 4082);

        let taxiways: i64 = sqlx::query_scalar(
            "select count(*) from flow.airport_taxiway where icao = 'KDCA' and source = 'faa'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(taxiways, 105);
    }

    #[sqlx::test]
    async fn seeding_twice_produces_no_duplicates(pool: sqlx::PgPool) {
        seed(&pool).await.unwrap();
        let summary = seed(&pool).await.unwrap();

        let taxiways: i64 =
            sqlx::query_scalar("select count(*) from flow.airport_taxiway where source = 'faa'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(taxiways as usize, summary.taxiways_inserted);
        // second run's osm-retirement counts are 0 — nothing left to retire.
        assert_eq!(summary.osm_taxiways_retired, 0);
        assert_eq!(summary.osm_ramps_retired, 0);
    }

    #[sqlx::test]
    async fn a_manual_row_for_a_seeded_icao_survives_unchanged(pool: sqlx::PgPool) {
        sqlx::query(
            "insert into flow.airport_taxiway (icao, name, points, source) \
             values ('KDCA', 'Hand-drawn', '[[1,2]]', 'manual')",
        )
        .execute(&pool)
        .await
        .unwrap();

        seed(&pool).await.unwrap();

        let manual: i64 = sqlx::query_scalar(
            "select count(*) from flow.airport_taxiway \
             where icao = 'KDCA' and source = 'manual' and name = 'Hand-drawn'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(manual, 1);
    }

    #[sqlx::test]
    async fn kdcas_pre_existing_osm_rows_are_retired(pool: sqlx::PgPool) {
        // KDCA's migration-0067 osm seed (57 gates, 4 ramps, 84 taxiways) is already present from
        // the fresh #[sqlx::test] database's migrations.
        let before: i64 = sqlx::query_scalar(
            "select count(*) from flow.airport_taxiway where icao = 'KDCA' and source = 'osm'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(before, 84);

        let summary = seed(&pool).await.unwrap();
        assert_eq!(summary.osm_taxiways_retired, 84);
        assert_eq!(summary.osm_ramps_retired, 4);

        let after: i64 = sqlx::query_scalar(
            "select count(*) from flow.airport_taxiway where icao = 'KDCA' and source = 'osm'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(after, 0);

        // KDCA's osm gates are untouched — this seed writes no gate rows at all.
        let gates: i64 = sqlx::query_scalar(
            "select count(*) from flow.airport_gate where icao = 'KDCA' and source = 'osm'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(gates, 57);
    }

    #[sqlx::test]
    async fn an_osm_row_for_an_icao_the_extract_does_not_cover_survives(pool: sqlx::PgPool) {
        sqlx::query(
            "insert into flow.airport_taxiway (icao, name, points, source) \
             values ('KZZZ', 'Untouched', '[[1,2]]', 'osm')",
        )
        .execute(&pool)
        .await
        .unwrap();

        seed(&pool).await.unwrap();

        let still_there: i64 = sqlx::query_scalar(
            "select count(*) from flow.airport_taxiway where icao = 'KZZZ' and source = 'osm'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(still_there, 1);
    }
}
