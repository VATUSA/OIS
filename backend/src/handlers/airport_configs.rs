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
    // #512 AC4: a rule must not name a runway this config does not have. Rejected rather than ignored
    // at read time, so the operator finds out when they save instead of wondering why their rule never
    // fires. Checked against the request's own `departure_runways`, since both can move together.
    for rules in [req.sid_rules.as_ref(), req.gate_rules.as_ref()]
        .into_iter()
        .flatten()
    {
        if !rules_fit(rules.values().map(String::as_str), &req.departure_runways) {
            return Err(ApiError::BadRequest);
        }
    }
    Ok(())
}

/// Whether every runway a rule map names is one the config actually has.
///
/// Comparison is on the trimmed, upper-cased name: a rule typed `31l` means the same runway as `31L`,
/// and rejecting it for case would be a trap rather than a safeguard.
fn rules_fit<'a>(mut named: impl Iterator<Item = &'a str>, available: &[String]) -> bool {
    let have: std::collections::HashSet<String> = available
        .iter()
        .map(|r| r.trim().to_ascii_uppercase())
        .collect();
    named.all(|r| have.contains(&r.trim().to_ascii_uppercase()))
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
        principal.user_id().ok_or(ApiError::Forbidden)?,
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
    // The other half of AC4. `validate` only sees the rules the request carries; a save that omits them
    // while narrowing `departure_runways` would leave the STORED rules naming a runway that no longer
    // exists — orphaned silently, which is exactly what the AC forbids. Read them and check.
    if req.sid_rules.is_none() || req.gate_rules.is_none() {
        let stored = config_repo::get(pool, &id)
            .await?
            .ok_or(ApiError::NotFound)?;
        let keeping = [
            (req.sid_rules.is_none(), &stored.sid_rules),
            (req.gate_rules.is_none(), &stored.gate_rules),
        ];
        for (_, rules) in keeping.iter().filter(|(omitted, _)| *omitted) {
            if !rules_fit(rules.0.values().map(String::as_str), &req.departure_runways) {
                return Err(ApiError::BadRequest);
            }
        }
    }

    let mut row = config_repo::update(
        pool,
        &id,
        &icao,
        &req,
        principal.user_id().ok_or(ApiError::Forbidden)?,
    )
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
    use std::collections::HashSet;

    use sqlx::PgPool;

    use super::*;
    use crate::feed::facilities::Facility;

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
            departure_runways: vec![],
            // None, not empty: the default fixture must exercise the omit-to-keep path, which is what
            // every pre-#512 client sends.
            sid_rules: None,
            gate_rules: None,
            wind_from_deg: 0,
            wind_to_deg: 360,
            calm_default: false,
        }
    }

    /// #509 AC 3: `departure_runways` has to survive a create *and* an update. Both SQL statements
    /// list their columns positionally, so a column added to one and not the other, or bound out of
    /// order, writes the wrong value — and with two `text[]` columns side by side that silently swaps
    /// arrivals for departures rather than failing.
    #[sqlx::test]
    async fn departure_runways_round_trip_through_create_and_update(pool: PgPool) {
        let user = seed_user(&pool).await;
        let created = config_repo::create(
            &pool,
            "KJFK",
            &UpsertAirportConfigRequest {
                landing_runways: vec!["04R".into(), "22L".into()],
                departure_runways: vec!["04L".into(), "31L".into()],
                ..upsert("JFK south")
            },
            "ZNY",
            &user,
        )
        .await
        .unwrap();
        assert_eq!(created.departure_runways, vec!["04L", "31L"]);
        assert_eq!(
            created.landing_runways,
            vec!["04R", "22L"],
            "the two arrays must not be swapped"
        );

        let updated = config_repo::update(
            &pool,
            &created.id,
            "KJFK",
            &UpsertAirportConfigRequest {
                landing_runways: vec!["13L".into()],
                departure_runways: vec!["13R".into()],
                ..upsert("JFK south")
            },
            &user,
        )
        .await
        .unwrap()
        .expect("the config exists");
        assert_eq!(updated.departure_runways, vec!["13R"]);
        assert_eq!(updated.landing_runways, vec!["13L"]);
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

    /// A map of rules, for the fixtures below.
    fn rules(pairs: &[(&str, &str)]) -> std::collections::HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// AC3, and the one that protects the other 184 airports: a config created without rules has
    /// empty maps and behaves exactly as it did before #512.
    #[sqlx::test]
    async fn a_config_with_no_rules_is_unchanged(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        let created = config_repo::create(&pool, "KJFK", &upsert("South"), "ZNY", &user)
            .await
            .unwrap();
        assert!(created.sid_rules.0.is_empty());
        assert!(created.gate_rules.0.is_empty());
    }

    /// AC2, the patch convention: a save that omits the rule maps leaves the stored ones alone. A
    /// client predating #512 sends exactly this shape on every unrelated edit, so without `coalesce`
    /// it would wipe an ARTCC's rules each time it changed an AAR.
    #[sqlx::test]
    async fn omitting_the_rules_leaves_them_unchanged(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        let mut req = upsert("South");
        req.departure_runways = vec!["31L".into()];
        req.sid_rules = Some(rules(&[("CAMRN", "31L")]));
        let created = config_repo::create(&pool, "KJFK", &req, "ZNY", &user)
            .await
            .unwrap();

        // An unrelated edit, in the shape an older client sends: no rule fields at all.
        let mut later = upsert("South");
        later.departure_runways = vec!["31L".into()];
        later.aar = 44;
        assert!(later.sid_rules.is_none() && later.gate_rules.is_none());
        let updated = config_repo::update(&pool, &created.id, "KJFK", &later, &user)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(updated.aar, 44, "the edit applied");
        assert_eq!(
            updated.sid_rules.0.get("CAMRN").map(String::as_str),
            Some("31L"),
            "and the rules it never mentioned survived it"
        );
    }

    /// AC1: rules round-trip, and the two maps stay distinct — a gate and a SID may share a name.
    #[sqlx::test]
    async fn rules_round_trip_per_config(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        let mut req = upsert("South");
        req.departure_runways = vec!["31L".into(), "04L".into()];
        req.sid_rules = Some(rules(&[("CAMRN", "31L")]));
        req.gate_rules = Some(rules(&[("CAMRN", "04L")]));
        let row = config_repo::create(&pool, "KJFK", &req, "ZNY", &user)
            .await
            .unwrap();

        assert_eq!(
            row.sid_rules.0.get("CAMRN").map(String::as_str),
            Some("31L")
        );
        assert_eq!(
            row.gate_rules.0.get("CAMRN").map(String::as_str),
            Some("04L"),
            "a gate named like a SID is a different rule, which is why there are two maps"
        );
    }

    /// AC4: a rule naming a runway the config does not have is rejected, not stored and ignored.
    #[test]
    fn a_rule_naming_an_absent_runway_is_rejected() {
        let mut req = upsert("South");
        req.departure_runways = vec!["31L".into()];
        req.sid_rules = Some(rules(&[("CAMRN", "22R")]));
        assert!(matches!(validate(&req), Err(ApiError::BadRequest)));

        // The same runway, differently cased, is the same runway — rejecting it would be a trap.
        req.sid_rules = Some(rules(&[("CAMRN", "31l")]));
        assert!(validate(&req).is_ok());
    }

    /// AC4's other half, which `validate` alone cannot see: narrowing `departure_runways` while the
    /// request says nothing about the rules would orphan the STORED ones. The update handler reads them
    /// and refuses, so a config can never hold a rule pointing at a runway it no longer has.
    #[sqlx::test]
    async fn narrowing_the_runways_cannot_orphan_a_stored_rule(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        grant(&pool, &user, "events.config.update", Some("ZDC")).await;
        let mut req = upsert("South");
        req.departure_runways = vec!["31L".into(), "04L".into()];
        req.sid_rules = Some(rules(&[("CAMRN", "04L")]));
        let created = config_repo::create(&pool, "KDCA", &req, "ZDC", &user)
            .await
            .unwrap();

        // Drop 04L, saying nothing about the rules — the shape a pre-#512 client sends. The stored
        // CAMRN→04L rule would be left pointing at a runway the config no longer has.
        let narrowing = serde_json::json!({
            "name": "South",
            "aar": 30,
            "adr": 30,
            "landing_runways": [],
            "departure_runways": ["31L"],
            "wind_from_deg": 0,
            "wind_to_deg": 360
        });
        let cookie = session_cookie(&pool, &user).await;
        let state = test_state(
            pool,
            std::collections::HashMap::from([("ZDC".to_string(), artcc(&["KDCA"]))]),
        );
        let status = send(
            &state,
            Method::PUT,
            &format!("/api/v1/airport-configs/KDCA/{}", created.id),
            &cookie,
            Some(narrowing),
        )
        .await;

        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "narrowing the runways must not silently orphan the stored CAMRN rule"
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

/// `wind_for` is the one place the observed-vs-forecast choice is made (#510 AC3), and these pin it.
///
/// Hermetic: `scope_test_support::test_state` builds an empty `metar_cache`, and `metar_for` returns a
/// fresh cached entry *before* any network call — so seeding one entry exercises the observed arm with
/// no upstream. An empty airports map leaves the forecast with nothing to answer, which is the no-data
/// arm (AC4).
#[cfg(test)]
mod wind_for_tests {
    use super::*;
    use crate::feed::metar::{MetarInfo, MetarWind};
    use crate::scope_test_support::test_state;
    use sqlx::PgPool;
    use std::collections::HashMap;

    fn seed_metar(state: &AppState, icao: &str, wind: Option<MetarWind>) {
        let info = MetarInfo {
            raw: format!("{icao} AUTO"),
            category: "VFR".to_string(),
            wind: None,
            wind_obs: wind,
        };
        state
            .metar_cache
            .lock()
            .unwrap()
            .insert(icao.to_string(), (info, Utc::now().timestamp_millis()));
    }

    fn wind(dir: Option<i32>, spd_kt: i32) -> MetarWind {
        MetarWind {
            dir,
            spd_kt,
            gust_kt: None,
        }
    }

    /// The observed arm: a cached METAR answers "now", and says so.
    #[sqlx::test]
    async fn an_observed_wind_answers_for_now(pool: PgPool) {
        let state = test_state(pool, HashMap::new());
        seed_metar(&state, "KJFK", Some(wind(Some(270), 15)));

        let w = wind_for(&state, "KJFK", Utc::now()).await;

        assert_eq!(w.source, "metar", "the observation should win for now");
        assert_eq!(w.dir, Some(270));
        assert_eq!(w.spd_kt, 15);
    }

    /// The calm rule has to be the *same* rule on both sources. Without it a 2-knot METAR keeps its
    /// direction while a 2-knot forecast does not, so the airport would match a different configuration
    /// depending on which source answered — the divergence AC3 exists to prevent.
    #[sqlx::test]
    async fn an_observed_wind_below_the_calm_threshold_has_no_direction(pool: PgPool) {
        let state = test_state(pool, HashMap::new());
        seed_metar(&state, "KJFK", Some(wind(Some(270), 2)));

        let w = wind_for(&state, "KJFK", Utc::now()).await;

        assert_eq!(w.source, "metar");
        assert_eq!(
            w.dir, None,
            "2 knots is calm, exactly as the forecast path treats it"
        );
        assert_eq!(w.spd_kt, 2, "the speed is still reported");
    }

    /// At the threshold the wind keeps its direction — the boundary is `>=`, matching the forecast.
    #[sqlx::test]
    async fn an_observed_wind_at_the_calm_threshold_keeps_its_direction(pool: PgPool) {
        let state = test_state(pool, HashMap::new());
        seed_metar(&state, "KJFK", Some(wind(Some(270), 3)));

        assert_eq!(wind_for(&state, "KJFK", Utc::now()).await.dir, Some(270));
    }

    /// Beyond the window there is nothing to observe, so the observation must not answer however fresh
    /// it is. With no airports seeded the forecast cannot answer either, which is how this distinguishes
    /// "did not prefer METAR" from "preferred METAR and got a direction".
    #[sqlx::test]
    async fn a_future_time_does_not_take_the_observation(pool: PgPool) {
        let state = test_state(pool, HashMap::new());
        seed_metar(&state, "KJFK", Some(wind(Some(270), 15)));

        // Two hours out, stated absolutely rather than as `METAR_PREFERRED_WITHIN_MIN + n`: a test
        // written in terms of the constant moves with it, so widening the window would never fail it.
        // Nobody observes the wind two hours from now, whatever the window is set to.
        let later = Utc::now() + chrono::Duration::hours(2);
        let w = wind_for(&state, "KJFK", later).await;

        assert_ne!(w.source, "metar", "beyond the window the forecast answers");
        assert_eq!(w.dir, None, "and with no forecast available, no direction");
    }

    /// Just inside the window it still does.
    #[sqlx::test]
    async fn just_inside_the_window_still_takes_the_observation(pool: PgPool) {
        let state = test_state(pool, HashMap::new());
        seed_metar(&state, "KJFK", Some(wind(Some(90), 12)));

        // Five minutes out is unambiguously "now" in any reasonable window.
        let soon = Utc::now() + chrono::Duration::minutes(5);

        assert_eq!(wind_for(&state, "KJFK", soon).await.source, "metar");
    }

    /// AC4: no wind from either source is a *defined* result, and it must say `none` rather than claim
    /// a forecast it never got — a caller tells "the wind chose this" from "we have no idea" by that
    /// string alone.
    ///
    /// Seeds a METAR carrying no wind group rather than leaving the cache empty: on a cache *miss*
    /// `metar_for` reaches `aviationweather.gov`, so an empty cache would make this test depend on the
    /// network and on KJFK's real weather. A cached entry with no wind exercises the same fall-through
    /// hermetically — observation present but silent, no airports for the forecast, so `none`.
    #[sqlx::test]
    async fn no_wind_from_either_source_reports_none(pool: PgPool) {
        let state = test_state(pool, HashMap::new());
        seed_metar(&state, "KJFK", None);

        let w = wind_for(&state, "KJFK", Utc::now()).await;

        assert_eq!(w.source, "none", "not `forecast`, which it never got");
        assert_eq!(w.dir, None);
        assert_eq!(w.spd_kt, 0);
        assert_eq!(w.gust_kt, None);
    }

    /// A variable (`VRB`) wind is an observation with no direction. It still answers — the speed is
    /// real — but gives nothing to match a configuration against, so the caller takes the calm default.
    #[sqlx::test]
    async fn a_variable_observed_wind_answers_without_a_direction(pool: PgPool) {
        let state = test_state(pool, HashMap::new());
        seed_metar(&state, "KJFK", Some(wind(None, 8)));

        let w = wind_for(&state, "KJFK", Utc::now()).await;

        assert_eq!(w.source, "metar");
        assert_eq!(w.dir, None);
    }
}
