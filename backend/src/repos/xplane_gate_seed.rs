//! Seeds airport parking stands (gates) from the committed X-Plane Scenery Gateway extract
//! (`backend/data/xplane_gates.json`, produced by the hand-run `xplane-gate-importer` binary).
//!
//! Mirrors [`super::faa_surface_seed`] — the same two-stage shape the FAA surface data uses: an
//! offline importer commits an extract, and a boot job seeds it idempotently. Nothing here fetches
//! anything at runtime.
//!
//! ## Seed-once, per airport
//!
//! An airport is seeded when it is in neither `flow.airport_gate_xplane_seeded` (migration `0087`) nor
//! already carrying `source='xplane'` gates. The marker table is the half that matters: without it, a
//! facility that deliberately deleted every imported stand at their field would have them all come
//! back on the next restart.
//!
//! ## Why gates cannot copy the FAA re-pull
//!
//! `stats.taxi_observation.gate_id` is `references flow.airport_gate(id) on delete set null`
//! (migration `0070`). The FAA tables have no such foreign key, so its re-pull is free to delete its
//! own rows and re-insert them. Doing that to gates would mint fresh uuids and **silently null every
//! learned observation at that airport**, destroying the taxi history that the `gate_type_runway`
//! estimate tier is built from — the exact thing #431 exists to make reachable.
//!
//! So [`seed_for_icao`] matches on `(icao, name)` and updates in place: positions and kinds are
//! refreshed, ids survive, and observations stay attached. The importer guarantees names are unique
//! within an airport so that match is unambiguous.
//!
//! ## Retiring the osm gates
//!
//! KDCA carries 57 hand-committed `osm` gates from migration `0067` and the extract has 75 stands for
//! the same airport. Keeping both would put two rows at nearly the same position for one physical
//! stand, and `feed::flow::nearest_gate` would split that stand's observations across two ids —
//! halving the samples counted against `MIN_SAMPLES`, which delays the very tier this unblocks. So the
//! seed retires `osm` gates at the airports it covers, exactly as
//! `faa_surface_seed::retire_osm` retires `osm` taxiways and ramps. `manual` and `crc` rows are never
//! touched.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;
use sqlx::PgPool;

use crate::errors::ApiError;

/// Advisory-lock key, so two concurrent runs cannot both see an airport as unseeded and both insert
/// its stands. Distinct from `faa_surface_seed`'s key: the two seeds write different tables and have
/// no reason to block each other.
const SEED_LOCK_KEY: i64 = 0x004F_4953_5F58_5047; // "OIS_XPG"

/// One stand as committed by the importer. `heading` is present in the extract but deliberately not
/// deserialized: the model has no heading column, and parsing a field we discard would invite someone
/// to believe it is stored.
#[derive(Debug, Deserialize)]
struct ExtractStand {
    name: String,
    lat: f64,
    lon: f64,
    kind: String,
}

fn load_bundled_extract() -> HashMap<String, Vec<ExtractStand>> {
    serde_json::from_str(include_str!("../../data/xplane_gates.json"))
        .expect("bundled backend/data/xplane_gates.json should parse")
}

fn db_err(e: sqlx::Error) -> ApiError {
    tracing::error!(error = %e, "X-Plane gate seed query failed");
    ApiError::Internal
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct GateSeedSummary {
    pub airports_seeded: usize,
    pub airports_skipped: usize,
    pub gates_inserted: usize,
    /// Stands updated in place by a re-pull (always 0 for the boot seed, which only ever inserts).
    /// Reported separately because a healthy re-pull refreshes every stand and inserts none — without
    /// this an operator sees `gates_inserted: 0` and concludes the re-pull did nothing.
    pub gates_refreshed: usize,
    pub osm_gates_retired: usize,
}

impl std::fmt::Display for GateSeedSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "seeded {} airport(s) ({} gate(s)), skipped {} already seeded, retired {} osm gate(s)",
            self.airports_seeded,
            self.gates_inserted,
            self.airports_skipped,
            self.osm_gates_retired
        )
    }
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

/// Seeds every extract airport not yet seeded, in one transaction under [`SEED_LOCK_KEY`]. Safe to
/// call repeatedly and concurrently.
pub async fn seed(pool: &PgPool) -> Result<GateSeedSummary, ApiError> {
    let extract = load_bundled_extract();
    let mut tx = begin_locked(pool).await?;

    // Read after taking the lock, so a run that waited sees what the previous run committed.
    let seeded: HashSet<String> = sqlx::query_scalar(
        "select icao from flow.airport_gate_xplane_seeded \
         union select icao from flow.airport_gate where source = 'xplane'",
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

    let osm_gates_retired = retire_osm_gates(&mut tx, &to_seed).await?;
    let gates_inserted = insert_gates(&mut tx, &extract, &to_seed).await?;
    record_seeded(&mut tx, &to_seed).await?;
    tx.commit().await.map_err(db_err)?;

    Ok(GateSeedSummary {
        airports_seeded: to_seed.len(),
        airports_skipped: extract.len() - to_seed.len(),
        gates_inserted,
        gates_refreshed: 0,
        osm_gates_retired,
    })
}

/// Re-pulls one airport's stands from the extract, on operator request.
///
/// Matches existing `xplane` rows on `(icao, name)` and updates them in place, so gate ids — and the
/// `stats.taxi_observation` rows pointing at them — survive. New stands are inserted; `xplane` rows
/// the extract no longer lists are deleted. `manual` and `crc` rows are left alone entirely.
///
/// `NotFound` when the extract does not cover `icao`, touching nothing: clearing an uncovered
/// airport's rows would leave it with fewer gates than before, which is not what "re-pull" means.
pub async fn seed_for_icao(pool: &PgPool, icao: &str) -> Result<GateSeedSummary, ApiError> {
    let extract = load_bundled_extract();
    let stands = extract.get(icao).ok_or(ApiError::NotFound)?;
    let mut tx = begin_locked(pool).await?;

    let names: Vec<&str> = stands.iter().map(|s| s.name.as_str()).collect();
    // Drop the stands this airport no longer has, before inserting — scoped to our own source.
    sqlx::query(
        "delete from flow.airport_gate \
         where icao = $1 and source = 'xplane' and name <> all($2)",
    )
    .bind(icao)
    .bind(&names)
    .execute(&mut *tx)
    .await
    .map_err(db_err)?;

    // Refresh the ones we already have, by name, so their ids (and observations) survive.
    let (mut u_name, mut u_lat, mut u_lon, mut u_kind) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for s in stands {
        u_name.push(s.name.as_str());
        u_lat.push(s.lat);
        u_lon.push(s.lon);
        u_kind.push(s.kind.as_str());
    }
    let updated = sqlx::query(
        "update flow.airport_gate as g \
         set lat = t.lat, lon = t.lon, kind = t.kind \
         from unnest($2::text[], $3::float8[], $4::float8[], $5::text[]) \
              as t(name, lat, lon, kind) \
         where g.icao = $1 and g.source = 'xplane' and g.name = t.name",
    )
    .bind(icao)
    .bind(&u_name)
    .bind(&u_lat)
    .bind(&u_lon)
    .bind(&u_kind)
    .execute(&mut *tx)
    .await
    .map_err(db_err)?
    .rows_affected() as usize;

    // Insert whatever is genuinely new. `where not exists` rather than an upsert: there is no unique
    // constraint on (icao, name) — manual rows may legitimately share a stand's name.
    let inserted = sqlx::query(
        "insert into flow.airport_gate (icao, name, lat, lon, kind, source) \
         select $1, t.name, t.lat, t.lon, t.kind, 'xplane' \
         from unnest($2::text[], $3::float8[], $4::float8[], $5::text[]) \
              as t(name, lat, lon, kind) \
         where not exists ( \
             select 1 from flow.airport_gate g \
             where g.icao = $1 and g.source = 'xplane' and g.name = t.name \
         )",
    )
    .bind(icao)
    .bind(&u_name)
    .bind(&u_lat)
    .bind(&u_lon)
    .bind(&u_kind)
    .execute(&mut *tx)
    .await
    .map_err(db_err)?
    .rows_affected() as usize;

    let osm_gates_retired = retire_osm_gates(&mut tx, &[icao]).await?;
    record_seeded(&mut tx, &[icao]).await?;
    tx.commit().await.map_err(db_err)?;

    tracing::info!(icao, updated, inserted, "re-pulled X-Plane gates");
    Ok(GateSeedSummary {
        airports_seeded: 1,
        airports_skipped: 0,
        gates_inserted: inserted,
        gates_refreshed: updated,
        osm_gates_retired,
    })
}

/// Records `icaos` as gate-seeded (migration `0087`), so the boot seed never re-populates them even
/// after every imported stand is deleted.
async fn record_seeded(conn: &mut sqlx::PgConnection, icaos: &[&str]) -> Result<(), ApiError> {
    sqlx::query(
        "insert into flow.airport_gate_xplane_seeded (icao) select unnest($1::text[]) \
         on conflict (icao) do nothing",
    )
    .bind(icaos)
    .execute(&mut *conn)
    .await
    .map_err(db_err)?;
    Ok(())
}

/// Retires the `osm` gates of `icaos` — see the module doc on why both sets cannot coexist. `manual`
/// and `crc` rows are never touched.
async fn retire_osm_gates(
    conn: &mut sqlx::PgConnection,
    icaos: &[&str],
) -> Result<usize, ApiError> {
    Ok(
        sqlx::query("delete from flow.airport_gate where source = 'osm' and icao = any($1)")
            .bind(icaos)
            .execute(&mut *conn)
            .await
            .map_err(db_err)?
            .rows_affected() as usize,
    )
}

/// Inserts every extract stand for `icaos` as `source='xplane'` — one set-based insert via `unnest`
/// rather than a round trip per row.
async fn insert_gates(
    conn: &mut sqlx::PgConnection,
    extract: &HashMap<String, Vec<ExtractStand>>,
    icaos: &[&str],
) -> Result<usize, ApiError> {
    let (mut icao, mut name, mut lat, mut lon, mut kind) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for i in icaos {
        for s in &extract[*i] {
            icao.push(*i);
            name.push(s.name.as_str());
            lat.push(s.lat);
            lon.push(s.lon);
            kind.push(s.kind.as_str());
        }
    }
    Ok(sqlx::query(
        "insert into flow.airport_gate (icao, name, lat, lon, kind, source) \
         select icao, name, lat, lon, kind, 'xplane' \
         from unnest($1::text[], $2::text[], $3::float8[], $4::float8[], $5::text[]) \
              as t(icao, name, lat, lon, kind)",
    )
    .bind(&icao)
    .bind(&name)
    .bind(&lat)
    .bind(&lon)
    .bind(&kind)
    .execute(&mut *conn)
    .await
    .map_err(db_err)?
    .rows_affected() as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn count(pool: &PgPool, sql: &str) -> i64 {
        sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
    }

    async fn gate_count(pool: &PgPool, icao: &str, source: &str) -> i64 {
        sqlx::query_scalar("select count(*) from flow.airport_gate where icao = $1 and source = $2")
            .bind(icao)
            .bind(source)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// An ICAO the extract covers, chosen from the data rather than hard-coded, so the suite does not
    /// break the next time the importer runs against a changed Gateway.
    fn a_covered_icao() -> String {
        let extract = load_bundled_extract();
        let mut keys: Vec<&String> = extract.keys().collect();
        keys.sort();
        keys.into_iter()
            .find(|k| *k != "KDCA")
            .expect("the extract should cover more than KDCA")
            .clone()
    }

    #[test]
    fn bundled_extract_parses() {
        let extract = load_bundled_extract();
        assert!(
            extract.len() > 150,
            "expected substantially all 185 airports, got {}",
            extract.len()
        );
        let stands: usize = extract.values().map(Vec::len).sum();
        assert!(stands > 10_000, "expected ~12,962 stands, got {stands}");
    }

    /// Imported rows bypass the HTTP handler, so nothing would otherwise stop the extract putting a
    /// row into the table that the gate editor would reject
    /// (`handlers::airport_surface::validate_gate`).
    #[test]
    fn every_bundled_stand_would_pass_the_gate_editors_validation() {
        for (icao, stands) in load_bundled_extract() {
            for s in stands {
                assert!(
                    !s.name.trim().is_empty() && s.name.len() <= 64,
                    "{icao} has an unusable stand name {:?}",
                    s.name
                );
                assert!(
                    (-90.0..=90.0).contains(&s.lat) && (-180.0..=180.0).contains(&s.lon),
                    "{icao}/{} is out of range at {},{}",
                    s.name,
                    s.lat,
                    s.lon
                );
            }
        }
    }

    /// The re-pull matches on `(icao, name)`, which is only unambiguous while names are unique within
    /// an airport — the importer's `disambiguate` guarantees that, and this is the assertion that
    /// notices if a future extract arrives without it.
    #[test]
    fn stand_names_are_unique_within_each_airport() {
        for (icao, stands) in load_bundled_extract() {
            let unique: HashSet<&str> = stands.iter().map(|s| s.name.as_str()).collect();
            assert_eq!(
                unique.len(),
                stands.len(),
                "{icao} repeats a stand name, which would make a re-pull's name match ambiguous"
            );
        }
    }

    #[sqlx::test]
    async fn the_seed_covers_substantially_every_extract_airport(pool: PgPool) {
        let summary = seed(&pool).await.unwrap();
        let extract = load_bundled_extract();
        assert_eq!(summary.airports_seeded, extract.len());
        assert_eq!(summary.airports_skipped, 0);

        let airports: i64 =
            count(&pool, "select count(distinct icao) from flow.airport_gate").await;
        assert!(
            airports > 150,
            "gate coverage should go from 1 airport to substantially all; got {airports}"
        );
        let stands: i64 = count(
            &pool,
            "select count(*) from flow.airport_gate where source = 'xplane'",
        )
        .await;
        assert_eq!(
            stands as usize,
            extract.values().map(Vec::len).sum::<usize>()
        );
    }

    #[sqlx::test]
    async fn a_second_run_touches_nothing(pool: PgPool) {
        let first = seed(&pool).await.unwrap();
        let before = count(&pool, "select count(*) from flow.airport_gate").await;

        let second = seed(&pool).await.unwrap();
        assert_eq!(second.airports_seeded, 0);
        assert_eq!(second.airports_skipped, first.airports_seeded);
        assert_eq!(second.gates_inserted, 0);
        assert_eq!(second.osm_gates_retired, 0);
        assert_eq!(
            count(&pool, "select count(*) from flow.airport_gate").await,
            before,
            "a re-run must not duplicate a single stand"
        );
    }

    /// Without the advisory lock both runs read "unseeded" and both insert, doubling every stand.
    #[sqlx::test]
    async fn concurrent_runs_do_not_duplicate_rows(pool: PgPool) {
        let (a, b) = tokio::join!(seed(&pool), seed(&pool));
        let (a, b) = (a.unwrap(), b.unwrap());
        let extract = load_bundled_extract();
        assert_eq!(
            a.airports_seeded + b.airports_seeded,
            extract.len(),
            "each airport must be seeded exactly once across the two runs"
        );
        assert_eq!(
            count(
                &pool,
                "select count(*) from flow.airport_gate where source = 'xplane'"
            )
            .await as usize,
            extract.values().map(Vec::len).sum::<usize>()
        );
    }

    /// KDCA's 57 hand-committed osm gates (migration `0067`) describe the same physical stands the
    /// extract does; keeping both would split that stand's taxi observations across two ids.
    #[sqlx::test]
    async fn kdcas_osm_gates_are_retired_in_favour_of_the_import(pool: PgPool) {
        assert_eq!(gate_count(&pool, "KDCA", "osm").await, 57, "0067's seed");

        let summary = seed(&pool).await.unwrap();
        assert_eq!(summary.osm_gates_retired, 57);
        assert_eq!(gate_count(&pool, "KDCA", "osm").await, 0);
        assert!(gate_count(&pool, "KDCA", "xplane").await > 0);
    }

    #[sqlx::test]
    async fn a_manual_gate_survives_the_seed(pool: PgPool) {
        let icao = a_covered_icao();
        sqlx::query(
            "insert into flow.airport_gate (icao, name, lat, lon, source) \
             values ($1, 'HAND ENTERED', 1.0, 2.0, 'manual')",
        )
        .bind(&icao)
        .execute(&pool)
        .await
        .unwrap();

        seed(&pool).await.unwrap();

        assert_eq!(
            gate_count(&pool, &icao, "manual").await,
            1,
            "a facility's own gate must never be touched by the import"
        );
    }

    /// The marker table's whole purpose: a facility that deleted the imported stands at their field
    /// must not have them reappear on the next restart.
    #[sqlx::test]
    async fn an_airport_whose_stands_were_all_deleted_is_not_reseeded(pool: PgPool) {
        let icao = a_covered_icao();
        seed(&pool).await.unwrap();
        sqlx::query("delete from flow.airport_gate where icao = $1 and source = 'xplane'")
            .bind(&icao)
            .execute(&pool)
            .await
            .unwrap();

        let again = seed(&pool).await.unwrap();

        assert_eq!(again.gates_inserted, 0);
        assert_eq!(gate_count(&pool, &icao, "xplane").await, 0);
    }

    /// **The test that matters most.** `stats.taxi_observation.gate_id` is
    /// `on delete set null` (migration `0070`), so a re-pull that deleted and re-inserted stands would
    /// mint new ids and silently wipe the learned taxi history this issue exists to make reachable.
    #[sqlx::test]
    async fn a_repull_preserves_gate_ids_and_their_observations(pool: PgPool) {
        let icao = a_covered_icao();
        seed(&pool).await.unwrap();

        let (gate_id, name): (String, String) = sqlx::query_as(
            "select id, name from flow.airport_gate \
             where icao = $1 and source = 'xplane' order by name limit 1",
        )
        .bind(&icao)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into stats.taxi_observation \
             (airport, gate_id, aircraft, runway, taxi_sec, observed_at) \
             values ($1, $2, 'B738', '27', 600, now())",
        )
        .bind(&icao)
        .bind(&gate_id)
        .execute(&pool)
        .await
        .unwrap();

        seed_for_icao(&pool, &icao).await.unwrap();

        let still: Option<String> = sqlx::query_scalar(
            "select gate_id from stats.taxi_observation where airport = $1 limit 1",
        )
        .bind(&icao)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            still.as_deref(),
            Some(gate_id.as_str()),
            "the re-pull must keep the gate's id so its learned observations stay attached"
        );
        let surviving: i64 = sqlx::query_scalar(
            "select count(*) from flow.airport_gate where id = $1 and name = $2",
        )
        .bind(&gate_id)
        .bind(&name)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(surviving, 1, "the row itself must be updated, not replaced");
    }

    /// A re-pull of an airport whose stands all already exist inserts nothing and refreshes
    /// everything. Both counts are reported so that reads as "up to date", not "did nothing".
    #[sqlx::test]
    async fn a_repull_reports_what_it_refreshed_not_just_what_it_inserted(pool: PgPool) {
        let icao = a_covered_icao();
        seed(&pool).await.unwrap();
        let stands = gate_count(&pool, &icao, "xplane").await as usize;

        let summary = seed_for_icao(&pool, &icao).await.unwrap();

        assert_eq!(summary.gates_inserted, 0, "nothing is new");
        assert_eq!(
            summary.gates_refreshed, stands,
            "but every stand was refreshed, and the operator has to be able to see that"
        );
    }

    #[sqlx::test]
    async fn a_repull_moves_a_stand_that_moved_in_the_extract(pool: PgPool) {
        let icao = a_covered_icao();
        seed(&pool).await.unwrap();
        sqlx::query(
            "update flow.airport_gate set lat = 0, lon = 0 \
             where icao = $1 and source = 'xplane'",
        )
        .bind(&icao)
        .execute(&pool)
        .await
        .unwrap();

        seed_for_icao(&pool, &icao).await.unwrap();

        let at_origin: i64 = sqlx::query_scalar(
            "select count(*) from flow.airport_gate \
             where icao = $1 and source = 'xplane' and lat = 0 and lon = 0",
        )
        .bind(&icao)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            at_origin, 0,
            "every stand should be back at its extract position"
        );
    }

    #[sqlx::test]
    async fn a_repull_drops_a_stand_the_extract_no_longer_lists(pool: PgPool) {
        let icao = a_covered_icao();
        seed(&pool).await.unwrap();
        sqlx::query(
            "insert into flow.airport_gate (icao, name, lat, lon, kind, source) \
             values ($1, 'RETIRED STAND', 1.0, 2.0, 'gate', 'xplane')",
        )
        .bind(&icao)
        .execute(&pool)
        .await
        .unwrap();

        seed_for_icao(&pool, &icao).await.unwrap();

        let left: i64 = sqlx::query_scalar(
            "select count(*) from flow.airport_gate where icao = $1 and name = 'RETIRED STAND'",
        )
        .bind(&icao)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(left, 0, "an xplane row the extract dropped should go");
    }

    #[sqlx::test]
    async fn a_repull_leaves_a_manual_gate_alone(pool: PgPool) {
        let icao = a_covered_icao();
        sqlx::query(
            "insert into flow.airport_gate (icao, name, lat, lon, source) \
             values ($1, 'HAND ENTERED', 1.0, 2.0, 'manual')",
        )
        .bind(&icao)
        .execute(&pool)
        .await
        .unwrap();

        seed_for_icao(&pool, &icao).await.unwrap();

        assert_eq!(gate_count(&pool, &icao, "manual").await, 1);
    }

    /// An uncovered airport must be left exactly as it was: clearing its rows would leave it with
    /// fewer gates than before the "refresh".
    #[sqlx::test]
    async fn a_repull_of_an_uncovered_airport_is_not_found_and_touches_nothing(pool: PgPool) {
        sqlx::query(
            "insert into flow.airport_gate (icao, name, lat, lon, source) \
             values ('ZZZZ', 'ONLY ONE', 1.0, 2.0, 'manual')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let err = seed_for_icao(&pool, "ZZZZ").await.unwrap_err();

        assert!(matches!(err, ApiError::NotFound), "got {err:?}");
        assert_eq!(gate_count(&pool, "ZZZZ", "manual").await, 1);
    }

    #[sqlx::test]
    async fn a_repull_records_the_airport_so_the_boot_seed_skips_it(pool: PgPool) {
        let icao = a_covered_icao();
        seed_for_icao(&pool, &icao).await.unwrap();

        let marked: i64 = sqlx::query_scalar(
            "select count(*) from flow.airport_gate_xplane_seeded where icao = $1",
        )
        .bind(&icao)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(marked, 1);

        let before = gate_count(&pool, &icao, "xplane").await;
        let boot = seed(&pool).await.unwrap();
        assert_eq!(
            boot.airports_skipped, 1,
            "the boot seed must treat the re-pulled airport as already seeded"
        );
        assert_eq!(
            gate_count(&pool, &icao, "xplane").await,
            before,
            "and must not insert its stands a second time"
        );
    }

    /// AC6 — the point of the whole issue. Tier 1 keys on `(gate_id, aircraft, runway)` and needs
    /// `MIN_SAMPLES` observations, so an import cannot *populate* it; what it does is make it
    /// **reachable**, which before this change it was not at 184 of 185 airports. This walks the real
    /// chain: seeded stand -> its id on observations -> `feed::taxi_estimate` returning the top tier.
    #[sqlx::test]
    async fn a_seeded_stand_makes_the_gate_type_runway_tier_reachable(pool: PgPool) {
        use crate::feed::taxi_estimate::{EstimateTier, TaxiSample, estimate};

        let icao = a_covered_icao();
        seed(&pool).await.unwrap();

        let gate_id: String = sqlx::query_scalar(
            "select id from flow.airport_gate where icao = $1 and source = 'xplane' \
             order by name limit 1",
        )
        .bind(&icao)
        .fetch_one(&pool)
        .await
        .unwrap();

        let samples: Vec<TaxiSample> = (0..5)
            .map(|_| TaxiSample {
                gate_id: Some(gate_id.clone()),
                aircraft: Some("B738".to_string()),
                runway: Some("27".to_string()),
                pushback_sec: Some(90),
                startup_sec: Some(60),
                taxi_sec: 600,
            })
            .collect();

        let est = estimate(&samples, Some(&gate_id), Some("B738"), Some("27"));

        assert_eq!(
            est.taxi.tier,
            EstimateTier::GateTypeRunway,
            "a seeded stand with enough samples must reach tier 1"
        );
    }

    /// The imported `kind` is what a future refinement of `feed::taxi_observations::phases_at_stand`
    /// would key on, so it has to survive the import rather than land as null.
    #[sqlx::test]
    async fn imported_stands_carry_their_x_plane_kind(pool: PgPool) {
        seed(&pool).await.unwrap();

        let unknown: i64 = sqlx::query_scalar(
            "select count(*) from flow.airport_gate where source = 'xplane' \
             and (kind is null or kind not in ('gate', 'tie_down', 'misc', 'hangar'))",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(unknown, 0, "every imported stand should carry a known kind");

        let tie_downs: i64 = sqlx::query_scalar(
            "select count(*) from flow.airport_gate where source = 'xplane' and kind = 'tie_down'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            tie_downs > 0,
            "the extract is mostly tie-downs; losing the distinction would be silent"
        );
    }
}
