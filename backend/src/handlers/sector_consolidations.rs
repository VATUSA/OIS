//! Sector consolidation (#723, epic #720): working a sector at another sector's position, so its
//! airspace counts in the target's occupancy row. Shared by everyone watching the ARTCC.
//!
//! Reads are gated `flow.sectors.read`. Writes are gated `flow.sector_consolidations.update` **and**
//! scoped to the ARTCC in the path — the typed extractor says only *whether* the caller holds it, not
//! *where*. Both sectors are looked up in that ARTCC alone, so a body naming another ARTCC's sector is
//! an unknown sector, never a cross-ARTCC consolidation. Every write force-reloads
//! `AppState::sector_consolidations`, so the answer is the table's, never a cache another replica's
//! write has left behind; one that changed anything also publishes `topic::SECTOR_CONSOLIDATIONS`, so
//! every viewer's rows merge or split at once rather than at the next refresh.

use std::{collections::BTreeMap, sync::Arc};

use axum::{
    Json,
    extract::{Path, State},
};

use crate::{
    auth::{
        permissions::{FlowSectorConsolidationsUpdate, FlowSectorsRead},
        principal::{Actor, Principal},
        require_permission::RequirePermission,
    },
    errors::ApiError,
    models::{
        ConsolidateSectorRequest, ConsolidateSectorsRequest, SectorConsolidationBody,
        SectorConsolidationsBody,
    },
    realtime::topic,
    repos::sector_consolidations::{self as repo, Refusal},
    state::AppState,
};

/// The permission a consolidation write is scoped against.
pub const SECTOR_CONSOLIDATIONS_UPDATE: &str = "flow.sector_consolidations.update";

/// Whether `sector_id` is one of `artcc`'s sectors in the cached dataset.
fn is_sector(state: &AppState, artcc: &str, sector_id: &str) -> bool {
    state
        .airspace_sectors
        .load()
        .volumes
        .iter()
        .any(|v| v.artcc == artcc && v.sector_id == sector_id)
}

/// Whether the caller may change `artcc`'s consolidations.
async fn may_edit(state: &AppState, principal: &Principal, artcc: &str) -> Result<bool, ApiError> {
    Ok(principal
        .permission_scope(state, SECTOR_CONSOLIDATIONS_UPDATE)
        .await?
        .allows(Some(artcc)))
}

/// `artcc`'s consolidations from the cache, ordered by `sector_id`.
fn body(state: &AppState, artcc: String, editable: bool) -> SectorConsolidationsBody {
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
    SectorConsolidationsBody {
        artcc,
        editable,
        consolidations,
    }
}

/// Reload the cache from the table, and tell every viewer when the write `changed` anything. A no-op
/// reloads too: on a replica whose cache lags another's write, the answer must still be the table's.
async fn republish(state: &AppState, pool: &sqlx::PgPool, changed: bool) -> Result<(), ApiError> {
    state
        .sector_consolidations
        .store(Arc::new(repo::load_all(pool).await?));
    if changed {
        state.publish(topic::SECTOR_CONSOLIDATIONS);
    }
    Ok(())
}

/// `artcc`'s consolidations: which sectors are worked at which, with `editable` for the caller.
#[utoipa::path(
    get, path = "/api/v1/flow/sector-consolidations/{artcc}", tag = "flow",
    params(("artcc" = String, Path, description = "ARTCC id, case-insensitive")),
    responses(
        (status = 200, body = SectorConsolidationsBody),
        (status = 401, description = "Not signed in, or without `flow.sectors.read`"),
        (status = 503)
    ),
    security(("session" = ["flow.sectors.read"]), ("api_key" = ["flow.sectors.read"]), ("service_account" = ["flow.sectors.read"]))
)]
pub async fn list_sector_consolidations(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowSectorsRead>,
    Actor(principal): Actor,
    Path(artcc): Path<String>,
) -> Result<Json<SectorConsolidationsBody>, ApiError> {
    let artcc = artcc.trim().to_ascii_uppercase();
    let editable = may_edit(&state, &principal, &artcc).await?;
    Ok(Json(body(&state, artcc, editable)))
}

/// Work a sector at another of this ARTCC's sectors. Both must be this ARTCC's (404 otherwise, which
/// is how a cross-ARTCC consolidation is refused). A sector can't be worked at itself (400), and a save
/// that would make a loop is refused (409); neither writes anything. The arrangement stays flat: a
/// target worked elsewhere resolves to where it is worked, and the sectors worked at this one move with
/// it. Answers with the ARTCC's consolidations after the save.
#[utoipa::path(
    put, path = "/api/v1/flow/sector-consolidations/{artcc}/{sector_id}", tag = "flow",
    params(
        ("artcc" = String, Path, description = "ARTCC id, case-insensitive"),
        ("sector_id" = String, Path, description = "The sector to work elsewhere")
    ),
    request_body = ConsolidateSectorRequest,
    responses(
        (status = 200, body = SectorConsolidationsBody, description = "The ARTCC's consolidations after the save, written or not"),
        (status = 400, description = "A sector can't be worked at itself; nothing is written"),
        (status = 401, description = "Not signed in, or without `flow.sector_consolidations.update`"),
        (status = 403, description = "The caller's `flow.sector_consolidations.update` does not cover this ARTCC"),
        (status = 404, description = "Either sector is not one of this ARTCC's"),
        (status = 409, description = "The target is worked at this sector, so the save would make a loop; nothing is written"),
        (status = 503)
    ),
    security(("session" = ["flow.sector_consolidations.update"]), ("api_key" = ["flow.sector_consolidations.update"]), ("service_account" = ["flow.sector_consolidations.update"]))
)]
pub async fn consolidate_sector(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowSectorConsolidationsUpdate>,
    Actor(principal): Actor,
    Path((artcc, sector_id)): Path<(String, String)>,
    Json(payload): Json<ConsolidateSectorRequest>,
) -> Result<Json<SectorConsolidationsBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let artcc = artcc.trim().to_ascii_uppercase();

    // Scope before anything else: a neighbour's TMU learns nothing about this ARTCC's sectors.
    if !may_edit(&state, &principal, &artcc).await? {
        return Err(ApiError::Forbidden);
    }
    // The path's sector is trimmed like the body's, so the two name a sector the same way.
    let sector_id = sector_id.trim();
    let target = payload.target_sector_id.trim();
    if !is_sector(&state, &artcc, sector_id) || !is_sector(&state, &artcc, target) {
        return Err(ApiError::NotFound);
    }
    match repo::consolidate(pool, &artcc, sector_id, target, principal.user_id()).await? {
        Err(Refusal::SelfReference) => return Err(ApiError::BadRequest),
        Err(Refusal::Loop) => return Err(ApiError::Conflict),
        Ok(changed) => republish(&state, pool, changed).await?,
    }
    Ok(Json(body(&state, artcc, true)))
}

/// Change several of this ARTCC's consolidations at once, all or nothing: the Sector Monitor's "All"
/// commands and its checklists (#794, #792). `into` maps each sector to the sector to work it at, or to
/// null to give it its own row back. Every sector being worked somewhere, and every target, must be
/// this ARTCC's (404 otherwise); a release is not checked against the dataset, like the single release.
/// A self-reference (or two keys naming one sector) is a 400 and a loop, including one between the
/// batch's own entries, a 409, and neither writes anything. Answers with the ARTCC's consolidations after the save; a batch that
/// changed something tells every viewer once.
#[utoipa::path(
    put, path = "/api/v1/flow/sector-consolidations/{artcc}", tag = "flow",
    params(("artcc" = String, Path, description = "ARTCC id, case-insensitive")),
    request_body = ConsolidateSectorsRequest,
    responses(
        (status = 200, body = SectorConsolidationsBody, description = "The ARTCC's consolidations after the save, written or not"),
        (status = 400, description = "An entry works a sector at itself, or two entries name the same sector; nothing is written"),
        (status = 401, description = "Not signed in, or without `flow.sector_consolidations.update`"),
        (status = 403, description = "The caller's `flow.sector_consolidations.update` does not cover this ARTCC"),
        (status = 404, description = "A sector being consolidated, or a target, is not one of this ARTCC's; nothing is written"),
        (status = 409, description = "The batch would make a loop; nothing is written"),
        (status = 503)
    ),
    security(("session" = ["flow.sector_consolidations.update"]), ("api_key" = ["flow.sector_consolidations.update"]), ("service_account" = ["flow.sector_consolidations.update"]))
)]
pub async fn consolidate_sectors(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowSectorConsolidationsUpdate>,
    Actor(principal): Actor,
    Path(artcc): Path<String>,
    Json(payload): Json<ConsolidateSectorsRequest>,
) -> Result<Json<SectorConsolidationsBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let artcc = artcc.trim().to_ascii_uppercase();
    if !may_edit(&state, &principal, &artcc).await? {
        return Err(ApiError::Forbidden);
    }
    // Trimmed like the single routes. Two keys that trim to one sector would silently drop one.
    let mut entries = BTreeMap::new();
    for (source, target) in payload.into {
        let target = target.map(|t| t.trim().to_string());
        if entries.insert(source.trim().to_string(), target).is_some() {
            return Err(ApiError::BadRequest);
        }
    }
    let unknown = entries.iter().any(|(source, target)| {
        target
            .as_deref()
            .is_some_and(|t| !is_sector(&state, &artcc, source) || !is_sector(&state, &artcc, t))
    });
    if unknown {
        return Err(ApiError::NotFound);
    }
    match repo::apply_batch(pool, &artcc, &entries, principal.user_id()).await? {
        Err(Refusal::SelfReference) => return Err(ApiError::BadRequest),
        Err(Refusal::Loop) => return Err(ApiError::Conflict),
        Ok(changed) => republish(&state, pool, changed).await?,
    }
    Ok(Json(body(&state, artcc, true)))
}

/// Give a consolidated sector its own row back. A sector that isn't worked elsewhere is a no-op (200,
/// nothing written). Not checked against the dataset, so a consolidation left behind by a re-import
/// can still be released. Answers with the ARTCC's consolidations after the release.
#[utoipa::path(
    delete, path = "/api/v1/flow/sector-consolidations/{artcc}/{sector_id}", tag = "flow",
    params(
        ("artcc" = String, Path, description = "ARTCC id, case-insensitive"),
        ("sector_id" = String, Path, description = "The sector to work on its own again")
    ),
    responses(
        (status = 200, body = SectorConsolidationsBody, description = "The ARTCC's consolidations after the release"),
        (status = 401, description = "Not signed in, or without `flow.sector_consolidations.update`"),
        (status = 403, description = "The caller's `flow.sector_consolidations.update` does not cover this ARTCC"),
        (status = 503)
    ),
    security(("session" = ["flow.sector_consolidations.update"]), ("api_key" = ["flow.sector_consolidations.update"]), ("service_account" = ["flow.sector_consolidations.update"]))
)]
pub async fn release_sector(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowSectorConsolidationsUpdate>,
    Actor(principal): Actor,
    Path((artcc, sector_id)): Path<(String, String)>,
) -> Result<Json<SectorConsolidationsBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let artcc = artcc.trim().to_ascii_uppercase();
    if !may_edit(&state, &principal, &artcc).await? {
        return Err(ApiError::Forbidden);
    }
    let changed = repo::release(pool, &artcc, sector_id.trim()).await?;
    republish(&state, pool, changed).await?;
    Ok(Json(body(&state, artcc, true)))
}
