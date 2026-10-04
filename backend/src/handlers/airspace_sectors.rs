//! ATC sector volumes for the admin sector map (#602). Served from `AppState.airspace_sectors`, the
//! cache the refresh job keeps current (#594), so a read never touches the database. Internal
//! monitoring data: gated on its own `flow.sectors.read` and drawn only on that admin page.

use axum::{Json, extract::State};

use crate::{
    auth::{permissions::FlowSectorsRead, require_permission::RequirePermission},
    models::SectorVolumeBody,
    state::AppState,
};

#[utoipa::path(
    get, path = "/api/v1/flow/airspace/sectors", tag = "flow",
    responses(
        (status = 200, body = Vec<SectorVolumeBody>),
        (status = 401, description = "Not signed in, or without `flow.sectors.read`"),
    ),
    security(("session" = ["flow.sectors.read"]), ("api_key" = ["flow.sectors.read"]), ("service_account" = ["flow.sectors.read"]))
)]
pub async fn list_sectors(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowSectorsRead>,
) -> Json<Vec<SectorVolumeBody>> {
    let table = state.airspace_sectors.load();
    Json(
        table
            .volumes
            .iter()
            .map(|v| SectorVolumeBody {
                artcc: v.artcc.clone(),
                sector_id: v.sector_id.clone(),
                volume_id: v.volume_id.clone(),
                name: v.name.clone(),
                tier: v.tier.clone(),
                base_alt_ft: v.base_alt_ft,
                top_alt_ft: v.top_alt_ft,
                rings: v.rings.clone(),
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use axum::http::{Method, StatusCode};
    use sqlx::PgPool;

    use crate::{
        feed::sectors::{SectorTable, tests::volume},
        scope_test_support::{grant, seed_user, send_json, session_cookie, test_state},
    };

    const URI: &str = "/api/v1/flow/airspace/sectors";

    /// Internal data: no session, and a signed-in user without `flow.sectors.read`, are both refused
    /// (401, as `ensure_permission` answers every missing permission), and a holder gets the cached
    /// volumes.
    #[sqlx::test]
    async fn only_a_holder_of_flow_sectors_read_sees_the_sectors(pool: PgPool) {
        let state = test_state(pool.clone(), HashMap::new());
        state.airspace_sectors.store(Arc::new(SectorTable {
            volumes: vec![volume("ZDC", "03201")],
        }));

        assert_eq!(
            send_json(&state, Method::GET, URI, "").await.0,
            StatusCode::UNAUTHORIZED
        );

        let user = seed_user(&pool).await;
        let cookie = session_cookie(&pool, &user).await;
        grant(&pool, &user, "events.plan.read", None).await;
        assert_eq!(
            send_json(&state, Method::GET, URI, &cookie).await.0,
            StatusCode::UNAUTHORIZED,
            "a planning permission is not enough"
        );

        grant(&pool, &user, "flow.sectors.read", None).await;
        let (status, body) = send_json(&state, Method::GET, URI, &cookie).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body[0]["artcc"], "ZDC");
        assert_eq!(body[0]["volume_id"], "03201");
        assert_eq!(body[0]["tier"], "low");
        assert_eq!(body[0]["top_alt_ft"], 23_000);
        assert_eq!(body[0]["rings"][0][0], serde_json::json!([38.0, -77.0]));
    }
}
