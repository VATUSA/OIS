//! The Monitor's alert ladder (VATUSA/OIS#600): how a sector's peak count compares with its Monitor
//! Alert Parameter, in the three states the FAA Monitor draws (vTBFM manual §10.4).
//!
//! Evaluated per request, like `gdp::demand_bins`: the alert is a pure function of the binned peaks,
//! which are recomputed from the live feed caches on every read. There is no alert-state table, and a
//! cleared overload simply stops matching. Edge detection ("went red at 1420Z") would need a stateful
//! job and isn't built.

use serde::Serialize;

/// One cell of the Monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum SectorAlert {
    /// Neither peak exceeds the MAP.
    Green,
    /// Only airborne + proposed exceeds it — still preventable by holding departures ("act now").
    Amber,
    /// The airborne peak alone exceeds it — an overload already locked in ("too late").
    Red,
}

/// Classifies one bin. `active_peak` is the airborne peak and `combined_peak` the airborne + proposed
/// peak (#597's `BinPeak::active` / `::combined`); `map` is the sector's Monitor Alert Parameter.
///
/// Strictly greater than: a peak **equal** to the MAP never alerts. Red is judged on the airborne
/// peak alone, so proposed load can only ever make a cell amber.
pub fn sector_alert(active_peak: usize, combined_peak: usize, map: u32) -> SectorAlert {
    let map = map as usize;
    if active_peak > map {
        SectorAlert::Red
    } else if combined_peak > map {
        SectorAlert::Amber
    } else {
        SectorAlert::Green
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAP: u32 = 18;
    const AT: usize = MAP as usize;

    #[test]
    fn a_peak_equal_to_the_map_is_green() {
        assert_eq!(sector_alert(AT, AT, MAP), SectorAlert::Green);
    }

    #[test]
    fn one_over_the_map_alerts() {
        assert_eq!(sector_alert(AT - 3, AT + 1, MAP), SectorAlert::Amber);
        assert_eq!(sector_alert(AT + 1, AT + 1, MAP), SectorAlert::Red);
    }

    #[test]
    fn red_needs_the_airborne_peak_alone_over_the_map() {
        // However much proposed load sits on top, an airborne peak at the MAP is still preventable.
        assert_eq!(sector_alert(AT, AT + 5, MAP), SectorAlert::Amber);
    }

    #[test]
    fn a_proposed_flight_that_goes_airborne_turns_amber_red() {
        // MAP airborne plus one proposed: only the combined peak is over.
        let (active, combined) = (AT, AT + 1);
        assert_eq!(sector_alert(active, combined, MAP), SectorAlert::Amber);
        // The same flight departs: it moves from proposed to airborne, the combined peak is unchanged.
        assert_eq!(sector_alert(active + 1, combined, MAP), SectorAlert::Red);
    }

    #[test]
    fn a_zero_map_alerts_on_the_first_flight() {
        assert_eq!(sector_alert(0, 0, 0), SectorAlert::Green);
        assert_eq!(sector_alert(0, 1, 0), SectorAlert::Amber);
        assert_eq!(sector_alert(1, 1, 0), SectorAlert::Red);
    }

    #[test]
    fn serializes_as_the_web_names() {
        let names: Vec<String> = [SectorAlert::Green, SectorAlert::Amber, SectorAlert::Red]
            .iter()
            .map(|a| serde_json::to_string(a).unwrap())
            .collect();
        assert_eq!(names, [r#""green""#, r#""amber""#, r#""red""#]);
    }
}
