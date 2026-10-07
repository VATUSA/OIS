//! Sector demand computed once per feed snapshot (#725), not once per request.
//!
//! An ARTCC's demand projects six hours of every flight near it (`sector_tracks::project_tracks`) and
//! bins them against the whole sector table (`sector_loads`): 32–125 ms of CPU per ARTCC in a release
//! build, measured against a captured 1,486-pilot feed. Every viewer of an ARTCC, as its own facility or as someone's neighbour, asks for the same numbers, and each
//! open table refetches on every feed tick and on four flow topics. So the projection runs once per
//! ARTCC per set of inputs and every other request reads it.
//!
//! **Keyed on everything it reads.** An entry is reused only while each input is the one it was built
//! from:
//! - the feed snapshot, the airport, nav, aircraft-profile and wind tables and the sector table, by
//!   identity: a new feed cycle or a refresh job's reload replaces the `Arc`, and only then. The entry holds
//!   them as `Weak`, which keeps each address from being reused without keeping an old snapshot alive;
//! - the consolidations, by value: their refresh job reloads every 30 s whether or not anything changed,
//!   and a write force-reloads them (`handlers::sector_consolidations`), so a real change misses at once
//!   and a no-op reload doesn't;
//! - the excluded callsigns and the grounded flights' locked wheels-up (read from the database per
//!   request), by value, so a release, CFR, GDP or FCA write shows on the next read in the same cycle;
//! - the limits, by value, but they only colour the rows: a limit write (force-reloaded by
//!   `handlers::sector_limits`) re-renders the rows from the cached projection and does not re-project.
//!
//! **Computed on demand, once.** Nothing is computed for an ARTCC nobody is looking at, and the feed
//! tick stays as cheap as it was: the first request after a change computes, holding that ARTCC's slot,
//! and concurrent requests for it wait for that result rather than each projecting (single-flight).
//! Different ARTCCs compute in parallel. The projection runs under `spawn_blocking`, inside a task that
//! owns the slot until the entry is written, so a request dropped mid-projection (its client went away)
//! doesn't throw the result away for the requests waiting behind it.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Weak,
        atomic::{AtomicU64, Ordering},
    },
};

use chrono::{DateTime, Utc};

use crate::{
    errors::ApiError,
    feed::{
        Snapshot,
        airports::AirportDb,
        nav::NavData,
        sector_consolidations::SectorConsolidations,
        sector_limits::SectorLimits,
        sector_load::{SectorLoad, Track, sector_loads},
        sector_tracks::{Bbox, project_tracks},
        sectors::SectorTable,
        trajectory::ProfileTable,
        winds::Winds,
    },
    handlers::sector_demand::tables,
    models::SectorDemandRow,
};

/// Everything one ARTCC's demand is computed from, as the handler read it for this request.
pub(crate) struct Inputs {
    pub snapshot: Arc<Snapshot>,
    pub airports: Arc<AirportDb>,
    pub nav: Arc<NavData>,
    pub profiles: Arc<ProfileTable>,
    pub winds: Arc<Winds>,
    pub table: Arc<SectorTable>,
    pub consolidations: Arc<SectorConsolidations>,
    pub excluded: HashSet<String>,
    pub wheels_up: HashMap<String, i64>,
    pub limits: Arc<SectorLimits>,
}

/// An ARTCC's demand as the page draws it: the two tables' rows, judged against the limits.
#[derive(Debug)]
pub(crate) struct Demand {
    pub cycle_at: DateTime<Utc>,
    pub bin_starts_ms: Vec<i64>,
    pub enroute: Vec<SectorDemandRow>,
    pub tracon: Vec<SectorDemandRow>,
}

/// The inputs a projection was built from (see the module docs for why each is compared as it is).
struct Key {
    snapshot: Weak<Snapshot>,
    airports: Weak<AirportDb>,
    nav: Weak<NavData>,
    profiles: Weak<ProfileTable>,
    winds: Weak<Winds>,
    table: Weak<SectorTable>,
    consolidations: Arc<SectorConsolidations>,
    excluded: HashSet<String>,
    wheels_up: HashMap<String, i64>,
}

/// Whether `held` was taken from `current`. A `Weak` keeps its allocation, so no other value can be
/// at that address while the entry holds it.
fn same<T>(held: &Weak<T>, current: &Arc<T>) -> bool {
    std::ptr::eq(held.as_ptr(), Arc::as_ptr(current))
}

/// The same value, whether or not it was reloaded.
fn equal<T: PartialEq>(held: &Arc<T>, current: &Arc<T>) -> bool {
    Arc::ptr_eq(held, current) || **held == **current
}

impl Key {
    fn of(inputs: &Inputs) -> Self {
        Self {
            snapshot: Arc::downgrade(&inputs.snapshot),
            airports: Arc::downgrade(&inputs.airports),
            nav: Arc::downgrade(&inputs.nav),
            profiles: Arc::downgrade(&inputs.profiles),
            winds: Arc::downgrade(&inputs.winds),
            table: Arc::downgrade(&inputs.table),
            consolidations: inputs.consolidations.clone(),
            excluded: inputs.excluded.clone(),
            wheels_up: inputs.wheels_up.clone(),
        }
    }

    fn matches(&self, inputs: &Inputs) -> bool {
        same(&self.snapshot, &inputs.snapshot)
            && same(&self.airports, &inputs.airports)
            && same(&self.nav, &inputs.nav)
            && same(&self.profiles, &inputs.profiles)
            && same(&self.winds, &inputs.winds)
            && same(&self.table, &inputs.table)
            && equal(&self.consolidations, &inputs.consolidations)
            && self.excluded == inputs.excluded
            && self.wheels_up == inputs.wheels_up
    }
}

/// One ARTCC's projection and the rows last rendered from it.
struct Entry {
    key: Key,
    cycle_at: DateTime<Utc>,
    loads: Arc<Vec<SectorLoad>>,
    limits: Arc<SectorLimits>,
    demand: Arc<Demand>,
}

type Slot = Arc<tokio::sync::Mutex<Option<Entry>>>;

/// `AppState::sector_demand`: the last demand computed for each ARTCC that has been asked for. Bounded by
/// the ARTCCs with sector data, since the handler answers the rest without it.
#[derive(Default)]
pub struct SectorDemandCache {
    slots: std::sync::Mutex<HashMap<String, Slot>>,
    projections: AtomicU64,
    renders: AtomicU64,
}

impl SectorDemandCache {
    /// How many times an ARTCC's flights have been projected and binned.
    #[cfg(test)]
    pub(crate) fn projections(&self) -> u64 {
        self.projections.load(Ordering::Relaxed)
    }

    /// How many times rows have been judged against the limits, with or without a projection.
    #[cfg(test)]
    pub(crate) fn renders(&self) -> u64 {
        self.renders.load(Ordering::Relaxed)
    }

    fn slot(&self, artcc: &str) -> Slot {
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        slots.entry(artcc.to_string()).or_default().clone()
    }

    /// `artcc`'s demand from `inputs`: the held one when nothing it was built from has changed, its rows
    /// re-judged when only the limits have, and a fresh projection otherwise.
    pub(crate) async fn demand(
        self: &Arc<Self>,
        artcc: &str,
        inputs: Inputs,
    ) -> Result<Arc<Demand>, ApiError> {
        // Held across the projection: this is the single flight.
        let mut held = self.slot(artcc).lock_owned().await;
        if let Some(entry) = held.as_mut()
            && entry.key.matches(&inputs)
        {
            if !equal(&entry.limits, &inputs.limits) {
                entry.demand = Arc::new(self.render(&entry.loads, &inputs, artcc));
                entry.limits = inputs.limits.clone();
            }
            return Ok(entry.demand.clone());
        }

        // The slot moves into the task, so dropping this request leaves the projection and the write
        // running, and the next request in line reads the entry instead of projecting again.
        let (cache, artcc) = (Arc::clone(self), artcc.to_string());
        tokio::spawn(async move {
            let loads = Arc::new(cache.project(&artcc, &inputs).await?);
            let demand = Arc::new(cache.render(&loads, &inputs, &artcc));
            // A request that read the feed just before a new cycle landed must not replace that cycle's
            // entry with its older one: it gets its own answer and the newer entry stays.
            let cycle_at = inputs.snapshot.fetched_at;
            if held.as_ref().is_none_or(|e| e.cycle_at <= cycle_at) {
                *held = Some(Entry {
                    key: Key::of(&inputs),
                    cycle_at,
                    loads,
                    limits: inputs.limits.clone(),
                    demand: demand.clone(),
                });
            }
            Ok(demand)
        })
        .await
        .map_err(|_| ApiError::Internal)?
    }

    /// Project the flights near `artcc` and bin them against the whole table, keeping `artcc`'s rows.
    async fn project(&self, artcc: &str, inputs: &Inputs) -> Result<Vec<SectorLoad>, ApiError> {
        self.projections.fetch_add(1, Ordering::Relaxed);
        let snapshot = inputs.snapshot.clone();
        let (airports, nav, profiles, winds, table, consolidations) = (
            inputs.airports.clone(),
            inputs.nav.clone(),
            inputs.profiles.clone(),
            inputs.winds.clone(),
            inputs.table.clone(),
            inputs.consolidations.clone(),
        );
        let (excluded, wheels_up) = (inputs.excluded.clone(), inputs.wheels_up.clone());
        let owner = artcc.to_string();
        let started = std::time::Instant::now();
        let loads = tokio::task::spawn_blocking(move || {
            // The cycle is the clock: positions are as of the snapshot, so the bins start from it too.
            let now_ms = snapshot.fetched_at.timestamp_millis();
            let owned = project_tracks(
                &snapshot.data,
                &nav,
                &airports,
                &profiles,
                &winds,
                &wheels_up,
                &excluded,
                now_ms,
                Bbox::of_artcc(&table, &owner),
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
                .filter(|l| l.artcc == owner)
                .collect::<Vec<_>>()
        })
        .await
        .map_err(|_| ApiError::Internal)?;
        tracing::debug!(
            artcc,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "sector demand projected"
        );
        Ok(loads)
    }

    fn render(&self, loads: &[SectorLoad], inputs: &Inputs, artcc: &str) -> Demand {
        self.renders.fetch_add(1, Ordering::Relaxed);
        let (enroute, tracon) = tables(loads, &inputs.table, artcc, &inputs.limits);
        Demand {
            cycle_at: inputs.snapshot.fetched_at,
            bin_starts_ms: loads
                .first()
                .map(|l| l.bins.iter().map(|b| b.start_ms).collect())
                .unwrap_or_default(),
            enroute,
            tracon,
        }
    }
}
