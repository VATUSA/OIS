//! Editable airport surface geometry (gates/parking positions, ramp/apron areas, taxiways) — the
//! foundation the #164 epic's data-driven departure-timing work keys on. Reads are open to planners
//! (`events.plan.read`); writes are facility-scoped by the airport's owning ARTCC
//! (`flow.surface_data.update`), reusing the same scope infra as `airport_configs`.

use axum::{
    Json,
    extract::{Extension, Path, State},
    http::StatusCode,
};

use crate::{
    auth::{
        context::{CurrentApiKey, CurrentUser},
        permissions::{EventsPlanRead, FlowSurfaceDataUpdate},
        principal::Principal,
        require_permission::RequirePermission,
    },
    errors::ApiError,
    handlers::events::{normalize_icao, owning_artcc},
    models::{
        AirportGateBody, AirportRampAreaBody, AirportSurfaceBody, AirportTaxiwayBody,
        FaaRepullResult, UpsertAirportGateRequest, UpsertAirportRampAreaRequest,
        UpsertAirportTaxiwayRequest,
    },
    repos::airport_surface as surface_repo,
    repos::faa_surface_seed as faa_surface_seed_repo,
    state::AppState,
};

const CONFIG_PERMISSION: &str = "flow.surface_data.update";

fn validate_gate(req: &UpsertAirportGateRequest) -> Result<(), ApiError> {
    if req.name.trim().is_empty() || req.name.len() > 64 {
        return Err(ApiError::BadRequest);
    }
    if !(-90.0..=90.0).contains(&req.lat) || !(-180.0..=180.0).contains(&req.lon) {
        return Err(ApiError::BadRequest);
    }
    Ok(())
}

fn validate_ramp_area(req: &UpsertAirportRampAreaRequest) -> Result<(), ApiError> {
    if req.name.trim().is_empty() || req.name.len() > 64 {
        return Err(ApiError::BadRequest);
    }
    if !matches!(req.kind.as_str(), "ramp" | "apron") {
        return Err(ApiError::BadRequest);
    }
    if !valid_rings(&req.rings) {
        return Err(ApiError::BadRequest);
    }
    Ok(())
}

fn validate_taxiway(req: &UpsertAirportTaxiwayRequest) -> Result<(), ApiError> {
    if req.name.trim().is_empty() || req.name.len() > 64 {
        return Err(ApiError::BadRequest);
    }
    if !valid_rings(&req.rings) {
        return Err(ApiError::BadRequest);
    }
    Ok(())
}

/// A polygon (ramp area or taxiway pavement): at least one ring, each with at least 3 points, every
/// one of them a real `[lat, lon]`. An out-of-range point survives to the editor's `fitBounds` and
/// parks that airport's map on a garbage viewport for everyone.
fn valid_rings(rings: &[Vec<[f64; 2]>]) -> bool {
    !rings.is_empty()
        && rings.iter().all(|r| {
            r.len() >= 3
                && r.iter().all(|&[lat, lon]| {
                    (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)
                })
        })
}

/// Does the caller hold `flow.surface_data.update` nationally or for `icao`'s owning ARTCC?
async fn can_edit(state: &AppState, principal: &Principal, icao: &str) -> Result<bool, ApiError> {
    let artcc = owning_artcc(state, icao).await;
    let scope = principal.permission_scope(state, CONFIG_PERMISSION).await?;
    Ok(scope.allows(artcc.as_deref()))
}

/// Fail-closed if the caller can't edit `icao`.
async fn require_edit(state: &AppState, principal: &Principal, icao: &str) -> Result<(), ApiError> {
    if can_edit(state, principal, icao).await? {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

/// Reload the DB gate catalog into `AppState::gates` so a write applies to `feed::taxi_observations`
/// gate matching at once, instead of waiting for `jobs::spawn_airport_gates_refresh`'s next poll.
async fn refresh_gates_cache(state: &AppState, pool: &sqlx::PgPool) -> Result<(), ApiError> {
    let by_icao = surface_repo::load_all_gates(pool).await?;
    state.gates.store(std::sync::Arc::new(by_icao));
    Ok(())
}

#[utoipa::path(
    get, path = "/api/v1/airports/{icao}/surface", tag = "events",
    params(("icao" = String, Path)),
    responses((status = 200, body = AirportSurfaceBody), (status = 401))
)]
pub async fn get_airport_surface(
    State(state): State<AppState>,
    _permission: RequirePermission<EventsPlanRead>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path(icao): Path<String>,
) -> Result<Json<AirportSurfaceBody>, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let icao = normalize_icao(&icao).ok_or(ApiError::BadRequest)?;

    let editable = can_edit(&state, &principal, &icao).await?;
    let mut gates = surface_repo::list_gates(pool, &icao).await?;
    let mut ramp_areas = surface_repo::list_ramp_areas(pool, &icao).await?;
    let mut taxiways = surface_repo::list_taxiways(pool, &icao).await?;
    for g in &mut gates {
        g.editable = editable;
    }
    for r in &mut ramp_areas {
        r.editable = editable;
    }
    for t in &mut taxiways {
        t.editable = editable;
    }
    Ok(Json(AirportSurfaceBody {
        gates,
        ramp_areas,
        taxiways,
    }))
}

// ---- gates ------------------------------------------------------------------

#[utoipa::path(
    post, path = "/api/v1/airports/{icao}/gates", tag = "events",
    params(("icao" = String, Path)), request_body = UpsertAirportGateRequest,
    responses((status = 200, body = AirportGateBody), (status = 400), (status = 401), (status = 403))
)]
pub async fn create_airport_gate(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowSurfaceDataUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path(icao): Path<String>,
    Json(req): Json<UpsertAirportGateRequest>,
) -> Result<Json<AirportGateBody>, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let icao = normalize_icao(&icao).ok_or(ApiError::BadRequest)?;
    validate_gate(&req)?;
    require_edit(&state, &principal, &icao).await?;

    let mut row = surface_repo::create_gate(pool, &icao, &req, principal.user_id()).await?;
    refresh_gates_cache(&state, pool).await?;
    row.editable = true;
    Ok(Json(row))
}

#[utoipa::path(
    put, path = "/api/v1/airports/{icao}/gates/{id}", tag = "events",
    params(("icao" = String, Path), ("id" = String, Path)), request_body = UpsertAirportGateRequest,
    responses((status = 200, body = AirportGateBody), (status = 400), (status = 401), (status = 403), (status = 404))
)]
pub async fn update_airport_gate(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowSurfaceDataUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path((icao, id)): Path<(String, String)>,
    Json(req): Json<UpsertAirportGateRequest>,
) -> Result<Json<AirportGateBody>, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let icao = normalize_icao(&icao).ok_or(ApiError::BadRequest)?;
    validate_gate(&req)?;
    require_edit(&state, &principal, &icao).await?;

    let mut row = surface_repo::update_gate(pool, &id, &icao, &req, principal.user_id())
        .await?
        .ok_or(ApiError::NotFound)?;
    refresh_gates_cache(&state, pool).await?;
    row.editable = true;
    Ok(Json(row))
}

#[utoipa::path(
    delete, path = "/api/v1/airports/{icao}/gates/{id}", tag = "events",
    params(("icao" = String, Path), ("id" = String, Path)),
    responses((status = 204), (status = 401), (status = 403), (status = 404))
)]
pub async fn delete_airport_gate(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowSurfaceDataUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path((icao, id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let icao = normalize_icao(&icao).ok_or(ApiError::BadRequest)?;
    require_edit(&state, &principal, &icao).await?;

    if surface_repo::delete_gate(pool, &id, &icao).await? {
        refresh_gates_cache(&state, pool).await?;
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

// ---- ramp / apron areas -------------------------------------------------------

#[utoipa::path(
    post, path = "/api/v1/airports/{icao}/ramp-areas", tag = "events",
    params(("icao" = String, Path)), request_body = UpsertAirportRampAreaRequest,
    responses((status = 200, body = AirportRampAreaBody), (status = 400), (status = 401), (status = 403))
)]
pub async fn create_airport_ramp_area(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowSurfaceDataUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path(icao): Path<String>,
    Json(req): Json<UpsertAirportRampAreaRequest>,
) -> Result<Json<AirportRampAreaBody>, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let icao = normalize_icao(&icao).ok_or(ApiError::BadRequest)?;
    validate_ramp_area(&req)?;
    require_edit(&state, &principal, &icao).await?;

    let mut row = surface_repo::create_ramp_area(pool, &icao, &req, principal.user_id()).await?;
    row.editable = true;
    Ok(Json(row))
}

#[utoipa::path(
    put, path = "/api/v1/airports/{icao}/ramp-areas/{id}", tag = "events",
    params(("icao" = String, Path), ("id" = String, Path)), request_body = UpsertAirportRampAreaRequest,
    responses((status = 200, body = AirportRampAreaBody), (status = 400), (status = 401), (status = 403), (status = 404))
)]
pub async fn update_airport_ramp_area(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowSurfaceDataUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path((icao, id)): Path<(String, String)>,
    Json(req): Json<UpsertAirportRampAreaRequest>,
) -> Result<Json<AirportRampAreaBody>, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let icao = normalize_icao(&icao).ok_or(ApiError::BadRequest)?;
    validate_ramp_area(&req)?;
    require_edit(&state, &principal, &icao).await?;

    let mut row = surface_repo::update_ramp_area(pool, &id, &icao, &req, principal.user_id())
        .await?
        .ok_or(ApiError::NotFound)?;
    row.editable = true;
    Ok(Json(row))
}

#[utoipa::path(
    delete, path = "/api/v1/airports/{icao}/ramp-areas/{id}", tag = "events",
    params(("icao" = String, Path), ("id" = String, Path)),
    responses((status = 204), (status = 401), (status = 403), (status = 404))
)]
pub async fn delete_airport_ramp_area(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowSurfaceDataUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path((icao, id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let icao = normalize_icao(&icao).ok_or(ApiError::BadRequest)?;
    require_edit(&state, &principal, &icao).await?;

    if surface_repo::delete_ramp_area(pool, &id, &icao).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

// ---- taxiways -----------------------------------------------------------------

#[utoipa::path(
    post, path = "/api/v1/airports/{icao}/taxiways", tag = "events",
    params(("icao" = String, Path)), request_body = UpsertAirportTaxiwayRequest,
    responses((status = 200, body = AirportTaxiwayBody), (status = 400), (status = 401), (status = 403))
)]
pub async fn create_airport_taxiway(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowSurfaceDataUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path(icao): Path<String>,
    Json(req): Json<UpsertAirportTaxiwayRequest>,
) -> Result<Json<AirportTaxiwayBody>, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let icao = normalize_icao(&icao).ok_or(ApiError::BadRequest)?;
    validate_taxiway(&req)?;
    require_edit(&state, &principal, &icao).await?;

    let mut row = surface_repo::create_taxiway(pool, &icao, &req, principal.user_id()).await?;
    row.editable = true;
    Ok(Json(row))
}

#[utoipa::path(
    put, path = "/api/v1/airports/{icao}/taxiways/{id}", tag = "events",
    params(("icao" = String, Path), ("id" = String, Path)), request_body = UpsertAirportTaxiwayRequest,
    responses((status = 200, body = AirportTaxiwayBody), (status = 400), (status = 401), (status = 403), (status = 404))
)]
pub async fn update_airport_taxiway(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowSurfaceDataUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path((icao, id)): Path<(String, String)>,
    Json(req): Json<UpsertAirportTaxiwayRequest>,
) -> Result<Json<AirportTaxiwayBody>, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let icao = normalize_icao(&icao).ok_or(ApiError::BadRequest)?;
    validate_taxiway(&req)?;
    require_edit(&state, &principal, &icao).await?;

    let mut row = surface_repo::update_taxiway(pool, &id, &icao, &req, principal.user_id())
        .await?
        .ok_or(ApiError::NotFound)?;
    row.editable = true;
    Ok(Json(row))
}

#[utoipa::path(
    delete, path = "/api/v1/airports/{icao}/taxiways/{id}", tag = "events",
    params(("icao" = String, Path), ("id" = String, Path)),
    responses((status = 204), (status = 401), (status = 403), (status = 404))
)]
pub async fn delete_airport_taxiway(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowSurfaceDataUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path((icao, id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let icao = normalize_icao(&icao).ok_or(ApiError::BadRequest)?;
    require_edit(&state, &principal, &icao).await?;

    if surface_repo::delete_taxiway(pool, &id, &icao).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

// ---- FAA re-pull (#232) ------------------------------------------------------

#[utoipa::path(
    post, path = "/api/v1/airports/{icao}/surface/repull-faa", tag = "events",
    params(("icao" = String, Path)),
    responses(
        (status = 200, body = FaaRepullResult), (status = 401), (status = 403),
        (status = 404, description = "The bundled FAA extract has no data for this airport")
    )
)]
pub async fn repull_faa_surface(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowSurfaceDataUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path(icao): Path<String>,
) -> Result<Json<FaaRepullResult>, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let icao = normalize_icao(&icao).ok_or(ApiError::BadRequest)?;
    require_edit(&state, &principal, &icao).await?;

    let summary = faa_surface_seed_repo::seed_for_icao(pool, &icao).await?;
    Ok(Json(FaaRepullResult {
        taxiways_inserted: summary.taxiways_inserted,
        ramps_inserted: summary.ramps_inserted,
        osm_taxiways_retired: summary.osm_taxiways_retired,
        osm_ramps_retired: summary.osm_ramps_retired,
    }))
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    use super::*;

    async fn seed_user(pool: &PgPool) -> String {
        sqlx::query_scalar::<_, String>(
            "insert into identity.users (full_name, display_name) \
             values ('Test User', 'Test User') returning id",
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[sqlx::test]
    async fn gate_crud_round_trip(pool: PgPool) {
        let user = seed_user(&pool).await;
        let req = UpsertAirportGateRequest {
            name: "A1".to_string(),
            lat: 38.85,
            lon: -77.04,
        };
        let created = surface_repo::create_gate(&pool, "KTST", &req, &user)
            .await
            .unwrap();
        assert_eq!(created.icao, "KTST");
        assert_eq!(created.source, "manual");

        let listed = surface_repo::list_gates(&pool, "KTST").await.unwrap();
        assert_eq!(listed.len(), 1);

        let update_req = UpsertAirportGateRequest {
            name: "A2".to_string(),
            lat: 38.86,
            lon: -77.05,
        };
        let updated = surface_repo::update_gate(&pool, &created.id, "KTST", &update_req, &user)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.name, "A2");

        // Wrong icao scoping: no row affected.
        let wrong_scope = surface_repo::update_gate(&pool, &created.id, "KJFK", &update_req, &user)
            .await
            .unwrap();
        assert!(wrong_scope.is_none());

        assert!(
            !surface_repo::delete_gate(&pool, &created.id, "KJFK")
                .await
                .unwrap()
        );
        assert!(
            surface_repo::delete_gate(&pool, &created.id, "KTST")
                .await
                .unwrap()
        );
        assert!(
            surface_repo::list_gates(&pool, "KTST")
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[sqlx::test]
    async fn ramp_area_crud_round_trip(pool: PgPool) {
        let user = seed_user(&pool).await;
        let req = UpsertAirportRampAreaRequest {
            name: "North Apron".to_string(),
            kind: "apron".to_string(),
            rings: vec![vec![
                [38.85, -77.04],
                [38.86, -77.04],
                [38.86, -77.05],
                [38.85, -77.04],
            ]],
        };
        let created = surface_repo::create_ramp_area(&pool, "KTST", &req, &user)
            .await
            .unwrap();
        assert_eq!(created.kind, "apron");
        assert_eq!(created.rings.0.len(), 1);

        let fetched = surface_repo::get_ramp_area(&pool, &created.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.rings.0[0].len(), 4);

        assert!(
            surface_repo::delete_ramp_area(&pool, &created.id, "KTST")
                .await
                .unwrap()
        );
    }

    #[test]
    fn a_taxiway_needs_a_name_like_every_other_surface_shape() {
        let named = |name: &str| UpsertAirportTaxiwayRequest {
            name: name.to_string(),
            rings: vec![vec![[38.85, -77.04], [38.86, -77.05], [38.85, -77.05]]],
        };
        assert!(validate_taxiway(&named("")).is_err());
        assert!(validate_taxiway(&named("   ")).is_err());
        assert!(validate_taxiway(&named(&"A".repeat(65))).is_err());
        assert!(validate_taxiway(&named("A")).is_ok());
    }

    #[test]
    fn a_ring_point_outside_the_world_is_rejected() {
        let rings = |p: [f64; 2]| vec![vec![p, [38.86, -77.05], [38.85, -77.05]]];
        for bad in [
            [900.0, -77.04],
            [-91.0, -77.04],
            [38.85, 181.0],
            [38.85, -180.1],
        ] {
            assert!(
                validate_taxiway(&UpsertAirportTaxiwayRequest {
                    name: "A".to_string(),
                    rings: rings(bad),
                })
                .is_err(),
                "taxiway accepted {bad:?}"
            );
            assert!(
                validate_ramp_area(&UpsertAirportRampAreaRequest {
                    name: "North apron".to_string(),
                    kind: "apron".to_string(),
                    rings: rings(bad),
                })
                .is_err(),
                "ramp area accepted {bad:?}"
            );
        }
    }

    #[test]
    fn a_taxiway_must_be_a_polygon_not_a_centerline() {
        let taxiway = |rings: Vec<Vec<[f64; 2]>>| UpsertAirportTaxiwayRequest {
            name: "A".to_string(),
            rings,
        };
        // An open two-point line (the pre-#278 centerline shape) is rejected.
        assert!(validate_taxiway(&taxiway(vec![vec![[38.85, -77.04], [38.86, -77.05]]])).is_err());
        assert!(validate_taxiway(&taxiway(vec![])).is_err());
        assert!(
            validate_taxiway(&taxiway(vec![vec![
                [38.85, -77.04],
                [38.86, -77.05],
                [38.85, -77.05]
            ]]))
            .is_ok()
        );
    }

    #[sqlx::test]
    async fn taxiway_crud_round_trip(pool: PgPool) {
        let user = seed_user(&pool).await;
        let req = UpsertAirportTaxiwayRequest {
            name: "A".to_string(),
            rings: vec![vec![
                [38.85, -77.04],
                [38.86, -77.05],
                [38.85, -77.05],
                [38.85, -77.04],
            ]],
        };
        assert!(validate_taxiway(&req).is_ok());
        let created = surface_repo::create_taxiway(&pool, "KTST", &req, &user)
            .await
            .unwrap();
        assert_eq!(created.rings.0, req.rings);

        let listed = surface_repo::list_taxiways(&pool, "KTST").await.unwrap();
        assert_eq!(listed.len(), 1);

        assert!(
            surface_repo::delete_taxiway(&pool, &created.id, "KTST")
                .await
                .unwrap()
        );
    }

    /// The KDCA OSM seed embedded in migration 0067 — verifies row counts, not exact contents,
    /// so this stays stable if OSM tags shift slightly but breaks loudly if the seed itself
    /// silently regresses in a future migration edit.
    #[sqlx::test]
    async fn kdca_seed_data_has_expected_row_counts(pool: PgPool) {
        let gates = surface_repo::list_gates(&pool, "KDCA").await.unwrap();
        assert_eq!(gates.len(), 57);
        assert!(gates.iter().all(|g| g.source == "osm"));

        let ramp_areas = surface_repo::list_ramp_areas(&pool, "KDCA").await.unwrap();
        assert_eq!(ramp_areas.len(), 4);
        assert!(ramp_areas.iter().all(|r| r.kind == "apron"));

        // Migration 0067 seeded 84 OSM taxiway centerlines; 0076 deleted them when taxiways became
        // pavement polygons (#278), since a centerline can't become one.
        let taxiways = surface_repo::list_taxiways(&pool, "KDCA").await.unwrap();
        assert!(taxiways.is_empty());
    }

    /// Migration 0068's backfill runs once, at migration time, against whatever
    /// `events.config.update` grants already exist then — it can't see grants seeded by a test
    /// afterwards. This re-runs the same statements the migration uses directly, to prove the
    /// query logic itself (matching on the old permission, preserving `granted`/`artcc_id`,
    /// idempotent via `on conflict`) is correct, independent of migration-ordering concerns.
    #[sqlx::test]
    async fn permission_backfill_repoints_existing_events_config_update_grants(pool: PgPool) {
        sqlx::query(
            "insert into access.role_permissions (role_name, permission_name) \
             values ('EC', 'events.config.update') on conflict do nothing",
        )
        .execute(&pool)
        .await
        .unwrap();
        let user = seed_user(&pool).await;
        sqlx::query(
            "insert into access.user_permissions (user_id, permission_name, granted, artcc_id) \
             values ($1, 'events.config.update', false, 'ZDC')",
        )
        .bind(&user)
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "insert into access.role_permissions (role_name, permission_name) \
             select role_name, 'flow.surface_data.update' from access.role_permissions \
             where permission_name = 'events.config.update' \
             on conflict (role_name, permission_name) do nothing",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "insert into access.user_permissions (user_id, permission_name, granted, artcc_id) \
             select user_id, 'flow.surface_data.update', granted, artcc_id \
             from access.user_permissions where permission_name = 'events.config.update' \
             on conflict (user_id, permission_name, (coalesce(artcc_id, ''))) do nothing",
        )
        .execute(&pool)
        .await
        .unwrap();

        let role_has_it: bool = sqlx::query_scalar(
            "select exists(select 1 from access.role_permissions \
             where role_name = 'EC' and permission_name = 'flow.surface_data.update')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            role_has_it,
            "EC's events.config.update grant must carry over"
        );

        // The user's original grant was a facility-scoped *denial* (granted = false, artcc_id =
        // ZDC) — the backfill must preserve both fields exactly, not just blanket-grant the new
        // permission.
        let (granted, artcc_id): (bool, Option<String>) = sqlx::query_as(
            "select granted, artcc_id from access.user_permissions \
             where user_id = $1 and permission_name = 'flow.surface_data.update'",
        )
        .bind(&user)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            !granted,
            "a denial must carry over as a denial, not flip to a grant"
        );
        assert_eq!(artcc_id.as_deref(), Some("ZDC"));
    }

    // --- ARTCC-scope authorization boundary (#198) ---
    //
    // Every test above drives `surface_repo` directly, never `can_edit`/`require_edit` — so a
    // `require_edit` stubbed to `Ok(())` (ARTCC-scope enforcement silently disabled) would not
    // fail any of them. These tests close that gap.
    //
    // Grants `flow.surface_data.update` (this file's `CONFIG_PERMISSION`, not the earlier
    // `events.config.update` placeholder these tests were originally written against — updated
    // while resolving a rebase conflict against #178's already-merged permission rename).

    use crate::scope_test_support;
    use crate::scope_test_support::{artcc, grant, principal_for, test_state};
    use std::collections::HashMap;

    #[sqlx::test]
    async fn national_scope_can_edit_any_artccs_airport(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        grant(&pool, &user, "flow.surface_data.update", None).await;
        let principal = principal_for(&user);
        let state = test_state(pool, HashMap::from([("ZDC".to_string(), artcc(&["KDCA"]))]));
        assert!(can_edit(&state, &principal, "KDCA").await.unwrap());
        assert!(require_edit(&state, &principal, "KDCA").await.is_ok());
    }

    #[sqlx::test]
    async fn matching_artcc_scope_can_edit_its_own_airport(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        grant(&pool, &user, "flow.surface_data.update", Some("ZDC")).await;
        let principal = principal_for(&user);
        let state = test_state(pool, HashMap::from([("ZDC".to_string(), artcc(&["KDCA"]))]));
        assert!(can_edit(&state, &principal, "KDCA").await.unwrap());
    }

    #[sqlx::test]
    async fn wrong_artcc_scope_is_rejected(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        // Granted for ZAU, but KDCA is owned by ZDC.
        grant(&pool, &user, "flow.surface_data.update", Some("ZAU")).await;
        let principal = principal_for(&user);
        let state = test_state(pool, HashMap::from([("ZDC".to_string(), artcc(&["KDCA"]))]));
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
        let state = test_state(pool, HashMap::from([("ZDC".to_string(), artcc(&["KDCA"]))]));
        assert!(!can_edit(&state, &principal, "KDCA").await.unwrap());
        assert!(matches!(
            require_edit(&state, &principal, "KDCA").await,
            Err(ApiError::Forbidden)
        ));
    }
}
