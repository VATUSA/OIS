//! Sector limits (#722, epic #720): how many aircraft one controller can work in a sector, and the
//! level a bin's occupancy reads against it.
//!
//! Every sector reads [`DEFAULT_LIMIT`] until a TMU at its ARTCC overrides it. Overrides are keyed per
//! `(artcc, sector_id)` — a sector is one workload however many volumes its airspace is in — and cached
//! in `AppState::sector_limits` for this DB-less module: reloaded by `jobs::spawn_sector_limits_refresh`
//! and force-reloaded by `handlers::sector_limits` after every write.
//!
//! The level is computed here, server-side, so everyone watching a facility sees the same colours.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::feed::sector_load::BinPeak;

/// The limit a sector reads until it is overridden. Real high-sector values run about 16–20; 10 is a
/// deliberate tuning for VATSIM traffic levels, carried over from vTBFM.
pub const DEFAULT_LIMIT: i32 = 10;

/// Stored overrides by `(artcc, sector_id)`, as cached in `AppState::sector_limits`. A sector with no
/// entry reads [`DEFAULT_LIMIT`].
pub type SectorLimits = HashMap<(String, String), i32>;

/// A sector's limit: its override, or [`DEFAULT_LIMIT`].
pub fn limit_for(limits: &SectorLimits, artcc: &str, sector_id: &str) -> i32 {
    limits
        .get(&(artcc.to_string(), sector_id.to_string()))
        .copied()
        .unwrap_or(DEFAULT_LIMIT)
}

/// A bin's load against its sector's limit, drawn as `--level-ok` / `--level-watch` / `--level-over`
/// (`SectorGrid`'s `LoadLevel` in `@ois/ui`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SectorLoadLevel {
    /// Neither active alone nor active + proposed exceeds the limit.
    Ok,
    /// Only active + proposed exceeds it: still preventable by holding departures.
    Watch,
    /// Active alone exceeds it: the overload is already locked in.
    Over,
}

/// The level of one bin against `limit`. Strictly greater than: a peak **equal** to the limit is
/// [`SectorLoadLevel::Ok`]. [`Over`](SectorLoadLevel::Over) when the active peak alone exceeds the
/// limit, else [`Watch`](SectorLoadLevel::Watch) when the combined peak does. `combined` is the
/// engine's minute-by-minute peak of active + proposed, never `active + proposed`.
pub fn level(bin: &BinPeak, limit: i32) -> SectorLoadLevel {
    // Widen both sides: `usize` and `i32` meet in `i64` without truncation either way.
    let limit = i64::from(limit);
    let exceeds = |n: usize| i64::try_from(n).unwrap_or(i64::MAX) > limit;
    if exceeds(bin.active) {
        SectorLoadLevel::Over
    } else if exceeds(bin.combined) {
        SectorLoadLevel::Watch
    } else {
        SectorLoadLevel::Ok
    }
}
