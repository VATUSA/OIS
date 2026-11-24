//! Configurable aircraft performance profiles (see migration 0059). CRUD for the profile catalog
//! plus `load_all`, which builds the in-memory [`ProfileTable`] the trajectory model resolves
//! against. National reference data; writes are gated in the handler by `flow.aircraft_profiles.update`.

use std::collections::HashMap;

use sqlx::PgPool;

use crate::{
    auth::principal::Attribution,
    errors::ApiError,
    feed::{
        performance::EramTable,
        trajectory::{AircraftProfile, ProfileTable},
    },
    models::{AircraftProfileBody, UpsertAircraftProfileRequest},
};

const PROFILE_SELECT: &str = "select p.kind, p.key, p.name, \
    p.climb_ias_lo, p.climb_ias_hi, p.climb_mach, p.climb_fpm_lo, p.climb_fpm_hi, \
    p.cruise_tas, p.cruise_mach, p.service_ceiling_ft, \
    p.desc_mach, p.desc_ias_hi, p.desc_ias_lo, p.desc_fpm, \
    p.updated_at, coalesce(u.display_name, a.display_name) as updated_by \
    from flow.aircraft_profile p left join identity.users u on u.id = p.updated_by \
    left join access.actors a on a.id = p.updated_by_actor";

/// The whole catalog, ordered default → wake → family → type for display.
pub async fn list(pool: &PgPool) -> Result<Vec<AircraftProfileBody>, ApiError> {
    sqlx::query_as::<_, AircraftProfileBody>(&format!(
        "{PROFILE_SELECT} order by \
         case p.kind when 'default' then 0 when 'wake' then 1 when 'family' then 2 else 3 end, p.key"
    ))
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn get(
    pool: &PgPool,
    kind: &str,
    key: &str,
) -> Result<Option<AircraftProfileBody>, ApiError> {
    sqlx::query_as::<_, AircraftProfileBody>(&format!(
        "{PROFILE_SELECT} where p.kind = $1 and p.key = $2"
    ))
    .bind(kind)
    .bind(key)
    .fetch_optional(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Insert or replace a profile at `(kind, key)`.
pub async fn upsert(
    pool: &PgPool,
    kind: &str,
    key: &str,
    req: &UpsertAircraftProfileRequest,
    by: &Attribution,
) -> Result<AircraftProfileBody, ApiError> {
    sqlx::query(
        "insert into flow.aircraft_profile \
         (kind, key, name, climb_ias_lo, climb_ias_hi, climb_mach, climb_fpm_lo, climb_fpm_hi, \
          cruise_tas, cruise_mach, service_ceiling_ft, desc_mach, desc_ias_hi, desc_ias_lo, \
          desc_fpm, updated_by, updated_by_actor) \
         values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17) \
         on conflict (kind, key) do update set \
          name = excluded.name, climb_ias_lo = excluded.climb_ias_lo, \
          climb_ias_hi = excluded.climb_ias_hi, climb_mach = excluded.climb_mach, \
          climb_fpm_lo = excluded.climb_fpm_lo, climb_fpm_hi = excluded.climb_fpm_hi, \
          cruise_tas = excluded.cruise_tas, cruise_mach = excluded.cruise_mach, \
          service_ceiling_ft = excluded.service_ceiling_ft, desc_mach = excluded.desc_mach, \
          desc_ias_hi = excluded.desc_ias_hi, desc_ias_lo = excluded.desc_ias_lo, \
          desc_fpm = excluded.desc_fpm, updated_by = excluded.updated_by, \
          updated_by_actor = excluded.updated_by_actor, updated_at = now()",
    )
    .bind(kind)
    .bind(key)
    .bind(&req.name)
    .bind(req.climb_ias_lo)
    .bind(req.climb_ias_hi)
    .bind(req.climb_mach)
    .bind(req.climb_fpm_lo)
    .bind(req.climb_fpm_hi)
    .bind(req.cruise_tas)
    .bind(req.cruise_mach)
    .bind(req.service_ceiling_ft)
    .bind(req.desc_mach)
    .bind(req.desc_ias_hi)
    .bind(req.desc_ias_lo)
    .bind(req.desc_fpm)
    .bind(&by.user_id)
    .bind(&by.actor_id)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;

    get(pool, kind, key).await?.ok_or(ApiError::Internal)
}

/// Delete a profile. The global default (`kind='default'`) is never deletable.
pub async fn delete(pool: &PgPool, kind: &str, key: &str) -> Result<bool, ApiError> {
    if kind == "default" {
        return Err(ApiError::BadRequest);
    }
    let res = sqlx::query("delete from flow.aircraft_profile where kind = $1 and key = $2")
        .bind(kind)
        .bind(key)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(res.rows_affected() > 0)
}

/// Build the in-memory resolution table the trajectory model uses: the stored rows and the bundled ERAM
/// table. Falls back to the legacy `AircraftProfile::default()` when no `default` row exists (e.g. an
/// empty DB).
pub async fn load_all(pool: &PgPool) -> Result<ProfileTable, ApiError> {
    let rows = list(pool).await?;
    let t = StoredTiers::from_rows(&rows);
    Ok(ProfileTable::new(
        t.default,
        t.by_wake,
        t.by_family,
        t.by_type,
        Some(EramTable::bundled()),
    ))
}

/// Stored rows split into the tiers [`ProfileTable`] resolves through: type and wake keys uppercased,
/// family keys lowercased. The one place a row's `kind` picks its tier, shared by [`load_all`] and the aircraft-profile
/// backtest's stored set (#828).
#[derive(Debug, Clone)]
pub struct StoredTiers {
    pub default: AircraftProfile,
    pub by_wake: HashMap<String, AircraftProfile>,
    pub by_family: HashMap<String, AircraftProfile>,
    pub by_type: HashMap<String, AircraftProfile>,
}

impl StoredTiers {
    pub fn from_rows(rows: &[AircraftProfileBody]) -> Self {
        let mut t = Self {
            default: AircraftProfile::default(),
            by_wake: HashMap::new(),
            by_family: HashMap::new(),
            by_type: HashMap::new(),
        };
        for r in rows {
            let p = to_profile(r);
            match r.kind.as_str() {
                "default" => t.default = p,
                "wake" => {
                    t.by_wake.insert(r.key.to_ascii_uppercase(), p);
                }
                "family" => {
                    t.by_family.insert(r.key.to_ascii_lowercase(), p);
                }
                "type" => {
                    t.by_type.insert(r.key.to_ascii_uppercase(), p);
                }
                other => {
                    tracing::warn!(kind = other, key = %r.key, "aircraft profile row of unknown kind skipped");
                }
            }
        }
        t
    }
}

/// Map a stored row to the pure trajectory profile.
fn to_profile(r: &AircraftProfileBody) -> AircraftProfile {
    AircraftProfile {
        climb_ias_lo: r.climb_ias_lo,
        climb_ias_hi: r.climb_ias_hi,
        climb_mach: r.climb_mach,
        climb_fpm_lo: r.climb_fpm_lo,
        climb_fpm_hi: r.climb_fpm_hi,
        cruise_tas: r.cruise_tas,
        cruise_mach: r.cruise_mach,
        service_ceiling_ft: r.service_ceiling_ft,
        desc_mach: r.desc_mach,
        desc_ias_hi: r.desc_ias_hi,
        desc_ias_lo: r.desc_ias_lo,
        desc_fpm: r.desc_fpm,
    }
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    use super::load_all;

    /// Each stored tier reaches the table the trajectory model resolves through: a type row (stored
    /// lowercase, matched uppercase), a wake row and the default, each told apart by its descent
    /// rate.
    #[sqlx::test]
    async fn load_all_puts_each_stored_row_in_its_own_tier(pool: PgPool) {
        for (kind, key, fpm) in [("default", "", 1_111.0), ("wake", "H", 2_222.0)] {
            sqlx::query(
                "update flow.aircraft_profile set desc_fpm = $3 where kind = $1 and key = $2",
            )
            .bind(kind)
            .bind(key)
            .bind(fpm)
            .execute(&pool)
            .await
            .unwrap();
        }
        sqlx::query(
            "insert into flow.aircraft_profile (kind, key, desc_fpm) values ('type', 'zz9z', 3333)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let table = load_all(&pool).await.unwrap();

        assert_eq!(table.row("ZZ9Z", "H").desc_fpm, 3_333.0, "type tier");
        assert_eq!(table.row("QQQQ", "H").desc_fpm, 2_222.0, "wake tier");
        assert_eq!(table.row("QQQQ", "").desc_fpm, 1_111.0, "default tier");
    }

    // Coverage matrix (VATUSA/OIS#818)
    //
    // Requirement                                   | Test
    // ----------------------------------------------|---------------------------------------------------------------
    // migration 0134 admits kind 'family'           | load_all_puts_a_family_row_in_the_family_tier
    // AC1 order through the stored table            | load_all_puts_a_family_row_in_the_family_tier
    // constraint still refuses an unknown kind      | the_kind_check_still_refuses_an_unknown_kind
    // admin page lists groups in order              | list_orders_default_then_wake_then_family_then_type
    // from_rows: family lowercased, unknown skipped | from_rows_keys_family_rows_lowercase_and_skips_an_unknown_kind

    /// Migration 0134 + `load_all`: a family row lands in the family tier, between type and wake. The
    /// types are ones `ENGINE_REF` knows and ERAM has no record of (#985), so the family tier is theirs.
    #[sqlx::test]
    async fn load_all_puts_a_family_row_in_the_family_tier(pool: PgPool) {
        for (kind, key, fpm) in [("wake", "H", 2_222.0), ("type", "PA28", 3_333.0)] {
            let n = sqlx::query(
                "update flow.aircraft_profile set desc_fpm = $3 where kind = $1 and key = $2",
            )
            .bind(kind)
            .bind(key)
            .bind(fpm)
            .execute(&pool)
            .await
            .unwrap()
            .rows_affected();
            assert_eq!(n, 1, "0059 seeds {kind} {key}");
        }
        sqlx::query(
            "insert into flow.aircraft_profile (kind, key, desc_fpm) \
             values ('family', 'jet', 4444), ('family', 'piston', 5555)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let table = load_all(&pool).await.unwrap();

        assert_eq!(
            table.row("E175", "H").desc_fpm,
            4_444.0,
            "family beats wake"
        );
        assert_eq!(table.resolve_label("E175", "H"), "family:jet");
        assert_eq!(table.row("DA20", "H").desc_fpm, 5_555.0, "its own family");
        assert_eq!(table.resolve_label("DA20", "H"), "family:piston");
        assert_eq!(
            table.row("PA28", "H").desc_fpm,
            3_333.0,
            "type beats family"
        );
        assert_eq!(table.row("QQQQ", "H").desc_fpm, 2_222.0, "no family: wake");
    }

    /// Migration 0134 replaces 0059's unnamed kind check with one named check that still refuses a
    /// kind it does not list.
    #[sqlx::test]
    async fn the_kind_check_still_refuses_an_unknown_kind(pool: PgPool) {
        let err =
            sqlx::query("insert into flow.aircraft_profile (kind, key) values ('bogus', 'X')")
                .execute(&pool)
                .await
                .unwrap_err();
        let db = err.as_database_error().expect("a database error");
        assert_eq!(db.code().as_deref(), Some("23514"), "check_violation");
        assert_eq!(db.constraint(), Some("aircraft_profile_kind_check"));
        let bogus: i64 =
            sqlx::query_scalar("select count(*) from flow.aircraft_profile where kind = 'bogus'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(bogus, 0);

        let checks: Vec<String> = sqlx::query_scalar(
            "select conname::text from pg_constraint \
             where conrelid = 'flow.aircraft_profile'::regclass and contype = 'c' order by 1",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(checks, vec!["aircraft_profile_kind_check".to_string()]);

        sqlx::query("insert into flow.aircraft_profile (kind, key) values ('family', 'piston')")
            .execute(&pool)
            .await
            .expect("positive control: a family row is admitted");
    }

    /// The admin page groups rows in list order: default, wake, family, type, then key.
    #[sqlx::test]
    async fn list_orders_default_then_wake_then_family_then_type(pool: PgPool) {
        for key in ["turboprop", "piston", "jet"] {
            sqlx::query("insert into flow.aircraft_profile (kind, key) values ('family', $1)")
                .bind(key)
                .execute(&pool)
                .await
                .unwrap();
        }

        let rows = super::list(&pool).await.unwrap();

        let kinds: Vec<&str> = rows.iter().map(|r| r.kind.as_str()).collect();
        let n_type = kinds.iter().filter(|k| **k == "type").count();
        let mut want = vec![
            "default", "wake", "wake", "wake", "wake", "family", "family", "family",
        ];
        want.extend(std::iter::repeat_n("type", n_type));
        assert!(n_type > 0, "0059 seeds type rows");
        assert_eq!(kinds, want);
        let families: Vec<&str> = rows
            .iter()
            .filter(|r| r.kind == "family")
            .map(|r| r.key.as_str())
            .collect();
        assert_eq!(families, ["jet", "piston", "turboprop"]);
    }

    fn body(kind: &str, key: &str, desc_fpm: f64) -> crate::models::AircraftProfileBody {
        let d = crate::feed::trajectory::AircraftProfile::default();
        crate::models::AircraftProfileBody {
            kind: kind.into(),
            key: key.into(),
            name: String::new(),
            climb_ias_lo: d.climb_ias_lo,
            climb_ias_hi: d.climb_ias_hi,
            climb_mach: None,
            climb_fpm_lo: d.climb_fpm_lo,
            climb_fpm_hi: d.climb_fpm_hi,
            cruise_tas: None,
            cruise_mach: None,
            service_ceiling_ft: d.service_ceiling_ft,
            desc_mach: None,
            desc_ias_hi: d.desc_ias_hi,
            desc_ias_lo: d.desc_ias_lo,
            desc_fpm,
            updated_at: chrono::DateTime::from_timestamp(1_790_000_000, 0).unwrap(),
            updated_by: None,
        }
    }

    /// `StoredTiers::from_rows` maps each kind explicitly; an unknown kind is skipped, not a type.
    #[test]
    fn from_rows_keys_family_rows_lowercase_and_skips_an_unknown_kind() {
        let t = super::StoredTiers::from_rows(&[
            body("family", "Turboprop", 2_000.0),
            body("type", "b738", 3_000.0),
            body("bogus", "A321", 9_999.0),
        ]);
        let fam: Vec<(&str, f64)> = t
            .by_family
            .iter()
            .map(|(k, p)| (k.as_str(), p.desc_fpm))
            .collect();
        assert_eq!(fam, [("turboprop", 2_000.0)]);
        let ty: Vec<(&str, f64)> = t
            .by_type
            .iter()
            .map(|(k, p)| (k.as_str(), p.desc_fpm))
            .collect();
        assert_eq!(ty, [("B738", 3_000.0)]);
        assert!(t.by_wake.is_empty());
    }
}

// Coverage matrix (VATUSA/OIS#985)
//
// Requirement                           | Test
// --------------------------------------|-------------------------------------------------------------
// AC 5 load_all carries ERAM            | load_all_resolves_an_eram_type_to_its_record
// AC 5 an operator's row beats ERAM     | load_all_lets_an_operators_type_row_beat_eram
// AC 5, 11 after the migration          | after_the_migration_the_ten_types_fly_eram_and_pa28_its_row
// AC 11 the ten go                      | the_eram_migration_removes_the_ten_seeded_rows
// AC 11 numbers: each of 12 columns     | a_seeded_row_with_any_one_number_edited_survives
// AC 11 kind and key                    | a_row_of_another_kind_or_key_with_seeded_numbers_survives
// AC 11 null against value              | a_null_speed_is_compared_as_a_value
// AC 11 everything else                 | the_migration_leaves_every_other_row_untouched
// AC 11 the deploy's audit              | the_0138_audit_names_the_rows_that_go
#[cfg(test)]
mod eram_tests {
    //! Each migration test re-inserts the 0059 rows it needs (the migration has already run on a
    //! `#[sqlx::test]` database), then runs the file through
    //! `include_str!("../../migrations/0138_aircraft_profile_eram.sql")`. The ten types: C172, C208, CRJ7,
    //! E75L, B738, A320, B763, B77W, B789, A359; the seeded numbers are 0059's.

    use sqlx::PgPool;

    use super::{list, load_all};
    use crate::feed::performance::{EramTable, Performance};

    const TEN: [&str; 10] = [
        "C172", "C208", "CRJ7", "E75L", "B738", "A320", "B763", "B77W", "B789", "A359",
    ];

    const MIGRATION: &str = include_str!("../../migrations/0138_aircraft_profile_eram.sql");

    /// Put back the type rows 0059 seeds, by running 0059's own insert.
    async fn seed_0059(pool: &PgPool) {
        let sql = include_str!("../../migrations/0059_aircraft_profiles.sql");
        let at = sql
            .rfind("insert into flow.aircraft_profile")
            .expect("0059 seeds flow.aircraft_profile");
        sqlx::raw_sql(&sql[at..]).execute(pool).await.unwrap();
    }

    async fn run_0138(pool: &PgPool) {
        sqlx::raw_sql(MIGRATION).execute(pool).await.unwrap();
    }

    async fn type_keys(pool: &PgPool) -> Vec<String> {
        sqlx::query_scalar("select key from flow.aircraft_profile where kind = 'type' order by key")
            .fetch_all(pool)
            .await
            .unwrap()
    }

    // AC 5. load_all on a migrated database: "eram:B738", "eram:C172", "wake:H" for QQQQ/H, "default" for
    // QQQQ. Resolved B738 climbs at 289 kt and 3,067 fpm at 10,000 ft. Red when: load_all omits the table.
    #[sqlx::test]
    async fn load_all_resolves_an_eram_type_to_its_record(pool: PgPool) {
        let table = load_all(&pool).await.unwrap();
        for (ty, wake, label) in [
            ("B738", "M", "eram:B738"),
            ("C172", "L", "eram:C172"),
            ("QQQQ", "H", "wake:H"),
            ("QQQQ", "", "default"),
        ] {
            assert_eq!(table.resolve_label(ty, wake), label);
        }
        let (tas, fpm) = table.resolve("B738", "M").climb(10_000.0, None);
        assert!((tas - 289.0).abs() < 1e-9, "{tas}");
        assert!((fpm - 3_067.0).abs() < 1e-9, "{fpm}");
    }

    // AC 5. Insert ('type', 'b738') with desc_fpm 3,333: "type:B738", desc_fpm 3,333. Family jet 4,444 and
    // the wake H row present: A321 is still "eram:A321", E175 "family:jet".
    // Red when: ERAM is consulted before by_type, or after by_family.
    #[sqlx::test]
    async fn load_all_lets_an_operators_type_row_beat_eram(pool: PgPool) {
        sqlx::query(
            "insert into flow.aircraft_profile (kind, key, desc_fpm) \
             values ('type', 'b738', 3333), ('family', 'jet', 4444)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let table = load_all(&pool).await.unwrap();
        assert_eq!(table.resolve_label("B738", "M"), "type:B738");
        assert!(matches!(table.resolve("B738", "M"), Performance::Row(p) if p.desc_fpm == 3_333.0));
        // The seeded wake H row is there too: ERAM is asked before the family and wake tiers.
        assert_eq!(table.resolve_label("A321", "H"), "eram:A321");
        assert_eq!(table.resolve_label("E175", "H"), "family:jet");
        assert_eq!(table.row("E175", "H").desc_fpm, 4_444.0);
    }

    // AC 5, 11. On a freshly migrated database the only type row is PA28: list() has default, four wake rows
    // and PA28; each of the ten labels "eram:<T>"; PA28 "type:PA28" (and is not in the ERAM file: control).
    #[sqlx::test]
    async fn after_the_migration_the_ten_types_fly_eram_and_pa28_its_row(pool: PgPool) {
        let rows = list(&pool).await.unwrap();
        let of = |kind: &str| -> Vec<&str> {
            rows.iter()
                .filter(|r| r.kind == kind)
                .map(|r| r.key.as_str())
                .collect()
        };
        assert_eq!(of("type"), ["PA28"]);
        assert_eq!(of("default"), [""]);
        assert_eq!(of("wake"), ["H", "J", "L", "M"]);
        let table = load_all(&pool).await.unwrap();
        for ty in TEN {
            assert_eq!(table.resolve_label(ty, "M"), format!("eram:{ty}"));
        }
        assert_eq!(table.resolve_label("PA28", "L"), "type:PA28");
        assert!(EramTable::bundled().get("PA28").is_none());
    }

    // AC 11. The deploy's audit, run from its own blocks around the migration: an edited B738 stays, an
    // untouched A320 goes, and an operator's A321 stays.
    #[sqlx::test]
    async fn the_0138_audit_names_the_rows_that_go(pool: PgPool) {
        const AUDIT: &str = include_str!("../../audits/0138_profile_rows.sql");
        // It runs the real file between the blocks, and nothing it does is kept.
        let lines: Vec<&str> = AUDIT.lines().map(str::trim).collect();
        let line = |l: &str| lines.iter().position(|x| *x == l).expect(l);
        let applied = line("\\i backend/migrations/0138_aircraft_profile_eram.sql");
        assert!(line("begin;") < line("-- BEGIN SETUP") && line("-- END SETUP") < applied);
        assert!(applied < line("-- BEGIN REPORT"));
        assert_eq!(lines.iter().rfind(|l| !l.is_empty()), Some(&"rollback;"));
        let block = |name: &str| {
            let from = AUDIT.split_once(&format!("-- BEGIN {name}")).unwrap().1;
            from.split_once(&format!("-- END {name}")).unwrap().0
        };
        seed_0059(&pool).await;
        sqlx::raw_sql(
            "update flow.aircraft_profile set desc_fpm = 2000 where key = 'B738';              insert into flow.aircraft_profile (kind, key) values ('type', 'A321')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let mut tx = pool.begin().await.unwrap();
        sqlx::raw_sql(block("SETUP"))
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::raw_sql(MIGRATION).execute(&mut *tx).await.unwrap();
        let rows: Vec<(String, bool, chrono::DateTime<chrono::Utc>)> =
            sqlx::query_as(block("REPORT"))
                .fetch_all(&mut *tx)
                .await
                .unwrap();
        tx.rollback().await.unwrap();

        let of = |removed: bool| -> Vec<&str> {
            rows.iter()
                .filter(|r| r.1 == removed)
                .map(|r| r.0.as_str())
                .collect()
        };
        assert_eq!(of(false), ["A321", "B738", "PA28"]);
        let mut gone: Vec<&str> = TEN.into_iter().filter(|t| *t != "B738").collect();
        gone.sort_unstable();
        assert_eq!(of(true), gone);
        assert_eq!(
            type_keys(&pool).await.len(),
            12,
            "the transaction rolled back"
        );
    }

    // AC 11. Re-insert all eleven 0059 type rows, run the file: PA28 alone is left. Run it again: no error,
    // same rows. Red when: one key is missing from the list (loop and name the survivor).
    #[sqlx::test]
    async fn the_eram_migration_removes_the_ten_seeded_rows(pool: PgPool) {
        seed_0059(&pool).await;
        let mut seeded: Vec<String> = TEN.iter().map(|t| t.to_string()).collect();
        seeded.push("PA28".into());
        seeded.sort();
        assert_eq!(type_keys(&pool).await, seeded, "0059's rows are back");
        for run in 1..=2 {
            run_0138(&pool).await;
            assert_eq!(type_keys(&pool).await, ["PA28"], "run {run}");
        }
    }

    // AC 11. For B738 and for each of climb_ias_lo, climb_ias_hi, climb_mach, climb_fpm_lo, climb_fpm_hi,
    // cruise_tas, cruise_mach, service_ceiling_ft, desc_mach, desc_ias_hi, desc_ias_lo and desc_fpm:
    // re-insert the seed, add 1 (0.01 for a Mach) to that column, run the file: the row survives with the
    // edit. Beside it an unedited A320 is gone (control). Red when: that column's predicate is deleted.
    #[sqlx::test]
    async fn a_seeded_row_with_any_one_number_edited_survives(pool: PgPool) {
        for col in [
            "climb_ias_lo",
            "climb_ias_hi",
            "climb_mach",
            "climb_fpm_lo",
            "climb_fpm_hi",
            "cruise_tas",
            "cruise_mach",
            "service_ceiling_ft",
            "desc_mach",
            "desc_ias_hi",
            "desc_ias_lo",
            "desc_fpm",
        ] {
            sqlx::query("delete from flow.aircraft_profile where kind = 'type'")
                .execute(&pool)
                .await
                .unwrap();
            seed_0059(&pool).await;
            let by = if col.ends_with("_mach") { 0.01 } else { 1.0 };
            // `col` is one of the literals above.
            let edited: f64 = sqlx::query_scalar(&format!(
                "update flow.aircraft_profile set {col} = {col} + $1 \
                 where kind = 'type' and key = 'B738' returning {col}"
            ))
            .bind(by)
            .fetch_one(&pool)
            .await
            .unwrap();
            run_0138(&pool).await;
            assert_eq!(type_keys(&pool).await, ["B738", "PA28"], "{col} edited");
            let kept: f64 = sqlx::query_scalar(&format!(
                "select {col} from flow.aircraft_profile where kind = 'type' and key = 'B738'"
            ))
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(kept, edited, "{col} keeps its edit");
        }
    }

    // AC 11. B738's seeded numbers stored as ('wake','B738'), ('type','B739') and ('type','b738'): all
    // survive, while ('type','B738') goes. Red when: the kind or key predicate is deleted. A lowercase type
    // key is an operator's own insert: the handler stores uppercase.
    #[sqlx::test]
    async fn a_row_of_another_kind_or_key_with_seeded_numbers_survives(pool: PgPool) {
        seed_0059(&pool).await;
        for (kind, key) in [("wake", "B738"), ("type", "B739"), ("type", "b738")] {
            sqlx::query(
                "insert into flow.aircraft_profile \
                 (kind, key, name, climb_ias_lo, climb_ias_hi, climb_mach, climb_fpm_lo, climb_fpm_hi, \
                  cruise_tas, cruise_mach, service_ceiling_ft, desc_mach, desc_ias_hi, desc_ias_lo, desc_fpm) \
                 select $1, $2, name, climb_ias_lo, climb_ias_hi, climb_mach, climb_fpm_lo, climb_fpm_hi, \
                  cruise_tas, cruise_mach, service_ceiling_ft, desc_mach, desc_ias_hi, desc_ias_lo, desc_fpm \
                 from flow.aircraft_profile where kind = 'type' and key = 'B738'",
            )
            .bind(kind)
            .bind(key)
            .execute(&pool)
            .await
            .unwrap();
        }
        run_0138(&pool).await;
        let left: Vec<(String, String)> = sqlx::query_as(
            "select kind, key from flow.aircraft_profile where upper(key) in ('B738', 'B739')",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        let mut left: Vec<(&str, &str)> = left.iter().map(|(k, v)| (&**k, &**v)).collect();
        left.sort_unstable();
        assert_eq!(left, [("type", "B739"), ("type", "b738"), ("wake", "B738")]);
    }

    // AC 11. C172 and C208 are seeded with null climb_mach, cruise_mach and desc_mach. Unedited both go (an
    // `=` on null would keep them); C172 with climb_mach 0.3 survives; B738 with cruise_mach null survives.
    // Red when: `is not distinct from` becomes `=`.
    #[sqlx::test]
    async fn a_null_speed_is_compared_as_a_value(pool: PgPool) {
        seed_0059(&pool).await;
        run_0138(&pool).await;
        assert_eq!(type_keys(&pool).await, ["PA28"], "null Mach columns match");

        seed_0059(&pool).await;
        for sql in [
            "update flow.aircraft_profile set climb_mach = 0.3 where kind = 'type' and key = 'C172'",
            "update flow.aircraft_profile set cruise_mach = null where kind = 'type' and key = 'B738'",
        ] {
            let n = sqlx::query(sql).execute(&pool).await.unwrap();
            assert_eq!(n.rows_affected(), 1);
        }
        run_0138(&pool).await;
        assert_eq!(type_keys(&pool).await, ["B738", "C172", "PA28"]);
    }

    // AC 11. Before and after the file, over the default, the four wake rows, a family row, PA28 and an
    // operator's A321 type row: identical rows, `updated_at` included. A seeded row saved with no number
    // changed goes: the numbers decide.
    #[sqlx::test]
    async fn the_migration_leaves_every_other_row_untouched(pool: PgPool) {
        seed_0059(&pool).await;
        sqlx::query(
            "insert into flow.aircraft_profile (kind, key, desc_fpm) \
             values ('family', 'jet', 4444), ('type', 'A321', 3333)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let others = || async {
            let rows: Vec<serde_json::Value> = sqlx::query_scalar(
                "select to_jsonb(p) from flow.aircraft_profile p \
                 where not (kind = 'type' and key = any($1)) order by kind, key",
            )
            .bind(TEN.map(String::from).to_vec())
            .fetch_all(&pool)
            .await
            .unwrap();
            rows
        };
        let before = others().await;
        assert_eq!(before.len(), 8, "default, four wake, jet, PA28, A321");
        run_0138(&pool).await;
        assert_eq!(others().await, before);
        assert_eq!(type_keys(&pool).await, ["A321", "PA28"], "the ten went");
    }
}
