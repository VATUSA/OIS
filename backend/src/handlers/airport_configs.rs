//! Reusable per-airport runway configurations (default AAR/ADR + favored-wind rule). Reads are open
//! to planners (`events.plan.read`); writes are facility-scoped by the airport's owning ARTCC
//! (`events.config.update`), reusing the same scope infra as the per-event airport rates.

use axum::{
    Json,
    extract::{Extension, Path, Query, State},
    http::StatusCode,
};
use chrono::{TimeZone, Utc};
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
    let airports = state.feed.read().await.airports.clone();
    match feed::forecast::wind_at(&airports, &icao, at).await {
        Some(h) => Ok(Json(AirportForecastBody {
            icao,
            time: h.time,
            wind_dir: h.dir,
            wind_kt: h.spd_kt,
            gust_kt: h.gust_kt,
            source: "forecast".to_string(),
        })),
        None => Ok(Json(AirportForecastBody {
            icao,
            time: at,
            wind_dir: None,
            wind_kt: 0,
            gust_kt: None,
            source: "none".to_string(),
        })),
    }
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

    let artcc = owning_artcc(&state, &icao).await;
    let scope = principal
        .permission_scope(&state, CONFIG_PERMISSION)
        .await?;
    if !scope.allows(artcc.as_deref()) {
        return Err(ApiError::Forbidden);
    }

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

    let artcc = owning_artcc(&state, &icao).await;
    let scope = principal
        .permission_scope(&state, CONFIG_PERMISSION)
        .await?;
    if !scope.allows(artcc.as_deref()) {
        return Err(ApiError::Forbidden);
    }

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

    let artcc = owning_artcc(&state, &icao).await;
    let scope = principal
        .permission_scope(&state, CONFIG_PERMISSION)
        .await?;
    if !scope.allows(artcc.as_deref()) {
        return Err(ApiError::Forbidden);
    }

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
}
