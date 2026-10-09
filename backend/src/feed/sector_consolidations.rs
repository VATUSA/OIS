//! Sector consolidation (#723, epic #720): sectors worked at another sector's position.
//!
//! A consolidated sector has no row of its own. Its volumes are filed under its target's row **before**
//! counting ([`sector_loads`](super::sector_load::sector_loads)), so the combined row counts distinct
//! flights per minute across the union of the airspace — never the sum of the rows — and is judged
//! against the **target's** limit ([`row_limit`]): one controller, one workload.
//!
//! Cached in `AppState::sector_consolidations` for this DB-less module: reloaded by
//! `jobs::spawn_sector_consolidations_refresh` and force-reloaded by `handlers::sector_consolidations`
//! after every write. The one writer (`repos::sector_consolidations::consolidate`) keeps it flat — a
//! target is never itself a source — and same-ARTCC by construction.

use std::collections::HashMap;

use crate::feed::{
    sector_limits::{SectorLimits, limit_for},
    sector_load::SectorLoad,
};

/// `(artcc, source) → target`, same ARTCC, already flattened.
pub type SectorConsolidations = HashMap<(String, String), String>;

/// The sector whose row `sector_id`'s airspace counts under: its target if it is worked elsewhere,
/// else itself.
pub fn row_of<'a>(
    consolidations: &'a SectorConsolidations,
    artcc: &str,
    sector_id: &'a str,
) -> &'a str {
    consolidations
        .get(&(artcc.to_string(), sector_id.to_string()))
        .map_or(sector_id, String::as_str)
}

/// The limit a row is judged against: its own sector's, which for a combined row is the target's.
/// Never a sum of the sources' (that would make the busiest arrangement the hardest to alert) and
/// never their maximum.
pub fn row_limit(limits: &SectorLimits, load: &SectorLoad) -> i32 {
    limit_for(limits, &load.artcc, &load.sector_id)
}
