//! Seeds `flow.airport_ramp_area` / `flow.airport_taxiway` from the bundled FAA Aerodrome Mapping
//! extract produced by `bin/faa_surface_importer.rs` (#230), tagging every row `source = 'faa'`.
//!
//! **Seed once per airport (#231):** an airport is seeded only if it isn't recorded in
//! `flow.airport_surface_faa_seeded` (migration `0074`) and has no `faa` rows; every seeded airport
//! is recorded. FAA geometry is a starting layer that facilities edit in the map editor, and an edit
//! keeps the row's `source = 'faa'` — so re-running never touches an already-seeded airport: edits,
//! deletes (even of every faa row) and row ids all persist. Refreshing one airport's FAA baseline
//! is the explicit, permissioned re-pull (#232), not this boot job. `manual`/`crc` rows are never touched. One exception, an
//! operator decision recorded on VATUSA/OIS#231: when an airport is seeded, its pre-existing
//! `source='osm'` ramps/taxiways (migration `0067`'s KDCA seed) are retired, since FAA data
//! supersedes them. No gate rows are written: the FAA AM layer set has no gate layer (see #230).
//!
//! **Runways (#279)** follow the same seed-once rule with their own marker,
//! `flow.airport_runway_faa_seeded` (migration `0077`): airports seeded before runways existed are
//! already in `airport_surface_faa_seeded` but still get their runways seeded exactly once.
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
    #[serde(default)]
    runways: Vec<ExtractRunway>,
}

#[derive(Debug, Deserialize)]
struct ExtractRunway {
    name: String,
    rings: Vec<Vec<[f64; 2]>>,
}

#[derive(Debug, Deserialize)]
struct ExtractTaxiway {
    name: String,
    rings: Vec<Vec<[f64; 2]>>,
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
    pub runways_inserted: usize,
    pub osm_taxiways_retired: usize,
    pub osm_ramps_retired: usize,
}

impl std::fmt::Display for SeedSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "seeded {} airports ({} taxiways + {} ramps; retired {} osm taxiways, {} osm ramps), \
             {} already seeded; {} runways",
            self.airports_seeded,
            self.taxiways_inserted,
            self.ramps_inserted,
            self.osm_taxiways_retired,
            self.osm_ramps_retired,
            self.airports_skipped,
            self.runways_inserted
        )
    }
}

/// Seeds every extract airport not yet seeded (see the module doc), in one transaction under
/// [`SEED_LOCK_KEY`]. Safe to call repeatedly and concurrently: already-seeded airports are skipped.
pub async fn seed(pool: &PgPool) -> Result<SeedSummary, ApiError> {
    let extract = load_bundled_extract();
    let mut tx = begin_locked(pool).await?;

    // Read after taking the lock, so a run that waited sees what the previous run committed. The
    // marker table is what keeps an airport whose faa rows were all deleted from being re-seeded.
    let seeded: HashSet<String> = sqlx::query_scalar(
        "select icao from flow.airport_surface_faa_seeded \
         union select icao from flow.airport_taxiway where source = 'faa' \
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

    let (osm_taxiways_retired, osm_ramps_retired) = retire_osm(&mut tx, &to_seed).await?;
    let (taxiways_inserted, ramps_inserted) = insert_airports(&mut tx, &extract, &to_seed).await?;
    record_seeded(&mut tx, &to_seed).await?;

    let runways_seeded: HashSet<String> = sqlx::query_scalar(
        "select icao from flow.airport_runway_faa_seeded \
         union select icao from flow.airport_runway where source = 'faa'",
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(db_err)?
    .into_iter()
    .collect();
    let runways_to_seed: Vec<&str> = extract
        .keys()
        .map(String::as_str)
        .filter(|icao| !runways_seeded.contains(*icao))
        .collect();
    let runways_inserted = insert_runways(&mut tx, &extract, &runways_to_seed).await?;
    record_runways_seeded(&mut tx, &runways_to_seed).await?;
    tx.commit().await.map_err(db_err)?;

    Ok(SeedSummary {
        airports_seeded: to_seed.len(),
        airports_skipped: extract.len() - to_seed.len(),
        taxiways_inserted,
        ramps_inserted,
        runways_inserted,
        osm_taxiways_retired,
        osm_ramps_retired,
    })
}

/// The "re-pull from FAA" admin action (#232): an explicit refresh that **replaces** one airport's
/// `source='faa'` rows with the current bundled extract (reverting facility edits to them, unlike
/// [`seed`]), under the same [`SEED_LOCK_KEY`] so it can't race the boot seed. `NotFound`, touching
/// nothing, if the extract doesn't cover `icao` — clearing its faa rows with nothing to replace them
/// would leave an airport whose `osm` rows were already retired with no geometry at all.
/// `manual`/`crc` rows are never touched.
pub async fn seed_for_icao(pool: &PgPool, icao: &str) -> Result<SeedSummary, ApiError> {
    let extract = load_bundled_extract();
    if !extract.contains_key(icao) {
        return Err(ApiError::NotFound);
    }
    let covered = [icao];
    let mut tx = begin_locked(pool).await?;

    for sql in [
        "delete from flow.airport_taxiway where source = 'faa' and icao = $1",
        "delete from flow.airport_ramp_area where source = 'faa' and icao = $1",
        "delete from flow.airport_runway where source = 'faa' and icao = $1",
    ] {
        sqlx::query(sql)
            .bind(icao)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    let (osm_taxiways_retired, osm_ramps_retired) = retire_osm(&mut tx, &covered).await?;
    let (taxiways_inserted, ramps_inserted) = insert_airports(&mut tx, &extract, &covered).await?;
    let runways_inserted = insert_runways(&mut tx, &extract, &covered).await?;
    record_seeded(&mut tx, &covered).await?;
    record_runways_seeded(&mut tx, &covered).await?;
    tx.commit().await.map_err(db_err)?;

    Ok(SeedSummary {
        airports_seeded: 1,
        airports_skipped: 0,
        taxiways_inserted,
        ramps_inserted,
        runways_inserted,
        osm_taxiways_retired,
        osm_ramps_retired,
    })
}

async fn begin_locked(
    pool: &PgPool,
) -> Result<sqlx::Transaction<'static, sqlx::Postgres>, ApiError> {
    let mut tx = pool.begin().await.map_err(db_err)?;
    sqlx::query("select pg_advisory_xact_lock($1)")
        .bind(SEED_LOCK_KEY)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    Ok(tx)
}

/// Records `icaos` as FAA-seeded (`flow.airport_surface_faa_seeded`, migration `0074`) so the boot
/// seed never re-populates them, even after every faa row is deleted.
async fn record_seeded(conn: &mut sqlx::PgConnection, icaos: &[&str]) -> Result<(), ApiError> {
    sqlx::query(
        "insert into flow.airport_surface_faa_seeded (icao) select unnest($1::text[]) \
         on conflict (icao) do nothing",
    )
    .bind(icaos)
    .execute(&mut *conn)
    .await
    .map_err(db_err)?;
    Ok(())
}

/// Retires the `osm` ramps/taxiways of `icaos` (airports FAA data is being written for). Returns
/// `(taxiways, ramps)` deleted. Gates are never touched.
async fn retire_osm(
    conn: &mut sqlx::PgConnection,
    icaos: &[&str],
) -> Result<(usize, usize), ApiError> {
    let taxiways =
        sqlx::query("delete from flow.airport_taxiway where source = 'osm' and icao = any($1)")
            .bind(icaos)
            .execute(&mut *conn)
            .await
            .map_err(db_err)?
            .rows_affected() as usize;
    let ramps =
        sqlx::query("delete from flow.airport_ramp_area where source = 'osm' and icao = any($1)")
            .bind(icaos)
            .execute(&mut *conn)
            .await
            .map_err(db_err)?
            .rows_affected() as usize;
    Ok((taxiways, ramps))
}

/// Inserts every extract row for `icaos` as `source='faa'` — one set-based insert per table (column
/// arrays via `unnest`) rather than a round trip per row. Returns `(taxiways, ramps)` inserted.
async fn insert_airports(
    conn: &mut sqlx::PgConnection,
    extract: &HashMap<String, ExtractAirport>,
    icaos: &[&str],
) -> Result<(usize, usize), ApiError> {
    let (mut tw_icao, mut tw_name, mut tw_rings) = (Vec::new(), Vec::new(), Vec::new());
    let (mut ra_icao, mut ra_name, mut ra_kind, mut ra_rings) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for icao in icaos {
        let airport = &extract[*icao];
        for t in &airport.taxiways {
            tw_icao.push(*icao);
            tw_name.push(t.name.as_str());
            tw_rings.push(sqlx::types::Json(&t.rings));
        }
        for r in &airport.ramps {
            ra_icao.push(*icao);
            ra_name.push(r.name.as_str());
            ra_kind.push(r.kind.as_str());
            ra_rings.push(sqlx::types::Json(&r.rings));
        }
    }

    let taxiways_inserted = sqlx::query(
        "insert into flow.airport_taxiway (icao, name, rings, source) \
         select icao, name, rings, 'faa' from unnest($1::text[], $2::text[], $3::jsonb[]) \
         as t(icao, name, rings)",
    )
    .bind(&tw_icao)
    .bind(&tw_name)
    .bind(&tw_rings)
    .execute(&mut *conn)
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
    .execute(&mut *conn)
    .await
    .map_err(db_err)?
    .rows_affected() as usize;

    Ok((taxiways_inserted, ramps_inserted))
}

/// Records `icaos` as runway-seeded (`flow.airport_runway_faa_seeded`, migration `0077`).
async fn record_runways_seeded(
    conn: &mut sqlx::PgConnection,
    icaos: &[&str],
) -> Result<(), ApiError> {
    sqlx::query(
        "insert into flow.airport_runway_faa_seeded (icao) select unnest($1::text[]) \
         on conflict (icao) do nothing",
    )
    .bind(icaos)
    .execute(&mut *conn)
    .await
    .map_err(db_err)?;
    Ok(())
}

/// Inserts every extract runway for `icaos` as `source='faa'` in one set-based insert. Returns the
/// number inserted.
async fn insert_runways(
    conn: &mut sqlx::PgConnection,
    extract: &HashMap<String, ExtractAirport>,
    icaos: &[&str],
) -> Result<usize, ApiError> {
    let (mut rw_icao, mut rw_name, mut rw_rings) = (Vec::new(), Vec::new(), Vec::new());
    for icao in icaos {
        for r in &extract[*icao].runways {
            rw_icao.push(*icao);
            rw_name.push(r.name.as_str());
            rw_rings.push(sqlx::types::Json(&r.rings));
        }
    }
    Ok(sqlx::query(
        "insert into flow.airport_runway (icao, name, rings, source) \
         select icao, name, rings, 'faa' from unnest($1::text[], $2::text[], $3::jsonb[]) \
         as t(icao, name, rings)",
    )
    .bind(&rw_icao)
    .bind(&rw_name)
    .bind(&rw_rings)
    .execute(&mut *conn)
    .await
    .map_err(db_err)?
    .rows_affected() as usize)
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
                "taxiways": [{ "name": "A", "rings": [[[38.85, -77.04], [38.86, -77.05], [38.85, -77.05]]] }],
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
        assert_eq!(summary.runways_inserted, 432);
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

    async fn kdca_faa_runways(pool: &PgPool) -> i64 {
        count(
            pool,
            "select count(*) from flow.airport_runway where icao = 'KDCA' and source = 'faa'",
        )
        .await
    }

    /// #279: an existing deploy's airports are all recorded in `airport_surface_faa_seeded` from
    /// before runways existed — the boot seed must still add their runways, once.
    #[sqlx::test]
    async fn runways_are_seeded_for_an_airport_already_surface_seeded(pool: PgPool) {
        sqlx::query("insert into flow.airport_surface_faa_seeded (icao) values ('KDCA')")
            .execute(&pool)
            .await
            .unwrap();

        seed(&pool).await.unwrap();

        assert_eq!(kdca_faa_runways(&pool).await, 3);
        // KDCA's surface layers were already seeded, so none were re-inserted.
        assert_eq!(
            count(
                &pool,
                "select count(*) from flow.airport_taxiway where icao = 'KDCA' and source = 'faa'"
            )
            .await,
            0
        );
    }

    #[sqlx::test]
    async fn a_deleted_faa_runway_stays_deleted_until_a_repull(pool: PgPool) {
        seed(&pool).await.unwrap();
        sqlx::query(
            "delete from flow.airport_runway where id = \
             (select id from flow.airport_runway where icao = 'KDCA' and source = 'faa' limit 1)",
        )
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(seed(&pool).await.unwrap().runways_inserted, 0);
        assert_eq!(kdca_faa_runways(&pool).await, 2);

        assert_eq!(
            seed_for_icao(&pool, "KDCA").await.unwrap().runways_inserted,
            3
        );
        assert_eq!(kdca_faa_runways(&pool).await, 3);
    }

    /// Deleting *every* faa runway an airport has leaves the marker as the only thing standing
    /// between the facility and a re-seed on the next boot — the row guard can't help here.
    #[sqlx::test]
    async fn an_airport_whose_faa_runways_are_all_deleted_does_not_get_them_back(pool: PgPool) {
        seed(&pool).await.unwrap();
        sqlx::query("delete from flow.airport_runway where icao = 'KDCA' and source = 'faa'")
            .execute(&pool)
            .await
            .unwrap();

        assert_eq!(seed(&pool).await.unwrap().runways_inserted, 0);
        assert_eq!(kdca_faa_runways(&pool).await, 0);
    }

    /// And with the marker gone but the rows still there, the existing-rows guard is what prevents
    /// a duplicate set.
    #[sqlx::test]
    async fn an_airport_missing_its_runway_marker_is_not_seeded_twice(pool: PgPool) {
        seed(&pool).await.unwrap();
        sqlx::query("delete from flow.airport_runway_faa_seeded where icao = 'KDCA'")
            .execute(&pool)
            .await
            .unwrap();

        assert_eq!(seed(&pool).await.unwrap().runways_inserted, 0);
        assert_eq!(kdca_faa_runways(&pool).await, 3);
    }

    #[sqlx::test]
    async fn a_second_run_touches_nothing(pool: PgPool) {
        seed(&pool).await.unwrap();
        let summary = seed(&pool).await.unwrap();

        assert_eq!(summary.airports_seeded, 0);
        assert_eq!(summary.airports_skipped, 185);
        assert_eq!(
            summary.taxiways_inserted + summary.ramps_inserted + summary.runways_inserted,
            0
        );
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
            rings: vec![vec![[38.85, -77.04], [38.86, -77.05], [38.85, -77.05]]],
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

    async fn delete_all_faa_rows(pool: &PgPool, icao: &str) {
        for table in ["flow.airport_taxiway", "flow.airport_ramp_area"] {
            sqlx::query(&format!(
                "delete from {table} where icao = $1 and source = 'faa'"
            ))
            .bind(icao)
            .execute(pool)
            .await
            .unwrap();
        }
    }

    /// Regression (#231 QA): "already seeded" was inferred from faa rows existing, so a facility
    /// deleting all of an airport's faa rows (to redraw it by hand) got them re-inserted next boot.
    #[sqlx::test]
    async fn an_airport_whose_faa_rows_were_all_deleted_is_not_reseeded(pool: PgPool) {
        seed(&pool).await.unwrap();
        delete_all_faa_rows(&pool, "KRUT").await;

        let summary = seed(&pool).await.unwrap();

        assert_eq!(summary.airports_seeded, 0);
        assert_eq!(
            count(
                &pool,
                "select count(*) from flow.airport_taxiway where icao = 'KRUT' and source = 'faa'"
            )
            .await,
            0
        );
    }

    #[sqlx::test]
    async fn an_unrecorded_airport_with_no_faa_rows_is_seeded_without_touching_others(
        pool: PgPool,
    ) {
        seed(&pool).await.unwrap();
        let kdca_id = first_faa_taxiway(&pool, "KDCA").await;
        delete_all_faa_rows(&pool, "KRUT").await;
        sqlx::query("delete from flow.airport_surface_faa_seeded where icao = 'KRUT'")
            .execute(&pool)
            .await
            .unwrap();

        let summary = seed(&pool).await.unwrap();

        assert_eq!(summary.airports_seeded, 1);
        assert_eq!(summary.taxiways_inserted, 45);
        assert_eq!(first_faa_taxiway(&pool, "KDCA").await, kdca_id);
    }

    #[sqlx::test]
    async fn every_seeded_airport_is_recorded(pool: PgPool) {
        seed(&pool).await.unwrap();
        assert_eq!(
            count(
                &pool,
                "select count(*) from flow.airport_surface_faa_seeded"
            )
            .await,
            185
        );
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
            "insert into flow.airport_taxiway (icao, name, rings, source) values \
             ('KDCA', 'Hand-drawn', '[[[1,2]]]', 'manual'), ('KDCA', 'Imported', '[[[1,2]]]', 'crc')",
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

    async fn insert_osm_taxiways(pool: &PgPool, icao: &str, n: i32) {
        sqlx::query(
            "insert into flow.airport_taxiway (icao, name, rings, source) \
             select $1, 'OSM ' || i, '[[[1,2],[1,3],[2,3]]]', 'osm' from generate_series(1, $2) as i",
        )
        .bind(icao)
        .bind(n)
        .execute(pool)
        .await
        .unwrap();
    }

    #[sqlx::test]
    async fn kdcas_pre_existing_osm_rows_are_retired(pool: PgPool) {
        // KDCA's migration-0067 osm seed (57 gates, 4 ramps) is already present from the fresh
        // #[sqlx::test] database's migrations; its taxiway centerlines were deleted by 0076 (#278),
        // so add osm taxiway rows here to cover their retirement too.
        insert_osm_taxiways(&pool, "KDCA", 2).await;

        let summary = seed(&pool).await.unwrap();
        assert_eq!(summary.osm_taxiways_retired, 2);
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

    /// `osm` retirement applies only to airports being seeded in that run — an `osm` row added to an
    /// already-seeded airport afterwards survives a rerun.
    #[sqlx::test]
    async fn osm_rows_of_an_already_seeded_airport_survive_a_rerun(pool: PgPool) {
        seed(&pool).await.unwrap();
        sqlx::query(
            "insert into flow.airport_taxiway (icao, name, rings, source) \
             values ('KDCA', 'Later', '[[[1,2]]]', 'osm')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into flow.airport_ramp_area (icao, name, kind, rings, source) \
             values ('KDCA', 'Later', 'apron', '[[[1,2]]]', 'osm')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let summary = seed(&pool).await.unwrap();

        assert_eq!(summary.osm_taxiways_retired + summary.osm_ramps_retired, 0);
        assert_eq!(
            count(
                &pool,
                "select count(*) from flow.airport_taxiway where icao = 'KDCA' and source = 'osm'"
            )
            .await,
            1
        );
        assert_eq!(
            count(
                &pool,
                "select count(*) from flow.airport_ramp_area where icao = 'KDCA' and source = 'osm'"
            )
            .await,
            1
        );
    }

    #[sqlx::test]
    async fn an_osm_row_for_an_icao_the_extract_does_not_cover_survives(pool: PgPool) {
        sqlx::query(
            "insert into flow.airport_taxiway (icao, name, rings, source) \
             values ('KZZZ', 'Untouched', '[[[1,2]]]', 'osm')",
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

    #[sqlx::test]
    async fn seed_for_icao_only_touches_the_requested_airport(pool: PgPool) {
        let summary = seed_for_icao(&pool, "KDCA").await.unwrap();
        assert_eq!(summary.taxiways_inserted, 105);
        assert_eq!(summary.ramps_inserted, 15);

        // A different airport the extract also covers is untouched by this call.
        let katl: i64 = sqlx::query_scalar(
            "select count(*) from flow.airport_taxiway where icao = 'KATL' and source = 'faa'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(katl, 0);
    }

    #[sqlx::test]
    async fn seed_for_icao_retires_only_that_airports_osm_rows(pool: PgPool) {
        insert_osm_taxiways(&pool, "KDCA", 2).await;
        let summary = seed_for_icao(&pool, "KDCA").await.unwrap();
        assert_eq!(summary.osm_taxiways_retired, 2);
        assert_eq!(summary.osm_ramps_retired, 4);

        let gates: i64 = sqlx::query_scalar(
            "select count(*) from flow.airport_gate where icao = 'KDCA' and source = 'osm'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(gates, 57);
    }

    #[sqlx::test]
    async fn seed_for_icao_leaves_a_manual_row_untouched(pool: PgPool) {
        sqlx::query(
            "insert into flow.airport_taxiway (icao, name, rings, source) \
             values ('KDCA', 'Hand-drawn', '[[[1,2]]]', 'manual')",
        )
        .execute(&pool)
        .await
        .unwrap();

        seed_for_icao(&pool, "KDCA").await.unwrap();

        let manual: i64 = sqlx::query_scalar(
            "select count(*) from flow.airport_taxiway \
             where icao = 'KDCA' and source = 'manual' and name = 'Hand-drawn'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(manual, 1);
    }

    /// Regression (#232 QA): re-pulling an airport the extract doesn't cover (e.g. one a regenerated
    /// extract dropped) used to delete its faa rows with nothing to replace them and report success.
    #[sqlx::test]
    async fn seed_for_icao_not_covered_by_the_extract_is_not_found_and_touches_nothing(
        pool: PgPool,
    ) {
        sqlx::query(
            "insert into flow.airport_taxiway (icao, name, rings, source) values \
             ('KZZZ', 'Previously seeded', '[[[1,2]]]', 'faa'), ('KZZZ', 'Untouched', '[[[1,2]]]', 'osm')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let r = seed_for_icao(&pool, "KZZZ").await;

        assert!(matches!(r, Err(ApiError::NotFound)));
        let rows: i64 =
            sqlx::query_scalar("select count(*) from flow.airport_taxiway where icao = 'KZZZ'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(rows, 2, "neither the faa nor the osm row may be deleted");
    }

    /// The re-pull is an explicit refresh: unlike the boot seed, it replaces the airport's faa rows,
    /// reverting a facility edit, without duplicating anything.
    #[sqlx::test]
    async fn seed_for_icao_restores_the_faa_baseline_after_an_edit(pool: PgPool) {
        seed(&pool).await.unwrap();
        let actor = seed_user(&pool).await;
        let edited = first_faa_taxiway(&pool, "KDCA").await;
        let req = UpsertAirportTaxiwayRequest {
            name: "EDITED".into(),
            rings: vec![vec![[38.85, -77.04], [38.86, -77.05], [38.85, -77.05]]],
        };
        airport_surface::update_taxiway(&pool, &edited, "KDCA", &req, &actor)
            .await
            .unwrap()
            .expect("the faa row is editable");

        let summary = seed_for_icao(&pool, "KDCA").await.unwrap();

        assert_eq!(summary.taxiways_inserted, 105);
        assert_eq!(
            count(
                &pool,
                "select count(*) from flow.airport_taxiway where icao = 'KDCA' and source = 'faa'"
            )
            .await,
            105
        );
        assert_eq!(
            count(
                &pool,
                "select count(*) from flow.airport_taxiway where icao = 'KDCA' and name = 'EDITED'"
            )
            .await,
            0
        );
    }

    /// The re-pull and the boot seed share `SEED_LOCK_KEY`, so racing them can't double-insert.
    #[sqlx::test]
    async fn seed_for_icao_and_seed_do_not_duplicate_when_concurrent(pool: PgPool) {
        let (a, b) = tokio::join!(seed(&pool), seed_for_icao(&pool, "KDCA"));
        a.unwrap();
        b.unwrap();
        assert_eq!(
            count(
                &pool,
                "select count(*) from flow.airport_taxiway where icao = 'KDCA' and source = 'faa'"
            )
            .await,
            105
        );
    }

    /// A re-pull records the airport as FAA-seeded, so a facility later deleting all of its faa rows
    /// isn't undone by the next boot seed; a rejected re-pull of an airport the extract doesn't cover
    /// records nothing.
    #[sqlx::test]
    async fn seed_for_icao_records_the_airport_as_seeded(pool: PgPool) {
        seed_for_icao(&pool, "KDCA").await.unwrap();
        assert!(matches!(
            seed_for_icao(&pool, "KZZZ").await,
            Err(ApiError::NotFound)
        ));
        assert_eq!(
            count(
                &pool,
                "select count(*) from flow.airport_surface_faa_seeded where icao in ('KDCA', 'KZZZ')"
            )
            .await,
            1
        );

        delete_all_faa_rows(&pool, "KDCA").await;
        seed(&pool).await.unwrap();
        assert_eq!(
            count(
                &pool,
                "select count(*) from flow.airport_taxiway where icao = 'KDCA' and source = 'faa'"
            )
            .await,
            0
        );
    }
}
