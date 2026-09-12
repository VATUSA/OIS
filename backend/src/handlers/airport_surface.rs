//! Editable airport surface geometry (gates/parking positions, ramp/apron areas, taxiways) — the
//! foundation the #164 epic's data-driven departure-timing work keys on. Reads are open to planners
//! (`events.plan.read`); writes are facility-scoped by the airport's owning ARTCC
//! (`events.config.update`), reusing the same scope infra as `airport_configs`. The dedicated
//! `flow.surface_data.update` permission + web editor are sub-issue B (#178).

use axum::{
    Json,
    extract::{Extension, Path, State},
    http::StatusCode,
};

use crate::{
    auth::{
        context::{CurrentApiKey, CurrentUser},
        permissions::{EventsConfigUpdate, EventsPlanRead},
        principal::Principal,
        require_permission::RequirePermission,
    },
    errors::ApiError,
    handlers::events::{normalize_icao, owning_artcc},
    models::{
        AirportGateBody, AirportRampAreaBody, AirportSurfaceBody, AirportTaxiwayBody,
        UpsertAirportGateRequest, UpsertAirportRampAreaRequest, UpsertAirportTaxiwayRequest,
    },
    repos::airport_surface as surface_repo,
    state::AppState,
};

const CONFIG_PERMISSION: &str = "events.config.update";

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
    if req.rings.is_empty() || req.rings.iter().any(|r| r.len() < 3) {
        return Err(ApiError::BadRequest);
    }
    Ok(())
}

fn validate_taxiway(req: &UpsertAirportTaxiwayRequest) -> Result<(), ApiError> {
    if req.points.len() < 2 {
        return Err(ApiError::BadRequest);
    }
    Ok(())
}

/// Does the caller hold `events.config.update` nationally or for `icao`'s owning ARTCC?
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
    _permission: RequirePermission<EventsConfigUpdate>,
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
    _permission: RequirePermission<EventsConfigUpdate>,
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
    _permission: RequirePermission<EventsConfigUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path((icao, id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let icao = normalize_icao(&icao).ok_or(ApiError::BadRequest)?;
    require_edit(&state, &principal, &icao).await?;

    if surface_repo::delete_gate(pool, &id, &icao).await? {
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
    _permission: RequirePermission<EventsConfigUpdate>,
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
    _permission: RequirePermission<EventsConfigUpdate>,
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
    _permission: RequirePermission<EventsConfigUpdate>,
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
    _permission: RequirePermission<EventsConfigUpdate>,
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
    _permission: RequirePermission<EventsConfigUpdate>,
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
    _permission: RequirePermission<EventsConfigUpdate>,
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

    #[sqlx::test]
    async fn taxiway_crud_round_trip(pool: PgPool) {
        let user = seed_user(&pool).await;
        let req = UpsertAirportTaxiwayRequest {
            name: "A".to_string(),
            points: vec![[38.85, -77.04], [38.86, -77.05]],
        };
        let created = surface_repo::create_taxiway(&pool, "KTST", &req, &user)
            .await
            .unwrap();
        assert_eq!(created.points.0.len(), 2);

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

        let taxiways = surface_repo::list_taxiways(&pool, "KDCA").await.unwrap();
        assert_eq!(taxiways.len(), 84);
    }
}
