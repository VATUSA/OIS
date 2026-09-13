//! Runtime NASR data fetch. Assembles a fresh [`NavData`] from live sources so the route
//! engine stays current without a redeploy:
//!   - **FAA FIX/NAV/PFR CSV** (28-day cycle, `nfdc.faa.gov`) → fixes, navaids, preferred
//!     routes. Authoritative and timely; this is what the compile-time bundle was built
//!     from.
//!   - **@squawk airway/procedure snapshots** (gzipped JSON on unpkg) → airways, SID/STAR.
//!
//! Every component falls back to the compile-time bundle when its source is unreachable,
//! so the returned dataset is always coherent and never worse than the seed. The refresh
//! job compares the cycle before hot-swapping.

use std::collections::HashMap;
use std::io::{Cursor, Read};

use chrono::{Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use super::nav::{CoordList, NavData};

type Fetched<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Geographic coverage box `[minLat, minLon, maxLat, maxLon]`. Unbounded (whole globe) so we
/// keep every point the sources provide — FAA NASR covers the entire US NAS (incl. Hawaii,
/// Alaska, Puerto Rico, Guam), not just CONUS. Narrow this to re-scope.
const COVERAGE: [f64; 4] = [-90.0, -180.0, 90.0, 180.0];

/// First NASR effective date we anchor the 28-day cycle math on (a known boundary).
const CYCLE_ANCHOR: (i32, u32, u32) = (2026, 7, 9);

const SQUAWK_AIRWAYS: &str = "https://unpkg.com/@squawk/airway-data@0.5.10/data/airways.json.gz";
const SQUAWK_PROCEDURES: &str =
    "https://unpkg.com/@squawk/procedure-data@0.7.8/data/procedures.json.gz";
const SQUAWK_FIXES: &str = "https://unpkg.com/@squawk/fix-data@0.6.10/data/fixes.json.gz";
const SQUAWK_NAVAIDS: &str = "https://unpkg.com/@squawk/navaid-data@0.6.10/data/navaids.json.gz";

// Compile-time bundle, used as the ultimate fallback for each component.
const BUNDLED_FIXES: &str = include_str!("../../data/nav/fixes.json");
const BUNDLED_NAVAIDS: &str = include_str!("../../data/nav/navaids.json");
const BUNDLED_AIRWAYS: &str = include_str!("../../data/nav/airways.json");
const BUNDLED_PROCEDURES: &str = include_str!("../../data/nav/procedures.json");
const BUNDLED_PREFERRED: &str = include_str!("../../data/nav/preferred.json");
// Magnetic variation is quasi-static (drifts over years), so the bundled table is reused
// across NASR cycle refreshes rather than re-fetched.
const BUNDLED_NAVVAR: &str = include_str!("../../data/nav/navvar.json");

/// Fetch the latest NASR data and assemble a fresh [`NavData`]. Best-effort per component
/// with bundle fallback; returns `Err` only if assembly itself fails (never for a single
/// unreachable source).
pub async fn fetch_latest() -> Fetched<NavData> {
    let client = reqwest::Client::builder()
        .user_agent("ois-nav/1.0 (+https://vatusa.net)")
        .timeout(std::time::Duration::from_secs(90))
        .build()?;

    // 1. Base fixes/navaids/preferred + cycle: FAA cycle → @squawk → bundle.
    let mut fixes: HashMap<String, CoordList> = HashMap::new();
    let mut navaids: HashMap<String, CoordList> = HashMap::new();
    let mut preferred_json = BUNDLED_PREFERRED.to_string();
    let mut cycle = format!(
        "{:04}-{:02}-{:02}",
        CYCLE_ANCHOR.0, CYCLE_ANCHOR.1, CYCLE_ANCHOR.2
    );
    let mut source = "bundle";

    match fetch_faa(&client).await {
        Ok(faa) => {
            fixes = faa.fixes;
            navaids = faa.navaids;
            if !faa.preferred.is_empty() {
                preferred_json = serde_json::to_string(&faa.preferred)?;
            }
            cycle = faa.cycle;
            source = "faa";
        }
        Err(e) => {
            tracing::warn!(error = %e, "FAA NASR fetch failed; trying @squawk base");
            if let Ok((f, n)) = fetch_squawk_base(&client).await {
                fixes = f;
                navaids = n;
                source = "squawk";
            }
        }
    }

    if fixes.is_empty() {
        tracing::warn!("no live fixes fetched; seeding fixes/navaids from bundle");
        fixes = serde_json::from_str(BUNDLED_FIXES).unwrap_or_default();
        navaids = serde_json::from_str(BUNDLED_NAVAIDS).unwrap_or_default();
    }

    // 2. Airways + procedures from @squawk; on failure use the bundled blobs as-is.
    let (airways_json, procedures_json) = match fetch_squawk_enroute(&client, &mut fixes).await {
        Ok((a, p, sq_cycle)) => {
            // If FAA didn't provide a cycle, reflect the @squawk snapshot's.
            if source != "faa"
                && let Some(c) = sq_cycle
            {
                cycle = c;
            }
            (a, p)
        }
        Err(e) => {
            tracing::warn!(error = %e, "@squawk enroute fetch failed; using bundled airways/procedures");
            (BUNDLED_AIRWAYS.to_string(), BUNDLED_PROCEDURES.to_string())
        }
    };

    let fixes_json = serde_json::to_string(&fixes)?;
    let navaids_json = serde_json::to_string(&navaids)?;
    let meta_json = serde_json::to_string(&OutMeta {
        nasr_cycle_date: cycle,
        source: format!("runtime fetch ({source})"),
        bbox: COVERAGE,
    })?;

    Ok(NavData::from_json(
        &navaids_json,
        &fixes_json,
        &airways_json,
        &procedures_json,
        &preferred_json,
        &meta_json,
        BUNDLED_NAVVAR,
    ))
}

// --- FAA 28-day CSV path ---

struct FaaData {
    fixes: HashMap<String, CoordList>,
    navaids: HashMap<String, CoordList>,
    preferred: HashMap<String, String>,
    cycle: String,
}

/// Download FIX/NAV/PFR for the most recent published cycle (walking back if the newest
/// isn't up yet).
async fn fetch_faa(client: &reqwest::Client) -> Fetched<FaaData> {
    let mut last_err: Option<Box<dyn std::error::Error + Send + Sync>> = None;
    for date in candidate_cycles() {
        match fetch_faa_cycle(client, date).await {
            Ok(d) => return Ok(d),
            Err(e) => {
                tracing::debug!(cycle = %date, error = %e, "FAA cycle unavailable");
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| "no FAA cycle candidates".into()))
}

async fn fetch_faa_cycle(client: &reqwest::Client, date: NaiveDate) -> Fetched<FaaData> {
    let fix_zip = download(client, &faa_url(date, "FIX")).await?;
    let nav_zip = download(client, &faa_url(date, "NAV")).await?;

    let fix_csv = read_zip_member(&fix_zip, &["FIX_BASE.csv", "FIX.csv"])?;
    let nav_csv = read_zip_member(&nav_zip, &["NAV_BASE.csv", "NAV.csv"])?;
    let fixes = build_fixes(&fix_csv);
    let navaids = build_navaids(&nav_csv);
    if fixes.is_empty() || navaids.is_empty() {
        return Err("FAA FIX/NAV parsed empty".into());
    }

    // Preferred routes ride the same cycle but may 404 on off-cycles; tolerate it.
    let preferred = match download(client, &faa_url(date, "PFR")).await {
        Ok(zip) => read_zip_member(&zip, &["PFR_BASE.csv", "PFR.csv"])
            .map(|csv| build_preferred(&csv))
            .unwrap_or_default(),
        Err(_) => HashMap::new(),
    };

    Ok(FaaData {
        fixes,
        navaids,
        preferred,
        cycle: date.format("%Y-%m-%d").to_string(),
    })
}

/// The current 28-day cycle and the two before it (fallbacks when the newest isn't posted).
fn candidate_cycles() -> Vec<NaiveDate> {
    let anchor = NaiveDate::from_ymd_opt(CYCLE_ANCHOR.0, CYCLE_ANCHOR.1, CYCLE_ANCHOR.2)
        .expect("valid cycle anchor");
    let today = Utc::now().date_naive();
    let days = (today - anchor).num_days();
    let n = if days >= 0 { days / 28 } else { 0 };
    let current = anchor + Duration::days(28 * n);
    vec![
        current,
        current - Duration::days(28),
        current - Duration::days(56),
    ]
}

fn faa_url(date: NaiveDate, group: &str) -> String {
    // FAA "extra" subscription path: e.g. 06_Aug_2026_FIX_CSV.zip
    format!(
        "https://nfdc.faa.gov/webContent/28DaySub/extra/{}_{group}_CSV.zip",
        date.format("%d_%b_%Y")
    )
}

// --- @squawk gzipped-JSON path ---

/// Fetch @squawk airways + procedures, transform into the bundled schema, and fold their
/// waypoint coordinates into `fixes` (so enroute fix names always resolve). Returns the
/// two JSON blobs plus the snapshot's NASR cycle date.
async fn fetch_squawk_enroute(
    client: &reqwest::Client,
    fixes: &mut HashMap<String, CoordList>,
) -> Fetched<(String, String, Option<String>)> {
    let awy_pack: SqPack<SqAirway> = fetch_gz_json(client, SQUAWK_AIRWAYS).await?;
    let proc_pack: SqPack<SqProc> = fetch_gz_json(client, SQUAWK_PROCEDURES).await?;

    let mut airways: HashMap<String, OutAirway> = HashMap::new();
    for a in &awy_pack.records {
        let Some(des) = a.designation.as_deref().map(str::to_uppercase) else {
            continue;
        };
        let mut w = Vec::new();
        for wp in &a.waypoints {
            let (Some(lat), Some(lon)) = (wp.lat, wp.lon) else {
                continue;
            };
            if !in_coverage(lat, lon) {
                continue;
            }
            let id = wp
                .identifier
                .as_deref()
                .or(wp.name.as_deref())
                .unwrap_or("")
                .to_uppercase();
            w.push((id.clone(), round5(lat), round5(lon)));
            if !id.is_empty() {
                add_candidate(fixes, &id, lat, lon);
            }
        }
        if w.len() >= 2 {
            let t = des.chars().next().map(String::from).unwrap_or_default();
            airways.insert(des, OutAirway { t, w });
        }
    }

    let mut procedures: HashMap<String, OutProc> = HashMap::new();
    for p in &proc_pack.records {
        let typ = p.ptype.as_deref().unwrap_or("").to_uppercase();
        if typ != "SID" && typ != "STAR" {
            continue;
        }
        let id: String = p
            .identifier
            .as_deref()
            .or(p.name.as_deref())
            .unwrap_or("")
            .to_uppercase()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        if id.len() < 4 {
            continue;
        }

        let mut common = Vec::new();
        for route in &p.common_routes {
            for leg in &route.legs {
                push_leg(&mut common, leg, fixes);
            }
        }
        let mut transitions: HashMap<String, Vec<(String, f64, f64)>> = HashMap::new();
        for tr in &p.transitions {
            let Some(tname) = tr.name.as_deref().or(tr.identifier.as_deref()) else {
                continue;
            };
            let mut tlegs = Vec::new();
            for leg in &tr.legs {
                push_leg(&mut tlegs, leg, fixes);
            }
            if tlegs.len() >= 2 {
                transitions.insert(tname.to_uppercase(), tlegs);
            }
        }
        if common.len() < 2 && transitions.is_empty() {
            continue;
        }

        let out = OutProc {
            ptype: typ,
            apt: p.airports.iter().map(|a| a.to_uppercase()).collect(),
            common,
            transitions,
        };
        // Alias without the trailing revision digit(s), e.g. DOTSS2 → DOTSS.
        let base = strip_revision(&id);
        if base.len() >= 4 {
            procedures.entry(base).or_insert_with(|| out.clone());
        }
        procedures.insert(id, out);
    }

    let cycle = awy_pack
        .meta
        .nasr_cycle_date
        .or(proc_pack.meta.nasr_cycle_date);
    Ok((
        serde_json::to_string(&airways)?,
        serde_json::to_string(&procedures)?,
        cycle,
    ))
}

/// Fallback base fixes/navaids from @squawk when the FAA cycle is unreachable.
async fn fetch_squawk_base(
    client: &reqwest::Client,
) -> Fetched<(HashMap<String, CoordList>, HashMap<String, CoordList>)> {
    let fix_pack: SqPack<SqPoint> = fetch_gz_json(client, SQUAWK_FIXES).await?;
    let nav_pack: SqPack<SqPoint> = fetch_gz_json(client, SQUAWK_NAVAIDS).await?;
    let mut fixes = HashMap::new();
    let mut navaids = HashMap::new();
    for r in &fix_pack.records {
        if let (Some(id), Some(lat), Some(lon)) = (r.identifier.as_deref(), r.lat, r.lon) {
            add_candidate(&mut fixes, &id.to_uppercase(), lat, lon);
        }
    }
    for r in &nav_pack.records {
        let (Some(lat), Some(lon)) = (r.lat, r.lon) else {
            continue;
        };
        if let Some(id) = r.identifier.as_deref() {
            add_candidate(&mut navaids, &id.to_uppercase(), lat, lon);
        }
        if let Some(name) = r.name.as_deref() {
            add_candidate(&mut navaids, &name.to_uppercase(), lat, lon);
        }
    }
    if fixes.is_empty() {
        return Err("@squawk fixes empty".into());
    }
    Ok((fixes, navaids))
}

// --- CSV parsing (FAA NASR) ---

fn build_fixes(csv: &str) -> HashMap<String, CoordList> {
    let mut out = HashMap::new();
    let Some(rows) = CsvRows::new(csv) else {
        return out;
    };
    let (id_i, lat_i, lon_i) = (
        rows.col(&["FIX_ID", "FIX_ID_OLD", "IDENT"]),
        rows.col(&["LAT_DECIMAL", "LAT"]),
        rows.col(&["LONG_DECIMAL", "LON"]),
    );
    for row in rows {
        if let (Some(id), Some(lat), Some(lon)) =
            (field(&row, id_i), num(&row, lat_i), num(&row, lon_i))
        {
            add_candidate(&mut out, &id.to_uppercase(), lat, lon);
        }
    }
    out
}

fn build_navaids(csv: &str) -> HashMap<String, CoordList> {
    let mut out = HashMap::new();
    let Some(rows) = CsvRows::new(csv) else {
        return out;
    };
    let (id_i, name_i, lat_i, lon_i) = (
        rows.col(&["NAV_ID", "IDENT"]),
        rows.col(&["NAME"]),
        rows.col(&["LAT_DECIMAL", "LAT"]),
        rows.col(&["LONG_DECIMAL", "LON"]),
    );
    for row in rows {
        let (Some(lat), Some(lon)) = (num(&row, lat_i), num(&row, lon_i)) else {
            continue;
        };
        if let Some(id) = field(&row, id_i) {
            add_candidate(&mut out, &id.to_uppercase(), lat, lon);
        }
        if let Some(name) = field(&row, name_i) {
            add_candidate(&mut out, &name.to_uppercase(), lat, lon);
        }
    }
    out
}

fn build_preferred(csv: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Some(rows) = CsvRows::new(csv) else {
        return out;
    };
    let (dep_i, arr_i, route_i) = (
        rows.col(&["ORIGIN_ID", "DEPARTURE_AIRPORT", "DEPARTURE"]),
        rows.col(&["DSTN_ID", "ARRIVAL_AIRPORT", "ARRIVAL"]),
        rows.col(&["ROUTE_STRING", "ROUTE"]),
    );
    for row in rows {
        if let (Some(dep), Some(arr), Some(route)) =
            (field(&row, dep_i), field(&row, arr_i), field(&row, route_i))
            && !route.trim().is_empty()
        {
            out.insert(
                format!("{}|{}", dep.to_uppercase(), arr.to_uppercase()),
                route.trim().to_uppercase(),
            );
        }
    }
    out
}

/// Line-oriented CSV reader (one record per line; quoted fields may contain commas).
struct CsvRows {
    header: Vec<String>,
    lines: std::vec::IntoIter<String>,
}

impl CsvRows {
    fn new(text: &str) -> Option<Self> {
        let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
        if lines.is_empty() {
            return None;
        }
        let header = parse_csv_line(&lines.remove(0));
        Some(Self {
            header,
            lines: lines.into_iter(),
        })
    }

    /// Index of the first header name present, or `None`.
    fn col(&self, names: &[&str]) -> Option<usize> {
        names
            .iter()
            .find_map(|n| self.header.iter().position(|h| h.eq_ignore_ascii_case(n)))
    }
}

impl Iterator for CsvRows {
    type Item = Vec<String>;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let line = self.lines.next()?;
            if line.trim().is_empty() {
                continue;
            }
            return Some(parse_csv_line(&line));
        }
    }
}

fn parse_csv_line(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if in_q {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    cur.push('"');
                    chars.next();
                } else {
                    in_q = false;
                }
            } else {
                cur.push(c);
            }
        } else if c == '"' {
            in_q = true;
        } else if c == ',' {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

fn field(row: &[String], idx: Option<usize>) -> Option<&str> {
    let v = row.get(idx?)?.trim();
    (!v.is_empty()).then_some(v)
}

fn num(row: &[String], idx: Option<usize>) -> Option<f64> {
    field(row, idx)?
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
}

// --- shared helpers ---

async fn download(client: &reqwest::Client, url: &str) -> Fetched<Vec<u8>> {
    let bytes = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    Ok(bytes.to_vec())
}

async fn fetch_gz_json<T: for<'de> Deserialize<'de>>(
    client: &reqwest::Client,
    url: &str,
) -> Fetched<T> {
    let gz = download(client, url).await?;
    let mut decoder = flate2::read::GzDecoder::new(&gz[..]);
    let mut json = String::new();
    decoder.read_to_string(&mut json)?;
    Ok(serde_json::from_str(&json)?)
}

fn read_zip_member(bytes: &[u8], names: &[&str]) -> Fetched<String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    for name in names {
        if let Ok(mut f) = archive.by_name(name) {
            let mut s = String::new();
            f.read_to_string(&mut s)?;
            return Ok(s);
        }
    }
    Err(format!("zip has none of {names:?}").into())
}

fn in_coverage(lat: f64, lon: f64) -> bool {
    (COVERAGE[0]..=COVERAGE[2]).contains(&lat) && (COVERAGE[1]..=COVERAGE[3]).contains(&lon)
}

fn round5(n: f64) -> f64 {
    (n * 1e5).round() / 1e5
}

/// Append a `(lat, lon)` candidate for `id`, in-COVERAGE and de-duplicated.
fn add_candidate(map: &mut HashMap<String, CoordList>, id: &str, lat: f64, lon: f64) {
    if id.is_empty() || !lat.is_finite() || !lon.is_finite() || !in_coverage(lat, lon) {
        return;
    }
    let pt = [round5(lat), round5(lon)];
    let entry = map.entry(id.to_string()).or_default();
    if !entry.iter().any(|p| p[0] == pt[0] && p[1] == pt[1]) {
        entry.push(pt);
    }
}

fn push_leg(
    out: &mut Vec<(String, f64, f64)>,
    leg: &SqLeg,
    fixes: &mut HashMap<String, CoordList>,
) {
    let (Some(lat), Some(lon)) = (leg.lat, leg.lon) else {
        return;
    };
    if !in_coverage(lat, lon) {
        return;
    }
    let fix = leg.fix_identifier.as_deref().unwrap_or("").to_uppercase();
    out.push((fix.clone(), round5(lat), round5(lon)));
    if !fix.is_empty() {
        add_candidate(fixes, &fix, lat, lon);
    }
}

/// Strip a trailing revision like `2` or `2A` from a procedure id (`DOTSS2` → `DOTSS`).
fn strip_revision(id: &str) -> String {
    let bytes = id.as_bytes();
    let mut end = bytes.len();
    if end > 0 && bytes[end - 1].is_ascii_alphabetic() {
        end -= 1;
    }
    while end > 0 && bytes[end - 1].is_ascii_digit() {
        end -= 1;
    }
    id[..end].to_string()
}

// --- serde shapes ---

#[derive(Deserialize)]
struct SqPack<T> {
    #[serde(default)]
    records: Vec<T>,
    #[serde(default)]
    meta: SqMeta,
}

#[derive(Deserialize, Default)]
struct SqMeta {
    #[serde(rename = "nasrCycleDate")]
    nasr_cycle_date: Option<String>,
}

#[derive(Deserialize, Default)]
struct SqPoint {
    identifier: Option<String>,
    name: Option<String>,
    lat: Option<f64>,
    lon: Option<f64>,
}

#[derive(Deserialize, Default)]
struct SqAirway {
    designation: Option<String>,
    #[serde(default)]
    waypoints: Vec<SqWp>,
}

#[derive(Deserialize)]
struct SqWp {
    identifier: Option<String>,
    name: Option<String>,
    lat: Option<f64>,
    lon: Option<f64>,
}

#[derive(Deserialize, Default)]
struct SqProc {
    #[serde(rename = "type")]
    ptype: Option<String>,
    identifier: Option<String>,
    name: Option<String>,
    #[serde(default)]
    airports: Vec<String>,
    #[serde(default, rename = "commonRoutes")]
    common_routes: Vec<SqRoute>,
    #[serde(default)]
    transitions: Vec<SqTrans>,
}

#[derive(Deserialize)]
struct SqRoute {
    #[serde(default)]
    legs: Vec<SqLeg>,
}

#[derive(Deserialize)]
struct SqTrans {
    name: Option<String>,
    identifier: Option<String>,
    #[serde(default)]
    legs: Vec<SqLeg>,
}

#[derive(Deserialize)]
struct SqLeg {
    #[serde(rename = "fixIdentifier")]
    fix_identifier: Option<String>,
    lat: Option<f64>,
    lon: Option<f64>,
}

#[derive(Serialize)]
struct OutAirway {
    t: String,
    w: Vec<(String, f64, f64)>,
}

#[derive(Serialize, Clone)]
struct OutProc {
    #[serde(rename = "type")]
    ptype: String,
    apt: Vec<String>,
    common: Vec<(String, f64, f64)>,
    transitions: HashMap<String, Vec<(String, f64, f64)>>,
}

#[derive(Serialize)]
struct OutMeta {
    #[serde(rename = "nasrCycleDate")]
    nasr_cycle_date: String,
    source: String,
    bbox: [f64; 4],
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    /// Round-trips a real deflate-compressed archive through `read_zip_member` — a compile-clean
    /// `zip` major bump (e.g. #193's 3→8) doesn't guarantee the `deflate` feature still actually
    /// decompresses correctly at runtime, only that the API shape matches.
    #[test]
    fn read_zip_member_round_trips_a_deflated_entry() {
        let mut buf = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            writer.start_file("NAV_BASE.csv", opts).unwrap();
            writer.write_all(b"hello,world").unwrap();
            writer.finish().unwrap();
        }

        let s = read_zip_member(&buf, &["NAV_BASE.csv", "NAV.csv"]).unwrap();
        assert_eq!(s, "hello,world");
    }

    #[test]
    fn read_zip_member_errs_when_none_of_the_names_are_present() {
        let mut buf = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(Cursor::new(&mut buf));
            let opts = zip::write::SimpleFileOptions::default();
            writer.start_file("OTHER.csv", opts).unwrap();
            writer.finish().unwrap();
        }
        assert!(read_zip_member(&buf, &["NAV_BASE.csv", "NAV.csv"]).is_err());
    }

    #[test]
    fn parses_quoted_csv_fields() {
        let row = parse_csv_line(r#""AAALL","K6","",42.12,"a,b""c""#);
        assert_eq!(row[0], "AAALL");
        assert_eq!(row[2], "");
        assert_eq!(row[3], "42.12");
        assert_eq!(row[4], r#"a,b"c"#);
    }

    #[test]
    fn builds_fixes_from_nasr_csv() {
        let csv = "\"FIX_ID\",\"LAT_DECIMAL\",\"LONG_DECIMAL\"\n\
                   \"RBV\",40.2,-74.5\n\
                   \"OCEAN\",5.0,20.0\n\
                   \"BADLL\",95.0,-74.5\n"; // latitude out of range → dropped
        let fixes = build_fixes(csv);
        assert_eq!(fixes.get("RBV"), Some(&vec![[40.2, -74.5]]));
        assert!(fixes.contains_key("OCEAN")); // no geographic limit now — kept worldwide
        assert!(!fixes.contains_key("BADLL")); // invalid coordinate still rejected
    }

    #[test]
    fn builds_preferred_from_nasr_csv() {
        let csv = "\"ORIGIN_ID\",\"DSTN_ID\",\"ROUTE_STRING\"\n\
                   \"ABE\",\"ACY\",\"FJC ARD CYN\"\n";
        let pref = build_preferred(csv);
        assert_eq!(pref.get("ABE|ACY").map(String::as_str), Some("FJC ARD CYN"));
    }

    #[test]
    fn strips_procedure_revision() {
        assert_eq!(strip_revision("DOTSS2"), "DOTSS");
        assert_eq!(strip_revision("LUCIT3"), "LUCIT");
        assert_eq!(strip_revision("KKISS1A"), "KKISS");
    }

    /// Live end-to-end fetch. Ignored by default (hits FAA + unpkg):
    /// `cargo test --lib feed::nav_source -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "network: downloads FAA NASR + @squawk"]
    async fn live_fetch_assembles_current_cycle() {
        let nav = fetch_latest().await.expect("fetch_latest");
        println!("fetched cycle={} points={}", nav.cycle(), nav.len());
        assert!(
            nav.len() > 60_000,
            "expected tens of thousands of points, got {}",
            nav.len()
        );
        // Airway + procedure expansion must work on the freshly fetched data.
        let ap = HashMap::new();
        let direct = nav.build_anchors(&ap, "", "", "LAX TNP");
        let via = nav.build_anchors(&ap, "", "", "LAX J10 TNP");
        assert!(
            via.anchors.len() > direct.anchors.len(),
            "airway expansion on fetched data ({} vs {})",
            via.anchors.len(),
            direct.anchors.len()
        );
        assert!(!nav.cycle().is_empty(), "cycle date should be populated");
    }

    #[test]
    fn cycle_candidates_are_28_days_apart_and_not_future() {
        let cands = candidate_cycles();
        assert_eq!(cands.len(), 3);
        assert_eq!((cands[0] - cands[1]).num_days(), 28);
        assert!(cands[0] <= Utc::now().date_naive());
    }
}
