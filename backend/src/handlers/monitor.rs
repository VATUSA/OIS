//! Airspace Monitor alert parameters (#598, epic #593): the per-sector limit a sector's count is
//! coloured against, shared by everyone watching the ARTCC.
//!
//! Reads are gated `flow.monitor.read`. Writes are gated `flow.monitor.update` **and** scoped to the
//! sector's ARTCC — the typed extractor says only *whether* the caller holds it, not *where*, so a TMU
//! at one facility could otherwise set another's limits. A write force-reloads `AppState::sector_maps`
//! so the next Monitor cycle recolours without waiting for `jobs::spawn_sector_maps_refresh`.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

use crate::{
    auth::{
        permissions::{FlowMonitorRead, FlowMonitorUpdate},
        principal::{Actor, Principal},
        require_permission::RequirePermission,
    },
    errors::ApiError,
    feed::sectors::{DEFAULT_MAP, map_for},
    models::{SectorMapBody, SectorMapsBody, SetSectorMapRequest},
    repos::sector_maps as repo,
    state::AppState,
};

/// The permission a MAP write is scoped against.
const MONITOR_UPDATE: &str = "flow.monitor.update";

/// Whether `principal` may set `artcc`'s MAPs — `flow.monitor.update` nationally or for that ARTCC.
async fn may_edit(state: &AppState, principal: &Principal, artcc: &str) -> Result<bool, ApiError> {
    Ok(principal
        .permission_scope(state, MONITOR_UPDATE)
        .await?
        .allows(Some(artcc)))
}

#[utoipa::path(
    get, path = "/api/v1/flow/monitor/{artcc}/maps", tag = "flow",
    params(("artcc" = String, Path)),
    responses((status = 200, body = SectorMapsBody), (status = 401))
)]
pub async fn list_sector_maps(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowMonitorRead>,
    Actor(principal): Actor,
    Path(artcc): Path<String>,
) -> Result<Json<SectorMapsBody>, ApiError> {
    let artcc = artcc.trim().to_ascii_uppercase();
    let maps = state.sector_maps.load();
    let sectors = state
        .airspace_sectors
        .load()
        .sectors_of(&artcc)
        .into_iter()
        .map(|(sector_id, name)| SectorMapBody {
            overridden: maps.contains_key(&(artcc.clone(), sector_id.clone())),
            map: map_for(&maps, &artcc, &sector_id),
            sector_id,
            name,
        })
        .collect();
    Ok(Json(SectorMapsBody {
        editable: may_edit(&state, &principal, &artcc).await?,
        default_map: DEFAULT_MAP,
        sectors,
    }))
}

/// Set one sector's MAP. Only a positive whole number that differs from the current value is
/// written: zero or negative is refused (400) and leaves any override in place, and the current value
/// is a no-op (204, nothing written). There is no delete — typing the default is the reset.
#[utoipa::path(
    put, path = "/api/v1/flow/monitor/{artcc}/maps/{sector_id}", tag = "flow",
    params(("artcc" = String, Path), ("sector_id" = String, Path)),
    request_body = SetSectorMapRequest,
    responses(
        (status = 204),
        (status = 400, description = "`map` is not a positive whole number"),
        (status = 401),
        (status = 403, description = "The caller's `flow.monitor.update` does not cover this ARTCC"),
        (status = 404, description = "No such sector in this ARTCC")
    )
)]
pub async fn set_sector_map(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowMonitorUpdate>,
    Actor(principal): Actor,
    Path((artcc, sector_id)): Path<(String, String)>,
    Json(payload): Json<SetSectorMapRequest>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let artcc = artcc.trim().to_ascii_uppercase();
    if !may_edit(&state, &principal, &artcc).await? {
        return Err(ApiError::Forbidden);
    }
    if payload.map <= 0 {
        return Err(ApiError::BadRequest);
    }
    let known = state
        .airspace_sectors
        .load()
        .sectors_of(&artcc)
        .iter()
        .any(|(id, _)| *id == sector_id);
    if !known {
        return Err(ApiError::NotFound);
    }
    // Against the stored row, not this pod's cache: another replica may have written since our last
    // refresh, and a stale cache would turn a real change (14 → 10) into a silent no-op.
    let current = repo::get(pool, &artcc, &sector_id)
        .await?
        .unwrap_or(DEFAULT_MAP);
    if payload.map == current {
        return Ok(StatusCode::NO_CONTENT);
    }
    repo::upsert(pool, &artcc, &sector_id, payload.map, principal.user_id()).await?;
    state
        .sector_maps
        .store(Arc::new(repo::load_all(pool).await?));
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;
    use sqlx::PgPool;

    use crate::{
        feed::sectors::{SectorTable, SectorVolume},
        scope_test_support::{grant, seed_user, send, send_json, session_cookie, test_state},
        state::AppState,
    };

    fn volume(artcc: &str, sector_id: &str, volume_id: &str) -> SectorVolume {
        SectorVolume {
            artcc: artcc.into(),
            sector_id: sector_id.into(),
            volume_id: volume_id.into(),
            name: Some(format!("{artcc} {sector_id}")),
            tier: "high".into(),
            base_alt_ft: 24_000,
            top_alt_ft: 60_000,
            rings: vec![vec![
                [38.0, -77.0],
                [39.0, -77.0],
                [39.0, -76.0],
                [38.0, -77.0],
            ]],
        }
    }

    /// ZDC sectors 24 (two volumes) and 25, and ZNY 10, in the sector cache.
    fn state(pool: PgPool) -> AppState {
        let state = test_state(pool, Default::default());
        state.airspace_sectors.store(Arc::new(SectorTable {
            volumes: vec![
                volume("ZDC", "24", "24a"),
                volume("ZDC", "24", "24b"),
                volume("ZDC", "25", "25"),
                volume("ZNY", "10", "10"),
            ],
        }));
        state
    }

    /// A signed-in user holding each of `grants` (`(permission, artcc)`).
    async fn user(pool: &PgPool, grants: &[(&str, Option<&str>)]) -> String {
        let id = seed_user(pool).await;
        for (permission, artcc) in grants {
            grant(pool, &id, permission, *artcc).await;
        }
        session_cookie(pool, &id).await
    }

    async fn tmu(pool: &PgPool, artcc: &str) -> String {
        user(
            pool,
            &[
                ("flow.monitor.read", None),
                ("flow.monitor.update", Some(artcc)),
            ],
        )
        .await
    }

    async fn stored(pool: &PgPool) -> Vec<(String, String, i32)> {
        sqlx::query_as("select artcc, sector_id, map from flow.sector_map order by 1, 2")
            .fetch_all(pool)
            .await
            .unwrap()
    }

    async fn put(state: &AppState, cookie: &str, sector: &str, body: serde_json::Value) -> u16 {
        let uri = format!("/api/v1/flow/monitor/ZDC/maps/{sector}");
        send(state, http::Method::PUT, &uri, cookie, Some(body))
            .await
            .as_u16()
    }

    /// AC1: every sector — one row per sector, however many volumes — reads MAP 10 until overridden.
    #[sqlx::test]
    async fn every_sector_reads_the_default_until_overridden(pool: PgPool) {
        let viewer = user(&pool, &[("flow.monitor.read", None)]).await;
        let state = state(pool);

        let (status, body) = send_json(
            &state,
            http::Method::GET,
            "/api/v1/flow/monitor/zdc/maps",
            &viewer,
        )
        .await;

        assert_eq!(status, http::StatusCode::OK);
        assert_eq!(body["default_map"], 10);
        assert_eq!(body["editable"], false);
        let sectors = body["sectors"].as_array().unwrap();
        assert_eq!(
            sectors
                .iter()
                .map(|s| s["sector_id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["24", "25"]
        );
        for s in sectors {
            assert_eq!(s["map"], 10);
            assert_eq!(s["overridden"], false);
        }
    }

    /// AC2 + AC6: a TMU sets their own ARTCC's MAP, and every viewer sees it at once — the write
    /// reloads the cache the Monitor reads, so nothing waits for the refresh job or a restart.
    #[sqlx::test]
    async fn a_tmu_sets_their_own_artccs_map_and_every_viewer_sees_it(pool: PgPool) {
        let zdc = tmu(&pool, "ZDC").await;
        let viewer = user(&pool, &[("flow.monitor.read", None)]).await;
        let state = state(pool.clone());

        assert_eq!(put(&state, &zdc, "24", json!({"map": 14})).await, 204);

        assert_eq!(
            crate::feed::sectors::map_for(&state.sector_maps.load(), "ZDC", "24"),
            14,
            "the cache the Monitor reads holds the new value"
        );
        let (_, body) = send_json(
            &state,
            http::Method::GET,
            "/api/v1/flow/monitor/ZDC/maps",
            &viewer,
        )
        .await;
        assert_eq!(body["sectors"][0]["map"], 14);
        assert_eq!(body["sectors"][0]["overridden"], true);
        assert_eq!(
            body["sectors"][1]["map"], 10,
            "the other sector keeps the default"
        );
        assert_eq!(stored(&pool).await, [("ZDC".into(), "24".into(), 14)]);
    }

    /// AC3: a TMU at another ARTCC is refused, and nothing is written.
    #[sqlx::test]
    async fn a_tmu_at_another_artcc_is_refused(pool: PgPool) {
        let zny = tmu(&pool, "ZNY").await;
        let state = state(pool.clone());

        assert_eq!(put(&state, &zny, "24", json!({"map": 14})).await, 403);
        assert!(stored(&pool).await.is_empty());

        let (_, body) = send_json(
            &state,
            http::Method::GET,
            "/api/v1/flow/monitor/ZDC/maps",
            &zny,
        )
        .await;
        assert_eq!(body["editable"], false, "and isn't offered the control");
    }

    /// AC3: a viewer without `flow.monitor.update` is refused, and nothing is written.
    #[sqlx::test]
    async fn a_user_without_the_update_permission_is_refused(pool: PgPool) {
        let viewer = user(&pool, &[("flow.monitor.read", None)]).await;
        let state = state(pool.clone());

        assert_eq!(put(&state, &viewer, "24", json!({"map": 14})).await, 401);
        assert!(stored(&pool).await.is_empty());
    }

    /// AC4: an empty, zero, negative or unchanged entry writes nothing and deletes nothing.
    #[sqlx::test]
    async fn bad_or_unchanged_entries_write_and_delete_nothing(pool: PgPool) {
        let zdc = tmu(&pool, "ZDC").await;
        let state = state(pool.clone());
        assert_eq!(put(&state, &zdc, "24", json!({"map": 14})).await, 204);
        let stamp = || async {
            sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
                "select updated_at from flow.sector_map",
            )
            .fetch_one(&pool)
            .await
            .unwrap()
        };
        let before = stamp().await;

        for bad in [json!({"map": 0}), json!({"map": -3})] {
            assert_eq!(put(&state, &zdc, "24", bad).await, 400);
        }
        for empty in [
            json!({}),
            json!({"map": ""}),
            json!({"map": null}),
            json!({"map": 1.5}),
        ] {
            assert!((400..500).contains(&put(&state, &zdc, "24", empty).await));
        }
        assert_eq!(
            put(&state, &zdc, "24", json!({"map": 14})).await,
            204,
            "unchanged"
        );
        assert_eq!(
            put(&state, &zdc, "25", json!({"map": 10})).await,
            204,
            "the default, unchanged"
        );

        assert_eq!(stored(&pool).await, [("ZDC".into(), "24".into(), 14)]);
        assert_eq!(stamp().await, before, "an unchanged entry rewrites nothing");
    }

    /// A replica whose cache predates another replica's write still applies a real change: "unchanged"
    /// is judged against the stored row. Here the override was written elsewhere (cache still empty),
    /// so typing the default must reset it, not be dropped as a no-op.
    #[sqlx::test]
    async fn unchanged_is_judged_against_the_store_not_a_stale_cache(pool: PgPool) {
        let zdc = tmu(&pool, "ZDC").await;
        let state = state(pool.clone());
        sqlx::query("insert into flow.sector_map (artcc, sector_id, map) values ('ZDC', '24', 14)")
            .execute(&pool)
            .await
            .unwrap();

        assert_eq!(put(&state, &zdc, "24", json!({"map": 10})).await, 204);
        assert_eq!(stored(&pool).await, [("ZDC".into(), "24".into(), 10)]);
    }

    /// Typing the default over an override is the reset — it is written like any other value.
    #[sqlx::test]
    async fn typing_the_default_resets_an_override(pool: PgPool) {
        let zdc = tmu(&pool, "ZDC").await;
        let state = state(pool.clone());
        assert_eq!(put(&state, &zdc, "24", json!({"map": 14})).await, 204);
        assert_eq!(put(&state, &zdc, "24", json!({"map": 10})).await, 204);
        assert_eq!(
            crate::feed::sectors::map_for(&state.sector_maps.load(), "ZDC", "24"),
            10
        );
    }

    /// A sector that isn't in the ARTCC's dataset can't be given a MAP.
    #[sqlx::test]
    async fn an_unknown_sector_is_not_found(pool: PgPool) {
        let zdc = tmu(&pool, "ZDC").await;
        let state = state(pool.clone());
        assert_eq!(
            put(&state, &zdc, "10", json!({"map": 14})).await,
            404,
            "ZNY's sector"
        );
        assert!(stored(&pool).await.is_empty());
    }
}
