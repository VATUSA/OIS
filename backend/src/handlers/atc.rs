//! Online ATC for the map "ATC" layer — no auth, same public exposure as live traffic.
//! Classifies the datafeed's `controllers`/`atis` into airport ground stations (badges),
//! TRACON/approach areas (matched to SimAware polygons), and center positions.

use std::collections::HashMap;

use axum::{Json, extract::State};
use chrono::Utc;

use crate::{
    feed::airports::{AirportDb, IataMap},
    feed::tracon::TraconData,
    feed::vatsim::VatsimData,
    models::{AtcAirport, AtcArea, AtcBoard, AtcCenter, AtcPosition},
    state::AppState,
};

/// VATSIM voice band — drops observers/relief loggers parked on out-of-band frequencies.
fn in_atc_band(freq: &str) -> bool {
    freq.parse::<f64>()
        .map(|f| (117.0..=137.0).contains(&f))
        .unwrap_or(false)
}

fn facility_kind(facility: i32) -> Option<&'static str> {
    match facility {
        2 => Some("DEL"),
        3 => Some("GND"),
        4 => Some("TWR"),
        5 => Some("APP"),
        6 => Some("CTR"),
        _ => None, // 0 OBS, 1 FSS — not drawn
    }
}

/// US ARTCC id for a center callsign's first segment. Center positions log on with the FAA
/// radio prefix (`BOS`, `NY`, `DC`, `LAX`) rather than the `Zxx` id our boundaries use;
/// a bare `Zxx` id (e.g. `ZLA_CTR`) is accepted directly. Prefixes are from VATSpy `[FIRs]`.
/// Non-US centers return `None` (we have no boundary geometry to shade for them).
pub(crate) fn center_artcc(prefix: &str) -> Option<String> {
    if prefix.len() == 3
        && prefix.starts_with('Z')
        && prefix.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        return Some(prefix.to_string());
    }
    let mapped = match prefix {
        "ABQ" => "ZAB",
        "ATL" => "ZTL",
        "BDA" | "NY" => "ZNY",
        "BOS" => "ZBW",
        "CHI" | "ORD" => "ZAU",
        "CLE" => "ZOB",
        "DC" | "WAS" => "ZDC",
        "DEN" => "ZDV",
        "FTW" => "ZFW",
        "HOU" => "ZHU",
        "IND" => "ZID",
        "JAX" => "ZJX",
        "KC" | "MCI" => "ZKC",
        "LA" | "LAX" => "ZLA",
        "MEM" => "ZME",
        "MIA" | "ZMO" => "ZMA",
        "MSP" => "ZMP",
        "OAK" => "ZOA",
        "OO" | "OOR" | "OORO" => "ZAK",
        "SEA" => "ZSE",
        "SLC" => "ZLC",
        "ANC" => "ZAN",
        "HCF" => "ZHN",
        "SJU" => "ZSU",
        _ => return None,
    };
    Some(mapped.to_string())
}

/// Display order of ground positions in an airport's badge stack.
fn kind_rank(kind: &str) -> u8 {
    match kind {
        "DEL" => 0,
        "GND" => 1,
        "TWR" => 2,
        "ATIS" => 3,
        _ => 4,
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/flow/atc",
    tag = "flow",
    responses((status = 200, body = AtcBoard))
)]
pub async fn list_atc(State(state): State<AppState>) -> Json<AtcBoard> {
    let (snapshot, airports, iata) = {
        let guard = state.feed.read().await;
        (
            guard.snapshot.clone(),
            guard.airports.clone(),
            guard.iata.clone(),
        )
    };
    let tracons = state.tracons.load();
    let Some(snap) = snapshot else {
        return Json(AtcBoard {
            airports: Vec::new(),
            tracons: Vec::new(),
            centers: Vec::new(),
            as_of: Utc::now(),
        });
    };
    Json(board_from(&snap.data, &airports, &iata, &tracons))
}

/// Classify a network snapshot's `controllers`/`atis` into airport ground stations (badges),
/// TRACON/approach areas (matched to SimAware polygons), and center positions. Pure of the live
/// feed so the historical endpoint can replay it against a reconstructed snapshot.
pub fn board_from(
    data: &VatsimData,
    airports: &AirportDb,
    iata: &IataMap,
    tracons: &TraconData,
) -> AtcBoard {
    let mut board = AtcBoard {
        airports: Vec::new(),
        tracons: Vec::new(),
        centers: Vec::new(),
        as_of: Utc::now(),
    };

    // Resolve a callsign prefix to (ICAO, lat, lon): direct ICAO, IATA (SFO→KSFO), or the
    // US "K"+code convention.
    let resolve = |code: &str| -> Option<(String, f64, f64)> {
        let code = code.to_ascii_uppercase();
        if let Some(&(lat, lon)) = airports.get(&code) {
            return Some((code, lat, lon));
        }
        if let Some(icao) = iata.get(&code)
            && let Some(&(lat, lon)) = airports.get(icao)
        {
            return Some((icao.clone(), lat, lon));
        }
        let k = format!("K{code}");
        airports.get(&k).map(|&(lat, lon)| (k, lat, lon))
    };

    // Airport ground stations (badges), keyed by ICAO.
    let mut ground: HashMap<String, AtcAirport> = HashMap::new();
    // TRACON areas, keyed by a synthetic feature key so several controllers on the same
    // polygon merge; circle fallbacks keyed by airport.
    let mut areas: HashMap<String, AtcArea> = HashMap::new();
    let mut centers: HashMap<String, AtcCenter> = HashMap::new();

    let mut push_ground = |code: &str, pos: AtcPosition| {
        if let Some((icao, lat, lon)) = resolve(code) {
            ground
                .entry(icao.clone())
                .or_insert_with(|| AtcAirport {
                    icao,
                    lat,
                    lon,
                    positions: Vec::new(),
                })
                .positions
                .push(pos);
        }
    };

    for c in &data.controllers {
        if !in_atc_band(&c.frequency) {
            continue;
        }
        let Some(kind) = facility_kind(c.facility) else {
            continue;
        };
        let prefix = c
            .callsign
            .split('_')
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        let pos = AtcPosition {
            kind: kind.to_string(),
            callsign: c.callsign.clone(),
            frequency: c.frequency.clone(),
            name: c.name.clone(),
            rating: c.rating,
            logon_time: c.logon_time.clone(),
            atis_code: None,
        };
        match kind {
            "DEL" | "GND" | "TWR" => push_ground(&prefix, pos),
            "APP" => {
                if let Some(f) = tracons.match_callsign(&c.callsign) {
                    let key = format!("{}\u{1}{:?}\u{1}{:?}", f.id, f.suffix, f.prefixes);
                    areas
                        .entry(key)
                        .or_insert_with(|| AtcArea {
                            id: f.id.clone(),
                            name: f.name.clone(),
                            label: f.label,
                            positions: Vec::new(),
                            rings: f.rings.clone(),
                            circle: None,
                        })
                        .positions
                        .push(pos);
                } else if let Some((icao, lat, lon)) = resolve(&prefix) {
                    // No polygon matched — draw a labelled circle at the airport instead.
                    areas
                        .entry(format!("circle:{icao}"))
                        .or_insert_with(|| AtcArea {
                            id: prefix.clone(),
                            name: None,
                            label: Some([lat, lon]),
                            positions: Vec::new(),
                            rings: Vec::new(),
                            circle: Some([lat, lon]),
                        })
                        .positions
                        .push(pos);
                }
            }
            "CTR" => {
                // Map the radio prefix to the ARTCC id the client can shade; skip non-US.
                if let Some(id) = center_artcc(&prefix) {
                    centers
                        .entry(id.clone())
                        .or_insert_with(|| AtcCenter {
                            id,
                            positions: Vec::new(),
                        })
                        .positions
                        .push(pos);
                }
            }
            _ => {}
        }
    }

    // ATIS is its own datafeed array; render it as an airport badge with the broadcast letter.
    for a in &data.atis {
        if !in_atc_band(&a.frequency) {
            continue;
        }
        let prefix = a
            .callsign
            .split('_')
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        push_ground(
            &prefix,
            AtcPosition {
                kind: "ATIS".to_string(),
                callsign: a.callsign.clone(),
                frequency: a.frequency.clone(),
                name: String::new(),
                rating: 0,
                logon_time: String::new(),
                atis_code: a.atis_code.clone(),
            },
        );
    }

    board.airports = ground.into_values().collect();
    for ap in &mut board.airports {
        ap.positions
            .sort_by_key(|p| (kind_rank(&p.kind), p.callsign.clone()));
    }
    board.tracons = areas.into_values().collect();
    for a in &mut board.tracons {
        a.positions.sort_by(|x, y| x.callsign.cmp(&y.callsign));
    }
    board.centers = centers.into_values().collect();
    for c in &mut board.centers {
        c.positions.sort_by(|x, y| x.callsign.cmp(&y.callsign));
    }

    board
}
