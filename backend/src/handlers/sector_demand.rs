//! Sector demand (#725, epic #720): what the Operations page draws. Per sector of one ARTCC, the
//! occupancy engine's peak one-minute counts in each Zulu quarter-hour over six hours, with consolidation
//! (#723) applied and each bin judged against the row's limit (#722), split into an enroute and a TRACON
//! table.
//!
//! Read-only and gated `flow.sectors.read`, like the limit and consolidation reads. Computed per request
//! from the caches: the feed snapshot, the sector table, limits, consolidations and exclusions. Two
//! queries: the locked wheels-up of the grounded flights (`repos::flow::locked_wheels_up`) and the
//! active facilities that filter the neighbour list. Projection and binning run under `spawn_blocking`.
//!
//! The page refetches on `feed.tick` (a new cycle), `flow.sector_limits` and
//! `flow.sector_consolidations` (rows recolour or merge), and on `flow.release`, `flow.cfr` and
//! `tmu.gdp` (a wheels-up moved, so the proposed counts did). See `docs/features/monitor.md`.

use std::collections::{HashMap, HashSet};

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
        sector_load::{BIN_MIN, SectorLoad, Track, sector_loads},
        sector_tracks::{AIRBORNE_GS_KT, Bbox, project_tracks},
        sectors::{APPROACH_TIER, SectorTable},
        vatsim::VatsimData,
    },
    handlers::{
        flow::all_excluded_callsigns, sector_consolidations::SECTOR_CONSOLIDATIONS_UPDATE,
        sector_limits::SECTOR_LIMITS_UPDATE,
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
    loads: Vec<SectorLoad>,
    table: &SectorTable,
    artcc: &str,
    limits: &SectorLimits,
) -> (Vec<SectorDemandRow>, Vec<SectorDemandRow>) {
    let names: HashMap<String, Option<String>> = table.sectors_of(artcc).into_iter().collect();
    let (mut enroute, mut tracon) = (Vec::new(), Vec::new());
    for load in loads {
        let limit = row_limit(limits, &load);
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
            consolidated: load.consolidated,
            tier: load.tier,
            sector_id: load.sector_id,
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
    let nav = state.nav.load_full();
    let profiles = state.aircraft_profiles.load_full();
    let winds = state.winds.load_full();
    let consolidations = state.sector_consolidations.load_full();
    // The cycle is the clock: positions are as of the snapshot, so the bins start from it too.
    let cycle_at = snapshot.fetched_at;
    let now_ms = cycle_at.timestamp_millis();

    let loads = {
        let (table, artcc) = (table.clone(), artcc.clone());
        tokio::task::spawn_blocking(move || {
            let owned = project_tracks(
                &snapshot.data,
                &nav,
                &airports,
                &profiles,
                &winds,
                &wheels_up,
                &excluded,
                now_ms,
                Bbox::of_artcc(&table, &artcc),
            );
            let tracks: Vec<Track> = owned
                .iter()
                .map(|t| Track {
                    id: &t.id,
                    population: t.population,
                    fixes: &t.fixes,
                })
                .collect();
            // The whole table, never this ARTCC's slice: TRACON precedence is global (#726).
            sector_loads(&table, &consolidations, &tracks, now_ms)
                .into_iter()
                .filter(|l| l.artcc == artcc)
                .collect::<Vec<_>>()
        })
        .await
        .map_err(|_| ApiError::Internal)?
    };

    body.status = SectorDemandStatus::Ready;
    body.cycle_at = Some(cycle_at);
    body.bin_starts_ms = loads
        .first()
        .map(|l| l.bins.iter().map(|b| b.start_ms).collect())
        .unwrap_or_default();
    let (enroute, tracon) = tables(loads, &table, &artcc, &state.sector_limits.load());
    body.enroute.rows = enroute;
    body.tracon.rows = tracon;
    Ok(Json(body))
}
