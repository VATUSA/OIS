//! Airspace Monitor alert parameters (#598, epic #593): the per-sector limit a sector's count is
//! coloured against, shared by everyone watching the ARTCC.
//!
//! Reads are gated `flow.monitor.read`. Writes are gated `flow.monitor.update` **and** scoped to the
//! sector's ARTCC — the typed extractor says only *whether* the caller holds it, not *where*, so a TMU
//! at one facility could otherwise set another's limits. A write force-reloads `AppState::sector_maps`
//! so the next Monitor cycle recolours without waiting for `jobs::spawn_sector_maps_refresh`.

use std::sync::Arc;

use chrono::{DateTime, Utc};

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
    feed::{
        monitor::artcc_table,
        monitor_tracks::{AIRBORNE_GS_KT, Bbox, project_tracks},
        sectors::{DEFAULT_MAP, map_for},
        vatsim::VatsimData,
    },
    handlers::flow::all_excluded_callsigns,
    models::{
        BulkConsolidateMode, BulkConsolidateRequest, ConsolidateSectorRequest, MonitorBinBody,
        MonitorRowBody, MonitorTableBody, SectorConsolidationBody, SectorConsolidationsBody,
        SectorMapBody, SectorMapsBody, SetSectorMapRequest,
    },
    repos::{flow as flow_repo, sector_consolidations as consolidations_repo, sector_maps as repo},
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

/// Whether `sector_id` is one of `artcc`'s sectors in the imported dataset.
fn is_sector(state: &AppState, artcc: &str, sector_id: &str) -> bool {
    state
        .airspace_sectors
        .load()
        .sectors_of(artcc)
        .iter()
        .any(|(id, _)| id == sector_id)
}

/// Refuse unless `principal` may change `artcc`'s Monitor configuration.
async fn require_edit(
    state: &AppState,
    principal: &Principal,
    artcc: &str,
) -> Result<(), ApiError> {
    if may_edit(state, principal, artcc).await? {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

/// `artcc`'s Airspace Monitor (#701): every sector's peak occupancy per 15-minute bin over six hours,
/// classified against its MAP, with consolidations and vNAS staffing. Computed on request from the
/// live feed and the cached sectors, MAPs and consolidations, so nothing about it is stored. Live
/// flights are projected along their routes by the shared trajectory model (`feed::monitor_tracks`).
#[utoipa::path(
    get, path = "/api/v1/flow/monitor/{artcc}", tag = "flow",
    params(("artcc" = String, Path)),
    responses((status = 200, body = MonitorTableBody), (status = 401), (status = 503))
)]
pub async fn monitor_table(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowMonitorRead>,
    Actor(principal): Actor,
    Path(artcc): Path<String>,
) -> Result<Json<MonitorTableBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let artcc = artcc.trim().to_ascii_uppercase();
    let editable = may_edit(&state, &principal, &artcc).await?;
    let (snapshot, airports) = {
        let feed = state.feed.read().await;
        (feed.snapshot.clone(), feed.airports.clone())
    };
    // A release only matters for a flight still on the ground; an airborne one is projected from
    // where it is.
    let grounded: Vec<String> = snapshot
        .iter()
        .flat_map(|s| {
            let pilots = s
                .data
                .pilots
                .iter()
                .filter(|p| p.groundspeed < AIRBORNE_GS_KT);
            pilots
                .map(|p| p.callsign.clone())
                .chain(s.data.prefiles.iter().map(|p| p.callsign.clone()))
        })
        .collect();
    let releases = flow_repo::releases_for_callsigns(pool, &grounded).await?;
    let excluded = all_excluded_callsigns(&state.flight_exclusions.load());
    let (nav, profiles, winds) = (
        state.nav.load_full(),
        state.aircraft_profiles.load_full(),
        state.winds.load_full(),
    );
    let (sectors, consolidations, maps, staffing) = (
        state.airspace_sectors.load_full(),
        state.sector_consolidations.load_full(),
        state.sector_maps.load_full(),
        state.vnas.staffing.load_full(),
    );
    let now = Utc::now();
    let served = artcc.clone();
    let rows = tokio::task::spawn_blocking(move || {
        let empty = VatsimData::default();
        let data = snapshot.as_ref().map_or(&empty, |s| &s.data);
        let tracks = match Bbox::of_artcc(&sectors, &artcc) {
            Some(bbox) => project_tracks(
                data,
                &nav,
                &airports,
                &profiles,
                &winds,
                &releases,
                &excluded,
                now.timestamp_millis(),
                Some(bbox),
            ),
            None => Vec::new(), // no sectors, no rows: nothing to project for
        };
        artcc_table(
            &sectors,
            &consolidations,
            &maps,
            &staffing,
            &tracks,
            &artcc,
            now.timestamp_millis(),
        )
        .into_iter()
        .map(|row| MonitorRowBody {
            bins: row
                .bins
                .into_iter()
                .map(|b| MonitorBinBody {
                    start: DateTime::from_timestamp_millis(b.start_ms).unwrap_or(now),
                    active: b.active as i64,
                    proposed: b.proposed as i64,
                    combined: b.combined as i64,
                    alert: b.alert,
                })
                .collect(),
            sector_id: row.sector_id,
            name: row.name,
            map: row.map,
            consolidated: row.consolidated,
            staffed: row.staffed,
        })
        .collect::<Vec<_>>()
    })
    .await
    .map_err(|_| ApiError::Internal)?;
    Ok(Json(MonitorTableBody {
        artcc: served,
        editable,
        as_of: now,
        rows,
    }))
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
    require_edit(&state, &principal, &artcc).await?;
    if payload.map <= 0 {
        return Err(ApiError::BadRequest);
    }
    if !is_sector(&state, &artcc, &sector_id) {
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

/// Reload the consolidation cache so a write is visible to every viewer at once.
async fn refresh_consolidations(state: &AppState, pool: &sqlx::PgPool) -> Result<(), ApiError> {
    let loaded = consolidations_repo::load_all(pool).await?;
    state.sector_consolidations.store(Arc::new(loaded));
    Ok(())
}

#[utoipa::path(
    get, path = "/api/v1/flow/monitor/{artcc}/consolidations", tag = "flow",
    params(("artcc" = String, Path)),
    responses((status = 200, body = SectorConsolidationsBody), (status = 401))
)]
pub async fn list_consolidations(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowMonitorRead>,
    Actor(principal): Actor,
    Path(artcc): Path<String>,
) -> Result<Json<SectorConsolidationsBody>, ApiError> {
    let artcc = artcc.trim().to_ascii_uppercase();
    let mut consolidations: Vec<SectorConsolidationBody> = state
        .sector_consolidations
        .load()
        .iter()
        .filter(|((a, _), _)| *a == artcc)
        .map(|((_, source), target)| SectorConsolidationBody {
            sector_id: source.clone(),
            target_sector_id: target.clone(),
        })
        .collect();
    consolidations.sort_by(|a, b| a.sector_id.cmp(&b.sector_id));
    Ok(Json(SectorConsolidationsBody {
        editable: may_edit(&state, &principal, &artcc).await?,
        consolidations,
    }))
}

/// Work a sector at another sector's position (#599). Both must be this ARTCC's sectors, so a
/// cross-ARTCC target can't be named (404). A sector can't be worked at itself (400), and a save that
/// would make a loop is refused (409). The arrangement stays flat — see
/// [`consolidations_repo::consolidate`] — and the save is all-or-nothing.
#[utoipa::path(
    put, path = "/api/v1/flow/monitor/{artcc}/consolidations/{sector_id}", tag = "flow",
    params(("artcc" = String, Path), ("sector_id" = String, Path)),
    request_body = ConsolidateSectorRequest,
    responses(
        (status = 204),
        (status = 400, description = "A sector can't be worked at itself"),
        (status = 401),
        (status = 403, description = "The caller's `flow.monitor.update` does not cover this ARTCC"),
        (status = 404, description = "Either sector isn't one of this ARTCC's"),
        (status = 409, description = "The save would make a loop")
    )
)]
pub async fn consolidate_sector(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowMonitorUpdate>,
    Actor(principal): Actor,
    Path((artcc, sector_id)): Path<(String, String)>,
    Json(payload): Json<ConsolidateSectorRequest>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let artcc = artcc.trim().to_ascii_uppercase();
    require_edit(&state, &principal, &artcc).await?;
    let target = payload.target_sector_id.trim();
    if !is_sector(&state, &artcc, &sector_id) || !is_sector(&state, &artcc, target) {
        return Err(ApiError::NotFound);
    }
    if target == sector_id {
        return Err(ApiError::BadRequest);
    }
    consolidations_repo::consolidate(pool, &artcc, &sector_id, target, principal.user_id()).await?;
    refresh_consolidations(&state, pool).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Consolidate many of this ARTCC's sectors into one at once (#713): every other sector, or only those in
/// no consolidation yet. One transaction — a refused or failed save changes nothing — gated like the
/// single-sector write. See [`consolidations_repo::consolidate_all`].
#[utoipa::path(
    post, path = "/api/v1/flow/monitor/{artcc}/consolidations", tag = "flow",
    params(("artcc" = String, Path)),
    request_body = BulkConsolidateRequest,
    responses(
        (status = 204),
        (status = 401),
        (status = 403, description = "The caller's `flow.monitor.update` does not cover this ARTCC"),
        (status = 404, description = "The target isn't one of this ARTCC's sectors"),
        (status = 409, description = "`except_consolidated`, and the target is itself worked elsewhere")
    )
)]
pub async fn consolidate_all_sectors(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowMonitorUpdate>,
    Actor(principal): Actor,
    Path(artcc): Path<String>,
    Json(payload): Json<BulkConsolidateRequest>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let artcc = artcc.trim().to_ascii_uppercase();
    require_edit(&state, &principal, &artcc).await?;
    let target = payload.target_sector_id.trim();
    if !is_sector(&state, &artcc, target) {
        return Err(ApiError::NotFound);
    }
    let sectors: Vec<String> = state
        .airspace_sectors
        .load()
        .sectors_of(&artcc)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    consolidations_repo::consolidate_all(
        pool,
        &artcc,
        target,
        &sectors,
        payload.mode == BulkConsolidateMode::ExceptConsolidated,
        principal.user_id(),
    )
    .await?;
    refresh_consolidations(&state, pool).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Give a consolidated sector its own row back. A sector that isn't consolidated is a no-op.
#[utoipa::path(
    delete, path = "/api/v1/flow/monitor/{artcc}/consolidations/{sector_id}", tag = "flow",
    params(("artcc" = String, Path), ("sector_id" = String, Path)),
    responses(
        (status = 204),
        (status = 401),
        (status = 403, description = "The caller's `flow.monitor.update` does not cover this ARTCC")
    )
)]
pub async fn release_sector(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowMonitorUpdate>,
    Actor(principal): Actor,
    Path((artcc, sector_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let artcc = artcc.trim().to_ascii_uppercase();
    require_edit(&state, &principal, &artcc).await?;
    consolidations_repo::release(pool, &artcc, &sector_id).await?;
    refresh_consolidations(&state, pool).await?;
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

    /// #701 AC5: the Monitor table is gated on `flow.monitor.read` — no session and a session without
    /// it are both refused (401, as every missing permission is) — and a holder gets every one of the
    /// ARTCC's sectors as a row of six hours of bins. With no feed there are no flights, so every bin
    /// is empty and green.
    #[sqlx::test]
    async fn the_monitor_table_is_gated_and_shaped(pool: PgPool) {
        let state = state(pool.clone());
        const URI: &str = "/api/v1/flow/monitor/zdc";
        assert_eq!(
            send_json(&state, axum::http::Method::GET, URI, "").await.0,
            axum::http::StatusCode::UNAUTHORIZED
        );

        let user = seed_user(&pool).await;
        let cookie = session_cookie(&pool, &user).await;
        assert_eq!(
            send_json(&state, axum::http::Method::GET, URI, &cookie)
                .await
                .0,
            axum::http::StatusCode::UNAUTHORIZED
        );

        grant(&pool, &user, "flow.monitor.read", None).await;
        let (status, body) = send_json(&state, axum::http::Method::GET, URI, &cookie).await;
        assert_eq!(status, axum::http::StatusCode::OK, "{body}");
        assert_eq!(body["artcc"], "ZDC");
        assert_eq!(body["editable"], false);
        let rows = body["rows"].as_array().unwrap();
        let ids: Vec<&str> = rows
            .iter()
            .map(|r| r["sector_id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["24", "25"], "ZDC's sectors once each, not ZNY's");
        assert_eq!(rows[0]["name"], "ZDC 24");
        assert_eq!(rows[0]["map"], 10);
        assert_eq!(rows[0]["staffed"], false);
        let bins = rows[0]["bins"].as_array().unwrap();
        assert_eq!(bins.len(), 24, "six hours of quarter-hours");
        assert_eq!(bins[0]["combined"], 0);
        assert_eq!(bins[0]["alert"], "green");
    }
}

#[cfg(test)]
mod consolidation_tests {
    use std::sync::Arc;

    use serde_json::json;
    use sqlx::PgPool;

    use crate::{
        feed::sectors::{SectorTable, tests::volume},
        scope_test_support::{grant, seed_user, send, send_json, session_cookie, test_state},
        state::AppState,
    };

    /// ZLA sectors 018, 041 and 020, and ZDC 024 (the fixture names a sector by its volume id's
    /// first three characters).
    fn state(pool: PgPool) -> AppState {
        let state = test_state(pool, Default::default());
        state.airspace_sectors.store(Arc::new(SectorTable {
            volumes: vec![
                volume("ZLA", "0180"),
                volume("ZLA", "0410"),
                volume("ZLA", "0200"),
                volume("ZDC", "0240"),
            ],
        }));
        state
    }

    async fn user(pool: &PgPool, update_at: Option<&str>) -> String {
        let id = seed_user(pool).await;
        grant(pool, &id, "flow.monitor.read", None).await;
        if let Some(artcc) = update_at {
            grant(pool, &id, "flow.monitor.update", Some(artcc)).await;
        }
        session_cookie(pool, &id).await
    }

    async fn work_at(state: &AppState, cookie: &str, source: &str, target: &str) -> u16 {
        send(
            state,
            http::Method::PUT,
            &format!("/api/v1/flow/monitor/ZLA/consolidations/{source}"),
            cookie,
            Some(json!({"target_sector_id": target})),
        )
        .await
        .as_u16()
    }

    async fn stored(pool: &PgPool) -> Vec<(String, String)> {
        sqlx::query_as(
            "select sector_id, target_sector_id from flow.sector_consolidation order by 1",
        )
        .fetch_all(pool)
        .await
        .unwrap()
    }

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    fn cached(state: &AppState) -> Vec<(String, String)> {
        let mut rows: Vec<_> = state
            .sector_consolidations
            .load()
            .iter()
            .map(|((_, s), t)| (s.clone(), t.clone()))
            .collect();
        rows.sort();
        rows
    }

    // ---- #713: bulk consolidation ----------------------------------------------------------------

    /// ZLA sectors 010–060 — room for a consolidation, a hub and free sectors — and ZDC 024 plus ZDC
    /// sectors sharing ZLA's ids (030, 040, 050), so a write that forgot its ARTCC would show.
    fn wide_state(pool: PgPool) -> AppState {
        let state = test_state(pool, Default::default());
        let mut volumes: Vec<_> = ["0100", "0200", "0300", "0400", "0500", "0600"]
            .iter()
            .map(|v| volume("ZLA", v))
            .collect();
        for v in ["0240", "0300", "0400", "0500"] {
            volumes.push(volume("ZDC", v));
        }
        state
            .airspace_sectors
            .store(Arc::new(SectorTable { volumes }));
        state
    }

    /// ZDC's own arrangement, on sector ids ZLA has too: 030 and 040 worked at 050. A ZLA bulk write
    /// must neither delete these nor read them as ZLA's (the ARTCC predicate of each statement).
    async fn seed_zdc_neighbours(pool: &PgPool) {
        sqlx::query(
            "insert into flow.sector_consolidation (artcc, sector_id, target_sector_id) \
             values ('ZDC', '030', '050'), ('ZDC', '040', '050')",
        )
        .execute(pool)
        .await
        .unwrap();
    }

    async fn rows_at(pool: &PgPool, artcc: &str) -> Vec<(String, String)> {
        sqlx::query_as(
            "select sector_id, target_sector_id from flow.sector_consolidation \
             where artcc = $1 order by 1",
        )
        .bind(artcc)
        .fetch_all(pool)
        .await
        .unwrap()
    }

    async fn all_into(state: &AppState, cookie: &str, target: &str, mode: &str) -> u16 {
        send(
            state,
            http::Method::POST,
            "/api/v1/flow/monitor/ZLA/consolidations",
            cookie,
            Some(json!({"target_sector_id": target, "mode": mode})),
        )
        .await
        .as_u16()
    }

    /// AC1: "All into N" leaves N the only row — even when N was itself worked elsewhere, and whatever
    /// was consolidated before. Another ARTCC's sectors with the same ids are untouched.
    #[sqlx::test]
    async fn all_into_n_leaves_n_the_only_row(pool: PgPool) {
        let zla = user(&pool, Some("ZLA")).await;
        let state = wide_state(pool.clone());
        assert_eq!(work_at(&state, &zla, "010", "020").await, 204);
        assert_eq!(work_at(&state, &zla, "040", "050").await, 204);
        seed_zdc_neighbours(&pool).await;

        assert_eq!(all_into(&state, &zla, "040", "all").await, 204);

        let expected = pairs(&[
            ("010", "040"),
            ("020", "040"),
            ("030", "040"),
            ("050", "040"),
            ("060", "040"),
        ]);
        assert_eq!(
            rows_at(&pool, "ZLA").await,
            expected,
            "N has no row; everything else is at N"
        );
        assert_eq!(
            rows_at(&pool, "ZDC").await,
            pairs(&[("030", "050"), ("040", "050")]),
            "ZDC's own 040 survives releasing ZLA's"
        );
        let cached_zla: std::collections::BTreeSet<(String, String)> = state
            .sector_consolidations
            .load()
            .iter()
            .filter(|((artcc, _), _)| artcc == "ZLA")
            .map(|((_, s), t)| (s.clone(), t.clone()))
            .collect();
        assert_eq!(
            cached_zla.into_iter().collect::<Vec<_>>(),
            expected,
            "and every viewer sees it at once"
        );
    }

    /// AC2: "except consolidated" moves only free-standing sectors: one worked elsewhere (010) and the
    /// position it is worked at (020) both stay as they were. ZDC's consolidated 030 and 040 must not
    /// make ZLA's 030 look consolidated, nor ZLA's 040 look worked elsewhere.
    #[sqlx::test]
    async fn except_consolidated_leaves_existing_arrangements(pool: PgPool) {
        let zla = user(&pool, Some("ZLA")).await;
        let state = wide_state(pool.clone());
        assert_eq!(work_at(&state, &zla, "010", "020").await, 204);
        seed_zdc_neighbours(&pool).await;

        assert_eq!(
            all_into(&state, &zla, "040", "except_consolidated").await,
            204
        );

        assert_eq!(
            rows_at(&pool, "ZLA").await,
            pairs(&[
                ("010", "020"),
                ("030", "040"),
                ("050", "040"),
                ("060", "040")
            ])
        );
        assert_eq!(
            rows_at(&pool, "ZDC").await,
            pairs(&[("030", "050"), ("040", "050")])
        );
    }

    /// AC2/AC3: with N itself worked elsewhere, "except consolidated" can't keep its promise, so it is
    /// refused — and the refused save writes nothing.
    #[sqlx::test]
    async fn except_consolidated_is_refused_when_n_is_worked_elsewhere(pool: PgPool) {
        let zla = user(&pool, Some("ZLA")).await;
        let state = wide_state(pool.clone());
        assert_eq!(work_at(&state, &zla, "040", "050").await, 204);

        assert_eq!(
            all_into(&state, &zla, "040", "except_consolidated").await,
            409
        );
        assert_eq!(stored(&pool).await, pairs(&[("040", "050")]));
    }

    /// AC4: a TMU at another ARTCC, a viewer without the update grant and an unknown target are all
    /// refused, and nothing is written.
    #[sqlx::test]
    async fn a_bulk_consolidation_is_gated_like_one_sector(pool: PgPool) {
        let zdc = user(&pool, Some("ZDC")).await;
        let viewer = user(&pool, None).await;
        let zla = user(&pool, Some("ZLA")).await;
        let state = wide_state(pool.clone());

        assert_eq!(all_into(&state, &zdc, "040", "all").await, 403);
        assert_eq!(all_into(&state, &viewer, "040", "all").await, 401);
        assert_eq!(all_into(&state, &zla, "999", "all").await, 404);
        assert_eq!(
            all_into(&state, &zla, "024", "all").await,
            404,
            "another ARTCC's sector"
        );
        assert!(stored(&pool).await.is_empty());
    }

    /// Releasing is a write to another facility's Monitor too: a TMU at another ARTCC (and a viewer
    /// with no update grant) is refused on the DELETE route, and the consolidation survives. The owning
    /// ARTCC's TMU can release it.
    #[sqlx::test]
    async fn only_the_owning_artcc_can_release_a_consolidation(pool: PgPool) {
        let zla = user(&pool, Some("ZLA")).await;
        let zdc = user(&pool, Some("ZDC")).await;
        let viewer = user(&pool, None).await;
        let state = state(pool.clone());
        assert_eq!(work_at(&state, &zla, "018", "041").await, 204);

        let release = |cookie: String| {
            let state = state.clone();
            async move {
                send(
                    &state,
                    http::Method::DELETE,
                    "/api/v1/flow/monitor/ZLA/consolidations/018",
                    &cookie,
                    None,
                )
                .await
                .as_u16()
            }
        };
        assert_eq!(
            release(zdc).await,
            403,
            "a TMU at another ARTCC can't release ZLA's"
        );
        assert_eq!(
            release(viewer).await,
            401,
            "nor can a viewer without flow.monitor.update"
        );
        assert_eq!(
            stored(&pool).await,
            pairs(&[("018", "041")]),
            "nothing was released"
        );
        assert_eq!(cached(&state), pairs(&[("018", "041")]));

        assert_eq!(release(zla).await, 204, "ZLA's own TMU can");
        assert!(stored(&pool).await.is_empty());
    }

    /// AC1: a consolidation is server-side and every viewer sees it at once — the write reloads the
    /// cache the engine reads. Releasing it gives the sector its row back.
    #[sqlx::test]
    async fn a_consolidation_is_shared_with_every_viewer_at_once(pool: PgPool) {
        let tmu = user(&pool, Some("ZLA")).await;
        let viewer = user(&pool, None).await;
        let state = state(pool.clone());

        assert_eq!(work_at(&state, &tmu, "018", "041").await, 204);

        assert_eq!(cached(&state), pairs(&[("018", "041")]));
        let (status, body) = send_json(
            &state,
            http::Method::GET,
            "/api/v1/flow/monitor/ZLA/consolidations",
            &viewer,
        )
        .await;
        assert_eq!(status, http::StatusCode::OK);
        assert_eq!(body["editable"], false);
        assert_eq!(
            body["consolidations"],
            json!([{"sector_id": "018", "target_sector_id": "041"}])
        );

        let release = send(
            &state,
            http::Method::DELETE,
            "/api/v1/flow/monitor/ZLA/consolidations/018",
            &tmu,
            None,
        )
        .await;
        assert_eq!(release.as_u16(), 204);
        assert!(stored(&pool).await.is_empty());
        assert!(cached(&state).is_empty());
    }

    /// AC5: a sector can only be worked at another sector in the same ARTCC, not at itself, and a
    /// loop is refused — and so is a TMU from another ARTCC. None of them writes anything.
    #[sqlx::test]
    async fn cross_artcc_self_loops_and_other_facilities_are_refused(pool: PgPool) {
        let tmu = user(&pool, Some("ZLA")).await;
        let zdc = user(&pool, Some("ZDC")).await;
        let state = state(pool.clone());

        assert_eq!(
            work_at(&state, &tmu, "018", "024").await,
            404,
            "ZDC's sector"
        );
        assert_eq!(work_at(&state, &tmu, "018", "018").await, 400, "itself");
        assert_eq!(
            work_at(&state, &zdc, "018", "041").await,
            403,
            "another facility's TMU"
        );
        assert!(stored(&pool).await.is_empty());

        assert_eq!(work_at(&state, &tmu, "018", "041").await, 204);
        assert_eq!(
            work_at(&state, &tmu, "041", "018").await,
            409,
            "018 at 041, then 041 at 018"
        );
        assert_eq!(stored(&pool).await, pairs(&[("018", "041")]));
    }

    /// AC6: chains are flattened on every write. 018 at 041, then 041 at 020, leaves 018 at 020 —
    /// 041's row no longer exists to hold it. And a target that is itself worked elsewhere resolves
    /// to where it is worked.
    #[sqlx::test]
    async fn chains_are_flattened_on_every_write(pool: PgPool) {
        let tmu = user(&pool, Some("ZLA")).await;
        let state = state(pool.clone());

        assert_eq!(work_at(&state, &tmu, "018", "041").await, 204);
        assert_eq!(work_at(&state, &tmu, "041", "020").await, 204);
        assert_eq!(
            stored(&pool).await,
            pairs(&[("018", "020"), ("041", "020")])
        );

        sqlx::query("delete from flow.sector_consolidation where sector_id = '018'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            work_at(&state, &tmu, "018", "041").await,
            204,
            "041 is worked at 020"
        );
        assert_eq!(
            stored(&pool).await,
            pairs(&[("018", "020"), ("041", "020")])
        );
    }

    /// AC7: a refused save leaves the stored arrangement and the cache exactly as they were. The
    /// save is one transaction, so a failed one does too.
    #[sqlx::test]
    async fn a_refused_save_restores_nothing_because_it_changed_nothing(pool: PgPool) {
        let tmu = user(&pool, Some("ZLA")).await;
        let state = state(pool.clone());
        assert_eq!(work_at(&state, &tmu, "018", "041").await, 204);
        assert_eq!(work_at(&state, &tmu, "020", "041").await, 204);
        let (rows, cache) = (stored(&pool).await, cached(&state));

        assert_eq!(work_at(&state, &tmu, "041", "018").await, 409);

        assert_eq!(stored(&pool).await, rows);
        assert_eq!(cached(&state), cache);
    }
}
