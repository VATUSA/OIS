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
//! SID/STAR (`PD`/`PE`) records are also parsed here (VATUSA/OIS#253), reusing the same
//! fix-reference pointer scheme as airway legs. Approaches, holds, MSAs, and airspace remain out
//! of scope — see VATUSA/OIS#253's own follow-up note.

use std::collections::HashMap;
use std::collections::HashSet;

use chrono::NaiveDate;

use super::nav::CoordList;
use super::nav_source::{candidate_cycles, download, in_coverage, read_zip_member, round5};

type Fetched<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// One resolved airway, in the same shape `nav_source`'s `OutAirway` serializes to.
pub(super) struct CifpAirway {
    pub t: String,
    pub w: Vec<(String, f64, f64)>,
}

/// One resolved SID/STAR, in the same shape `nav_source`'s `OutProc` serializes to.
pub(super) struct CifpProc {
    pub ptype: String,
    pub apt: Vec<String>,
    pub common: Vec<(String, f64, f64)>,
    pub transitions: HashMap<String, Vec<(String, f64, f64)>>,
}

/// Fetch the current CIFP cycle's raw text (walking back like the FAA NASR path if the newest
/// isn't posted yet). Shared by the airway and procedure parsers so a refresh downloads the
/// ~9MB CIFP file once, not once per parser.
pub(super) async fn fetch_text(client: &reqwest::Client) -> Fetched<String> {
    let mut last_err: Option<Box<dyn std::error::Error + Send + Sync>> = None;
    for date in candidate_cycles() {
        match fetch_cycle(client, date).await {
            Ok(text) => return Ok(text),
            Err(e) => {
                tracing::debug!(cycle = %date, error = %e, "CIFP cycle unavailable");
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| "no CIFP cycle candidates".into()))
}

/// Fetch the current CIFP cycle and parse its Enroute Airway records into airways with resolved
/// coordinates. Only exercised by the live parity test in `nav_source` — the real fetch path
/// goes through the combined [`fetch`], which downloads the CIFP file once for both parsers.
#[cfg(test)]
pub(super) async fn fetch_airways(
    client: &reqwest::Client,
) -> Fetched<HashMap<String, CifpAirway>> {
    Ok(parse_airways(&fetch_text(client).await?))
}

/// Fetch the current CIFP cycle and parse its SID/STAR records into procedures with resolved
/// coordinates. Only exercised by the live parity test in `nav_source` — see [`fetch_airways`].
#[cfg(test)]
pub(super) async fn fetch_procedures(
    client: &reqwest::Client,
) -> Fetched<HashMap<String, CifpProc>> {
    Ok(parse_procedures(&fetch_text(client).await?))
}

/// Fetch the current CIFP cycle once and parse both airways and procedures from it — the
/// combined form `nav_source::fetch_latest` uses, to avoid downloading the CIFP file twice.
pub(super) async fn fetch(
    client: &reqwest::Client,
) -> Fetched<(HashMap<String, CifpAirway>, HashMap<String, CifpProc>)> {
    let text = fetch_text(client).await?;
    Ok((parse_airways(&text), parse_procedures(&text)))
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

/// Builds an `(area code, identifier) -> coordinates` table for one of the three fix-bearing
/// record types. Keyed by area code as well as identifier because ARINC 424's Customer/Area Code
/// (columns 1-4, e.g. `SUSA`, `SCAN`, `SPAC`) is not unique per file — the same short identifier
/// can legitimately appear in more than one FAA region, and conflating them would resolve a fix
/// reference to whichever region's record happened to be inserted first (VATUSA/OIS#221).
/// `id_range` is the 0-indexed byte range of the identifier field; column positions for section
/// code (5), subsection code (6), and the coordinate pair (33-41, 42-51) are the same across all
/// three tables.
fn parse_coord_table(
    text: &str,
    section: u8,
    subsection: u8,
    id_range: std::ops::Range<usize>,
) -> HashMap<(String, String), CoordList> {
    let mut out: HashMap<(String, String), CoordList> = HashMap::new();
    for line in text.lines() {
        let b = line.as_bytes();
        if b.len() < 51
            || !line.is_ascii()
            || b[4] != section
            || b[5] != subsection
            || !is_primary(b[21])
        {
            continue;
        }
        let id = line[id_range.clone()].trim();
        if id.is_empty() {
            continue;
        }
        let (Some(lat), Some(lon)) = (decode_lat(&line[32..41]), decode_lon(&line[41..51])) else {
            continue;
        };
        if !lat.is_finite() || !lon.is_finite() || !in_coverage(lat, lon) {
            continue;
        }
        let key = (line[0..4].to_string(), id.to_ascii_uppercase());
        let pt = [round5(lat), round5(lon)];
        let entry = out.entry(key).or_default();
        if !entry.contains(&pt) {
            entry.push(pt);
        }
    }
    out
}

/// Parses every Enroute Airway (`ER`) primary record, groups by (Area Code, Route Identifier) so
/// two unrelated same-named airways in different FAA regions are never merged, sorts each group by
/// Sequence Number, and resolves each point's Fix Identifier — within that same area — against the
/// VHF Navaid, NDB Navaid, or Enroute Waypoint table (chosen by the fix's own section/subsection
/// code carried on the airway record). An airway that resolves fewer than 2 points is dropped,
/// mirroring the `nav_source` @squawk airway path's same guard.
///
/// A bare Route Identifier is not unique across areas (VATUSA/OIS#221 — e.g. a live cycle had 73
/// IDs spanning more than one area, including a Kansas `V17` and an unrelated Hawaii `V17`). Since
/// the output map — and the filed-route tokens it's looked up by — carry no area qualifier, a
/// Route Identifier that resolves in more than one area is dropped entirely rather than guessing:
/// picking "whichever area has more points" was tried and rejected (VATUSA/OIS#221 second review)
/// because both areas' versions are typically real, currently-flown airways, and silently keeping
/// one deletes the other's geometry without any error — a CONUS flight filing a designator that
/// also exists in Alaska would get Alaska's geometry with no indication anything was wrong. A
/// dropped Route Identifier simply fails to resolve, the same as any other unrecognized token
/// (`nav.rs`'s `unresolved` list already handles a missing/empty airway lookup) — a route that
/// can't be resolved beats one that resolves to the wrong region silently.
fn parse_airways(text: &str) -> HashMap<String, CifpAirway> {
    let navaids = parse_coord_table(text, b'D', b' ', 13..17);
    let ndbs = parse_coord_table(text, b'D', b'B', 13..17);
    let waypoints = parse_coord_table(text, b'E', b'A', 13..18);

    // (area, route id) -> [(sequence, fix id, fix section, fix subsection)].
    type AreaRouteLegs = HashMap<(String, String), Vec<(u32, String, u8, u8)>>;
    let mut by_area_route: AreaRouteLegs = HashMap::new();
    for line in text.lines() {
        let b = line.as_bytes();
        if b.len() < 51 || !line.is_ascii() || b[4] != b'E' || b[5] != b'R' || !is_primary(b[38]) {
            continue;
        }
        let area = line[0..4].to_string();
        let route_id = line[13..18].trim().to_ascii_uppercase();
        let fix_id = line[29..34].trim().to_ascii_uppercase();
        let Ok(seq) = line[25..29].trim().parse::<u32>() else {
            continue;
        };
        if route_id.is_empty() || fix_id.is_empty() {
            continue;
        }
        by_area_route
            .entry((area, route_id))
            .or_default()
            .push((seq, fix_id, b[36], b[37]));
    }

    let mut out: HashMap<String, CifpAirway> = HashMap::new();
    let mut collided: HashSet<String> = HashSet::new();
    for ((area, route_id), mut points) in by_area_route {
        points.sort_by_key(|p| p.0);
        let mut w = Vec::with_capacity(points.len());
        for (_, fix_id, fix_sec, fix_sub) in &points {
            let table = match (*fix_sec, *fix_sub) {
                (b'D', b'B') => &ndbs,
                (b'D', _) => &navaids,
                (b'E', b'A') => &waypoints,
                _ => continue,
            };
            let key = (area.clone(), fix_id.clone());
            let Some([lat, lon]) = table.get(&key).and_then(|c| c.first()) else {
                continue;
            };
            w.push((fix_id.clone(), *lat, *lon));
        }
        if w.len() < 2 {
            continue;
        }
        if collided.contains(&route_id) {
            continue;
        }
        if out.remove(&route_id).is_some() {
            // A different area already produced a version of this designator — ambiguous, drop
            // both rather than guess which region a filed route meant (VATUSA/OIS#221).
            collided.insert(route_id);
            continue;
        }
        let t = route_id
            .chars()
            .next()
            .map(String::from)
            .unwrap_or_default();
        out.insert(route_id, CifpAirway { t, w });
    }
    out
}

/// Builds an `identifier -> coordinates` table for one of the four fix-bearing record types a
/// procedure leg can point at, **without** area-scoping (unlike [`parse_coord_table`], which
/// airways rely on for the `V17`-style same-designator-different-region case, VATUSA/OIS#221).
/// Deliberately different here: verified against a live CIFP cycle, ARINC enroute/terminal
/// waypoint and VHF navaid identifiers are nationally unique (0 collisions across 32,457 enroute
/// waypoints plus 37,628 terminal waypoints, 0/820 VHF navaids); only 2-letter NDB idents
/// legitimately repeat (29/350), an acceptable residual resolved first-match same as an airway
/// table's tie-break. Area-scoping a procedure's own fix lookups would actually be *wrong*: a
/// SID/STAR's Area Code doesn't reliably match the area its referenced fixes are catalogued under
/// near a FIR boundary (verified: Detroit-area STARs filed under Canada's `SCAN` area reference
/// waypoints only catalogued under `SUSA`).
///
/// `subsection_pos` is 5 for the enroute `D`/`DB`/`EA` record layout (subsection right after the
/// area code) and 12 for the airport-scoped `P`-section `PC` terminal-waypoint layout (subsection
/// after the airport identifier) — the only structural difference between the two record
/// families; every other offset (identifier, primary flag, coordinates) lines up.
fn parse_ident_coord_table(
    text: &str,
    section: u8,
    subsection: u8,
    subsection_pos: usize,
    id_range: std::ops::Range<usize>,
) -> HashMap<String, CoordList> {
    let mut out: HashMap<String, CoordList> = HashMap::new();
    for line in text.lines() {
        let b = line.as_bytes();
        if b.len() < 51
            || !line.is_ascii()
            || b[4] != section
            || b[subsection_pos] != subsection
            || !is_primary(b[21])
        {
            continue;
        }
        let id = line[id_range.clone()].trim();
        if id.is_empty() {
            continue;
        }
        let (Some(lat), Some(lon)) = (decode_lat(&line[32..41]), decode_lon(&line[41..51])) else {
            continue;
        };
        if !lat.is_finite() || !lon.is_finite() || !in_coverage(lat, lon) {
            continue;
        }
        let pt = [round5(lat), round5(lon)];
        let entry = out.entry(id.to_ascii_uppercase()).or_default();
        if !entry.contains(&pt) {
            entry.push(pt);
        }
    }
    out
}

/// Parses every SID (`PD`) / STAR (`PE`) primary leg record, resolving each leg's fix against
/// the VHF Navaid, NDB Navaid, Enroute Waypoint, or Terminal Waypoint table — chosen by the leg's
/// own fix section/subsection pointer, the same scheme [`parse_airways`] uses for `ER` legs. A
/// leg whose Path Terminator carries no fix (`CA`/`VA` course/heading-to-altitude, and similarly
/// fix-less types) simply contributes no point rather than breaking the chain — this one rule is
/// what handles all of the common leg types (IF/TF/CF/DF always carry a fix and resolve exactly
/// like an airway leg; CA/VA never do) without a leg-type whitelist.
///
/// The same procedure/transition legitimately repeats verbatim across several airports in one
/// metroplex-shared chart (VATUSA/OIS#253 — e.g. Detroit's `FOREY3` STAR is filed identically at
/// ten airports). Grouping keeps each airport's occurrence separate until the final fold, which
/// keeps whichever occurrence resolves the most points per transition — this both handles a
/// genuine collision safely and transparently deduplicates the verbatim metroplex repeats.
fn parse_procedures(text: &str) -> HashMap<String, CifpProc> {
    let navaids = parse_ident_coord_table(text, b'D', b' ', 5, 13..17);
    let ndbs = parse_ident_coord_table(text, b'D', b'B', 5, 13..17);
    let waypoints = parse_ident_coord_table(text, b'E', b'A', 5, 13..18);
    let terminal = parse_ident_coord_table(text, b'P', b'C', 12, 13..18);

    // (subsection, proc id, airport, transition id) -> [(sequence, fix id, fix sec, fix sub)].
    type LegGroups = HashMap<(u8, String, String, String), Vec<(u32, String, u8, u8)>>;
    let mut groups: LegGroups = HashMap::new();
    let mut apts_by_proc: HashMap<String, std::collections::HashSet<String>> = HashMap::new();

    for line in text.lines() {
        let b = line.as_bytes();
        if b.len() < 51 || !line.is_ascii() || b[4] != b'P' || !is_primary(b[38]) {
            continue;
        }
        let sub = b[12];
        if sub != b'D' && sub != b'E' {
            continue;
        }
        let apt = line[6..10].trim().to_ascii_uppercase();
        let proc_id = line[13..19].trim().to_ascii_uppercase();
        if apt.is_empty() || proc_id.is_empty() {
            continue;
        }
        let Ok(seq) = line[26..29].trim().parse::<u32>() else {
            continue;
        };
        let mut transition = line[20..25].trim().to_ascii_uppercase();
        if transition == "ALL" {
            transition.clear();
        }
        let fix_id = line[29..34].trim().to_ascii_uppercase();

        apts_by_proc
            .entry(proc_id.clone())
            .or_default()
            .insert(apt.clone());
        groups
            .entry((sub, proc_id, apt, transition))
            .or_default()
            .push((seq, fix_id, b[36], b[37]));
    }

    let mut out: HashMap<String, CifpProc> = HashMap::new();
    for ((sub, proc_id, _apt, transition), mut legs) in groups {
        legs.sort_by_key(|l| l.0);
        let mut w = Vec::with_capacity(legs.len());
        for (_, fix_id, fix_sec, fix_sub) in &legs {
            if fix_id.is_empty() {
                continue;
            }
            let table = match (*fix_sec, *fix_sub) {
                (b'D', b'B') => &ndbs,
                (b'D', _) => &navaids,
                (b'E', b'A') => &waypoints,
                (b'P', b'C') => &terminal,
                _ => continue,
            };
            let Some([lat, lon]) = table.get(fix_id).and_then(|c| c.first()) else {
                continue;
            };
            w.push((fix_id.clone(), *lat, *lon));
        }
        if w.is_empty() {
            continue;
        }

        let entry = out.entry(proc_id.clone()).or_insert_with(|| CifpProc {
            ptype: if sub == b'E' { "STAR" } else { "SID" }.to_string(),
            apt: Vec::new(),
            common: Vec::new(),
            transitions: HashMap::new(),
        });
        if transition.is_empty() {
            if w.len() > entry.common.len() {
                entry.common = w;
            }
        } else {
            match entry.transitions.get(&transition) {
                Some(existing) if existing.len() >= w.len() => {}
                _ => {
                    entry.transitions.insert(transition, w);
                }
            }
        }
    }

    for (proc_id, apts) in apts_by_proc {
        if let Some(p) = out.get_mut(&proc_id) {
            let mut apt: Vec<String> = apts.into_iter().collect();
            apt.sort();
            p.apt = apt;
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

    /// Regression (#221): a live CIFP cycle had 73 Route Identifiers reused across more than one
    /// FAA Customer/Area Code (e.g. a Kansas `V17` and an unrelated Hawaii `V17`), and a second
    /// live cycle showed picking "whichever area has more points" just silently substitutes one
    /// real airway for another equally real one in a different region (e.g. `V438`: Maryland vs.
    /// Alaska). Two unrelated airways sharing a designator, in different areas, must never be
    /// spliced together *or* have one silently picked over the other — build a 2-point "V17" in
    /// area SUSA and a 3-point "V17" in area SPAC (with its own, area-scoped navaid records) and
    /// confirm the collision drops the designator entirely rather than resolving to either side.
    #[test]
    fn same_route_id_in_two_areas_is_dropped_not_substituted() {
        let susa_navaid = record(|f| {
            f.area = "SUSA";
            f.section = b'D';
            f.subsection = b' ';
            f.id = "ZBV";
            f.lat = "N25421410";
            f.lon = "W079173710";
        });
        let susa_waypoint = record(|f| {
            f.area = "SUSA";
            f.section = b'E';
            f.subsection = b'A';
            f.id = "SWIMM";
            f.lat = "N25295920";
            f.lon = "W079021809";
        });
        let susa_leg1 = er_record("SUSA", "V17", 100, "ZBV", b'D', b' ');
        let susa_leg2 = er_record("SUSA", "V17", 110, "SWIMM", b'E', b'A');

        let spac_navaid1 = record(|f| {
            f.area = "SPAC";
            f.section = b'D';
            f.subsection = b' ';
            f.id = "OGG";
            f.lat = "N20543600";
            f.lon = "W156255700";
        });
        let spac_navaid2 = record(|f| {
            f.area = "SPAC";
            f.section = b'D';
            f.subsection = b' ';
            f.id = "MKK";
            f.lat = "N21091200";
            f.lon = "W157095700";
        });
        let spac_waypoint = record(|f| {
            f.area = "SPAC";
            f.section = b'E';
            f.subsection = b'A';
            f.id = "HAKLE";
            f.lat = "N21133600";
            f.lon = "W157001200";
        });
        let spac_leg1 = er_record("SPAC", "V17", 100, "OGG", b'D', b' ');
        let spac_leg2 = er_record("SPAC", "V17", 110, "MKK", b'D', b' ');
        let spac_leg3 = er_record("SPAC", "V17", 120, "HAKLE", b'E', b'A');

        let text = [
            susa_navaid,
            susa_waypoint,
            susa_leg1,
            susa_leg2,
            spac_navaid1,
            spac_navaid2,
            spac_waypoint,
            spac_leg1,
            spac_leg2,
            spac_leg3,
        ]
        .join("\r\n");

        assert!(
            !parse_airways(&text).contains_key("V17"),
            "an area-code collision must drop the designator, not resolve to either side"
        );
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

    /// Fields for a synthetic VHF Navaid / NDB Navaid / Enroute Waypoint record, placed at their
    /// real ARINC 424 column offsets rather than hand-aligned text (error-prone for a 132-column
    /// fixed-width layout).
    struct RecordFields {
        area: &'static str,
        section: u8,
        subsection: u8,
        id: &'static str,
        lat: &'static str,
        lon: &'static str,
    }

    /// Builds a 132-column fix-bearing record (`D`/`DB`/`EA`) from field values placed at their
    /// real column offsets: area 0-3, section 4, subsection 5, identifier from 13, primary flag
    /// 21 (always primary here), coordinates 32-41/41-51.
    fn record(f: impl FnOnce(&mut RecordFields)) -> String {
        let mut fields = RecordFields {
            area: "SUSA",
            section: b'D',
            subsection: b' ',
            id: "",
            lat: "",
            lon: "",
        };
        f(&mut fields);
        let mut b = vec![b' '; 132];
        b[0..4].copy_from_slice(fields.area.as_bytes());
        b[4] = fields.section;
        b[5] = fields.subsection;
        b[13..13 + fields.id.len()].copy_from_slice(fields.id.as_bytes());
        b[21] = b'0'; // primary record
        b[32..41].copy_from_slice(fields.lat.as_bytes());
        b[41..51].copy_from_slice(fields.lon.as_bytes());
        String::from_utf8(b).unwrap()
    }

    /// Builds a 132-column Enroute Airway (`ER`) leg record: area 0-3, `ER` at 4-5, route id from
    /// 13, sequence 25-29, fix id from 29, fix section/subsection 36-37, primary flag at 38.
    fn er_record(
        area: &str,
        route_id: &str,
        seq: u32,
        fix_id: &str,
        fix_sec: u8,
        fix_sub: u8,
    ) -> String {
        let mut b = vec![b' '; 132];
        b[0..4].copy_from_slice(area.as_bytes());
        b[4] = b'E';
        b[5] = b'R';
        b[13..13 + route_id.len()].copy_from_slice(route_id.as_bytes());
        let seq_str = format!("{seq:04}");
        b[25..29].copy_from_slice(seq_str.as_bytes());
        b[29..29 + fix_id.len()].copy_from_slice(fix_id.as_bytes());
        b[36] = fix_sec;
        b[37] = fix_sub;
        b[38] = b'0'; // primary record
        String::from_utf8(b).unwrap()
    }

    /// Builds a 132-column Terminal Waypoint (`PC`) record at its real column offsets: area 0-3,
    /// section 4='P', airport 6-9, subsection 12='C', identifier from 13, primary flag 21
    /// (always primary here), coordinates 32-41/41-51.
    fn terminal_record(area: &str, apt: &str, id: &str, lat: &str, lon: &str) -> String {
        let mut b = vec![b' '; 132];
        b[0..4].copy_from_slice(area.as_bytes());
        b[4] = b'P';
        b[6..6 + apt.len()].copy_from_slice(apt.as_bytes());
        b[12] = b'C';
        b[13..13 + id.len()].copy_from_slice(id.as_bytes());
        b[21] = b'0';
        b[32..41].copy_from_slice(lat.as_bytes());
        b[41..51].copy_from_slice(lon.as_bytes());
        String::from_utf8(b).unwrap()
    }

    /// Builds a 132-column SID/STAR (`PD`/`PE`) leg record at its real column offsets: area 0-3,
    /// section 4='P', airport 6-9, subsection 12 (`D`=SID, `E`=STAR), procedure id from 13,
    /// transition id from 20, sequence 26-28, fix id from 29, fix section/subsection 36-37,
    /// continuation flag 38 (always primary here). `fix_id` empty models a fix-less leg (`CA`/
    /// `VA`) — `fix_sec`/`fix_sub` are ignored in that case.
    #[allow(clippy::too_many_arguments)]
    fn proc_leg(
        area: &str,
        apt: &str,
        subsection: u8,
        proc_id: &str,
        transition: &str,
        seq: u32,
        fix_id: &str,
        fix_sec: u8,
        fix_sub: u8,
    ) -> String {
        let mut b = vec![b' '; 132];
        b[0..4].copy_from_slice(area.as_bytes());
        b[4] = b'P';
        b[6..6 + apt.len()].copy_from_slice(apt.as_bytes());
        b[12] = subsection;
        b[13..13 + proc_id.len()].copy_from_slice(proc_id.as_bytes());
        b[20..20 + transition.len()].copy_from_slice(transition.as_bytes());
        let seq_str = format!("{seq:03}");
        b[26..29].copy_from_slice(seq_str.as_bytes());
        b[29..29 + fix_id.len()].copy_from_slice(fix_id.as_bytes());
        b[36] = fix_sec;
        b[37] = fix_sub;
        b[38] = b'0'; // primary record
        String::from_utf8(b).unwrap()
    }

    /// A SID with a single runway transition (`RW01`) whose legs are all fix-bearing (`IF`-style
    /// initial fix, then `CF`/`TF` resolve identically — only the fix reference matters here, not
    /// the Path Terminator itself, since a leg's point comes purely from a populated fix id).
    #[test]
    fn sid_runway_transition_resolves_fix_bearing_legs() {
        let revge = terminal_record("SUSA", "KDCA", "REVGE", "N38512048", "W077005674");
        let beble = terminal_record("SUSA", "KDCA", "BEBLE", "N38561839", "W077073659");
        let leg1 = proc_leg(
            "SUSA", "KDCA", b'D', "AMEEE1", "RW01", 10, "REVGE", b'P', b'C',
        );
        let leg2 = proc_leg(
            "SUSA", "KDCA", b'D', "AMEEE1", "RW01", 20, "BEBLE", b'P', b'C',
        );
        let text = [revge, beble, leg1, leg2].join("\r\n");

        let procs = parse_procedures(&text);
        let p = procs.get("AMEEE1").expect("AMEEE1 should resolve");
        assert_eq!(p.ptype, "SID");
        assert_eq!(p.apt, vec!["KDCA".to_string()]);
        assert!(p.common.is_empty());
        let rw01 = p.transitions.get("RW01").expect("RW01 transition");
        assert_eq!(
            rw01.iter().map(|w| w.0.as_str()).collect::<Vec<_>>(),
            ["REVGE", "BEBLE"]
        );
    }

    /// A STAR whose common (`ALL`) route feeds a named enroute transition — the same
    /// common/transitions split `nav.rs`'s `merge_procedure_legs` expects.
    #[test]
    fn star_splits_common_route_from_a_named_transition() {
        let thhmp = record(|f| {
            f.section = b'E';
            f.subsection = b'A';
            f.id = "THHMP";
            f.lat = "N38590229";
            f.lon = "W077093051";
        });
        let bulii = terminal_record("SUSA", "KJFK", "BULII", "N38561839", "W077073659");
        let waves = record(|f| {
            f.section = b'E';
            f.subsection = b'A';
            f.id = "WAVES";
            f.lat = "N38545581";
            f.lon = "W077062692";
        });
        let common1 = proc_leg(
            "SUSA", "KJFK", b'E', "CAPSS4", "ALL", 10, "THHMP", b'E', b'A',
        );
        let common2 = proc_leg(
            "SUSA", "KJFK", b'E', "CAPSS4", "ALL", 20, "BULII", b'P', b'C',
        );
        let trans1 = proc_leg(
            "SUSA", "KJFK", b'E', "CAPSS4", "WAVES", 10, "WAVES", b'E', b'A',
        );
        let trans2 = proc_leg(
            "SUSA", "KJFK", b'E', "CAPSS4", "WAVES", 20, "BULII", b'P', b'C',
        );
        let text = [thhmp, bulii, waves, common1, common2, trans1, trans2].join("\r\n");

        let procs = parse_procedures(&text);
        let p = procs.get("CAPSS4").expect("CAPSS4 should resolve");
        assert_eq!(p.ptype, "STAR");
        assert_eq!(
            p.common.iter().map(|w| w.0.as_str()).collect::<Vec<_>>(),
            ["THHMP", "BULII"]
        );
        let waves_t = p.transitions.get("WAVES").expect("WAVES transition");
        assert_eq!(
            waves_t.iter().map(|w| w.0.as_str()).collect::<Vec<_>>(),
            ["WAVES", "BULII"]
        );
    }

    /// A fix-less leg (modeling `CA`/`VA` course/heading-to-altitude — no Fix Identifier field)
    /// mid-chain contributes no point but doesn't break the rest of the transition.
    #[test]
    fn fixless_leg_is_skipped_without_breaking_the_chain() {
        let fimbi = terminal_record("SUSA", "KDCA", "FIMBI", "N38495501", "W077070326");
        let mcnab = terminal_record("SUSA", "KDCA", "MCNAB", "N38454105", "W077014322");
        let leg1 = proc_leg("SUSA", "KDCA", b'D', "AMEEE1", "RW19", 10, "", b' ', b' '); // VA, no fix
        let leg2 = proc_leg(
            "SUSA", "KDCA", b'D', "AMEEE1", "RW19", 20, "FIMBI", b'P', b'C',
        );
        let leg3 = proc_leg(
            "SUSA", "KDCA", b'D', "AMEEE1", "RW19", 30, "MCNAB", b'P', b'C',
        );
        let text = [fimbi, mcnab, leg1, leg2, leg3].join("\r\n");

        let procs = parse_procedures(&text);
        let rw19 = &procs["AMEEE1"].transitions["RW19"];
        assert_eq!(
            rw19.iter().map(|w| w.0.as_str()).collect::<Vec<_>>(),
            ["FIMBI", "MCNAB"]
        );
    }

    /// The same procedure/transition filed identically at two airports (a metroplex-shared
    /// chart, VATUSA/OIS#253 — e.g. Detroit's `FOREY3` STAR spans ten real airports) folds into
    /// one clean, non-duplicated leg list, and both airports show up in `apt`.
    #[test]
    fn metroplex_duplicate_airports_fold_into_one_transition() {
        let bobct = record(|f| {
            f.section = b'E';
            f.subsection = b'A';
            f.id = "BOBCT";
            f.lat = "N40325562";
            f.lon = "W082004629";
        });
        let wwshr = record(|f| {
            f.section = b'E';
            f.subsection = b'A';
            f.id = "WWSHR";
            f.lat = "N41203409";
            f.lon = "W082030576";
        });
        let karb1 = proc_leg(
            "SUSA", "KARB", b'E', "FOREY3", "BOBCT", 10, "BOBCT", b'E', b'A',
        );
        let karb2 = proc_leg(
            "SUSA", "KARB", b'E', "FOREY3", "BOBCT", 20, "WWSHR", b'E', b'A',
        );
        let kdet1 = proc_leg(
            "SUSA", "KDET", b'E', "FOREY3", "BOBCT", 10, "BOBCT", b'E', b'A',
        );
        let kdet2 = proc_leg(
            "SUSA", "KDET", b'E', "FOREY3", "BOBCT", 20, "WWSHR", b'E', b'A',
        );
        let text = [bobct, wwshr, karb1, karb2, kdet1, kdet2].join("\r\n");

        let procs = parse_procedures(&text);
        let p = &procs["FOREY3"];
        assert_eq!(p.apt, vec!["KARB".to_string(), "KDET".to_string()]);
        let bobct_t = &p.transitions["BOBCT"];
        assert_eq!(
            bobct_t.iter().map(|w| w.0.as_str()).collect::<Vec<_>>(),
            ["BOBCT", "WWSHR"],
            "must fold to one clean copy, not duplicate the two airports' identical legs"
        );
    }

    /// A continuation record (Continuation Record No. at column 39 set to `'2'`) on a procedure
    /// leg must not be read as an extra point.
    #[test]
    fn procedure_continuation_record_is_excluded() {
        let revge = terminal_record("SUSA", "KDCA", "REVGE", "N38512048", "W077005674");
        let leg1 = proc_leg(
            "SUSA", "KDCA", b'D', "AMEEE1", "RW01", 10, "REVGE", b'P', b'C',
        );
        let mut cont = leg1.clone().into_bytes();
        cont[29..29 + 5].copy_from_slice(b"ZZZZZ"); // would-be extra point, unresolvable anyway
        cont[38] = b'2';
        let cont = String::from_utf8(cont).unwrap();
        let text = [revge, leg1, cont].join("\r\n");

        let procs = parse_procedures(&text);
        let rw01 = &procs["AMEEE1"].transitions["RW01"];
        assert_eq!(rw01.len(), 1);
        assert_eq!(rw01[0].0, "REVGE");
    }
}
