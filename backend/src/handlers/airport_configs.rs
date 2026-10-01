//! Reusable per-airport runway configurations (default AAR/ADR + favored-wind rule). Reads are open
//! to planners (`events.plan.read`); writes are facility-scoped by the airport's owning ARTCC
//! (`events.config.update`), reusing the same scope infra as the per-event airport rates.

use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;

use crate::{
    auth::{
        context::{CurrentApiKey, CurrentUser},
        permissions::{EventsConfigUpdate, EventsPlanRead},
        principal::Principal,
        require_permission::RequirePermission,
    },
    errors::ApiError,
    feed,
    feed::facilities::FacilityMap,
    handlers::events::{normalize_icao, owning_artcc},
    models::{AirportConfigBody, AirportForecastBody, UpsertAirportConfigRequest},
    repos::access::PermissionScope,
    repos::airport_configs as config_repo,
    state::AppState,
};

const CONFIG_PERMISSION: &str = "events.config.update";

#[derive(Deserialize)]
pub struct ForecastQuery {
    /// Unix seconds of the time to forecast for; defaults to now.
    pub at: Option<i64>,
}

#[utoipa::path(
    get, path = "/api/v1/forecast/{icao}", tag = "events",
    params(("icao" = String, Path), ("at" = Option<i64>, Query)),
    responses((status = 200, body = AirportForecastBody), (status = 401))
)]
pub async fn forecast_wind(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
    Path(icao): Path<String>,
    Query(q): Query<ForecastQuery>,
) -> Result<Json<AirportForecastBody>, ApiError> {
    let icao = normalize_icao(&icao).ok_or(ApiError::BadRequest)?;
    let at =
        q.at.and_then(|s| Utc.timestamp_opt(s, 0).single())
            .unwrap_or_else(Utc::now);
    let w = wind_for(&state, &icao, at).await;
    Ok(Json(AirportForecastBody {
        icao,
        time: w.time,
        wind_dir: w.dir,
        wind_kt: w.spd_kt,
        gust_kt: w.gust_kt,
        source: w.source.to_string(),
    }))
}

/// How near "now" a request has to be before the observed METAR answers it instead of the forecast.
///
/// A METAR describes the field as it is, so it is the better answer for now and useless for later. The
/// window is generous relative to the 10-minute METAR cache and narrow relative to the forecast's hourly
/// resolution, so "what is the wind doing" gets the observation and "what will it be at 2300Z" gets the
/// model.
const METAR_PREFERRED_WITHIN_MIN: i64 = 30;

/// The wind an airport's configuration is matched against, and where it came from (#510).
///
/// **Observed wind wins for the present**: METAR is a ten-minute observation that degrades to its last
/// cached value when the upstream fails, where the forecast is a two-hour-TTL 10-metre model value with
/// no fallback at all. Beyond [`METAR_PREFERRED_WITHIN_MIN`] there is nothing to observe, so the
/// forecast answers.
///
/// This is deliberately the **one** place that choice is made. `GET /forecast/{icao}` returns it and the
/// AADC path calls it, so the backend and the client's `matchConfig` cannot end up matching
/// configurations against different winds — which is how arrivals and departures would come to disagree
/// for a reason nobody intended (#510 AC3).
///
/// `source` fills in `metar`, which [`AirportForecastBody`] has documented since #242 and no code ever
/// emitted.
pub(crate) async fn wind_for(state: &AppState, icao: &str, at: DateTime<Utc>) -> ResolvedWind {
    let now = Utc::now();
    if (at - now).num_minutes().abs() <= METAR_PREFERRED_WITHIN_MIN
        && let Some(obs) = crate::handlers::runway::metar_for(state, icao)
            .await
            .and_then(|m| m.wind_obs)
    {
        return ResolvedWind {
            time: now,
            // The same calm rule the forecast applies, so a 2-knot wind is calm whichever source
            // reported it and the airport does not change config with the weather provider.
            dir: obs.dir.filter(|_| obs.spd_kt >= feed::forecast::CALM_KT),
            spd_kt: obs.spd_kt,
            gust_kt: obs.gust_kt,
            source: "metar",
        };
    }
    let airports = state.feed.read().await.airports.clone();
    match feed::forecast::wind_at(&airports, icao, at).await {
        Some(h) => ResolvedWind {
            time: h.time,
            dir: h.dir,
            spd_kt: h.spd_kt,
            gust_kt: h.gust_kt,
            source: "forecast",
        },
        // No observation and no forecast: say so rather than imply a direction. `dir: None` makes
        // `favored_config` take the calm default, which is the defined no-wind answer (#510 AC4).
        None => ResolvedWind {
            time: at,
            dir: None,
            spd_kt: 0,
            gust_kt: None,
            source: "none",
        },
    }
}

/// A wind with its provenance. `source` is `metar` | `forecast` | `none`.
pub(crate) struct ResolvedWind {
    pub time: DateTime<Utc>,
    pub dir: Option<i32>,
    pub spd_kt: i32,
    pub gust_kt: Option<i32>,
    pub source: &'static str,
}

fn validate(req: &UpsertAirportConfigRequest) -> Result<(), ApiError> {
    if req.name.trim().is_empty() || req.name.len() > 64 {
        return Err(ApiError::BadRequest);
    }
    if !(0..=200).contains(&req.aar) || !(0..=200).contains(&req.adr) {
        return Err(ApiError::BadRequest);
    }
    if !(0..=360).contains(&req.wind_from_deg) || !(0..=360).contains(&req.wind_to_deg) {
        return Err(ApiError::BadRequest);
    }
    Ok(())
}

/// Does the caller hold `events.config.update` nationally or for `icao`'s owning ARTCC?
/// Works for a signed-in user or an API key (whose scope is capped by its owner).
async fn can_edit(state: &AppState, principal: &Principal, icao: &str) -> Result<bool, ApiError> {
    let artcc = owning_artcc(state, icao).await;
    let scope = principal.permission_scope(state, CONFIG_PERMISSION).await?;
    Ok(scope.allows(artcc.as_deref()))
}

/// Fail-closed if the caller can't edit `icao`. Every write handler below calls this instead of
/// duplicating the `owning_artcc` + `permission_scope` + `scope.allows` check inline (#198: the
/// inline copies had silently drifted out of any test's reach).
async fn require_edit(state: &AppState, principal: &Principal, icao: &str) -> Result<(), ApiError> {
    if can_edit(state, principal, icao).await? {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

#[derive(Deserialize)]
pub struct ConfigListQuery {
    /// Scope to one owning ARTCC; omit for every airport.
    pub artcc: Option<String>,
}

#[utoipa::path(
    get, path = "/api/v1/airport-configs", tag = "events",
    params(("artcc" = Option<String>, Query, description = "Scope to one owning ARTCC")),
    responses((status = 200, body = Vec<AirportConfigBody>), (status = 401))
)]
pub async fn list_all_airport_configs(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Query(q): Query<ConfigListQuery>,
) -> Result<Json<Vec<AirportConfigBody>>, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let artcc = q
        .artcc
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_ascii_uppercase);

    let scope = principal
        .permission_scope(&state, CONFIG_PERMISSION)
        .await?;
    let rows = config_repo::list_all(pool).await?;
    let facilities = state.facilities.read().await;
    Ok(Json(annotate_and_filter(
        rows,
        &facilities,
        &scope,
        artcc.as_deref(),
    )))
}

/// Resolve each row's live owning ARTCC (not its stored snapshot — that can go stale after a
/// facility realignment) and use it for both `editable` and an optional `?artcc=` filter, matching
/// `can_edit`/`create`/`update` which always check live.
fn annotate_and_filter(
    rows: Vec<AirportConfigBody>,
    facilities: &FacilityMap,
    scope: &PermissionScope,
    artcc_filter: Option<&str>,
) -> Vec<AirportConfigBody> {
    rows.into_iter()
        .filter_map(|mut r| {
            let live = feed::facilities::artcc_for_airport(facilities, &r.icao);
            r.editable = scope.allows(live.as_deref());
            (artcc_filter.is_none() || live.as_deref() == artcc_filter).then_some(r)
        })
        .collect()
}

#[utoipa::path(
    get, path = "/api/v1/airport-configs/{icao}", tag = "events",
    params(("icao" = String, Path)),
    responses((status = 200, body = Vec<AirportConfigBody>), (status = 401))
)]
pub async fn list_airport_configs(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path(icao): Path<String>,
) -> Result<Json<Vec<AirportConfigBody>>, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let icao = normalize_icao(&icao).ok_or(ApiError::BadRequest)?;

    let editable = can_edit(&state, &principal, &icao).await?;
    let mut rows = config_repo::list_by_icao(pool, &icao).await?;
    for r in &mut rows {
        r.editable = editable;
    }
    Ok(Json(rows))
}

#[utoipa::path(
    post, path = "/api/v1/airport-configs/{icao}", tag = "events",
    params(("icao" = String, Path)), request_body = UpsertAirportConfigRequest,
    responses((status = 200, body = AirportConfigBody), (status = 400), (status = 401), (status = 403))
)]
pub async fn create_airport_config(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsConfigUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path(icao): Path<String>,
    Json(req): Json<UpsertAirportConfigRequest>,
) -> Result<Json<AirportConfigBody>, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let icao = normalize_icao(&icao).ok_or(ApiError::BadRequest)?;
    validate(&req)?;
    require_edit(&state, &principal, &icao).await?;

    let artcc = owning_artcc(&state, &icao).await;
    let mut row = config_repo::create(
        pool,
        &icao,
        &req,
        artcc.as_deref().unwrap_or(""),
        principal.user_id(),
    )
    .await?;
    row.editable = true;
    Ok(Json(row))
}

#[utoipa::path(
    put, path = "/api/v1/airport-configs/{icao}/{id}", tag = "events",
    params(("icao" = String, Path), ("id" = String, Path)), request_body = UpsertAirportConfigRequest,
    responses((status = 200, body = AirportConfigBody), (status = 400), (status = 401), (status = 403), (status = 404))
)]
pub async fn update_airport_config(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsConfigUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path((icao, id)): Path<(String, String)>,
    Json(req): Json<UpsertAirportConfigRequest>,
) -> Result<Json<AirportConfigBody>, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let icao = normalize_icao(&icao).ok_or(ApiError::BadRequest)?;
    validate(&req)?;
    require_edit(&state, &principal, &icao).await?;

    let mut row = config_repo::update(pool, &id, &icao, &req, principal.user_id())
        .await?
        .ok_or(ApiError::NotFound)?;
    row.editable = true;
    Ok(Json(row))
}

#[utoipa::path(
    delete, path = "/api/v1/airport-configs/{icao}/{id}", tag = "events",
    params(("icao" = String, Path), ("id" = String, Path)),
    responses((status = 204), (status = 401), (status = 403), (status = 404))
)]
pub async fn delete_airport_config(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsConfigUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path((icao, id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let icao = normalize_icao(&icao).ok_or(ApiError::BadRequest)?;
    require_edit(&state, &principal, &icao).await?;

    if config_repo::delete(pool, &id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use sqlx::PgPool;

    use super::*;
    use crate::feed::facilities::Facility;

    // --- the wind resolution point (#510 review) -------------------------------------------------
    //
    // `wind_for` is the one place the METAR-vs-forecast choice is made, which is what AC3 rests on, and
    // it had no test: five mutations survived it — dropping the `CALM_KT` filter, moving the window to 0
    // or to a week, and mislabelling either `source`. These are hermetic: `metar_for` returns a fresh
    // cached entry *before* building any HTTP client, and `forecast::wind_at` returns on
    // `airports.get(icao)?` for an airport the (empty) feed does not know, so nothing reaches the
    // network.

    /// A cached observation for `icao`, as `metar_for` would have left it.
    fn seed_metar(state: &AppState, icao: &str, obs: Option<crate::feed::metar::MetarWind>) {
        let info = crate::feed::metar::MetarInfo {
            raw: format!("{icao} 00000KT"),
            category: "VFR".to_string(),
            wind: None,
            wind_obs: obs,
        };
        state
            .metar_cache
            .lock()
            .expect("metar cache")
            .insert(icao.to_string(), (info, Utc::now().timestamp_millis()));
    }

    fn wind(dir: Option<i32>, spd_kt: i32) -> crate::feed::metar::MetarWind {
        crate::feed::metar::MetarWind {
            dir,
            spd_kt,
            gust_kt: None,
        }
    }

    /// The observation answers for the present, and says so.
    #[sqlx::test]
    async fn an_observed_wind_answers_for_the_present(pool: PgPool) {
        let state = test_state(pool, HashMap::new());
        seed_metar(&state, "KTST", Some(wind(Some(270), 15)));

        let w = wind_for(&state, "KTST", Utc::now()).await;

        assert_eq!(w.source, "metar");
        assert_eq!(w.dir, Some(270));
        assert_eq!(w.spd_kt, 15);
    }

    /// A wind below `CALM_KT` reports **no direction**, exactly as the forecast path does.
    ///
    /// This is the filter the code comment exists for: without it a 2-knot observation keeps its
    /// bearing while a 2-knot forecast does not, so the same airport matches a different configuration
    /// depending on which source answered — the divergence AC3 is about. The speed is still reported,
    /// because the banner shows it; only the direction is suppressed.
    #[sqlx::test]
    async fn a_sub_calm_observed_wind_reports_no_direction(pool: PgPool) {
        let state = test_state(pool, HashMap::new());
        seed_metar(&state, "KTST", Some(wind(Some(270), 2)));

        let w = wind_for(&state, "KTST", Utc::now()).await;

        assert_eq!(w.source, "metar", "still the observation, just a calm one");
        assert_eq!(w.dir, None, "below CALM_KT there is no useful direction");
        assert_eq!(w.spd_kt, 2, "the speed is still reported");
    }

    /// A variable wind has no bearing to match a configuration against, so it reads as calm.
    #[sqlx::test]
    async fn a_variable_observed_wind_reports_no_direction(pool: PgPool) {
        let state = test_state(pool, HashMap::new());
        seed_metar(&state, "KTST", Some(wind(None, 12)));

        let w = wind_for(&state, "KTST", Utc::now()).await;

        assert_eq!(w.source, "metar");
        assert_eq!(w.dir, None);
    }

    /// The window has two edges, and both matter: inside it the observation wins, outside it there is
    /// nothing to observe. Pinning only one side would leave "always prefer METAR" or "never prefer
    /// METAR" indistinguishable from the intended rule.
    #[sqlx::test]
    async fn the_observation_is_preferred_only_near_now(pool: PgPool) {
        let state = test_state(pool, HashMap::new());
        seed_metar(&state, "KTST", Some(wind(Some(270), 15)));
        let now = Utc::now();

        let inside = wind_for(&state, "KTST", now + chrono::Duration::minutes(29)).await;
        assert_eq!(
            inside.source, "metar",
            "29 minutes out is still the present"
        );

        // 45, not 31: `wind_for` recomputes `now` a few milliseconds later than this test did, and
        // `num_minutes()` truncates, so a nominal 31 reads as 30 and still counts as the present. The
        // window's exact edge is therefore fuzzy by a minute — immaterial for a weather observation, but
        // a test should not pretend otherwise by sitting on it.
        let outside = wind_for(&state, "KTST", now + chrono::Duration::minutes(45)).await;
        assert_ne!(
            outside.source, "metar",
            "well beyond the window must not be answered by an observation of now"
        );

        // The window is an absolute distance, so it closes in both directions. Asserting only that the
        // *recent* past still uses the observation proves nothing — a negative difference satisfies
        // `<= 30` on its own, so dropping the `.abs()` would leave that assertion green. The far past is
        // what pins it: an observation of now does not describe three quarters of an hour ago.
        let recent_past = wind_for(&state, "KTST", now - chrono::Duration::minutes(29)).await;
        assert_eq!(recent_past.source, "metar");
        let distant_past = wind_for(&state, "KTST", now - chrono::Duration::minutes(45)).await;
        assert_ne!(
            distant_past.source, "metar",
            "the window must close behind us as well as ahead"
        );
    }

    /// #510 AC4 — no observation and no forecast is a *defined* answer, not an implied direction.
    ///
    /// Driven through a cached METAR that carries no wind group, which is the real shape of "we have an
    /// observation but it tells us nothing about the wind", and leaves the test hermetic.
    #[sqlx::test]
    async fn no_wind_data_at_all_is_reported_as_none(pool: PgPool) {
        let state = test_state(pool, HashMap::new());
        seed_metar(&state, "KTST", None);

        let w = wind_for(&state, "KTST", Utc::now()).await;

        assert_eq!(w.source, "none", "neither observed nor forecast");
        assert_eq!(w.dir, None, "no direction may be implied");
        assert_eq!(w.spd_kt, 0);
        assert_eq!(w.gust_kt, None);
    }

    /// The composition both consumers perform, end to end: observation -> resolution -> configuration.
    ///
    /// This replaces a test that asserted `favored_config(.., 270) != "south"` and nothing else — a
    /// restatement of the containment case rather than a check of AC3. What AC3 actually guarantees is
    /// that there is one resolution point, so the same observation always selects the same
    /// configuration whichever caller asks. A sub-calm wind landing on the calm default is the
    /// interesting half: it only happens if the `CALM_KT` filter survived the trip.
    #[sqlx::test]
    async fn an_observation_selects_the_configuration_both_callers_would_get(pool: PgPool) {
        use crate::repos::airport_configs::favored_config;
        let state = test_state(pool, HashMap::new());

        let cfg = |id: &str, from: i32, to: i32, calm: bool| crate::models::AirportConfigBody {
            id: id.to_string(),
            icao: "KTST".to_string(),
            name: id.to_string(),
            aar: 30,
            adr: 30,
            landing_runways: vec![],
            wind_from_deg: from,
            wind_to_deg: to,
            calm_default: calm,
            artcc: "ZNY".to_string(),
            updated_at: Utc::now(),
            updated_by: None,
            editable: true,
        };
        let configs = vec![
            cfg("calm", 0, 0, true),
            cfg("west", 240, 300, false),
            cfg("south", 150, 210, false),
        ];

        seed_metar(&state, "KTST", Some(wind(Some(270), 15)));
        let w = wind_for(&state, "KTST", Utc::now()).await;
        assert_eq!(
            favored_config(&configs, w.dir).map(|c| c.id.as_str()),
            Some("west"),
            "a 15-knot westerly selects west"
        );

        seed_metar(&state, "KTST", Some(wind(Some(270), 2)));
        let calm_w = wind_for(&state, "KTST", Utc::now()).await;
        assert_eq!(
            favored_config(&configs, calm_w.dir).map(|c| c.id.as_str()),
            Some("calm"),
            "a 2-knot wind is calm, so it takes the calm default rather than west"
        );
    }

    async fn seed_user(pool: &PgPool) -> String {
        sqlx::query_scalar::<_, String>(
            "insert into identity.users (full_name, display_name) \
             values ('Test User', 'Test User') returning id",
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    fn upsert(name: &str) -> UpsertAirportConfigRequest {
        UpsertAirportConfigRequest {
            name: name.to_string(),
            aar: 30,
            adr: 30,
            landing_runways: vec![],
            wind_from_deg: 0,
            wind_to_deg: 360,
            calm_default: false,
        }
    }

    /// Realignment scenario: KORD's config was created while ZDC owned it (stored `artcc: "ZDC"`),
    /// but the live facility map now has it under ZAU. KDCA's stored value still matches live
    /// (ZDC, unaffected). KXXX has no resolvable owning ARTCC at all (absent from the facility map)
    /// — an empty-string stored `artcc` from a config created before any facility data existed.
    #[sqlx::test]
    async fn list_all_scopes_and_filters_by_the_live_artcc_not_the_stored_one(pool: PgPool) {
        let user = seed_user(&pool).await;
        config_repo::create(&pool, "KDCA", &upsert("DCA calm"), "ZDC", &user)
            .await
            .unwrap();
        config_repo::create(&pool, "KORD", &upsert("ORD calm"), "ZDC", &user)
            .await
            .unwrap(); // stale: stored ZDC, live (below) is ZAU
        config_repo::create(&pool, "KXXX", &upsert("XXX calm"), "", &user)
            .await
            .unwrap();

        let facilities = FacilityMap::from([
            (
                "ZDC".to_string(),
                Facility {
                    kind: "artcc".to_string(),
                    airports: vec!["KDCA".to_string()],
                },
            ),
            (
                "ZAU".to_string(),
                Facility {
                    kind: "artcc".to_string(),
                    airports: vec!["KORD".to_string()],
                },
            ),
        ]);

        // Ordering is untouched by annotate_and_filter — assert it directly on the raw fetch.
        let rows = config_repo::list_all(&pool).await.unwrap();
        assert_eq!(rows.len(), 3, "all three rows come back unfiltered");
        assert_eq!(
            rows.iter().map(|r| r.icao.as_str()).collect::<Vec<_>>(),
            ["KDCA", "KORD", "KXXX"],
            "icao, calm_default desc, name"
        );

        // A ZAU-scoped principal: editable only where the LIVE artcc is ZAU (KORD), despite its
        // stored value saying ZDC. The no-resolvable-ARTCC row (KXXX) is never editable under a
        // Facilities scope.
        let rows = config_repo::list_all(&pool).await.unwrap();
        let zau_scope = PermissionScope::Facilities(HashSet::from(["ZAU".to_string()]));
        let annotated = annotate_and_filter(rows, &facilities, &zau_scope, None);
        let editable: Vec<_> = annotated
            .iter()
            .filter(|r| r.editable)
            .map(|r| r.icao.as_str())
            .collect();
        assert_eq!(editable, ["KORD"]);

        // National scope sees everything as editable, live-artcc-filtered to ZDC: only KDCA, even
        // though KORD's *stored* value also says ZDC.
        let rows = config_repo::list_all(&pool).await.unwrap();
        let filtered =
            annotate_and_filter(rows, &facilities, &PermissionScope::National, Some("ZDC"));
        assert_eq!(
            filtered.iter().map(|r| r.icao.as_str()).collect::<Vec<_>>(),
            ["KDCA"]
        );
        assert!(filtered.iter().all(|r| r.editable));

        // A scope covering nobody's ARTCC: nothing is editable.
        let rows = config_repo::list_all(&pool).await.unwrap();
        let no_scope = PermissionScope::Facilities(HashSet::new());
        let annotated = annotate_and_filter(rows, &facilities, &no_scope, None);
        assert!(annotated.iter().all(|r| !r.editable));
    }

    // --- ARTCC-scope authorization boundary (#198) ---
    //
    // The test above (`annotate_and_filter`) exercises a *different*, list-only filtering
    // function against a plain `PermissionScope` value — it never calls `can_edit`/`require_edit`.
    // (create/update/delete_airport_config used to duplicate this check inline instead of calling
    // either helper, which meant these tests didn't actually cover the write path at all — fixed
    // alongside adding this coverage, so require_edit is now the single gate every write handler
    // calls.) These tests close that gap.

    use crate::scope_test_support::{self, artcc, grant, principal_for, test_state};

    #[sqlx::test]
    async fn national_scope_can_edit_any_artccs_airport(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        grant(&pool, &user, "events.config.update", None).await;
        let principal = principal_for(&user);
        let state = test_state(
            pool,
            std::collections::HashMap::from([("ZDC".to_string(), artcc(&["KDCA"]))]),
        );
        assert!(can_edit(&state, &principal, "KDCA").await.unwrap());
    }

    #[sqlx::test]
    async fn matching_artcc_scope_can_edit_its_own_airport(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        grant(&pool, &user, "events.config.update", Some("ZDC")).await;
        let principal = principal_for(&user);
        let state = test_state(
            pool,
            std::collections::HashMap::from([("ZDC".to_string(), artcc(&["KDCA"]))]),
        );
        assert!(can_edit(&state, &principal, "KDCA").await.unwrap());
    }

    #[sqlx::test]
    async fn wrong_artcc_scope_is_rejected(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        // Granted for ZAU, but KDCA is owned by ZDC.
        grant(&pool, &user, "events.config.update", Some("ZAU")).await;
        let principal = principal_for(&user);
        let state = test_state(
            pool,
            std::collections::HashMap::from([("ZDC".to_string(), artcc(&["KDCA"]))]),
        );
        assert!(!can_edit(&state, &principal, "KDCA").await.unwrap());
        assert!(matches!(
            require_edit(&state, &principal, "KDCA").await,
            Err(ApiError::Forbidden)
        ));
    }

    #[sqlx::test]
    async fn no_grant_at_all_is_rejected(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        let principal = principal_for(&user);
        let state = test_state(
            pool,
            std::collections::HashMap::from([("ZDC".to_string(), artcc(&["KDCA"]))]),
        );
        assert!(!can_edit(&state, &principal, "KDCA").await.unwrap());
        assert!(matches!(
            require_edit(&state, &principal, "KDCA").await,
            Err(ApiError::Forbidden)
        ));
    }

    // --- Through the router (VATUSA/OIS#364) ---
    //
    // The tests above cover `can_edit`/`require_edit` directly, which stays green if a write handler
    // stops calling them. These send real requests with a real session, so each write's
    // `RequirePermission<EventsConfigUpdate>` and its `require_edit` call are on the tested path. A
    // missing permission is 401 and the wrong facility 403, so each test pins one gate.

    use http::{Method, StatusCode};
    use scope_test_support::{send, session_cookie};

    fn config_json() -> serde_json::Value {
        serde_json::json!({
            "name": "South flow",
            "aar": 36,
            "adr": 40,
            "landing_runways": ["19"],
            "wind_from_deg": 90,
            "wind_to_deg": 270
        })
    }

    /// A KDCA (ZDC) config to update and delete, and a session for `user`. Returns every write:
    /// create, update, delete.
    async fn routed(
        pool: PgPool,
        user: &str,
    ) -> (crate::state::AppState, String, [(Method, String); 3]) {
        let req: crate::models::UpsertAirportConfigRequest =
            serde_json::from_value(config_json()).unwrap();
        let existing = config_repo::create(&pool, "KDCA", &req, "ZDC", user)
            .await
            .unwrap();
        let cookie = session_cookie(&pool, user).await;
        let state = test_state(
            pool,
            std::collections::HashMap::from([
                ("ZDC".to_string(), artcc(&["KDCA"])),
                ("ZNY".to_string(), artcc(&["KJFK"])),
            ]),
        );
        let one = format!("/api/v1/airport-configs/KDCA/{}", existing.id);
        let writes = [
            (Method::POST, "/api/v1/airport-configs/KDCA".to_string()),
            (Method::PUT, one.clone()),
            (Method::DELETE, one),
        ];
        (state, cookie, writes)
    }

    async fn statuses(
        state: &crate::state::AppState,
        cookie: &str,
        writes: &[(Method, String); 3],
    ) -> Vec<StatusCode> {
        let mut out = Vec::new();
        for (method, uri) in writes {
            let body = (*method != Method::DELETE).then(config_json);
            out.push(send(state, method.clone(), uri, cookie, body).await);
        }
        out
    }

    /// Fails if any write drops `RequirePermission`: `require_edit` would then answer 403.
    #[sqlx::test]
    async fn through_the_router_a_caller_without_the_permission_gets_401(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        let (state, cookie, writes) = routed(pool, &user).await;
        assert_eq!(
            statuses(&state, &cookie, &writes).await,
            [StatusCode::UNAUTHORIZED; 3]
        );
    }

    /// Fails if any write drops `require_edit`: the request would then go through.
    #[sqlx::test]
    async fn through_the_router_another_facilitys_grant_gets_403(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        grant(&pool, &user, "events.config.update", Some("ZNY")).await;
        let (state, cookie, writes) = routed(pool, &user).await;
        assert_eq!(
            statuses(&state, &cookie, &writes).await,
            [StatusCode::FORBIDDEN; 3]
        );
    }

    /// The positive control: ZDC's own editor gets through every write, so the refusals above are
    /// the gates answering and not a broken fixture.
    #[sqlx::test]
    async fn through_the_router_the_owning_facilitys_grant_goes_through(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        grant(&pool, &user, "events.config.update", Some("ZDC")).await;
        let (state, cookie, writes) = routed(pool, &user).await;
        assert_eq!(
            statuses(&state, &cookie, &writes).await,
            [StatusCode::OK, StatusCode::OK, StatusCode::NO_CONTENT]
        );
    }
}
