//! Sector occupancy limits (#722, epic #720): the per-sector value a sector's counts are judged
//! against, shared by everyone watching the ARTCC.
//!
//! Reads are gated `flow.sectors.read`. Writes are gated `flow.sector_limits.update` **and** scoped to
//! the sector's ARTCC — the typed extractor says only *whether* the caller holds it, not *where*, so a
//! TMU at one facility could otherwise set a neighbour's limits. A write force-reloads
//! `AppState::sector_limits` and publishes `topic::SECTOR_LIMITS`, so every viewer recolours at once.

use std::{collections::BTreeMap, sync::Arc};

use axum::{
    Json,
    extract::{Path, State},
};

use crate::{
    auth::{
        permissions::{FlowSectorLimitsUpdate, FlowSectorsRead},
        principal::Actor,
        require_permission::RequirePermission,
    },
    errors::ApiError,
    feed::{
        sector_limits::{DEFAULT_LIMIT, SectorLimits, limit_for},
        sectors::SectorTable,
    },
    models::{SectorLimitBody, SectorLimitsBody, SetSectorLimitRequest},
    realtime::topic,
    repos::sector_limits as repo,
    state::AppState,
};

/// The permission a limit write is scoped against.
pub const SECTOR_LIMITS_UPDATE: &str = "flow.sector_limits.update";

/// `artcc`'s sectors, each once, with the tier of its first volume in table order (the rule
/// `feed::sector_load::sector_loads` uses), ordered by `sector_id`.
fn sector_tiers(table: &SectorTable, artcc: &str) -> BTreeMap<String, String> {
    let mut tiers = BTreeMap::new();
    for v in table.volumes.iter().filter(|v| v.artcc == artcc) {
        tiers
            .entry(v.sector_id.clone())
            .or_insert_with(|| v.tier.clone());
    }
    tiers
}

fn sector_body(
    limits: &SectorLimits,
    artcc: &str,
    sector_id: String,
    tier: String,
) -> SectorLimitBody {
    SectorLimitBody {
        limit: limit_for(limits, artcc, &sector_id),
        overridden: limits.contains_key(&(artcc.to_string(), sector_id.clone())),
        sector_id,
        tier,
    }
}

/// `artcc`'s sectors, each with its limit. An ARTCC with no sector data answers with no sectors, so
/// the page can name the gap rather than draw an empty table.
#[utoipa::path(
    get, path = "/api/v1/flow/sector-limits/{artcc}", tag = "flow",
    params(("artcc" = String, Path, description = "ARTCC id, case-insensitive")),
    responses(
        (status = 200, body = SectorLimitsBody),
        (status = 401, description = "Not signed in, or without `flow.sectors.read`"),
        (status = 503)
    ),
    security(("session" = ["flow.sectors.read"]), ("api_key" = ["flow.sectors.read"]), ("service_account" = ["flow.sectors.read"]))
)]
pub async fn list_sector_limits(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowSectorsRead>,
    Actor(principal): Actor,
    Path(artcc): Path<String>,
) -> Result<Json<SectorLimitsBody>, ApiError> {
    let artcc = artcc.trim().to_ascii_uppercase();
    let editable = principal
        .permission_scope(&state, SECTOR_LIMITS_UPDATE)
        .await?
        .allows(Some(&artcc));
    let limits = state.sector_limits.load();
    let sectors = sector_tiers(&state.airspace_sectors.load(), &artcc)
        .into_iter()
        .map(|(sector_id, tier)| sector_body(&limits, &artcc, sector_id, tier))
        .collect();
    Ok(Json(SectorLimitsBody {
        artcc,
        default_limit: DEFAULT_LIMIT,
        editable,
        sectors,
    }))
}

/// Set one sector's limit. Only a positive whole number that differs from the stored value is written:
/// zero or negative is refused (400) and leaves any override in place, and the stored value is a no-op
/// (200, nothing written). Setting the default removes the override. There is no delete.
#[utoipa::path(
    put, path = "/api/v1/flow/sector-limits/{artcc}/{sector_id}", tag = "flow",
    params(
        ("artcc" = String, Path, description = "ARTCC id, case-insensitive"),
        ("sector_id" = String, Path)
    ),
    request_body = SetSectorLimitRequest,
    responses(
        (status = 200, body = SectorLimitBody, description = "The sector's limit after the request, written or not"),
        (status = 400, description = "`limit` is not a positive whole number; nothing is written"),
        (status = 401, description = "Not signed in, or without `flow.sector_limits.update`"),
        (status = 403, description = "The caller's `flow.sector_limits.update` does not cover this ARTCC"),
        (status = 404, description = "No such sector in this ARTCC"),
        (status = 503)
    ),
    security(("session" = ["flow.sector_limits.update"]), ("api_key" = ["flow.sector_limits.update"]), ("service_account" = ["flow.sector_limits.update"]))
)]
pub async fn set_sector_limit(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowSectorLimitsUpdate>,
    Actor(principal): Actor,
    Path((artcc, sector_id)): Path<(String, String)>,
    Json(payload): Json<SetSectorLimitRequest>,
) -> Result<Json<SectorLimitBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let artcc = artcc.trim().to_ascii_uppercase();

    // Scope before anything else: a neighbour's TMU learns nothing about this ARTCC's sectors.
    let scope = principal
        .permission_scope(&state, SECTOR_LIMITS_UPDATE)
        .await?;
    if !scope.allows(Some(&artcc)) {
        return Err(ApiError::Forbidden);
    }
    let limit = payload.limit;
    if limit <= 0 {
        return Err(ApiError::BadRequest);
    }
    let tier = sector_tiers(&state.airspace_sectors.load(), &artcc)
        .remove(&sector_id)
        .ok_or(ApiError::NotFound)?;

    // Compare against the row, not the cache: another replica may have written since this one
    // reloaded, and a stale cache would turn a real change into a silent no-op.
    let stored = repo::get(pool, &artcc, &sector_id).await?;
    let wrote = if limit == DEFAULT_LIMIT {
        // The default is the reset: a row holding it would read "overridden" forever.
        if stored.is_some() {
            repo::delete(pool, &artcc, &sector_id).await?;
        }
        stored.is_some()
    } else if stored == Some(limit) {
        false
    } else {
        repo::upsert(pool, &artcc, &sector_id, limit, principal.user_id()).await?;
        true
    };

    if wrote {
        state
            .sector_limits
            .store(Arc::new(repo::load_all(pool).await?));
        state.publish(topic::SECTOR_LIMITS);
    }

    let now_stored = if limit == DEFAULT_LIMIT {
        None
    } else {
        Some(limit)
    };
    Ok(Json(SectorLimitBody {
        sector_id,
        tier,
        limit: now_stored.unwrap_or(DEFAULT_LIMIT),
        overridden: now_stored.is_some(),
    }))
}
