//! Configurable aircraft performance profiles (see migration 0059). CRUD for the profile catalog
//! plus `load_all`, which builds the in-memory [`ProfileTable`] the trajectory model resolves
//! against. National reference data; writes are gated in the handler by `flow.aircraft_profiles.update`.

use std::collections::HashMap;

use sqlx::PgPool;

use crate::{
    auth::principal::Attribution,
    errors::ApiError,
    feed::trajectory::{AircraftProfile, ProfileTable},
    models::{AircraftProfileBody, UpsertAircraftProfileRequest},
};

const PROFILE_SELECT: &str = "select p.kind, p.key, p.name, \
    p.climb_ias_lo, p.climb_ias_hi, p.climb_mach, p.climb_fpm_lo, p.climb_fpm_hi, \
    p.cruise_tas, p.cruise_mach, p.service_ceiling_ft, \
    p.desc_mach, p.desc_ias_hi, p.desc_ias_lo, p.desc_fpm, \
    p.updated_at, coalesce(u.display_name, a.display_name) as updated_by \
    from flow.aircraft_profile p left join identity.users u on u.id = p.updated_by \
    left join access.actors a on a.id = p.updated_by_actor";

/// The whole catalog, ordered default → wake → type for display.
pub async fn list(pool: &PgPool) -> Result<Vec<AircraftProfileBody>, ApiError> {
    sqlx::query_as::<_, AircraftProfileBody>(&format!(
        "{PROFILE_SELECT} order by \
         case p.kind when 'default' then 0 when 'wake' then 1 else 2 end, p.key"
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

/// Build the in-memory resolution table the trajectory model uses. Falls back to the legacy
/// `AircraftProfile::default()` when no `default` row exists (e.g. an empty DB).
pub async fn load_all(pool: &PgPool) -> Result<ProfileTable, ApiError> {
    let rows = list(pool).await?;
    let t = StoredTiers::from_rows(&rows);
    Ok(ProfileTable::new(t.default, t.by_wake, t.by_type))
}

/// Stored rows split into the three tiers [`ProfileTable`] resolves through, keys uppercased. The
/// one place a row's `kind` picks its tier, shared by [`load_all`] and the aircraft-profile
/// backtest's stored set (#828).
#[derive(Debug, Clone)]
pub struct StoredTiers {
    pub default: AircraftProfile,
    pub by_wake: HashMap<String, AircraftProfile>,
    pub by_type: HashMap<String, AircraftProfile>,
}

impl StoredTiers {
    pub fn from_rows(rows: &[AircraftProfileBody]) -> Self {
        let mut t = Self {
            default: AircraftProfile::default(),
            by_wake: HashMap::new(),
            by_type: HashMap::new(),
        };
        for r in rows {
            let p = to_profile(r);
            match r.kind.as_str() {
                "default" => t.default = p,
                "wake" => {
                    t.by_wake.insert(r.key.to_ascii_uppercase(), p);
                }
                _ => {
                    t.by_type.insert(r.key.to_ascii_uppercase(), p);
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

        assert_eq!(table.resolve("ZZ9Z", "H").desc_fpm, 3_333.0, "type tier");
        assert_eq!(table.resolve("QQQQ", "H").desc_fpm, 2_222.0, "wake tier");
        assert_eq!(table.resolve("QQQQ", "").desc_fpm, 1_111.0, "default tier");
    }
}
