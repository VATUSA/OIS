//! Ground Delay Program engine — Ration-By-Schedule (RBS) assignment of control times to
//! inbound traffic, metering an airport's arrival demand down to its AAR.
//!
//! Each inbound is ordered by ETA and slotted into an arrival stream spaced at the AAR
//! interval. Airborne flights (and out-of-window / out-of-tier flights) are **exempt** — they
//! keep their ETA and receive no ground hold, but still consume a slot. **Controlled** flights
//! (not-yet-departed, in scope) take the next open slot at/after their ETA → Controlled Time
//! of Arrival (CTA); their EDCT (controlled wheels-up) is that CTA minus their enroute time.
//!
//! Pure + unit-tested. The handler projects the live feed into `Inbound`s, runs RBS, applies
//! any frozen (persisted-at-publish) control times, then bins demand and tallies delay.

use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;

/// Demand bins are 15 minutes wide.
pub const BIN_MIN: i64 = 15;
/// A bin at/above `cap * these` is amber / red.
const YELLOW_FACTOR: f64 = 1.0;
const RED_FACTOR: f64 = 1.25;

/// Why an inbound was exempted from control.
pub const EXEMPT_AIRBORNE: &str = "airborne";
pub const EXEMPT_WINDOW: &str = "outside window";
pub const EXEMPT_TIER: &str = "out of scope";

/// One inbound flight fed into RBS, projected from the live arrival flow.
#[derive(Debug, Clone)]
pub struct Inbound {
    pub cs: String,
    pub dep: String,
    /// `airborne` | `ground` | `proposed`.
    pub status: String,
    pub eta_ms: i64,
    /// Estimated wheels-up (ground/proposed inbounds); `None` once airborne.
    pub etd_ms: Option<i64>,
}

/// The RBS result for one inbound (control times in epoch-ms; the handler converts to
/// timestamps and may override with frozen values).
#[derive(Debug, Clone)]
pub struct Assignment {
    pub cs: String,
    pub dep: String,
    pub status: String,
    pub original_eta_ms: i64,
    /// Controlled Time of Arrival (assigned slot). For exempt flights this is their ETA.
    pub cta_ms: i64,
    /// Controlled wheels-up = CTA − enroute; `None` for exempt (no ground hold).
    pub edct_ms: Option<i64>,
    pub delay_min: i64,
    pub controlled: bool,
    /// True once the control time comes from a persisted (frozen-at-publish) slot.
    pub frozen: bool,
    /// Set for exempt flights: why they weren't controlled.
    pub exempt_reason: Option<String>,
}

/// Assign control times to `inbounds` via Ration-By-Schedule, metering to `aar` across the
/// program window `[window_start_ms, window_end_ms]`.
pub fn ration_by_schedule(
    mut inbounds: Vec<Inbound>,
    aar: i32,
    window_start_ms: i64,
    window_end_ms: i64,
    exempt_airborne: bool,
    max_enroute_min: Option<i32>,
) -> Vec<Assignment> {
    let slot_ms = (3_600_000_f64 / aar.max(1) as f64) as i64; // spacing between arrival slots
    inbounds.sort_by_key(|f| f.eta_ms); // earliest demand first

    // Next free arrival slot; advances as each flight (exempt or controlled) consumes one.
    let mut cursor = window_start_ms;
    let mut out = Vec::with_capacity(inbounds.len());
    for f in inbounds {
        let enroute_ms = f.etd_ms.map(|etd| (f.eta_ms - etd).max(0));
        let enroute_min = enroute_ms.map(|ms| ms / 60_000);

        let exempt_reason = if exempt_airborne && f.status == "airborne" {
            Some(EXEMPT_AIRBORNE.to_string())
        } else if f.eta_ms < window_start_ms || f.eta_ms > window_end_ms {
            Some(EXEMPT_WINDOW.to_string())
        } else if matches!((max_enroute_min, enroute_min), (Some(max), Some(er)) if er > max as i64)
        {
            Some(EXEMPT_TIER.to_string())
        } else {
            None
        };

        // The slot this flight lands in — no earlier than its own ETA or the running cursor.
        let slot = cursor.max(f.eta_ms);
        cursor = slot + slot_ms; // consume the slot either way

        if exempt_reason.is_some() {
            // Exempt: arrives at its own ETA, no delay, no EDCT.
            out.push(Assignment {
                cs: f.cs,
                dep: f.dep,
                status: f.status,
                original_eta_ms: f.eta_ms,
                cta_ms: f.eta_ms,
                edct_ms: None,
                delay_min: 0,
                controlled: false,
                frozen: false,
                exempt_reason,
            });
        } else {
            let cta_ms = slot;
            let delay_min = ((cta_ms - f.eta_ms).max(0)) / 60_000;
            let edct_ms = enroute_ms.map(|er| cta_ms - er);
            out.push(Assignment {
                cs: f.cs,
                dep: f.dep,
                status: f.status,
                original_eta_ms: f.eta_ms,
                cta_ms,
                edct_ms,
                delay_min,
                controlled: true,
                frozen: false,
                exempt_reason: None,
            });
        }
    }
    out
}

/// Recompute a frozen flight's control times after compression: the CTA is pulled to
/// `fresh_cta` when that's earlier but never pushed later than the already-issued
/// `frozen_cta_ms` (a controller/crew can absorb an earlier release, not a later one). The
/// enroute time is held constant (backed out of the fresh assignment). Returns
/// `(cta_ms, edct_ms, delay_min)`.
pub fn compress_slot(fresh: &Assignment, frozen_cta_ms: i64) -> (i64, Option<i64>, i64) {
    let cta = fresh.cta_ms.min(frozen_cta_ms);
    let enroute = fresh.edct_ms.map(|e| fresh.cta_ms - e); // cta − edct
    let edct = enroute.map(|er| cta - er);
    let delay_min = ((cta - fresh.original_eta_ms).max(0)) / 60_000;
    (cta, edct, delay_min)
}

/// Per-bin arrival demand vs the AAR-derived capacity, for the program window.
#[derive(Debug, Serialize, ToSchema)]
pub struct GdpDemand {
    pub start: DateTime<Utc>,
    pub count: i64,
    /// Slots available in this bin (AAR × bin width).
    pub cap: i64,
    /// `green` | `yellow` | `red`.
    pub level: String,
}

/// Bin every assignment (controlled + exempt, by its CTA) against per-bin capacity.
pub fn demand_bins(
    assignments: &[Assignment],
    window_start_ms: i64,
    window_end_ms: i64,
    aar: i32,
) -> Vec<GdpDemand> {
    let bin_ms = BIN_MIN * 60_000;
    let span = (window_end_ms - window_start_ms).max(0);
    let bins = ((span as f64) / (bin_ms as f64)).ceil() as usize;
    let cap = ((aar.max(1) as f64) * (BIN_MIN as f64) / 60.0).round() as i64;

    let mut counts = vec![0i64; bins];
    for a in assignments {
        let idx = (a.cta_ms - window_start_ms) / bin_ms;
        if idx >= 0 && (idx as usize) < bins {
            counts[idx as usize] += 1;
        }
    }
    counts
        .into_iter()
        .enumerate()
        .map(|(i, count)| {
            let level = if count as f64 <= cap as f64 * YELLOW_FACTOR {
                "green"
            } else if count as f64 <= cap as f64 * RED_FACTOR {
                "yellow"
            } else {
                "red"
            };
            GdpDemand {
                start: DateTime::from_timestamp_millis(window_start_ms + (i as i64) * bin_ms)
                    .unwrap_or_else(Utc::now),
                count,
                cap,
                level: level.to_string(),
            }
        })
        .collect()
}

/// Program-wide delay tallies over the controlled flights.
#[derive(Debug, Serialize, ToSchema)]
pub struct GdpStats {
    pub controlled: i64,
    pub exempt: i64,
    pub avg_delay_min: i64,
    pub max_delay_min: i64,
    pub total_delay_min: i64,
}

pub fn program_stats(assignments: &[Assignment]) -> GdpStats {
    let controlled: Vec<&Assignment> = assignments.iter().filter(|a| a.controlled).collect();
    let exempt = assignments.len() - controlled.len();
    let total: i64 = controlled.iter().map(|a| a.delay_min).sum();
    let max = controlled.iter().map(|a| a.delay_min).max().unwrap_or(0);
    let avg = if controlled.is_empty() {
        0
    } else {
        total / controlled.len() as i64
    };
    GdpStats {
        controlled: controlled.len() as i64,
        exempt: exempt as i64,
        avg_delay_min: avg,
        max_delay_min: max,
        total_delay_min: total,
    }
}

/// One flight on the GDP board — a controlled/exempt inbound with its control times joined
/// to current live state.
#[derive(Debug, Serialize, ToSchema)]
pub struct GdpFlightView {
    pub cs: String,
    pub dep: String,
    /// Current live status: `airborne` | `ground` | `proposed`.
    pub status: String,
    /// Current estimated time of arrival.
    pub eta: DateTime<Utc>,
    /// Controlled Time of Arrival (assigned slot); equals ETA for exempt flights.
    pub cta: DateTime<Utc>,
    /// Controlled wheels-up (EDCT); null for exempt flights.
    pub edct: Option<DateTime<Utc>>,
    pub delay_min: i64,
    pub controlled: bool,
    /// True once the control time is frozen (persisted at publish).
    pub frozen: bool,
    /// Set for exempt flights: why they weren't controlled.
    pub exempt_reason: Option<String>,
}

/// The full GDP board: the program, its window, controlled + exempt flights, demand vs AAR,
/// and delay stats — everything the frontend needs in one payload.
#[derive(Debug, Serialize, ToSchema)]
pub struct GdpBoard {
    pub id: String,
    pub airport: String,
    pub aar: i32,
    pub status: String,
    pub start_time: String,
    pub end_time: String,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub max_enroute_min: Option<i32>,
    pub exempt_airborne: bool,
    /// True when control times are frozen (program published).
    pub published: bool,
    /// Controlled flights (frozen when published, advisory when draft), sorted by CTA.
    pub flights: Vec<GdpFlightView>,
    /// Exempt inbounds (airborne / out-of-window / out-of-scope), sorted by ETA.
    pub exempt: Vec<GdpFlightView>,
    pub demand: Vec<GdpDemand>,
    pub stats: GdpStats,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ground(cs: &str, eta_ms: i64, etd_ms: i64) -> Inbound {
        Inbound {
            cs: cs.into(),
            dep: "KXXX".into(),
            status: "ground".into(),
            eta_ms,
            etd_ms: Some(etd_ms),
        }
    }

    const MIN: i64 = 60_000;
    const HOUR: i64 = 3_600_000;

    #[test]
    fn slots_space_at_aar_and_delay_accrues() {
        // AAR 30 → one slot every 2 minutes. Four flights all wanting to land at t=0.
        let inbounds = vec![
            ground("A", 0, -30 * MIN),
            ground("B", 0, -30 * MIN),
            ground("C", 0, -30 * MIN),
            ground("D", 0, -30 * MIN),
        ];
        let out = ration_by_schedule(inbounds, 30, 0, HOUR, true, None);
        let ctas: Vec<i64> = out.iter().map(|a| a.cta_ms / MIN).collect();
        assert_eq!(ctas, vec![0, 2, 4, 6]); // 2-min spacing
        let delays: Vec<i64> = out.iter().map(|a| a.delay_min).collect();
        assert_eq!(delays, vec![0, 2, 4, 6]);
        assert!(out.iter().all(|a| a.controlled));
    }

    #[test]
    fn edct_backs_out_enroute_time() {
        // Enroute 30 min, must be delayed to land at t=10min → EDCT = CTA − 30min.
        let inbounds = vec![
            ground("X", 10 * MIN, 10 * MIN - 30 * MIN), // eta 10min, etd -20min → enroute 30min
            ground("Y", 0, -30 * MIN),                  // eta 0, enroute 30min
        ];
        // AAR 30 (2-min slots). Y lands at 0, X wants 10min and is free → no delay for X.
        let out = ration_by_schedule(inbounds, 30, 0, HOUR, true, None);
        let x = out.iter().find(|a| a.cs == "X").unwrap();
        assert_eq!(x.delay_min, 0);
        assert_eq!(x.cta_ms, 10 * MIN);
        assert_eq!(x.edct_ms, Some(10 * MIN - 30 * MIN)); // CTA − enroute
    }

    #[test]
    fn airborne_is_exempt_no_edct() {
        let inbounds = vec![Inbound {
            cs: "AIR1".into(),
            dep: "KYYY".into(),
            status: "airborne".into(),
            eta_ms: 20 * MIN,
            etd_ms: None,
        }];
        let out = ration_by_schedule(inbounds, 30, 0, HOUR, true, None);
        let a = &out[0];
        assert!(!a.controlled);
        assert_eq!(a.exempt_reason.as_deref(), Some(EXEMPT_AIRBORNE));
        assert_eq!(a.cta_ms, 20 * MIN); // keeps its ETA
        assert_eq!(a.edct_ms, None);
        assert_eq!(a.delay_min, 0);
    }

    #[test]
    fn out_of_window_and_tier_are_exempt() {
        let inbounds = vec![
            ground("LATE", 2 * HOUR, 90 * MIN), // ETA past the 1h window end
            ground("FAR", 30 * MIN, 30 * MIN - 90 * MIN), // enroute 90min > tier 60
        ];
        let out = ration_by_schedule(inbounds, 30, 0, HOUR, true, Some(60));
        let late = out.iter().find(|a| a.cs == "LATE").unwrap();
        assert_eq!(late.exempt_reason.as_deref(), Some(EXEMPT_WINDOW));
        let far = out.iter().find(|a| a.cs == "FAR").unwrap();
        assert_eq!(far.exempt_reason.as_deref(), Some(EXEMPT_TIER));
        assert!(out.iter().all(|a| !a.controlled));
    }

    #[test]
    fn exempt_flights_still_consume_slots() {
        // An airborne flight at t=0 occupies the first slot; the controlled ground flight
        // behind it (also wanting t=0) is pushed to the next slot.
        let inbounds = vec![
            Inbound {
                cs: "AIR".into(),
                dep: "KZZZ".into(),
                status: "airborne".into(),
                eta_ms: 0,
                etd_ms: None,
            },
            ground("GND", 0, -30 * MIN),
        ];
        let out = ration_by_schedule(inbounds, 30, 0, HOUR, true, None);
        let gnd = out.iter().find(|a| a.cs == "GND").unwrap();
        assert_eq!(gnd.cta_ms, 2 * MIN); // bumped one slot behind the exempt arrival
        assert_eq!(gnd.delay_min, 2);
    }

    #[test]
    fn demand_bins_flag_over_capacity() {
        // AAR 4 → cap 1 per 15-min bin. Three flights in the first bin → red.
        let inbounds = vec![
            ground("A", 1 * MIN, 0),
            ground("B", 2 * MIN, 0),
            ground("C", 3 * MIN, 0),
        ];
        let out = ration_by_schedule(inbounds, 4, 0, HOUR, true, None);
        let bins = demand_bins(&out, 0, HOUR, 4);
        assert_eq!(bins[0].cap, 1);
        assert!(bins[0].count >= 1);
        assert_eq!(bins.len(), 4); // 60min / 15min
    }

    #[test]
    fn compress_pulls_earlier_never_later() {
        // Fresh RBS now lands this flight at t=5min (enroute 30min → EDCT −25min), but it was
        // frozen at t=20min. Compression pulls it to 5min.
        let fresh = Assignment {
            cs: "A".into(),
            dep: "KXXX".into(),
            status: "ground".into(),
            original_eta_ms: 0,
            cta_ms: 5 * MIN,
            edct_ms: Some(5 * MIN - 30 * MIN),
            delay_min: 5,
            controlled: true,
            frozen: false,
            exempt_reason: None,
        };
        let (cta, edct, delay) = compress_slot(&fresh, 20 * MIN);
        assert_eq!(cta, 5 * MIN); // pulled earlier
        assert_eq!(edct, Some(5 * MIN - 30 * MIN)); // enroute (30m) held constant
        assert_eq!(delay, 5);
        // If fresh RBS would push it later than the frozen time, keep the frozen time.
        let (cta2, _e2, _d2) = compress_slot(&fresh, 3 * MIN);
        assert_eq!(cta2, 3 * MIN); // clamped to the earlier frozen CTA
    }

    #[test]
    fn stats_tally_controlled_delay() {
        let inbounds = vec![
            ground("A", 0, -30 * MIN),
            ground("B", 0, -30 * MIN),
            ground("C", 0, -30 * MIN),
        ];
        let out = ration_by_schedule(inbounds, 30, 0, HOUR, true, None); // delays 0,2,4
        let s = program_stats(&out);
        assert_eq!(s.controlled, 3);
        assert_eq!(s.exempt, 0);
        assert_eq!(s.total_delay_min, 6);
        assert_eq!(s.max_delay_min, 4);
        assert_eq!(s.avg_delay_min, 2);
    }
}
