//! Reusable per-airport runway configurations (see migration 0036). Named configs with a
//! favored-wind rule + AAR/ADR, used to predict an event's rate from the forecast wind. Writes are
//! facility-scoped in the handler; the repo is unscoped.

use sqlx::PgPool;

use crate::{
    errors::ApiError,
    models::{AirportConfigBody, UpsertAirportConfigRequest},
};

const CONFIG_SELECT: &str = "select c.id, c.icao, c.name, c.aar, c.adr, c.landing_runways, \
    c.wind_from_deg, c.wind_to_deg, c.calm_default, c.artcc, c.updated_at, \
    u.display_name as updated_by \
    from flow.airport_config c left join identity.users u on u.id = c.updated_by";

pub async fn list_by_icao(pool: &PgPool, icao: &str) -> Result<Vec<AirportConfigBody>, ApiError> {
    sqlx::query_as::<_, AirportConfigBody>(&format!(
        "{CONFIG_SELECT} where c.icao = $1 order by c.calm_default desc, c.name"
    ))
    .bind(icao)
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

/// Every airport's configs — ordered by airport for the all-airports list view. Unfiltered: the
/// `artcc` column is a snapshot taken at row creation and can go stale after a facility
/// realignment, so scoping/filtering by owning ARTCC is done by the caller against the *live*
/// facility map instead (see `handlers::airport_configs::annotate_and_filter`).
pub async fn list_all(pool: &PgPool) -> Result<Vec<AirportConfigBody>, ApiError> {
    sqlx::query_as::<_, AirportConfigBody>(&format!(
        "{CONFIG_SELECT} order by c.icao, c.calm_default desc, c.name"
    ))
    .fetch_all(pool)
    .await
    .map_err(|_| ApiError::Internal)
}

pub async fn get(pool: &PgPool, id: &str) -> Result<Option<AirportConfigBody>, ApiError> {
    sqlx::query_as::<_, AirportConfigBody>(&format!("{CONFIG_SELECT} where c.id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|_| ApiError::Internal)
}

/// Only one calm-default config per airport — clear any existing one (optionally excluding `keep`).
async fn clear_calm(pool: &PgPool, icao: &str, keep: Option<&str>) -> Result<(), ApiError> {
    sqlx::query(
        "update flow.airport_config set calm_default = false \
         where icao = $1 and calm_default and ($2::text is null or id <> $2)",
    )
    .bind(icao)
    .bind(keep)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(())
}

pub async fn create(
    pool: &PgPool,
    icao: &str,
    req: &UpsertAirportConfigRequest,
    artcc: &str,
    actor: &str,
) -> Result<AirportConfigBody, ApiError> {
    if req.calm_default {
        clear_calm(pool, icao, None).await?;
    }
    let id: String = sqlx::query_scalar(
        "insert into flow.airport_config \
             (icao, name, aar, adr, landing_runways, wind_from_deg, wind_to_deg, calm_default, artcc, updated_by) \
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) returning id",
    )
    .bind(icao)
    .bind(&req.name)
    .bind(req.aar)
    .bind(req.adr)
    .bind(&req.landing_runways)
    .bind(req.wind_from_deg)
    .bind(req.wind_to_deg)
    .bind(req.calm_default)
    .bind(artcc)
    .bind(actor)
    .fetch_one(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    get(pool, &id).await?.ok_or(ApiError::Internal)
}

pub async fn update(
    pool: &PgPool,
    id: &str,
    icao: &str,
    req: &UpsertAirportConfigRequest,
    actor: &str,
) -> Result<Option<AirportConfigBody>, ApiError> {
    if req.calm_default {
        clear_calm(pool, icao, Some(id)).await?;
    }
    let r = sqlx::query(
        "update flow.airport_config set \
             name = $3, aar = $4, adr = $5, landing_runways = $6, \
             wind_from_deg = $7, wind_to_deg = $8, calm_default = $9, updated_by = $10 \
         where id = $1 and icao = $2",
    )
    .bind(id)
    .bind(icao)
    .bind(&req.name)
    .bind(req.aar)
    .bind(req.adr)
    .bind(&req.landing_runways)
    .bind(req.wind_from_deg)
    .bind(req.wind_to_deg)
    .bind(req.calm_default)
    .bind(actor)
    .execute(pool)
    .await
    .map_err(|_| ApiError::Internal)?;
    if r.rows_affected() == 0 {
        return Ok(None);
    }
    get(pool, id).await
}

pub async fn delete(pool: &PgPool, id: &str) -> Result<bool, ApiError> {
    let r = sqlx::query("delete from flow.airport_config where id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|_| ApiError::Internal)?;
    Ok(r.rows_affected() > 0)
}

/// True if `dir` falls within `[from, to]` degrees, inclusive, wrap-around allowed (e.g.
/// `from=350, to=10` covers 350..360 and 0..10). Port of the client's `inWindRange`
/// (`web/src/lib/airport-configs.ts`) — kept in sync by hand, not shared code, since one side is
/// TS and the other Rust.
pub fn in_wind_range(dir: i32, from: i32, to: i32) -> bool {
    if from <= to {
        dir >= from && dir <= to
    } else {
        dir >= from || dir <= to
    }
}

/// The config a wind direction selects: the first non-calm config whose rule contains the
/// direction, else the calm-default, else the first config. `None` wind (calm/unknown) always
/// falls through to the calm-default/first. Port of the client's `matchConfig` (#242's AADC AAR
/// line resolves this the same way the event-planning rate predictor already does).
pub fn favored_config(
    configs: &[AirportConfigBody],
    wind_dir: Option<i32>,
) -> Option<&AirportConfigBody> {
    if let Some(dir) = wind_dir
        && let Some(m) = configs
            .iter()
            .find(|c| !c.calm_default && in_wind_range(dir, c.wind_from_deg, c.wind_to_deg))
    {
        return Some(m);
    }
    configs.iter().find(|c| c.calm_default).or(configs.first())
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;

    fn cfg(id: &str, aar: i32, from: i32, to: i32, calm: bool) -> AirportConfigBody {
        AirportConfigBody {
            id: id.into(),
            icao: "KTST".into(),
            name: id.into(),
            aar,
            adr: aar,
            landing_runways: vec![],
            wind_from_deg: from,
            wind_to_deg: to,
            calm_default: calm,
            artcc: "ZZZ".into(),
            updated_at: Utc::now(),
            updated_by: None,
            editable: true,
        }
    }

    #[test]
    fn in_wind_range_handles_wraparound() {
        assert!(in_wind_range(5, 350, 10));
        assert!(in_wind_range(355, 350, 10));
        assert!(!in_wind_range(180, 350, 10));
        assert!(in_wind_range(180, 170, 190));
    }

    #[test]
    fn favored_config_matches_the_in_range_non_calm_config() {
        let configs = vec![
            cfg("calm", 30, 0, 0, true),
            cfg("north", 30, 340, 20, false),
            cfg("south", 40, 160, 200, false),
        ];
        let picked = favored_config(&configs, Some(180)).unwrap();
        assert_eq!(picked.id, "south");
    }

    #[test]
    fn favored_config_falls_back_to_calm_default_when_wind_is_none() {
        let configs = vec![
            cfg("calm", 30, 0, 0, true),
            cfg("north", 30, 340, 20, false),
        ];
        let picked = favored_config(&configs, None).unwrap();
        assert_eq!(picked.id, "calm");
    }

    #[test]
    fn favored_config_falls_back_to_calm_default_when_no_rule_matches() {
        let configs = vec![
            cfg("calm", 30, 0, 0, true),
            cfg("north", 30, 340, 20, false),
        ];
        // 180 matches neither the wraparound "north" rule nor anything else.
        let picked = favored_config(&configs, Some(180)).unwrap();
        assert_eq!(picked.id, "calm");
    }

    #[test]
    fn favored_config_falls_back_to_first_when_no_calm_default_exists() {
        let configs = vec![cfg("only", 30, 340, 20, false)];
        let picked = favored_config(&configs, Some(180)).unwrap();
        assert_eq!(picked.id, "only");
    }

    #[test]
    fn favored_config_returns_none_for_an_empty_list() {
        assert!(favored_config(&[], Some(180)).is_none());
    }
}
