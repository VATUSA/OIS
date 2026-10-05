//! ATC sector volumes (#594, migration 0111). `load_all` builds the [`SectorTable`] cached in
//! `AppState`; `replace_artcc` is the only write, used by the offline importer
//! (`bin/airspace_sector_importer.rs`). See `feed::sectors` for the model.

use sqlx::{PgPool, types::Json};

use crate::{
    errors::ApiError,
    feed::sectors::{SectorTable, SectorVolume, validate_volume},
};

#[derive(sqlx::FromRow)]
struct SectorRow {
    artcc: String,
    sector_id: String,
    volume_id: String,
    name: Option<String>,
    tier: String,
    base_alt_ft: i32,
    top_alt_ft: i32,
    rings: Json<Vec<Vec<[f64; 2]>>>,
}

/// Every stored volume, ordered by ARTCC then volume.
pub async fn load_all(pool: &PgPool) -> Result<SectorTable, ApiError> {
    let rows = sqlx::query_as::<_, SectorRow>(
        "select artcc, sector_id, volume_id, name, tier, base_alt_ft, top_alt_ft, rings \
         from flow.airspace_sector order by artcc, volume_id",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    let volumes = rows
        .into_iter()
        .map(|r| SectorVolume {
            artcc: r.artcc,
            sector_id: r.sector_id,
            volume_id: r.volume_id,
            name: r.name,
            tier: r.tier,
            base_alt_ft: r.base_alt_ft,
            top_alt_ft: r.top_alt_ft,
            rings: r.rings.0,
        })
        .collect();
    Ok(SectorTable { volumes })
}

/// Replace one ARTCC's volumes with `volumes`, atomically, stamping each with its provenance.
/// Every volume must belong to `artcc` and pass [`validate_volume`]; if any doesn't, nothing is
/// written and the error names it. Errors are plain text because the only caller is a CLI.
pub async fn replace_artcc(
    pool: &PgPool,
    artcc: &str,
    volumes: &[SectorVolume],
    source: &str,
    source_cycle: &str,
) -> Result<(), String> {
    for v in volumes {
        if v.artcc != artcc {
            return Err(format!("{} {} is not in {artcc}", v.artcc, v.volume_id));
        }
        validate_volume(v).map_err(|e| format!("{artcc} {}: {e}", v.volume_id))?;
    }
    let db = |e: sqlx::Error| format!("{artcc}: {e}");
    let mut tx = pool.begin().await.map_err(db)?;
    sqlx::query("delete from flow.airspace_sector where artcc = $1")
        .bind(artcc)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    for v in volumes {
        sqlx::query(
            "insert into flow.airspace_sector \
             (artcc, sector_id, volume_id, name, tier, base_alt_ft, top_alt_ft, rings, \
              source, source_cycle) \
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(&v.artcc)
        .bind(&v.sector_id)
        .bind(&v.volume_id)
        .bind(&v.name)
        .bind(&v.tier)
        .bind(v.base_alt_ft)
        .bind(v.top_alt_ft)
        .bind(Json(&v.rings))
        .bind(source)
        .bind(source_cycle)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    }
    tx.commit().await.map_err(db)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::sectors::tests::{bridged_ring, volume};

    async fn count(pool: &PgPool) -> i64 {
        sqlx::query_scalar("select count(*) from flow.airspace_sector")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[sqlx::test]
    async fn replace_round_trips_and_records_provenance(pool: PgPool) {
        let mut multi = volume("ZDC", "01002");
        multi.rings.push(vec![
            [37.0, -78.0],
            [37.0, -77.5],
            [37.5, -77.5],
            [37.0, -78.0],
        ]);
        let vols = [volume("ZDC", "01001"), multi];
        replace_artcc(&pool, "ZDC", &vols, "vtsd sectors.json", "abc123")
            .await
            .unwrap();

        assert_eq!(load_all(&pool).await.unwrap().volumes, vols);
        let prov: Vec<(String, String)> =
            sqlx::query_as("select distinct source, source_cycle from flow.airspace_sector")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(prov, [("vtsd sectors.json".into(), "abc123".into())]);
    }

    #[sqlx::test]
    async fn replace_swaps_only_its_own_artcc(pool: PgPool) {
        replace_artcc(&pool, "ZDC", &[volume("ZDC", "01001")], "s", "1")
            .await
            .unwrap();
        replace_artcc(&pool, "ZNY", &[volume("ZNY", "02001")], "s", "1")
            .await
            .unwrap();
        replace_artcc(&pool, "ZDC", &[volume("ZDC", "03001")], "s", "2")
            .await
            .unwrap();

        let ids: Vec<String> = load_all(&pool)
            .await
            .unwrap()
            .volumes
            .into_iter()
            .map(|v| v.volume_id)
            .collect();
        // ZDC's 01001 is gone, ZNY's volume is untouched.
        assert_eq!(ids, ["03001", "02001"]);
    }

    /// AC4: a bridged ring can't be stored — and a rejected batch writes nothing, so the ARTCC's
    /// previous volumes survive.
    #[sqlx::test]
    async fn a_bridged_ring_is_refused_and_nothing_is_written(pool: PgPool) {
        replace_artcc(&pool, "ZDC", &[volume("ZDC", "01001")], "s", "1")
            .await
            .unwrap();
        let mut bad = volume("ZDC", "01002");
        bad.rings = vec![bridged_ring()];

        let err = replace_artcc(&pool, "ZDC", &[volume("ZDC", "01003"), bad], "s", "2")
            .await
            .unwrap_err();
        assert!(err.contains("01002") && err.contains("bridged"), "{err}");
        let ids: Vec<String> = load_all(&pool)
            .await
            .unwrap()
            .volumes
            .into_iter()
            .map(|v| v.volume_id)
            .collect();
        assert_eq!(ids, ["01001"]);
    }

    /// AC5 at the write path: an inverted band (the source's ZTL 07007) is refused.
    #[sqlx::test]
    async fn an_inverted_altitude_band_is_refused(pool: PgPool) {
        let mut bad = volume("ZTL", "07007");
        (bad.base_alt_ft, bad.top_alt_ft) = (60_000, 10_000);
        assert!(replace_artcc(&pool, "ZTL", &[bad], "s", "1").await.is_err());
        assert_eq!(count(&pool).await, 0);
    }

    /// AC5 below the app: the schema itself refuses a row with base >= top, so a write that skips
    /// `replace_artcc` can't store one either.
    #[sqlx::test]
    async fn the_schema_refuses_base_at_or_above_top(pool: PgPool) {
        for (base, top) in [(10_000, 10_000), (60_000, 10_000)] {
            let res = sqlx::query(
                "insert into flow.airspace_sector \
                 (artcc, sector_id, volume_id, tier, base_alt_ft, top_alt_ft, rings, source, \
                  source_cycle) values ('ZTL', '70', '07007', 'low', $1, $2, '[]', 's', '1')",
            )
            .bind(base)
            .bind(top)
            .execute(&pool)
            .await;
            assert!(res.is_err(), "{base}..{top} was stored");
        }
    }

    #[sqlx::test]
    async fn a_volume_from_another_artcc_is_refused(pool: PgPool) {
        let err = replace_artcc(&pool, "ZDC", &[volume("ZNY", "02001")], "s", "1")
            .await
            .unwrap_err();
        assert!(err.contains("not in ZDC"), "{err}");
        assert_eq!(count(&pool).await, 0);
    }

    /// AC4 + AC5 over the stored table: every row `load_all` returns passes the same validation
    /// the write path enforces — rings closed, >= 4 points, in range, simple; base < top.
    #[sqlx::test]
    async fn every_stored_volume_is_valid(pool: PgPool) {
        replace_artcc(
            &pool,
            "ZDC",
            &[volume("ZDC", "01001"), volume("ZDC", "01002")],
            "s",
            "1",
        )
        .await
        .unwrap();
        let table = load_all(&pool).await.unwrap();
        assert_eq!(table.volumes.len(), 2);
        for v in &table.volumes {
            assert_eq!(validate_volume(v), Ok(()), "{} {}", v.artcc, v.volume_id);
            assert!(v.base_alt_ft < v.top_alt_ft);
        }
    }
}
