//! Shared trajectory / ETA model — a climb-profile transit-time estimator with winds-aloft
//! correction. This is the single predictor used by every surface that estimates where
//! connected aircraft will be at a given time (FCA metering, airport-flow demand), so those
//! surfaces stay consistent. Ported from vatflow's `fca-metering.js` ETA engine.
//!
//! The model climbs at 250 kt below 10,000 ft (2000 fpm), 290 kt from 10,000 ft to filed
//! cruise (1500 fpm), then holds filed TAS at cruise — corrected for the mean route
//! headwind ([`crate::feed::winds`]). It fails safe to still air when winds are unavailable.

pub const SPD_BELOW_10K: f64 = 250.0;
pub const SPD_CLIMB: f64 = 290.0;
pub const CLIMB_FPM_LOW: f64 = 2000.0;
pub const CLIMB_FPM_HIGH: f64 = 1500.0;
/// Fallback groundspeed when nothing better is known (kt).
pub const DEFAULT_GROUND_GS: f64 = 250.0;

/// Parse a filed cruise-altitude string (`"350"`, `"FL350"`, `"35000"`) to feet. Values
/// ≤ 600 are treated as flight levels (×100); junk falls back to 35,000 ft.
pub fn parse_alt_ft(s: &str) -> f64 {
    let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return 35000.0;
    }
    let mut n: f64 = digits.parse().unwrap_or(35000.0);
    if n <= 600.0 {
        n *= 100.0;
    }
    if n < 1000.0 { 35000.0 } else { n }
}

/// Filed TAS (kt) with a sane fallback estimated from cruise altitude.
pub fn tas_or_default(tas: f64, cruise_alt_ft: f64) -> f64 {
    if tas >= 60.0 {
        return tas;
    }
    if cruise_alt_ft >= 28000.0 {
        440.0
    } else if cruise_alt_ft >= 15000.0 {
        300.0
    } else {
        170.0
    }
}

/// Groundspeed from TAS and headwind (`+` = headwind, `-` = tailwind), clamped to a sane
/// band so a bad wind reading can't produce absurd speeds. Still air when `headwind` is None.
pub fn effective_gs(tas: f64, headwind: Option<f64>) -> f64 {
    match headwind {
        None => tas,
        Some(hw) => {
            // Order the bounds so a nonsensical negative TAS can't invert the clamp range.
            let (lo, hi) = if tas >= 0.0 {
                (tas * 0.4, tas * 1.6)
            } else {
                (tas * 1.6, tas * 0.4)
            };
            (tas - hw).clamp(lo, hi)
        }
    }
}

/// Seconds to cover `dist_nm` starting at `from_alt_ft` while climbing to `cruise_alt_ft`,
/// following the climb profile and holding cruise TAS (headwind-corrected) thereafter.
pub fn profile_transit_sec(
    dist_nm: f64,
    from_alt_ft: f64,
    cruise_alt_ft: f64,
    tas_kt: f64,
    headwind: Option<f64>,
) -> f64 {
    let mut remaining = dist_nm.max(0.0);
    let mut alt = from_alt_ft.max(0.0);
    let cruise = alt.max(cruise_alt_ft.max(0.0));
    let cruise_gs = effective_gs(tas_kt.max(120.0), headwind).max(120.0);
    let mut t = 0.0;

    // Segment A — below 10,000 ft (250 kt).
    let top_a = 10_000.0_f64.min(cruise);
    if alt < top_a {
        let climb_sec = (top_a - alt) / CLIMB_FPM_LOW * 60.0;
        let gs_a = SPD_BELOW_10K;
        let d_a = gs_a * climb_sec / 3600.0;
        if d_a >= remaining {
            return t + remaining / gs_a * 3600.0;
        }
        t += climb_sec;
        remaining -= d_a;
        alt = top_a;
    }

    // Segment B — 10,000 ft to cruise (290 kt; winds partially felt in the climb).
    if alt < cruise {
        let climb_sec = (cruise - alt) / CLIMB_FPM_HIGH * 60.0;
        let gs_b = effective_gs(SPD_CLIMB, headwind.map(|h| h * 0.7)).max(150.0);
        let d_b = gs_b * climb_sec / 3600.0;
        if d_b >= remaining {
            return t + remaining / gs_b * 3600.0;
        }
        t += climb_sec;
        remaining -= d_b;
    }

    // Segment C — cruise.
    t + remaining / cruise_gs * 3600.0
}

/// Predicted groundspeed far along the route (at cruise), for MIT → time conversion.
pub fn predicted_cross_speed(tas: f64, cruise_alt_ft: f64, headwind: Option<f64>) -> f64 {
    effective_gs(tas_or_default(tas, cruise_alt_ft), headwind).max(120.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_filed_altitude() {
        assert_eq!(parse_alt_ft("350"), 35000.0);
        assert_eq!(parse_alt_ft("FL350"), 35000.0);
        assert_eq!(parse_alt_ft("35000"), 35000.0);
        assert_eq!(parse_alt_ft(""), 35000.0);
        assert_eq!(parse_alt_ft("VFR"), 35000.0);
        assert_eq!(parse_alt_ft("80"), 8000.0);
    }

    #[test]
    fn tas_fallback_by_altitude() {
        assert_eq!(tas_or_default(450.0, 35000.0), 450.0);
        assert_eq!(tas_or_default(0.0, 35000.0), 440.0);
        assert_eq!(tas_or_default(0.0, 16000.0), 300.0);
        assert_eq!(tas_or_default(0.0, 5000.0), 170.0);
    }

    #[test]
    fn headwind_slows_tailwind_speeds() {
        assert_eq!(effective_gs(450.0, None), 450.0);
        assert_eq!(effective_gs(450.0, Some(50.0)), 400.0); // 50kt headwind
        assert_eq!(effective_gs(450.0, Some(-50.0)), 500.0); // 50kt tailwind
        // Absurd winds are clamped to ±60% of TAS.
        assert_eq!(effective_gs(450.0, Some(400.0)), 450.0 * 0.4);
    }

    #[test]
    fn climb_costs_time_versus_pure_cruise() {
        // A departure climbing from the surface to FL350 over 300 nm takes longer than the
        // same distance flown entirely at cruise speed.
        let climb = profile_transit_sec(300.0, 0.0, 35000.0, 450.0, None);
        let pure_cruise = 300.0 / 450.0 * 3600.0;
        assert!(
            climb > pure_cruise,
            "climb {climb} should exceed cruise {pure_cruise}"
        );
        // An aircraft already at cruise covers it at ~cruise speed.
        let at_cruise = profile_transit_sec(300.0, 35000.0, 35000.0, 450.0, None);
        assert!((at_cruise - pure_cruise).abs() < 1.0);
    }

    #[test]
    fn headwind_increases_transit_time() {
        let still = profile_transit_sec(500.0, 35000.0, 35000.0, 450.0, None);
        let headwind = profile_transit_sec(500.0, 35000.0, 35000.0, 450.0, Some(80.0));
        assert!(
            headwind > still,
            "headwind {headwind} should exceed still air {still}"
        );
    }
}
