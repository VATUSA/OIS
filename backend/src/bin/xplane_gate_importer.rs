//! Offline importer: bundles airport parking-stand (gate) positions from the **X-Plane Scenery
//! Gateway** into a committed data file, for the airports OIS already carries FAA surface geometry
//! for.
//!
//! Run by hand, occasionally, to refresh `backend/data/xplane_gates.json` — never called at runtime
//! or from CI, exactly like `faa_surface_importer`.
//!
//! ```text
//! cargo run -p ois-backend --bin xplane-gate-importer
//! ```
//!
//! ## Why this source, and what choosing it accepts
//!
//! The FAA Aerodrome Mapping layer set has **no parking-stand layer** — re-confirmed on 2026-09-30 by
//! enumerating all 106 services published by `services6.arcgis.com/ssFJjBXIUyZDrSYZ` and grepping
//! every layer name for `stand|park|gate|dock|bay|ramp|position`. So the MIT-clean, public-domain
//! source that supplies taxiways, aprons and runways cannot supply gates (VATUSA/OIS#431 AC1).
//!
//! The Gateway is the practical machine-readable alternative, and it is richer than what we store:
//! row `1300` gives position, heading, stand type and aircraft class, row `1301` the ICAO width code
//! and airline codes. **Its licence is unstated** — there is no licence file in a scenery pack, the
//! Gateway is a single-page app so its terms are not machine-readable, and the data is
//! community-contributed rather than surveyed. Using it is a risk the project owner accepted
//! deliberately, over CRC/vNAS profiles and over re-litigating OSM's ODbL; the reasoning is recorded
//! on VATUSA/OIS#431. This paragraph exists so the next person finds that decision here rather than
//! rediscovering the question.
//!
//! ## Output schema
//!
//! `backend/data/xplane_gates.json`, a JSON object keyed by uppercase ICAO. Only airports present in
//! `faa_surface.json` are fetched, so both extracts describe the same airport set.
//!
//! ```json
//! { "KDCA": [ { "name": "E57", "lat": 38.858, "lon": -77.043, "kind": "gate", "heading": 209.1 } ] }
//! ```
//!
//! `kind` is X-Plane's stand type verbatim (`gate` | `tie_down` | `misc` | `hangar`). All four are
//! imported: a tie-down is a real parking position, and knowing an aircraft is at one is what lets
//! the taxi-observation collector treat a zero pushback as a measurement rather than a session that
//! began mid-departure.
//!
//! `heading` is **normalised** into `[0, 360)`. The Gateway serves it unnormalised — `-510.9`,
//! `-377.8` and `-397.3` all appear in KDCA's first three rows — so a consumer that trusted it raw
//! would compute nonsense. It is carried here but **not stored**: the model has no heading column and
//! nothing would read it (VATUSA/OIS#431 scopes name/lat/lon/kind).
//!
//! Stand names are made **unique within an airport**: 121 of the 183 airports repeat a name (KATW has
//! two stands called "South Ramp"). Duplicates get a ` #2`, ` #3` … suffix. This is not cosmetic — the
//! seed's per-airport re-pull matches on `(icao, name)` so that gate ids survive a refresh, and ids
//! surviving is what keeps `stats.taxi_observation.gate_id` (on delete set null) from being wiped
//! along with that airport's learned taxi history. Two stands sharing a name would make that match
//! ambiguous. Operators can rename either one in the gate editor.

use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::time::Duration;

use base64::Engine;
use serde::{Deserialize, Serialize};

/// Polite spacing between per-airport scenery fetches. The Gateway is a community service, and a full
/// run is ~185 requests; there is no reason to hammer it.
const FETCH_DELAY: Duration = Duration::from_millis(150);

const AIRPORTS_URL: &str = "https://gateway.x-plane.com/apiv1/airports";

#[derive(Debug, Deserialize)]
struct GatewayAirports {
    airports: Vec<GatewayAirport>,
}

#[derive(Debug, Deserialize)]
struct GatewayAirport {
    #[serde(rename = "AirportCode")]
    code: String,
    /// The pack the Gateway itself recommends — curated, so this is the one to take rather than the
    /// most recent submission.
    #[serde(rename = "RecommendedSceneryId")]
    recommended_scenery_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct GatewaySceneryEnvelope {
    scenery: GatewayScenery,
}

#[derive(Debug, Deserialize)]
struct GatewayScenery {
    /// The whole pack, base64-encoded, zipped. There is no endpoint serving the `apt.dat` alone.
    #[serde(rename = "masterZipBlob")]
    master_zip_blob: String,
}

/// One parking stand, as committed to the extract.
#[derive(Debug, Serialize)]
struct Stand {
    name: String,
    lat: f64,
    lon: f64,
    kind: String,
    heading: f64,
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(90))
        .user_agent("OIS-xplane-gate-importer (VATUSA/OIS)")
        .build()
        .expect("http client should build")
}

/// Parse the `1300` startup-location rows out of an `apt.dat`.
///
/// Row shape: `1300 <lat> <lon> <heading> <type> <aircraft classes> <name...>`. The name is the rest
/// of the line — stand names contain spaces ("South Ramp", "Bohlke International Airways/Hanger") —
/// so it is joined rather than taken as one field.
///
/// `1301` rows (ICAO width code, operation type, airline codes) are deliberately ignored: nothing in
/// the model stores them, and parsing data we discard would invite someone to trust it later.
fn parse_stands(dat: &str) -> Vec<Stand> {
    let mut out = Vec::new();
    for line in dat.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.first() != Some(&"1300") || parts.len() < 7 {
            continue;
        }
        let (Ok(lat), Ok(lon), Ok(heading)) = (
            parts[1].parse::<f64>(),
            parts[2].parse::<f64>(),
            parts[3].parse::<f64>(),
        ) else {
            continue;
        };
        let name = parts[6..].join(" ");
        if name.trim().is_empty() {
            continue;
        }
        out.push(Stand {
            name,
            lat,
            lon,
            kind: parts[4].to_string(),
            // Normalised here, once, so no consumer has to know the source serves it unbounded.
            heading: normalise_heading(heading),
        });
    }
    out
}

/// Make every name in `stands` unique, in file order, by suffixing repeats ` #2`, ` #3` …
///
/// The seed's re-pull matches stands on `(icao, name)` to preserve gate ids (and with them the
/// `stats.taxi_observation` rows that reference them), so a repeated name would make the match
/// ambiguous. Suffixed names stay within the editor's 64-character limit: the longest raw name in the
/// extract is 35 characters.
fn disambiguate(stands: &mut [Stand]) {
    let mut seen: HashMap<String, usize> = HashMap::new();
    for s in stands.iter_mut() {
        let n = seen.entry(s.name.clone()).or_insert(0);
        *n += 1;
        if *n > 1 {
            s.name = format!("{} #{}", s.name, *n);
        }
    }
}

/// Fold a heading into `[0, 360)`. `rem_euclid` rather than `%`, which keeps the sign in Rust.
fn normalise_heading(deg: f64) -> f64 {
    (deg.rem_euclid(360.0) * 10.0).round() / 10.0
}

/// Pull the `{ICAO}.dat` out of a base64'd scenery pack.
fn apt_dat_from_blob(blob: &str) -> Result<String, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(blob.trim())
        .map_err(|e| format!("blob is not base64: {e}"))?;
    let mut zip =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| format!("not a zip: {e}"))?;
    let name = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string()))
        .find(|n| n.to_ascii_lowercase().ends_with(".dat"))
        .ok_or_else(|| "pack contains no .dat".to_string())?;
    let mut text = String::new();
    zip.by_name(&name)
        .map_err(|e| format!("cannot read {name}: {e}"))?
        .read_to_string(&mut text)
        .map_err(|e| format!("{name} is not utf-8: {e}"))?;
    Ok(text)
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let http = client();
    let out_path = concat!(env!("CARGO_MANIFEST_DIR"), "/data/xplane_gates.json");

    // Only the airports we already hold surface geometry for, so the two extracts stay aligned.
    // Read at runtime rather than `include_str!`: this is a hand-run tool sitting next to the file,
    // and compiling 13.9 MB into the binary buys nothing.
    let faa_path = concat!(env!("CARGO_MANIFEST_DIR"), "/data/faa_surface.json");
    let faa: HashMap<String, serde_json::Value> = serde_json::from_str(
        &std::fs::read_to_string(faa_path).expect("faa_surface.json should be readable"),
    )
    .expect("faa_surface.json should parse");
    let mut wanted: Vec<String> = faa.into_keys().collect();
    wanted.sort();
    tracing::info!(airports = wanted.len(), "airports to look up");

    tracing::info!("fetching the Gateway airport index");
    let index: GatewayAirports = http
        .get(AIRPORTS_URL)
        .send()
        .await
        .expect("airport index request failed")
        .json()
        .await
        .expect("airport index should deserialize");
    let by_icao: HashMap<&str, &GatewayAirport> = index
        .airports
        .iter()
        .map(|a| (a.code.as_str(), a))
        .collect();

    let mut extract: BTreeMap<String, Vec<Stand>> = BTreeMap::new();
    let (mut no_record, mut failed) = (Vec::new(), Vec::new());

    for (n, icao) in wanted.iter().enumerate() {
        let Some(scenery_id) = by_icao
            .get(icao.as_str())
            .and_then(|a| a.recommended_scenery_id)
        else {
            no_record.push(icao.clone());
            continue;
        };

        let url = format!("https://gateway.x-plane.com/apiv1/scenery/{scenery_id}");
        let fetched: Result<String, String> = async {
            let envelope: GatewaySceneryEnvelope = http
                .get(&url)
                .send()
                .await
                .map_err(|e| format!("request failed: {e}"))?
                .json()
                .await
                .map_err(|e| format!("response did not deserialize: {e}"))?;
            apt_dat_from_blob(&envelope.scenery.master_zip_blob)
        }
        .await;

        match fetched {
            Ok(dat) => {
                let mut stands = parse_stands(&dat);
                disambiguate(&mut stands);
                if !stands.is_empty() {
                    extract.insert(icao.clone(), stands);
                }
            }
            Err(e) => {
                tracing::warn!(icao = %icao, error = %e, "skipping airport");
                failed.push(icao.clone());
            }
        }
        if (n + 1) % 25 == 0 {
            tracing::info!(done = n + 1, with_stands = extract.len(), "progress");
        }
        tokio::time::sleep(FETCH_DELAY).await;
    }

    let total: usize = extract.values().map(Vec::len).sum();
    let out = serde_json::to_string(&extract).expect("extract should serialize");
    std::fs::write(out_path, out).expect("failed to write output file");
    tracing::info!(
        airports_with_stands = extract.len(),
        stands = total,
        no_gateway_record = no_record.len(),
        fetch_failed = failed.len(),
        "wrote {out_path}"
    );
    if !no_record.is_empty() {
        tracing::info!(icaos = ?no_record, "no Gateway record — left as-is");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Gateway serves headings unnormalised — these three are real values from KDCA's pack — so a
    /// consumer trusting them raw would compute nonsense.
    #[test]
    fn headings_are_folded_into_one_turn() {
        assert_eq!(normalise_heading(-510.9), 209.1);
        assert_eq!(normalise_heading(-377.8), 342.2);
        assert_eq!(normalise_heading(45.0), 45.0);
        assert_eq!(normalise_heading(360.0), 0.0);
        assert_eq!(normalise_heading(720.5), 0.5);
    }

    /// The name is the rest of the line: stand names contain spaces, and taking one field truncates
    /// "South Ramp" to "South".
    #[test]
    fn a_stand_name_keeps_its_spaces() {
        let dat = "1300  38.85853226 -077.04336496 -510.9 gate jets E57\n\
                   1300  38.1 -77.1 90 tie_down all South Ramp West\n";
        let stands = parse_stands(dat);
        assert_eq!(stands.len(), 2);
        assert_eq!(stands[0].name, "E57");
        assert_eq!(stands[0].kind, "gate");
        assert_eq!(stands[0].heading, 209.1);
        assert_eq!(stands[1].name, "South Ramp West");
        assert_eq!(stands[1].kind, "tie_down");
    }

    /// `1301` carries the width code and airline list; we store neither, so it must not be mistaken
    /// for a stand of its own.
    #[test]
    fn only_1300_rows_become_stands() {
        let dat = "1300  38.1 -77.1 90 gate jets A1\n\
                   1301 B airline aal dal\n\
                   1 123 0 0 KDCA Ronald Reagan\n\
                   100 29.87 3 0 ...\n";
        let stands = parse_stands(dat);
        assert_eq!(stands.len(), 1);
        assert_eq!(stands[0].name, "A1");
    }

    /// Repeated names would make the seed's `(icao, name)` re-pull match ambiguous, and an ambiguous
    /// match is what would silently wipe an airport's learned taxi history.
    #[test]
    fn repeated_stand_names_are_disambiguated() {
        let mut stands = parse_stands(
            "1300 38.1 -77.1 90 tie_down all South Ramp\n\
             1300 38.2 -77.2 90 tie_down all North Ramp\n\
             1300 38.3 -77.3 90 tie_down all South Ramp\n\
             1300 38.4 -77.4 90 tie_down all South Ramp\n",
        );
        disambiguate(&mut stands);
        let names: Vec<&str> = stands.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            ["South Ramp", "North Ramp", "South Ramp #2", "South Ramp #3"],
            "the first keeps its name; later repeats are suffixed in file order"
        );
    }

    /// A malformed row is skipped rather than panicking the whole import — one bad line in a
    /// community-contributed pack must not cost us an airport.
    #[test]
    fn a_malformed_row_is_skipped_not_fatal() {
        let dat = "1300 not-a-number -77.1 90 gate jets A1\n\
                   1300  38.1 -77.1 90 gate jets\n\
                   1300  38.2 -77.2 90 gate jets B2\n";
        let stands = parse_stands(dat);
        assert_eq!(stands.len(), 1, "only the well-formed row survives");
        assert_eq!(stands[0].name, "B2");
    }
}
