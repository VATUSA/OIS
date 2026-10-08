//! Sector demand (#725, epic #720): what the Operations page draws. Per sector of one ARTCC, the
//! occupancy engine's peak one-minute counts in each Zulu quarter-hour over six hours, with consolidation
//! (#723) applied and each bin judged against the row's limit (#722), split into an enroute and a TRACON
//! table.
//!
//! Read-only and gated `flow.sectors.read`, like the limit and consolidation reads. Computed from the
//! caches (the feed snapshot, the sector table, limits, consolidations and exclusions) once per ARTCC per
//! VATSIM publish or config change, not per request: see `handlers::sector_demand_cache`. The database is read for the caller's
//! two edit scopes, the locked wheels-up of the grounded flights (`repos::flow::locked_wheels_up`) and the
//! active facilities that filter the neighbour list.
//!
//! The page refetches on `feed.tick` (a new cycle), `flow.sector_limits` and
//! `flow.sector_consolidations` (rows recolour or merge), and on `flow.release`, `flow.cfr`, `tmu.gdp`
//! and `flow.fca` (a wheels-up moved, so the proposed counts did). See `docs/features/monitor.md`.

use std::{
    collections::{HashMap, HashSet},
    sync::atomic::Ordering,
};

use axum::{
    Json,
    extract::{Path, State},
};

use crate::{
    auth::{permissions::FlowSectorsRead, principal::Actor, require_permission::RequirePermission},
    errors::ApiError,
    feed::{
        neighbors,
        sector_consolidations::row_limit,
        sector_limits::{DEFAULT_LIMIT, SectorLimits, level},
        sector_load::{BIN_MIN, SectorLoad},
        sector_tracks::AIRBORNE_GS_KT,
        sectors::{APPROACH_TIER, SectorTable},
        vatsim::VatsimData,
    },
    handlers::{
        flow::all_excluded_callsigns, sector_consolidations::SECTOR_CONSOLIDATIONS_UPDATE,
        sector_demand_cache::Inputs, sector_limits::SECTOR_LIMITS_UPDATE,
    },
    models::{
        SectorDemandBin, SectorDemandBody, SectorDemandRow, SectorDemandStatus, SectorDemandTable,
    },
    repos::{flow as flow_repo, org as org_repo},
    state::AppState,
};

/// Whether `artcc` has any enroute volume, and any approach volume, in the dataset.
fn coverage(table: &SectorTable, artcc: &str) -> (bool, bool) {
    let mut volumes = table.volumes.iter().filter(|v| v.artcc == artcc);
    let enroute = volumes.clone().any(|v| v.tier != APPROACH_TIER);
    let tracon = volumes.any(|v| v.tier == APPROACH_TIER);
    (enroute, tracon)
}

/// A count as the contract carries it.
fn count(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

/// `loads` (one ARTCC's rows from the engine) as the page's two tables, each row judged against its
/// limit — a combined row against its target's ([`row_limit`]). Ordered by `sector_id` within each.
pub(crate) fn tables(
    loads: &[SectorLoad],
    table: &SectorTable,
    artcc: &str,
    limits: &SectorLimits,
) -> (Vec<SectorDemandRow>, Vec<SectorDemandRow>) {
    let names: HashMap<String, Option<String>> = table.sectors_of(artcc).into_iter().collect();
    let (mut enroute, mut tracon) = (Vec::new(), Vec::new());
    for load in loads {
        let limit = row_limit(limits, load);
        let row = SectorDemandRow {
            name: names.get(&load.sector_id).cloned().flatten(),
            limit,
            limit_overridden: limits.contains_key(&(load.artcc.clone(), load.sector_id.clone())),
            bins: load
                .bins
                .iter()
                .map(|bin| SectorDemandBin {
                    active: count(bin.active),
                    proposed: count(bin.proposed),
                    combined: count(bin.combined),
                    level: level(bin, limit),
                })
                .collect(),
            consolidated: load.consolidated.clone(),
            tier: load.tier.clone(),
            sector_id: load.sector_id.clone(),
        };
        if row.tier == APPROACH_TIER {
            tracon.push(row);
        } else {
            enroute.push(row);
        }
    }
    enroute.sort_by(|a, b| a.sector_id.cmp(&b.sector_id));
    tracon.sort_by(|a, b| a.sector_id.cmp(&b.sector_id));
    (enroute, tracon)
}

/// The callsigns that may hold a locked wheels-up: everyone on the ground with a flight plan, and
/// every prefile. Only these can be in the proposed population.
fn grounded_callsigns(data: &VatsimData, excluded: &HashSet<String>) -> Vec<String> {
    let pilots = data
        .pilots
        .iter()
        .filter(|p| p.groundspeed < AIRBORNE_GS_KT && p.flight_plan.is_some())
        .map(|p| &p.callsign);
    let prefiles = data
        .prefiles
        .iter()
        .filter(|p| p.flight_plan.is_some())
        .map(|p| &p.callsign);
    pilots
        .chain(prefiles)
        .filter(|c| !excluded.contains(*c))
        .cloned()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect()
}

/// `artcc`'s predicted sector demand: an enroute and a TRACON table, each row a sector (or a target with
/// the sectors worked at it) over 24 Zulu quarter-hours. An ARTCC with no sector data answers
/// `no_sector_data` and one asked before the first feed cycle answers `pending`, both with no rows, so
/// the page can say which rather than draw an empty grid.
#[utoipa::path(
    get, path = "/api/v1/flow/sector-demand/{artcc}", tag = "flow",
    params(("artcc" = String, Path, description = "ARTCC id, case-insensitive")),
    responses(
        (status = 200, body = SectorDemandBody),
        (status = 401, description = "Not signed in, or without `flow.sectors.read`"),
        (status = 503)
    ),
    security(("session" = ["flow.sectors.read"]), ("api_key" = ["flow.sectors.read"]), ("service_account" = ["flow.sectors.read"]))
)]
pub async fn get_sector_demand(
    State(state): State<AppState>,
    _permission: RequirePermission<FlowSectorsRead>,
    Actor(principal): Actor,
    Path(artcc): Path<String>,
) -> Result<Json<SectorDemandBody>, ApiError> {
    let pool = state.db.as_ref().ok_or(ApiError::ServiceUnavailable)?;
    let artcc = artcc.trim().to_ascii_uppercase();

    let limits_editable = principal
        .permission_scope(&state, SECTOR_LIMITS_UPDATE)
        .await?
        .allows(Some(&artcc));
    let consolidations_editable = principal
        .permission_scope(&state, SECTOR_CONSOLIDATIONS_UPDATE)
        .await?
        .allows(Some(&artcc));
    // OIS's active facilities: the filter that drops the Canadian and oceanic FIRs from the graph.
    let known: HashSet<String> = org_repo::list_facilities(pool)
        .await?
        .into_iter()
        .map(|f| f.id)
        .collect();
    let mut neighbours = neighbors::tier1(&artcc, &known);
    neighbours.sort();

    // Read before the table: the refresh job stores the table, then sets the flag.
    let sectors_loaded = state.airspace_sectors_loaded.load(Ordering::Acquire);
    let table = state.airspace_sectors.load_full();
    let (has_enroute, has_tracon) = coverage(&table, &artcc);
    let mut body = SectorDemandBody {
        artcc: artcc.clone(),
        status: SectorDemandStatus::NoSectorData,
        cycle_at: None,
        bin_minutes: BIN_MIN as i32,
        bin_starts_ms: Vec::new(),
        default_limit: DEFAULT_LIMIT,
        limits_editable,
        consolidations_editable,
        neighbours,
        enroute: SectorDemandTable {
            has_sector_data: has_enroute,
            rows: Vec::new(),
        },
        tracon: SectorDemandTable {
            has_sector_data: has_tracon,
            rows: Vec::new(),
        },
    };
    // Until the table has been read, its emptiness says nothing about this ARTCC, so this is `pending`
    // too, not `no_sector_data` for every ARTCC (#725 Q5).
    if !sectors_loaded {
        body.status = SectorDemandStatus::Pending;
        return Ok(Json(body));
    }
    if !has_enroute && !has_tracon {
        return Ok(Json(body));
    }

    let (snapshot, airports) = {
        let feed = state.feed.read().await;
        (feed.snapshot.clone(), feed.airports.clone())
    };
    let Some(snapshot) = snapshot else {
        body.status = SectorDemandStatus::Pending;
        return Ok(Json(body));
    };

    let excluded = all_excluded_callsigns(&state.flight_exclusions.load());
    let wheels_up =
        flow_repo::locked_wheels_up(pool, &grounded_callsigns(&snapshot.data, &excluded)).await?;
    let inputs = Inputs {
        snapshot,
        airports,
        nav: state.nav.load_full(),
        profiles: state.aircraft_profiles.load_full(),
        winds: state.winds.load_full(),
        table,
        consolidations: state.sector_consolidations.load_full(),
        excluded,
        wheels_up,
        limits: state.sector_limits.load_full(),
    };
    let demand = state.sector_demand.demand(&artcc, inputs).await?;

    body.status = SectorDemandStatus::Ready;
    body.cycle_at = Some(demand.cycle_at);
    body.bin_starts_ms = demand.bin_starts_ms.clone();
    body.enroute.rows = demand.enroute.clone();
    body.tracon.rows = demand.tracon.clone();
    Ok(Json(body))
}
