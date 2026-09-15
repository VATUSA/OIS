//! Seeds `flow.airport_ramp_area` / `flow.airport_taxiway` from the bundled FAA Aerodrome Mapping
//! extract produced by `bin/faa_surface_importer.rs` (#230), tagging every row `source = 'faa'`.
//!
//! **Seed-if-absent (#231):** an airport is seeded only when it has no `faa` rows in either table.
//! FAA geometry is a starting layer that facilities edit in the map editor, and an edit keeps the
//! row's `source = 'faa'` — so re-running never touches an already-seeded airport: edits, deletes
//! and row ids all persist. Refreshing one airport's FAA baseline is the explicit, permissioned
//! re-pull (#232), not this boot job. `manual`/`crc` rows are never touched. One exception, an
//! operator decision recorded on VATUSA/OIS#231: when an airport is seeded, its pre-existing
//! `source='osm'` ramps/taxiways (migration `0067`'s KDCA seed) are retired, since FAA data
//! supersedes them. No gate rows are written: the FAA AM layer set has no gate layer (see #230).
//!
//! Deliberately not folded into `repos::airport_surface` — that module is per-request CRUD for the
//! map editor; this is a bulk load from bundled, compiled-in data, closer in spirit to
//! `feed::runway_db`'s `include_str!` pattern than to a handler-facing repo.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;
use sqlx::PgPool;

use crate::errors::ApiError;

/// `pg_advisory_xact_lock` key serializing FAA surface seeds across backend processes (and any
/// other writer of `faa` rows): without it two concurrent runs both see an airport as unseeded and
/// both insert its rows.
const SEED_LOCK_KEY: i64 = 0x004F_4953_5F46_4141; // "OIS_FAA"

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

fn db_err(e: sqlx::Error) -> ApiError {
    tracing::error!(error = %e, "FAA surface seed query failed");
    ApiError::Internal
}

#[derive(Debug, Default)]
pub struct SeedSummary {
    pub airports_seeded: usize,
    pub airports_skipped: usize,
    pub taxiways_inserted: usize,
    pub ramps_inserted: usize,
    pub osm_taxiways_retired: usize,
    pub osm_ramps_retired: usize,
}

impl std::fmt::Display for SeedSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "seeded {} airports ({} taxiways + {} ramps; retired {} osm taxiways, {} osm ramps), \
             {} already seeded",
            self.airports_seeded,
            self.taxiways_inserted,
            self.ramps_inserted,
            self.osm_taxiways_retired,
            self.osm_ramps_retired,
            self.airports_skipped
        )
    }
}

/// Seeds every extract airport that has no `faa` rows yet, in one transaction under
/// [`SEED_LOCK_KEY`]. Safe to call repeatedly and concurrently: already-seeded airports are skipped.
pub async fn seed(pool: &PgPool) -> Result<SeedSummary, ApiError> {
    let extract = load_bundled_extract();

    let mut tx = pool.begin().await.map_err(db_err)?;
    sqlx::query("select pg_advisory_xact_lock($1)")
        .bind(SEED_LOCK_KEY)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

    // Read after taking the lock, so a run that waited sees what the previous run committed.
    let seeded: HashSet<String> = sqlx::query_scalar(
        "select icao from flow.airport_taxiway where source = 'faa' \
         union select icao from flow.airport_ramp_area where source = 'faa'",
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(db_err)?
    .into_iter()
    .collect();
    let to_seed: Vec<&str> = extract
        .keys()
        .map(String::as_str)
        .filter(|icao| !seeded.contains(*icao))
        .collect();

    let osm_taxiways_retired =
        sqlx::query("delete from flow.airport_taxiway where source = 'osm' and icao = any($1)")
            .bind(&to_seed)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?
            .rows_affected() as usize;
    let osm_ramps_retired =
        sqlx::query("delete from flow.airport_ramp_area where source = 'osm' and icao = any($1)")
            .bind(&to_seed)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?
            .rows_affected() as usize;

    // One set-based insert per table (column arrays via `unnest`) rather than ~24k round trips.
    let (mut tw_icao, mut tw_name, mut tw_points) = (Vec::new(), Vec::new(), Vec::new());
    let (mut ra_icao, mut ra_name, mut ra_kind, mut ra_rings) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for icao in &to_seed {
        let airport = &extract[*icao];
        for t in &airport.taxiways {
            tw_icao.push(*icao);
            tw_name.push(t.name.as_str());
            tw_points.push(sqlx::types::Json(&t.points));
        }
        for r in &airport.ramps {
            ra_icao.push(*icao);
            ra_name.push(r.name.as_str());
            ra_kind.push(r.kind.as_str());
            ra_rings.push(sqlx::types::Json(&r.rings));
        }
    }

    let taxiways_inserted = sqlx::query(
        "insert into flow.airport_taxiway (icao, name, points, source) \
         select icao, name, points, 'faa' from unnest($1::text[], $2::text[], $3::jsonb[]) \
         as t(icao, name, points)",
    )
    .bind(&tw_icao)
    .bind(&tw_name)
    .bind(&tw_points)
    .execute(&mut *tx)
    .await
    .map_err(db_err)?
    .rows_affected() as usize;
    let ramps_inserted = sqlx::query(
        "insert into flow.airport_ramp_area (icao, name, kind, rings, source) \
         select icao, name, kind, rings, 'faa' \
         from unnest($1::text[], $2::text[], $3::text[], $4::jsonb[]) as r(icao, name, kind, rings)",
    )
    .bind(&ra_icao)
    .bind(&ra_name)
    .bind(&ra_kind)
    .bind(&ra_rings)
    .execute(&mut *tx)
    .await
    .map_err(db_err)?
    .rows_affected() as usize;

    tx.commit().await.map_err(db_err)?;

    Ok(SeedSummary {
        airports_seeded: to_seed.len(),
        airports_skipped: extract.len() - to_seed.len(),
        taxiways_inserted,
        ramps_inserted,
        osm_taxiways_retired,
        osm_ramps_retired,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::UpsertAirportTaxiwayRequest;
    use crate::repos::airport_surface;
    use crate::scope_test_support::seed_user;

    async fn count(pool: &PgPool, sql: &str) -> i64 {
        sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
    }

    async fn first_faa_taxiway(pool: &PgPool, icao: &str) -> String {
        sqlx::query_scalar(
            "select id from flow.airport_taxiway where icao = $1 and source = 'faa' \
             order by id limit 1",
        )
        .bind(icao)
        .fetch_one(pool)
        .await
        .unwrap()
    }

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
    async fn seed_inserts_rows_from_the_bundled_extract(pool: PgPool) {
        let summary = seed(&pool).await.unwrap();
        assert_eq!(summary.taxiways_inserted, 20069);
        assert_eq!(summary.ramps_inserted, 4082);
        assert_eq!(summary.airports_seeded, 185);
        assert_eq!(
            count(
                &pool,
                "select count(*) from flow.airport_taxiway where icao = 'KDCA' and source = 'faa'"
            )
            .await,
            105
        );
    }

    #[sqlx::test]
    async fn a_second_run_touches_nothing(pool: PgPool) {
        seed(&pool).await.unwrap();
        let summary = seed(&pool).await.unwrap();

        assert_eq!(summary.airports_seeded, 0);
        assert_eq!(summary.airports_skipped, 185);
        assert_eq!(summary.taxiways_inserted + summary.ramps_inserted, 0);
        assert_eq!(
            count(
                &pool,
                "select count(*) from flow.airport_taxiway where source = 'faa'"
            )
            .await,
            20069
        );
    }

    /// Regression (#231 QA): the seed used to replace every `faa` row on each run, reverting
    /// map-editor edits, resurrecting deleted rows and churning every row id.
    #[sqlx::test]
    async fn facility_edits_and_deletes_of_faa_rows_survive_a_rerun(pool: PgPool) {
        seed(&pool).await.unwrap();
        let actor = seed_user(&pool).await;
        let edited = first_faa_taxiway(&pool, "KDCA").await;
        let req = UpsertAirportTaxiwayRequest {
            name: "EDITED".into(),
            points: vec![[38.85, -77.04], [38.86, -77.05]],
        };
        airport_surface::update_taxiway(&pool, &edited, "KDCA", &req, &actor)
            .await
            .unwrap()
            .expect("the faa row is editable");
        let deleted: String = sqlx::query_scalar(
            "select id from flow.airport_taxiway where icao = 'KDCA' and source = 'faa' \
             and id <> $1 limit 1",
        )
        .bind(&edited)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            airport_surface::delete_taxiway(&pool, &deleted, "KDCA")
                .await
                .unwrap()
        );

        seed(&pool).await.unwrap();

        let name: String =
            sqlx::query_scalar("select name from flow.airport_taxiway where id = $1")
                .bind(&edited)
                .fetch_one(&pool)
                .await
                .expect("edited row keeps its id");
        assert_eq!(name, "EDITED");
        assert_eq!(
            count(
                &pool,
                "select count(*) from flow.airport_taxiway where icao = 'KDCA' and source = 'faa'"
            )
            .await,
            104,
            "the deleted faa row must stay deleted"
        );
    }

    #[sqlx::test]
    async fn concurrent_runs_do_not_duplicate_rows(pool: PgPool) {
        let (a, b) = tokio::join!(seed(&pool), seed(&pool));
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_eq!(a.airports_seeded + b.airports_seeded, 185);
        assert_eq!(
            count(
                &pool,
                "select count(*) from flow.airport_taxiway where source = 'faa'"
            )
            .await,
            20069
        );
    }

    #[sqlx::test]
    async fn only_airports_without_faa_rows_are_seeded(pool: PgPool) {
        seed(&pool).await.unwrap();
        let kdca_id = first_faa_taxiway(&pool, "KDCA").await;
        // An airport whose faa rows are all gone (both tables) counts as unseeded again.
        sqlx::query("delete from flow.airport_taxiway where icao = 'KRUT' and source = 'faa'")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("delete from flow.airport_ramp_area where icao = 'KRUT' and source = 'faa'")
            .execute(&pool)
            .await
            .unwrap();

        let summary = seed(&pool).await.unwrap();

        assert_eq!(summary.airports_seeded, 1);
        assert_eq!(summary.taxiways_inserted, 45);
        assert_eq!(first_faa_taxiway(&pool, "KDCA").await, kdca_id);
    }

    #[sqlx::test]
    async fn an_airport_with_only_faa_ramps_left_is_not_reseeded(pool: PgPool) {
        seed(&pool).await.unwrap();
        sqlx::query("delete from flow.airport_taxiway where icao = 'KRUT' and source = 'faa'")
            .execute(&pool)
            .await
            .unwrap();

        let summary = seed(&pool).await.unwrap();

        assert_eq!(summary.airports_seeded, 0);
        assert_eq!(
            count(
                &pool,
                "select count(*) from flow.airport_ramp_area where icao = 'KRUT' and source = 'faa'"
            )
            .await,
            11
        );
    }

    #[sqlx::test]
    async fn manual_and_crc_rows_for_a_seeded_icao_survive(pool: PgPool) {
        sqlx::query(
            "insert into flow.airport_taxiway (icao, name, points, source) values \
             ('KDCA', 'Hand-drawn', '[[1,2]]', 'manual'), ('KDCA', 'Imported', '[[1,2]]', 'crc')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into flow.airport_ramp_area (icao, name, kind, rings, source) values \
             ('KDCA', 'Hand-drawn', 'ramp', '[[[1,2]]]', 'manual'), \
             ('KDCA', 'Imported', 'ramp', '[[[1,2]]]', 'crc')",
        )
        .execute(&pool)
        .await
        .unwrap();

        seed(&pool).await.unwrap();

        assert_eq!(
            count(
                &pool,
                "select count(*) from flow.airport_taxiway \
                 where icao = 'KDCA' and source in ('manual', 'crc')"
            )
            .await,
            2
        );
        assert_eq!(
            count(
                &pool,
                "select count(*) from flow.airport_ramp_area \
                 where icao = 'KDCA' and source in ('manual', 'crc')"
            )
            .await,
            2
        );
    }

    #[sqlx::test]
    async fn kdcas_pre_existing_osm_rows_are_retired(pool: PgPool) {
        // KDCA's migration-0067 osm seed (57 gates, 4 ramps, 84 taxiways) is already present from
        // the fresh #[sqlx::test] database's migrations.
        assert_eq!(
            count(
                &pool,
                "select count(*) from flow.airport_taxiway where icao = 'KDCA' and source = 'osm'"
            )
            .await,
            84
        );

        let summary = seed(&pool).await.unwrap();
        assert_eq!(summary.osm_taxiways_retired, 84);
        assert_eq!(summary.osm_ramps_retired, 4);
        assert_eq!(
            count(
                &pool,
                "select count(*) from flow.airport_taxiway where icao = 'KDCA' and source = 'osm'"
            )
            .await,
            0
        );
        // KDCA's osm gates are untouched — this seed writes no gate rows at all.
        assert_eq!(
            count(
                &pool,
                "select count(*) from flow.airport_gate where icao = 'KDCA' and source = 'osm'"
            )
            .await,
            57
        );
    }

    #[sqlx::test]
    async fn an_osm_row_for_an_icao_the_extract_does_not_cover_survives(pool: PgPool) {
        sqlx::query(
            "insert into flow.airport_taxiway (icao, name, points, source) \
             values ('KZZZ', 'Untouched', '[[1,2]]', 'osm')",
        )
        .execute(&pool)
        .await
        .unwrap();

        seed(&pool).await.unwrap();

        assert_eq!(
            count(
                &pool,
                "select count(*) from flow.airport_taxiway where icao = 'KZZZ' and source = 'osm'"
            )
            .await,
            1
        );
    }
}
