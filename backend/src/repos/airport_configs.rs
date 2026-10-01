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

/// Degrees between two bearings, the short way round — 0..=180.
///
/// Same formula as `feed::runway::angle_diff`, deliberately not shared: a `repos` module reaching into
/// `feed` for arithmetic would be a worse dependency than two copies of one line. Unifying them is
/// follow-up work, not #510's.
fn arc_deg(a: i32, b: i32) -> i32 {
    let d = (a - b).rem_euclid(360);
    d.min(360 - d)
}

/// How far `dir` is from a config's wind rule: 0 when the rule contains it, else the degrees to the
/// nearer edge of the range.
fn distance_to_rule(dir: i32, c: &AirportConfigBody) -> i32 {
    if in_wind_range(dir, c.wind_from_deg, c.wind_to_deg) {
        0
    } else {
        arc_deg(dir, c.wind_from_deg).min(arc_deg(dir, c.wind_to_deg))
    }
}

/// **The authoritative rule for which way an airport is running, departures included (#510).**
///
/// The wind is matched to the **closest** config stored for that airport: a config whose rule contains
/// the direction is a perfect match, and where the rules leave a gap the nearest one wins rather than
/// nothing. A facility's ranges should cover the compass, so the gap case is a safety net, not the
/// normal path. **A tie goes to the config marked `calm_default`** — ties are the one case where
/// "closest" has no answer, and deferring to the configured default beats resolving them by name.
///
/// Calm or unknown wind (`None`) takes the calm-default outright, before any distance is computed:
/// there is no direction to be near. `calm_default` is excluded from the distance candidates for the
/// same reason it was excluded from the old containment search — its range columns default to `0..360`,
/// so it would contain every direction and always win.
///
/// # Why the Runway Balancer's preset is *not* authoritative
///
/// `feed::runway::apply_preset` looks like a second wind rule and is not one: it reads no wind at all.
/// A controller presses WEST/EAST/NORTH/SOUTH and every runway end within `PRESET_TOL` of that cardinal
/// becomes active — the human is the wind sensor. It also answers a different question (which ends we
/// are *landing* on) and its Rust copy is unreachable, the live logic being reimplemented in
/// `web/src/pages/runway/index.tsx`. So it cannot be the source of truth for a departure runway.
///
/// # Arrivals and departures may legitimately disagree
///
/// Arrivals follow `flow.runway_config.active_ends`, which is a controller's deliberate choice and may
/// be held against the wind — a crosswind runway for noise, a closure, a tailwind within limits. This
/// function follows the wind. The two diverging is therefore a valid state, not a bug, and
/// `arrivals_may_differ_from_the_wind_favoured_config` pins that it is allowed.
///
/// Mirrored by hand in `matchConfig` (`web/src/lib/airport-configs.ts`); the two must move together.
pub fn favored_config(
    configs: &[AirportConfigBody],
    wind_dir: Option<i32>,
) -> Option<&AirportConfigBody> {
    let calm = || configs.iter().find(|c| c.calm_default);
    let Some(dir) = wind_dir else {
        return calm().or(configs.first());
    };
    let best = configs
        .iter()
        .filter(|c| !c.calm_default)
        .map(|c| (distance_to_rule(dir, c), c))
        .min_by_key(|(d, _)| *d);
    let Some((best_d, best_c)) = best else {
        return calm().or(configs.first());
    };
    let tied = configs
        .iter()
        .filter(|c| !c.calm_default && distance_to_rule(dir, c) == best_d)
        .count();
    if tied > 1 {
        // Equally close: the configured default decides, rather than config name order.
        return calm().or(Some(best_c));
    }
    Some(best_c)
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

    /// Changed deliberately by #510: a wind in a gap between the authored ranges now takes the
    /// **closest** config rather than falling through to the calm default. A gap means the facility's
    /// ranges do not cover the compass, and the nearest rule is a better answer than pretending the
    /// wind is calm when it is blowing at 180.
    #[test]
    fn a_wind_in_a_gap_takes_the_closest_rule_not_the_calm_default() {
        let configs = vec![
            cfg("calm", 30, 0, 0, true),
            cfg("north", 30, 340, 20, false),
        ];
        // 180 is in neither rule. "north" wraps 340..20, so its nearer edge (20) is 160 away.
        let picked = favored_config(&configs, Some(180)).unwrap();
        assert_eq!(picked.id, "north");

        // With a single candidate the above only proves "a non-calm config was chosen" — any distance
        // function picks the only option. Two candidates at different distances is what actually pins
        // *nearest*: 220 is 10 degrees outside south (edge 210) and 20 outside west (edge 240).
        let three = vec![
            cfg("calm", 30, 0, 0, true),
            cfg("west", 30, 240, 300, false),
            cfg("south", 30, 150, 210, false),
        ];
        assert_eq!(
            favored_config(&three, Some(220)).unwrap().id,
            "south",
            "the nearer edge wins, and not merely the first in list order"
        );
    }

    /// The owner's tiebreak: equally close is the one case where "closest" has no answer, so the
    /// configured default decides rather than config-name order.
    #[test]
    fn an_equally_close_tie_goes_to_the_calm_default() {
        let configs = vec![
            cfg("calm", 30, 0, 0, true),
            // 225 sits exactly 15 degrees outside both.
            cfg("south", 30, 150, 210, false),
            cfg("west", 30, 240, 300, false),
        ];
        let picked = favored_config(&configs, Some(225)).unwrap();
        assert_eq!(picked.id, "calm");
    }

    /// A containing rule beats a merely-near one, so an authored range keeps meaning what it says.
    #[test]
    fn a_containing_rule_beats_a_nearer_edge() {
        let configs = vec![
            cfg("west", 30, 240, 300, false),
            // Its edge (305) is only 5 from the wind, but "west" actually contains 300.
            cfg("northwest", 30, 305, 345, false),
        ];
        let picked = favored_config(&configs, Some(300)).unwrap();
        assert_eq!(picked.id, "west");
    }

    /// Without a calm default there is nothing to defer to, so a tie resolves deterministically to the
    /// first of the tied configs rather than panicking or returning none.
    #[test]
    fn a_tie_without_a_calm_default_still_returns_a_config() {
        let configs = vec![
            cfg("south", 30, 150, 210, false),
            cfg("west", 30, 240, 300, false),
        ];
        assert_eq!(favored_config(&configs, Some(225)).unwrap().id, "south");
    }

    /// AC3's escape hatch. The wind-favoured config and the controller's active arrival runways are
    /// allowed to disagree, and this pins that `favored_config` neither reads nor constrains the arrival
    /// side — a controller may hold a crosswind runway for noise, a closure, or a tailwind within limits.
    ///
    /// Asserted as *independence*: the same arrival state is paired with two different winds, and the
    /// answer moves with the wind alone. An earlier version of this test compared the result against a
    /// hard-coded `"south"` string and so asserted only that 270 picks `west`, which a neighbouring test
    /// already covered — it would have passed even if this function had started consulting
    /// `active_ends`.
    #[test]
    fn the_favoured_config_is_independent_of_the_arrival_side() {
        let configs = vec![
            cfg("calm", 30, 0, 0, true),
            cfg("south", 30, 150, 210, false),
            cfg("west", 30, 240, 300, false),
        ];
        // `flow.runway_config.active_ends` is the arrival side's state. It is not an input here, and
        // that is the point: this function cannot see it, so the two are free to disagree.
        let westerly = favored_config(&configs, Some(270)).unwrap().id.clone();
        let southerly = favored_config(&configs, Some(180)).unwrap().id.clone();

        assert_eq!(westerly, "west");
        assert_eq!(southerly, "south");
        assert_ne!(
            westerly, southerly,
            "the answer tracks the wind, with nothing reconciling it against the arrival config"
        );
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
