//! Online ATC for the map "ATC" layer — no auth, same public exposure as live traffic.
//! Classifies the datafeed's `controllers`/`atis` into airport ground stations (badges),
//! TRACON/approach areas (matched to SimAware polygons), and center positions.

use std::collections::HashMap;

use axum::{Json, extract::State};
use chrono::Utc;

use crate::{
    feed::airports::{Airport, AirportDb, IataMap},
    feed::tracon::TraconData,
    feed::vatsim::VatsimData,
    models::{AtcAirport, AtcArea, AtcBoard, AtcCenter, AtcPosition, FlowFacility},
    state::AppState,
};

/// The facility directory (ARTCCs + TRACONs and their member airports) for scoping dashboard widgets
/// to a whole facility. Public read; served from the daily-refreshed `feed/facilities.rs` map.
#[utoipa::path(
    get,
    path = "/api/v1/flow/facilities",
    tag = "flow",
    responses((status = 200, body = Vec<FlowFacility>))
)]
pub async fn list_flow_facilities(State(state): State<AppState>) -> Json<Vec<FlowFacility>> {
    let map = state.facilities.read().await;
    let mut out: Vec<FlowFacility> = map
        .iter()
        .map(|(id, f)| FlowFacility {
            id: id.clone(),
            kind: f.kind.clone(),
            name: None,
            airports: f.airports.clone(),
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Json(out)
}

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
/// Non-US centers return `None`.
///
/// This answers **"which US ARTCC is this?"** and nothing more. It deliberately does *not* check
/// whether we hold a boundary polygon for the answer, because its two callers ask different
/// questions: the ATC board wants something it can shade, while `feed::stats::is_us_controller`
/// wants to know whether a controller counts as American. `ZAK` (Oakland Oceanic) and `ZSU`
/// (San Juan) are real US ARTCCs with no polygon in the bundled set — filtering here would quietly
/// drop them from stats collection. The map filters on [`Boundaries::has`] at its own call site
/// instead (VATUSA/OIS#482).
pub(crate) fn center_artcc(prefix: &str) -> Option<String> {
    // Honolulu is `HCF` in OIS — `org.facilities`, VATUSA, grants and the boundary assets all say so
    // (#556). `ZHN` is the old FAA-style spelling some data still uses; it is checked **before** the
    // bare-`Zxx` branch below, which would otherwise return it verbatim as an id no facility has.
    if prefix == "ZHN" {
        return Some("HCF".to_string());
    }
    if prefix.len() == 3
        && prefix.starts_with('Z')
        && prefix.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        return Some(prefix.to_string());
    }
    let mapped = match prefix {
        "ABQ" => "ZAB",
        "ATL" => "ZTL",
        // `BDA` (Bermuda, TXKF) deliberately absent: it is not a ZNY position, and mapping it
        // here shaded the whole of New York for a Bermuda controller (VATUSA/OIS#482).
        "NY" => "ZNY",
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
        // `ZMO` was listed here too, but the bare-`Zxx` branch above returns it verbatim before
        // this table is reached, so the arm never fired. Removed rather than left as a promise the
        // code does not keep.
        "MIA" => "ZMA",
        "MSP" => "ZMP",
        "OAK" => "ZOA",
        "OO" | "OOR" | "OORO" => "ZAK",
        "SEA" => "ZSE",
        "SLC" => "ZLC",
        "ANC" => "ZAN",
        // VATSpy lists **both** prefixes for Honolulu (`PHZH|Honolulu|HNL` and `…|HCF`), and the
        // controller actually on the network logs on as `HNL_*` — which was missing, so they were
        // never drawn and never counted as a US controller (#556).
        "HNL" | "HCF" => "HCF",
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
    Json(board_from(
        &snap.data,
        &airports,
        &iata,
        &tracons,
        &state.airspace,
    ))
}

/// Classify a network snapshot's `controllers`/`atis` into airport ground stations (badges),
/// TRACON/approach areas (matched to SimAware polygons), and center positions. Pure of the live
/// feed so the historical endpoint can replay it against a reconstructed snapshot.
pub fn board_from(
    data: &VatsimData,
    airports: &AirportDb,
    iata: &IataMap,
    tracons: &TraconData,
    boundaries: &crate::feed::airspace::Boundaries,
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
        if let Some(&Airport { lat, lon, .. }) = airports.get(&code) {
            return Some((code, lat, lon));
        }
        if let Some(icao) = iata.get(&code)
            && let Some(&Airport { lat, lon, .. }) = airports.get(icao)
        {
            return Some((icao.clone(), lat, lon));
        }
        let k = format!("K{code}");
        airports
            .get(&k)
            .map(|&Airport { lat, lon, .. }| (k, lat, lon))
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
                // Map the radio prefix to the ARTCC id the client can shade; skip non-US, and skip
                // anything we hold no polygon for. The board exists to be drawn: emitting a centre
                // the client cannot outline gave it a choice between ignoring the row and shading
                // nothing, and an unknown or typo'd `Zxx` id used to get through here unchecked
                // (VATUSA/OIS#482).
                if let Some(id) = center_artcc(&prefix).filter(|id| boundaries.has(id)) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::airspace::Boundaries;

    // --- which ARTCC a centre prefix belongs to (VATUSA/OIS#482) ---

    /// Bermuda is its own place. Mapping `BDA` to ZNY shaded the whole of New York whenever a
    /// Bermuda controller was online — the defect this issue was filed for.
    #[test]
    fn bermuda_is_not_new_york() {
        assert_eq!(center_artcc("BDA"), None);
        assert_eq!(center_artcc("NY"), Some("ZNY".to_string()));
    }

    /// A bare `Zxx` id is taken verbatim, which is why the table below it can never claim one.
    #[test]
    fn a_bare_artcc_id_is_taken_as_itself() {
        assert_eq!(center_artcc("ZLA"), Some("ZLA".to_string()));
        // `ZMO` used to appear in the table as an alias for ZMA; it never fired, because this
        // branch answers first. Pinning it stops the arm being reintroduced.
        assert_eq!(center_artcc("ZMO"), Some("ZMO".to_string()));
    }

    #[test]
    fn a_radio_prefix_maps_to_its_artcc() {
        assert_eq!(center_artcc("BOS"), Some("ZBW".to_string()));
        assert_eq!(center_artcc("DC"), Some("ZDC".to_string()));
    }

    #[test]
    fn a_prefix_we_do_not_know_is_not_a_us_centre() {
        assert_eq!(center_artcc("EGLL"), None);
        assert_eq!(center_artcc(""), None);
    }

    /// `center_artcc` answers "which US ARTCC", not "can we draw it" — its other caller
    /// (`feed::stats::is_us_controller`) depends on that distinction, so these real US centres must
    /// keep resolving even though no polygon exists for them.
    #[test]
    fn a_real_us_centre_resolves_even_with_no_polygon() {
        let boundaries = Boundaries::load();
        for prefix in ["OO", "SJU"] {
            let id = center_artcc(prefix).unwrap_or_else(|| panic!("{prefix} should resolve"));
            assert!(
                !boundaries.has(&id),
                "{id} unexpectedly has geometry — this test is asserting the wrong thing now"
            );
        }
    }

    // --- what the board is willing to hand the map ---

    fn ctr(callsign: &str) -> crate::feed::vatsim::Controller {
        crate::feed::vatsim::Controller {
            callsign: callsign.to_string(),
            frequency: "133.000".to_string(),
            facility: 6, // CTR
            rating: 5,
            cid: 1,
            name: "A Controller".to_string(),
            server: None,
            visual_range: None,
            logon_time: String::new(),
            last_updated: String::new(),
        }
    }

    fn board_with(callsigns: &[&str]) -> AtcBoard {
        let data = VatsimData {
            controllers: callsigns.iter().map(|c| ctr(c)).collect(),
            ..Default::default()
        };
        board_from(
            &data,
            &AirportDb::new(),
            &IataMap::new(),
            &TraconData::default(),
            &Boundaries::load(),
        )
    }

    /// The board exists to be drawn. A centre with no polygon left the client shading nothing, or
    /// shading the wrong thing.
    #[test]
    fn a_centre_with_no_polygon_never_reaches_the_board() {
        let ids: Vec<String> = board_with(&["ZAK_CTR", "ZSU_CTR", "ZBW_CTR"])
            .centers
            .into_iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(ids, vec!["ZBW".to_string()]);
    }

    /// The no-allowlist hole: any three characters starting with `Z` used to select a polygon.
    #[test]
    fn a_typod_centre_id_draws_nothing() {
        assert!(board_with(&["ZQQ_CTR"]).centers.is_empty());
    }

    #[test]
    fn a_bermuda_controller_no_longer_shades_new_york() {
        assert!(board_with(&["BDA_CTR"]).centers.is_empty());
    }

    /// VATUSA/OIS#556. VATSpy lists two prefixes for Honolulu (`PHZH|Honolulu|HNL` and `…|HCF`), and
    /// the controller actually on the network logs on as `HNL_*` — `HNL_02_CTR` was online while this
    /// was written, and resolved to nothing. Every spelling now lands on OIS's one id, `HCF`.
    #[test]
    fn every_honolulu_prefix_resolves_to_hcf() {
        for prefix in ["HNL", "HCF", "ZHN"] {
            assert_eq!(center_artcc(prefix).as_deref(), Some("HCF"), "{prefix}");
        }
    }

    /// `ZHN` must be caught before the bare-`Zxx` branch, which otherwise returns it verbatim. The
    /// branch itself still works for every other centre.
    #[test]
    fn the_zhn_alias_is_not_swallowed_by_the_bare_zxx_branch() {
        assert_eq!(center_artcc("ZHN").as_deref(), Some("HCF"));
        assert_eq!(center_artcc("ZLA").as_deref(), Some("ZLA"));
        assert_eq!(center_artcc("ZUA").as_deref(), Some("ZUA"));
    }

    /// The board shades a centre only if the boundary set has that id, so `HCF` has to be in it — and
    /// Honolulu itself has to fall inside the polygon.
    #[test]
    fn an_hnl_centre_has_a_boundary_to_draw() {
        let boundaries = crate::feed::airspace::Boundaries::load();
        let id = center_artcc("HNL").expect("HNL resolves");

        assert!(
            boundaries.has(&id),
            "the board filters on `has`, so this is what makes it drawn"
        );
        assert!(
            boundaries.contains(&id, 21.32, -157.92),
            "PHNL sits inside the HCF polygon"
        );
        assert!(
            !boundaries.has("ZHN"),
            "and no stale ZHN feature is left behind"
        );
    }
}
