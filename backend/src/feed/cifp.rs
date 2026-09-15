//! Clean-room ARINC 424 (v18) fixed-width record parser for the FAA's CIFP product —
//! currently only the Enroute Airway (`ER`) records, plus the VHF Navaid (`D`), NDB Navaid
//! (`DB`), and Enroute Waypoint (`EA`) records needed to resolve an airway's fixes to
//! coordinates. ARINC 424 airway records store each fix as an identifier plus a
//! section/subsection pointer to one of those three tables — never a literal lat/lon on the
//! airway record itself.
//!
//! Every column offset below is taken from the independently published **ARINC Specification
//! 424-17** (§4.1.6 "Enroute Airways Records", §4.1.2 "VHF NAVAID Record", §4.1.3 "NDB NAVAID
//! Record", §4.1.4 "Waypoint Record"), and verified against a live-downloaded CIFP cycle before
//! being written here. [`cifparse`](https://github.com/misterrodg/cifparse) (GPL-3.0, by
//! misterrodg) was a helpful pointer to which fields exist on each record type while researching
//! this — its plain-English `docs/*.md` field glossary was consulted, never its (GPL) source —
//! but no code or column layout here was copied or ported from it; the layout below comes from
//! the spec.
//!
//! SID/STAR, approaches, and airspace records are out of scope here — see VATUSA/OIS#221's
//! follow-up issue.

use std::collections::HashMap;

use chrono::NaiveDate;

use super::nav::CoordList;
use super::nav_source::{add_candidate, candidate_cycles, download, read_zip_member};

type Fetched<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// One resolved airway, in the same shape `nav_source`'s `OutAirway` serializes to.
pub(super) struct CifpAirway {
    pub t: String,
    pub w: Vec<(String, f64, f64)>,
}

/// Fetch the current CIFP cycle (walking back like the FAA NASR path) and parse its Enroute
/// Airway records into airways with resolved coordinates.
pub(super) async fn fetch_airways(
    client: &reqwest::Client,
) -> Fetched<HashMap<String, CifpAirway>> {
    let mut last_err: Option<Box<dyn std::error::Error + Send + Sync>> = None;
    for date in candidate_cycles() {
        match fetch_cycle(client, date).await {
            Ok(text) => return Ok(parse_airways(&text)),
            Err(e) => {
                tracing::debug!(cycle = %date, error = %e, "CIFP cycle unavailable");
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| "no CIFP cycle candidates".into()))
}

async fn fetch_cycle(client: &reqwest::Client, date: NaiveDate) -> Fetched<String> {
    let url = format!(
        "https://aeronav.faa.gov/Upload_313-d/cifp/CIFP_{}.zip",
        date.format("%y%m%d")
    );
    let zip = download(client, &url).await?;
    read_zip_member(&zip, &["FAACIFP18"])
}

/// Decode an ARINC 424 latitude field: `[N|S]` + 8 digits (`DDMMSSHH`, hundredths of a second).
fn decode_lat(s: &str) -> Option<f64> {
    if s.len() != 9 || !s.is_ascii() {
        return None;
    }
    let sign = match s.as_bytes()[0] {
        b'N' => 1.0,
        b'S' => -1.0,
        _ => return None,
    };
    decode_dms(&s[1..], sign, 2)
}

/// Decode an ARINC 424 longitude field: `[E|W]` + 9 digits (`DDDMMSSHH`, hundredths of a second).
fn decode_lon(s: &str) -> Option<f64> {
    if s.len() != 10 || !s.is_ascii() {
        return None;
    }
    let sign = match s.as_bytes()[0] {
        b'E' => 1.0,
        b'W' => -1.0,
        _ => return None,
    };
    decode_dms(&s[1..], sign, 3)
}

/// `deg_len` is 2 for latitude (max 90) or 3 for longitude (max 180); the remaining 6 digits are
/// always `MMSSHH` (minutes, seconds, hundredths of a second).
fn decode_dms(digits: &str, sign: f64, deg_len: usize) -> Option<f64> {
    if digits.len() != deg_len + 6 {
        return None;
    }
    let deg: f64 = digits[..deg_len].parse().ok()?;
    let min: f64 = digits[deg_len..deg_len + 2].parse().ok()?;
    let sec: f64 = digits[deg_len + 2..deg_len + 4].parse().ok()?;
    let hundredths: f64 = digits[deg_len + 4..deg_len + 6].parse().ok()?;
    Some(sign * (deg + min / 60.0 + (sec + hundredths / 100.0) / 3600.0))
}

/// Column 39 (Continuation Record No., 0-indexed 38) on Enroute Airway records; column 22
/// (0-indexed 21) on VHF Navaid/NDB Navaid/Waypoint records. Per ARINC 424 §5.16: `'0'` (no
/// continuations) or `'1'` (continuations follow) marks a primary record; `'2'`-`'9'`/`'A'`-`'Z'`
/// are continuations, which carry notes rather than more geometry.
fn is_primary(b: u8) -> bool {
    b == b'0' || b == b'1'
}

/// Builds an identifier -> coordinates table for one of the three fix-bearing record types.
/// `id_range` is the 0-indexed byte range of the identifier field; column positions for section
/// code (5), subsection code (6), and the coordinate pair (33-41, 42-51) are the same across all
/// three tables.
fn parse_coord_table(
    text: &str,
    section: u8,
    subsection: u8,
    id_range: std::ops::Range<usize>,
) -> HashMap<String, CoordList> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let b = line.as_bytes();
        if b.len() < 51 || b[4] != section || b[5] != subsection || !is_primary(b[21]) {
            continue;
        }
        let id = line[id_range.clone()].trim();
        if id.is_empty() {
            continue;
        }
        let (Some(lat), Some(lon)) = (decode_lat(&line[32..41]), decode_lon(&line[41..51])) else {
            continue;
        };
        add_candidate(&mut out, &id.to_ascii_uppercase(), lat, lon);
    }
    out
}

/// Parses every Enroute Airway (`ER`) primary record, groups by Route Identifier, sorts by
/// Sequence Number, and resolves each point's Fix Identifier against the VHF Navaid, NDB Navaid,
/// or Enroute Waypoint table (chosen by the fix's own section/subsection code carried on the
/// airway record). An airway that resolves fewer than 2 points is dropped, mirroring the
/// `nav_source` @squawk airway path's same guard.
fn parse_airways(text: &str) -> HashMap<String, CifpAirway> {
    let navaids = parse_coord_table(text, b'D', b' ', 13..17);
    let ndbs = parse_coord_table(text, b'D', b'B', 13..17);
    let waypoints = parse_coord_table(text, b'E', b'A', 13..18);

    let mut by_route: HashMap<String, Vec<(u32, String, u8, u8)>> = HashMap::new();
    for line in text.lines() {
        let b = line.as_bytes();
        if b.len() < 51 || b[4] != b'E' || b[5] != b'R' || !is_primary(b[38]) {
            continue;
        }
        let route_id = line[13..18].trim().to_ascii_uppercase();
        let fix_id = line[29..34].trim().to_ascii_uppercase();
        let Ok(seq) = line[25..29].trim().parse::<u32>() else {
            continue;
        };
        if route_id.is_empty() || fix_id.is_empty() {
            continue;
        }
        by_route
            .entry(route_id)
            .or_default()
            .push((seq, fix_id, b[36], b[37]));
    }

    let mut out = HashMap::new();
    for (route_id, mut points) in by_route {
        points.sort_by_key(|p| p.0);
        let mut w = Vec::with_capacity(points.len());
        for (_, fix_id, fix_sec, fix_sub) in &points {
            let table = match (*fix_sec, *fix_sub) {
                (b'D', b'B') => &ndbs,
                (b'D', _) => &navaids,
                (b'E', b'A') => &waypoints,
                _ => continue,
            };
            let Some([lat, lon]) = table.get(fix_id).and_then(|c| c.first()) else {
                continue;
            };
            w.push((fix_id.clone(), *lat, *lon));
        }
        if w.len() >= 2 {
            let t = route_id
                .chars()
                .next()
                .map(String::from)
                .unwrap_or_default();
            out.insert(route_id, CifpAirway { t, w });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_real_latitude_and_longitude() {
        // SWIMM, a real Miami Oceanic enroute waypoint (verified against a live CIFP cycle).
        assert!((decode_lat("N25295920").unwrap() - 25.499_777_8).abs() < 1e-6);
        assert!((decode_lon("W079021809").unwrap() - -79.038_358_3).abs() < 1e-6);
    }

    #[test]
    fn rejects_malformed_coordinates() {
        assert!(decode_lat("X25295920").is_none()); // bad hemisphere
        assert!(decode_lat("N2529592").is_none()); // wrong length
        assert!(decode_lon("W07902180").is_none()); // wrong length
    }

    #[test]
    fn primary_vs_continuation() {
        assert!(is_primary(b'0'));
        assert!(is_primary(b'1'));
        assert!(!is_primary(b'2'));
        assert!(!is_primary(b'A'));
    }

    /// A 3-point airway: one VHF navaid fix, one enroute waypoint fix, and a continuation record
    /// (out of sequence, carrying no geometry) that must be excluded rather than treated as a
    /// fourth point.
    #[test]
    fn resolves_an_airway_from_navaid_and_waypoint_tables() {
        let navaid = "SUSAD        ZBV   MY011670VDH  N25421410W079173710    N25421500W079174000W0040000102     NARBIMINI                        259562510";
        let waypoint = "SUSAEAENRT   SWIMM K70    R   B N25295920W079021809                       W0080     NAR           SWIMM                    504052605";
        let leg1 = pad(
            "SUSAER       A315        0100ZBV  MYD 0V    O                         13540185     05000     60000                         55733",
        );
        let leg2 = pad(
            "SUSAER       A315        0110SWIMMK7EA0E    O                         135404691355 08000     60000                         55734",
        );
        // A continuation record on the first point (Cont Rec No at column 39 set to '2', all
        // other fields identical to leg1): must not be read as a third airway point.
        let mut cont = leg1.clone().into_bytes();
        cont[38] = b'2';
        let cont = String::from_utf8(cont).unwrap();

        let text = format!("{navaid}\r\n{waypoint}\r\n{leg1}\r\n{leg2}\r\n{cont}\r\n");
        let airways = parse_airways(&text);

        let a315 = airways.get("A315").expect("A315 should resolve");
        assert_eq!(a315.t, "A");
        assert_eq!(a315.w.len(), 2);
        assert_eq!(a315.w[0].0, "ZBV");
        assert!((a315.w[0].1 - 25.703_916_7).abs() < 1e-5);
        assert_eq!(a315.w[1].0, "SWIMM");
    }

    #[test]
    fn drops_an_airway_with_fewer_than_two_resolved_points() {
        let waypoint = "SUSAEAENRT   SWIMM K70    R   B N25295920W079021809                       W0080     NAR           SWIMM                    504052605";
        let leg1 = pad(
            "SUSAER       LONE1        0100SWIMMK7EA0E    O                         135404691355 08000     60000                         55734",
        );
        let text = format!("{waypoint}\r\n{leg1}\r\n");
        assert!(!parse_airways(&text).contains_key("LONE1"));
    }

    /// Real ARINC 424 lines are exactly 132 columns; the literals above are trimmed for
    /// readability, so pad them back out before parsing.
    fn pad(line: &str) -> String {
        let mut s = line.to_string();
        while s.len() < 132 {
            s.push(' ');
        }
        s
    }
}
