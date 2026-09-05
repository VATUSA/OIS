//! Configurable aircraft performance profiles (see migration 0059). CRUD for the profile catalog
//! plus `load_all`, which builds the in-memory [`ProfileTable`] the trajectory model resolves
//! against. National reference data; writes are gated in the handler by `flow.aircraft_profiles.update`.

use std::collections::HashMap;

use sqlx::PgPool;

use crate::{
    errors::ApiError,
    feed::trajectory::{AircraftProfile, ProfileTable},
    models::{AircraftProfileBody, UpsertAircraftProfileRequest},
};

const PROFILE_SELECT: &str = "select p.kind, p.key, p.name, \
    p.climb_ias_lo, p.climb_ias_hi, p.climb_mach, p.climb_fpm_lo, p.climb_fpm_hi, \
    p.cruise_tas, p.cruise_mach, p.service_ceiling_ft, \
    p.desc_mach, p.desc_ias_hi, p.desc_ias_lo, p.desc_fpm, \
    p.updated_at, u.display_name as updated_by \
    from flow.aircraft_profile p left join identity.users u on u.id = p.updated_by";

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
    actor: &str,
) -> Result<AircraftProfileBody, ApiError> {
    sqlx::query(
        "insert into flow.aircraft_profile \
         (kind, key, name, climb_ias_lo, climb_ias_hi, climb_mach, climb_fpm_lo, climb_fpm_hi, \
          cruise_tas, cruise_mach, service_ceiling_ft, desc_mach, desc_ias_hi, desc_ias_lo, \
          desc_fpm, updated_by) \
         values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16) \
         on conflict (kind, key) do update set \
          name = excluded.name, climb_ias_lo = excluded.climb_ias_lo, \
          climb_ias_hi = excluded.climb_ias_hi, climb_mach = excluded.climb_mach, \
          climb_fpm_lo = excluded.climb_fpm_lo, climb_fpm_hi = excluded.climb_fpm_hi, \
          cruise_tas = excluded.cruise_tas, cruise_mach = excluded.cruise_mach, \
          service_ceiling_ft = excluded.service_ceiling_ft, desc_mach = excluded.desc_mach, \
          desc_ias_hi = excluded.desc_ias_hi, desc_ias_lo = excluded.desc_ias_lo, \
          desc_fpm = excluded.desc_fpm, updated_by = excluded.updated_by, updated_at = now()",
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
    .bind(actor)
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
    let mut default = AircraftProfile::default();
    let mut by_wake: HashMap<String, AircraftProfile> = HashMap::new();
    let mut by_type: HashMap<String, AircraftProfile> = HashMap::new();
    for r in &rows {
        let p = to_profile(r);
        match r.kind.as_str() {
            "default" => default = p,
            "wake" => {
                by_wake.insert(r.key.to_ascii_uppercase(), p);
            }
            _ => {
                by_type.insert(r.key.to_ascii_uppercase(), p);
            }
        }
    }
    Ok(ProfileTable::new(default, by_wake, by_type))
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
