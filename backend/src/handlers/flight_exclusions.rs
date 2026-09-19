//! Manually excluding a bogus flight from the flow picture (#342).
//!
//! A controller working an FCA drops one VATSIM callsign whose data is garbage — a teleporting
//! position, a mis-parsed route, a stuck ground squawk, a duplicate. The exclusion is stored against
//! the FCA's ARTCC (so a removal has an owner) and removes the flight from the map, the FCA crossing
//! lists, metering, counts and AADC demand for **every** viewer, because a bogus flight distorting a
//! metered flow is a shared problem rather than one each controller swats individually.
//!
//! Gated on the FCA page's existing `flow.fca.update` — no new permission. The ARTCC is always taken
//! from the FCA being worked, never from the client.

use axum::{
    Json,
    extract::{Extension, Path, State},
};

use crate::{
    auth::{
        context::{CurrentApiKey, CurrentUser},
        permissions::{FlowFcaRead, FlowFcaUpdate},
        principal::Principal,
        require_permission::RequirePermission,
    },
    errors::ApiError,
    models::{ExcludeFlightRequest, FlightExclusionBody},
    repos::{flight_exclusions as exclusions_repo, flow as flow_repo},
    state::AppState,
};

/// TTL backstop for a manual exclusion (#342). The primary auto-clear is the callsign leaving the
/// VATSIM feed (`jobs::spawn_flight_exclusions_refresh`); this only catches a flight that never
/// cleanly departs it — a stuck ground squawk being the motivating case. Comfortably longer than a
/// typical leg, so a genuinely bogus flight stays hidden while it is polluting the picture, but
/// short enough that stale rows cannot accumulate.
const EXCLUSION_TTL_HOURS: i64 = 2;

/// The permission a manual exclusion is scoped against — the FCA page's existing write gate.
const EXCLUSION_PERMISSION: &str = "flow.fca.update";

/// Reload the exclusion cache so a write applies to the flow surfaces at once, instead of waiting
/// for `jobs::spawn_flight_exclusions_refresh`'s next poll.
async fn refresh_exclusions_cache(state: &AppState, pool: &sqlx::PgPool) -> Result<(), ApiError> {
    let by_artcc = exclusions_repo::load_all(pool).await?;
    state.flight_exclusions.store(std::sync::Arc::new(by_artcc));
    Ok(())
}

/// The ARTCC that owns an FCA — the scope a manual exclusion is recorded under.
async fn fca_artcc(pool: &sqlx::PgPool, id: &str) -> Result<String, ApiError> {
    let fca = flow_repo::get_fca(pool, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(fca.artcc)
}

/// Fail closed unless the caller holds `flow.fca.update` nationally or for `artcc`.
///
/// `RequirePermission<FlowFcaUpdate>` only answers *whether* the caller holds the permission, not
/// *where* — so on its own a controller scoped to one facility could exclude a flight through
/// another facility's FCA. That matters more here than on the other FCA writes: the global surfaces
/// (`handlers::feed`, `handlers::gdp`, `traffic_from`) match on callsign alone via
/// `all_excluded_callsigns`, so one facility's removal hides the aircraft for **everyone**
/// nationally. Mirrors `handlers::airport_configs::require_edit` (#342).
async fn require_artcc_scope(
    state: &AppState,
    principal: &Principal,
    artcc: &str,
) -> Result<(), ApiError> {
    let scope = principal
        .permission_scope(state, EXCLUSION_PERMISSION)
        .await?;
    if scope.allows(Some(artcc)) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

#[utoipa::path(
    get, path = "/api/v1/flow/fcas/{id}/exclusions", tag = "flow",
    params(("id" = String, Path)),
    responses((status = 200, body = Vec<FlightExclusionBody>), (status = 401), (status = 404))
)]
pub async fn list_flight_exclusions(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaRead>,
    Path(id): Path<String>,
) -> Result<Json<Vec<FlightExclusionBody>>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let artcc = fca_artcc(pool, &id).await?;
    Ok(Json(exclusions_repo::list_by_artcc(pool, &artcc).await?))
}

#[utoipa::path(
    post, path = "/api/v1/flow/fcas/{id}/exclusions/{callsign}", tag = "flow",
    params(("id" = String, Path), ("callsign" = String, Path)),
    request_body = ExcludeFlightRequest,
    responses((status = 200, body = FlightExclusionBody), (status = 401), (status = 403), (status = 404))
)]
pub async fn exclude_flight(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path((id, callsign)): Path<(String, String)>,
    Json(req): Json<ExcludeFlightRequest>,
) -> Result<Json<FlightExclusionBody>, ApiError> {
    let user = current_user.as_ref().ok_or(ApiError::Unauthorized)?;
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let callsign = callsign.trim().to_ascii_uppercase();
    if callsign.is_empty() {
        return Err(ApiError::BadRequest);
    }
    let artcc = fca_artcc(pool, &id).await?;
    require_artcc_scope(&state, &principal, &artcc).await?;
    let row = exclusions_repo::upsert(
        pool,
        &artcc,
        &callsign,
        req.reason.trim(),
        EXCLUSION_TTL_HOURS,
        &user.id,
    )
    .await?;
    refresh_exclusions_cache(&state, pool).await?;
    state.publish(crate::realtime::topic::FCA);
    Ok(Json(row))
}

#[utoipa::path(
    delete, path = "/api/v1/flow/fcas/{id}/exclusions/{callsign}", tag = "flow",
    params(("id" = String, Path), ("callsign" = String, Path)),
    responses((status = 204), (status = 401), (status = 403), (status = 404))
)]
pub async fn restore_flight(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowFcaUpdate>,
    Extension(current_user): Extension<Option<CurrentUser>>,
    Extension(current_api_key): Extension<Option<CurrentApiKey>>,
    Path((id, callsign)): Path<(String, String)>,
) -> Result<axum::http::StatusCode, ApiError> {
    let principal = Principal::require(current_user.as_ref(), current_api_key.as_ref())?;
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let callsign = callsign.trim().to_ascii_uppercase();
    let artcc = fca_artcc(pool, &id).await?;
    require_artcc_scope(&state, &principal, &artcc).await?;
    if !exclusions_repo::delete(pool, &artcc, &callsign).await? {
        return Err(ApiError::NotFound);
    }
    refresh_exclusions_cache(&state, pool).await?;
    state.publish(crate::realtime::topic::FCA);
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Scope tests (#342). A manual exclusion hides an aircraft for **every** viewer nationally, so
/// holding `flow.fca.update` somewhere must not let a controller remove a flight through another
/// facility's FCA. Mirrors `handlers::airport_configs`'s scope tests.
#[cfg(test)]
mod scope_tests {
    use sqlx::PgPool;

    use super::require_artcc_scope;
    use crate::scope_test_support::{self, artcc, grant, principal_for, test_state};

    fn state_with_zdc(pool: PgPool) -> crate::state::AppState {
        test_state(
            pool,
            std::collections::HashMap::from([("ZDC".to_string(), artcc(&["KDCA"]))]),
        )
    }

    #[sqlx::test]
    async fn a_national_grant_can_exclude_for_any_facility(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        grant(&pool, &user, "flow.fca.update", None).await;
        let principal = principal_for(&user);
        let state = state_with_zdc(pool);
        assert!(require_artcc_scope(&state, &principal, "ZDC").await.is_ok());
        assert!(require_artcc_scope(&state, &principal, "ZNY").await.is_ok());
    }

    #[sqlx::test]
    async fn a_facility_grant_can_exclude_for_its_own_artcc(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        grant(&pool, &user, "flow.fca.update", Some("ZDC")).await;
        let principal = principal_for(&user);
        let state = state_with_zdc(pool);
        assert!(require_artcc_scope(&state, &principal, "ZDC").await.is_ok());
    }

    /// The regression this guards: `RequirePermission<FlowFcaUpdate>` alone answers "holds it", not
    /// "holds it here", so without the scope check a ZDC controller could hide an aircraft from
    /// ZNY's picture — and from everyone else's, since the global surfaces match on callsign alone.
    #[sqlx::test]
    async fn a_facility_grant_cannot_exclude_through_another_facilitys_fca(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        grant(&pool, &user, "flow.fca.update", Some("ZDC")).await;
        let principal = principal_for(&user);
        let state = state_with_zdc(pool);
        let err = require_artcc_scope(&state, &principal, "ZNY")
            .await
            .expect_err("a ZDC-scoped grant must not reach ZNY's FCA");
        assert!(
            matches!(err, crate::errors::ApiError::Forbidden),
            "expected 403 Forbidden, got {err:?}"
        );
    }

    #[sqlx::test]
    async fn no_grant_at_all_is_rejected(pool: PgPool) {
        let user = scope_test_support::seed_user(&pool).await;
        let principal = principal_for(&user);
        let state = state_with_zdc(pool);
        assert!(
            require_artcc_scope(&state, &principal, "ZDC")
                .await
                .is_err()
        );
    }
}
