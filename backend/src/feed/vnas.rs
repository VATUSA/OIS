//! vNAS sector identities and live sector staffing (#595), for the Airspace Monitor (#593).
//!
//! Two vNAS sources, both read-only and held in memory behind `ArcSwap`s:
//!
//! - **Sector identities** — every ARTCC's ERAM sectors (`facility.eramConfiguration.sectors`,
//!   e.g. ZDC `01` "Elkins 01") from <https://data-api.vnas.vatsim.net/api/artccs>, one response
//!   for all ARTCCs. Facility definitions change rarely, so it is refreshed daily.
//! - **Staffing** — which controller is working which sector right now, from each active position's
//!   `eramData.sectorId` in <https://live.env.vnas.vatsim.net/data-feed/controllers.json>. That feed
//!   regenerates every 15 s; it is polled every 30 s by the single task `spawn_refresh` starts —
//!   nothing fetches it per request.
//!
//! Staffing is an attribute of a sector, never a filter: the Monitor counts every sector and flags
//! the staffed ones. Observers don't staff anything.
//!
//! VATSIM/vNAS fields vary in type across and within endpoints (`sectorId` is a number in the
//! facility data and a string in the feed), so both parsers walk `serde_json::Value` and drop just
//! the entry that doesn't fit, rather than failing the whole payload on one odd field.

use std::{collections::HashMap, sync::Arc, time::Duration};

use arc_swap::ArcSwap;
use serde_json::Value;

use super::neighbors::from_dataset;

const ARTCCS_URL: &str = "https://data-api.vnas.vatsim.net/api/artccs";
const CONTROLLERS_URL: &str = "https://live.env.vnas.vatsim.net/data-feed/controllers.json";
const SECTORS_INTERVAL: Duration = Duration::from_secs(24 * 3600);
/// Never faster than the feed regenerates (15 s).
const STAFFING_INTERVAL: Duration = Duration::from_secs(30);

/// One ERAM sector as vNAS names it.
#[derive(Debug, Clone, PartialEq)]
pub struct VnasSector {
    /// Normalised, see [`sector_id`]: `"01"`, `"32"`.
    pub sector_id: String,
    pub name: String,
}

/// Every ARTCC's sectors, keyed by OIS ARTCC id (Honolulu is `HCF`).
#[derive(Debug, Default)]
pub struct SectorIds {
    pub by_artcc: HashMap<String, Vec<VnasSector>>,
}

/// A controller working a sector.
#[derive(Debug, Clone, PartialEq)]
pub struct Staffer {
    pub cid: String,
    pub callsign: String,
}

/// Who is working which sector now, keyed by `(ARTCC, sector_id)`. A controller working several
/// sectors (a consolidation) appears under each.
#[derive(Debug, Default)]
pub struct Staffing {
    by_sector: HashMap<(String, String), Vec<Staffer>>,
}

impl Staffing {
    /// Who is working `artcc`'s `sector_id` (normalised); empty when unstaffed.
    pub fn staffed_by(&self, artcc: &str, sector_id: &str) -> &[Staffer] {
        self.by_sector
            .get(&(artcc.to_string(), sector_id.to_string()))
            .map_or(&[], Vec::as_slice)
    }

    /// Every staffed sector with its controllers.
    pub fn staffed(&self) -> impl Iterator<Item = (&(String, String), &[Staffer])> {
        self.by_sector.iter().map(|(k, v)| (k, v.as_slice()))
    }
}

#[derive(Clone)]
pub struct VnasState {
    pub sectors: Arc<ArcSwap<SectorIds>>,
    pub staffing: Arc<ArcSwap<Staffing>>,
}

pub fn new_state() -> VnasState {
    VnasState {
        sectors: Arc::new(ArcSwap::from_pointee(SectorIds::default())),
        staffing: Arc::new(ArcSwap::from_pointee(Staffing::default())),
    }
}

/// The one vNAS poller: sector identities at startup then daily, staffing every 30 s. A failed
/// fetch keeps the current data (empty until the first success).
pub fn spawn_refresh(state: VnasState) {
    tokio::spawn(async move {
        let client = match reqwest::Client::builder()
            .user_agent("ois-backend/0.1 (+https://vatusa.net)")
            .timeout(Duration::from_secs(60))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(error = %e, "vnas: failed to build HTTP client");
                return;
            }
        };
        let mut sectors_tick = tokio::time::interval(SECTORS_INTERVAL);
        let mut staffing_tick = tokio::time::interval(STAFFING_INTERVAL);
        staffing_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = sectors_tick.tick() => match fetch(&client, ARTCCS_URL).await {
                    Ok(v) => {
                        let ids = parse_sectors(&v);
                        tracing::info!(artccs = ids.by_artcc.len(), "vNAS sector identities loaded");
                        state.sectors.store(Arc::new(ids));
                    }
                    Err(e) => tracing::warn!(error = %e, "vnas: sector refresh failed; keeping current"),
                },
                _ = staffing_tick.tick() => match fetch(&client, CONTROLLERS_URL).await {
                    Ok(v) => state.staffing.store(Arc::new(parse_controllers(&v))),
                    Err(e) => tracing::debug!(error = %e, "vnas: staffing refresh failed; keeping current"),
                },
            }
        }
    });
}

async fn fetch(client: &reqwest::Client, url: &str) -> Result<Value, reqwest::Error> {
    client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
}

/// A vNAS ARTCC id as OIS knows it (`ZHN` → `HCF`).
fn artcc_id(v: &Value) -> Option<String> {
    let id = v.as_str()?.trim().to_ascii_uppercase();
    (!id.is_empty()).then(|| from_dataset(&id).to_string())
}

/// A sector id in one canonical form, whichever type the source used: numbers and digit strings are
/// zero-padded to two (`1` → `"01"`, `"2"` → `"02"`, `"32"` stays), matching vNAS positions and the
/// sector volume dataset; anything else is trimmed and uppercased.
pub fn sector_id(v: &Value) -> Option<String> {
    let raw = match v {
        Value::Number(n) => n.as_u64()?.to_string(),
        Value::String(s) => s.trim().to_ascii_uppercase(),
        _ => return None,
    };
    if raw.is_empty() {
        return None;
    }
    if raw.bytes().all(|b| b.is_ascii_digit()) {
        return Some(format!("{:02}", raw.parse::<u64>().ok()?));
    }
    Some(raw)
}

/// `/api/artccs` (an array of ARTCC configurations) → sectors per ARTCC. An ARTCC with no ERAM
/// configuration, or a sector without an id, is skipped.
pub fn parse_sectors(v: &Value) -> SectorIds {
    let mut by_artcc: HashMap<String, Vec<VnasSector>> = HashMap::new();
    for artcc in v.as_array().into_iter().flatten() {
        let facility = &artcc["facility"];
        let Some(id) = artcc_id(&facility["id"]).or_else(|| artcc_id(&artcc["id"])) else {
            continue;
        };
        let sectors = facility["eramConfiguration"]["sectors"].as_array();
        let list: Vec<VnasSector> = sectors
            .into_iter()
            .flatten()
            .filter_map(|s| {
                Some(VnasSector {
                    sector_id: sector_id(&s["sectorId"])?,
                    name: s["name"].as_str().unwrap_or_default().trim().to_string(),
                })
            })
            .collect();
        if !list.is_empty() {
            by_artcc.insert(id, list);
        }
    }
    SectorIds { by_artcc }
}

/// `controllers.json` → who is working which sector. Counts an ERAM position only when the
/// controller is active and not an observer, and the position itself is active.
pub fn parse_controllers(v: &Value) -> Staffing {
    let mut by_sector: HashMap<(String, String), Vec<Staffer>> = HashMap::new();
    for c in v["controllers"].as_array().into_iter().flatten() {
        if c["isObserver"].as_bool() != Some(false) || c["isActive"].as_bool() != Some(true) {
            continue;
        }
        let staffer = Staffer {
            cid: match &c["vatsimData"]["cid"] {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                _ => continue,
            },
            callsign: c["vatsimData"]["callsign"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        };
        for p in c["positions"].as_array().into_iter().flatten() {
            if p["isActive"].as_bool() != Some(true) {
                continue;
            }
            let (Some(artcc), Some(sector)) = (
                artcc_id(&p["facilityId"]).or_else(|| artcc_id(&c["artccId"])),
                sector_id(&p["eramData"]["sectorId"]),
            ) else {
                continue;
            };
            let list = by_sector.entry((artcc, sector)).or_default();
            if !list.contains(&staffer) {
                list.push(staffer.clone());
            }
        }
    }
    Staffing { by_sector }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// Trimmed from a real `/api/artccs` response (ZDC, 2026-10-03), with the hazards the live
    /// network can serve injected: a null sector list, an ARTCC with no ERAM configuration at all, a
    /// null `sectorId`, a string `sectorId`, a null name, and Honolulu under vNAS's `ZHN`.
    const ARTCCS: &str = r#"[
      {"id": "ZDC", "facility": {"id": "ZDC", "type": "Artcc", "name": "Washington ARTCC",
        "eramConfiguration": {"nasId": "ZDC", "sectors": [
          {"id": "01KS8JEGVG5A4T88NY7XWNZ5R6", "sectorId": 1, "name": "Elkins 01", "isFromEramData": true},
          {"id": "01KS8JEGVG3GKMH3SJYF2A8G62", "sectorId": 2, "name": "Casanova 02", "isFromEramData": true},
          {"id": "x", "sectorId": null, "name": "No Id", "isFromEramData": true},
          {"id": "y", "sectorId": "32", "name": null, "isFromEramData": false}
        ]}}},
      {"id": "ZNY", "facility": {"id": "ZNY", "eramConfiguration": {"sectors": null}}},
      {"id": "ZAN", "facility": {"id": "ZAN", "type": "Artcc"}},
      {"id": "ZHN", "facility": {"id": "ZHN", "eramConfiguration": {"sectors": [
          {"sectorId": 2, "name": "Kauai 02"}]}}},
      null,
      "garbage"
    ]"#;

    #[test]
    fn a_payload_with_nulls_and_missing_eram_configuration_parses() {
        let ids = parse_sectors(&serde_json::from_str(ARTCCS).unwrap());
        let mut keys: Vec<_> = ids.by_artcc.keys().cloned().collect();
        keys.sort();
        // ZNY (null sectors) and ZAN (no eramConfiguration) have nothing; ZHN keys as HCF.
        assert_eq!(keys, ["HCF", "ZDC"]);
        let sector = |id: &str, name: &str| VnasSector {
            sector_id: id.into(),
            name: name.into(),
        };
        assert_eq!(
            ids.by_artcc["ZDC"],
            [
                sector("01", "Elkins 01"),
                sector("02", "Casanova 02"),
                sector("32", "")
            ]
        );
        assert_eq!(ids.by_artcc["HCF"], [sector("02", "Kauai 02")]);
    }

    fn controller(cid: &str, observer: bool, active: bool, positions: Value) -> Value {
        json!({
            "artccId": "ZDC", "isActive": active, "isObserver": observer, "role": "Controller",
            "vatsimData": {"cid": cid, "callsign": format!("CS_{cid}")},
            "positions": positions,
        })
    }

    fn eram(facility: &str, sector: Value, active: bool) -> Value {
        json!({"facilityId": facility, "isActive": active, "eramData": {"sectorId": sector},
               "starsData": null})
    }

    #[test]
    fn staffing_counts_active_eram_positions_of_non_observers() {
        let feed = json!({"updatedAt": "2026-10-03T22:00:00Z", "controllers": [
            // Working two sectors at once: staffed on both.
            controller("1", false, true, json!([eram("ZDC", json!("32"), true),
                                                eram("ZDC", json!(4), true)])),
            controller("2", true, true, json!([eram("ZDC", json!("10"), true)])), // observer
            controller("3", false, false, json!([eram("ZDC", json!("11"), true)])), // inactive
            controller("4", false, true, json!([eram("ZDC", json!("12"), false)])), // position off
            // A TRACON position: no eramData, so no sector.
            controller("5", false, true, json!([{"facilityId": "PCT", "isActive": true,
                                                 "eramData": null, "starsData": {"sectorId": "E"}}])),
            controller("6", false, true, json!([eram("ZHN", json!("02"), true)])),
            // No facilityId on the position: falls back to the controller's artccId.
            controller("7", false, true, json!([{"isActive": true, "eramData": {"sectorId": "33"}}])),
            {"isObserver": false, "isActive": true, "vatsimData": null, "positions": []},
            null,
        ]});
        let s = parse_controllers(&feed);

        let one = [Staffer {
            cid: "1".into(),
            callsign: "CS_1".into(),
        }];
        assert_eq!(s.staffed_by("ZDC", "32"), one);
        assert_eq!(s.staffed_by("ZDC", "04"), one);
        assert_eq!(s.staffed_by("HCF", "02")[0].cid, "6");
        assert_eq!(s.staffed_by("ZDC", "33")[0].cid, "7");
        for unstaffed in ["10", "11", "12"] {
            assert!(s.staffed_by("ZDC", unstaffed).is_empty(), "{unstaffed}");
        }
        assert_eq!(s.staffed().count(), 4);
    }

    #[test]
    fn an_observer_never_staffs_a_sector() {
        let feed = json!({"controllers": [
            controller("9", true, true, json!([eram("ZDC", json!("32"), true)]))]});
        assert_eq!(parse_controllers(&feed).staffed().count(), 0);
    }

    #[test]
    fn sector_ids_normalise_across_types() {
        assert_eq!(sector_id(&json!(1)).as_deref(), Some("01"));
        assert_eq!(sector_id(&json!("2")).as_deref(), Some("02"));
        assert_eq!(sector_id(&json!("02")).as_deref(), Some("02"));
        assert_eq!(sector_id(&json!(" 32 ")).as_deref(), Some("32"));
        assert_eq!(sector_id(&json!(123)).as_deref(), Some("123"));
        assert_eq!(sector_id(&json!("e1")).as_deref(), Some("E1"));
        for bad in [
            json!(null),
            json!(""),
            json!(-1),
            json!(1.5),
            json!(true),
            json!({}),
        ] {
            assert_eq!(sector_id(&bad), None, "{bad}");
        }
    }

    #[test]
    fn garbage_parses_to_nothing() {
        for v in [
            json!(null),
            json!([]),
            json!("x"),
            json!({}),
            json!({"controllers": "x"}),
        ] {
            assert!(parse_sectors(&v).by_artcc.is_empty());
            assert_eq!(parse_controllers(&v).staffed().count(), 0);
        }
    }

    /// Against the live network: `cargo test -p ois-backend --lib vnas -- --ignored`.
    #[tokio::test]
    #[ignore = "hits the live vNAS APIs"]
    async fn the_live_payloads_parse() {
        let client = reqwest::Client::new();
        let ids = parse_sectors(&fetch(&client, ARTCCS_URL).await.unwrap());
        assert!(ids.by_artcc.len() >= 20, "{} ARTCCs", ids.by_artcc.len());
        let raw = fetch(&client, CONTROLLERS_URL).await.unwrap();
        let eram_positions = raw["controllers"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["isObserver"] == json!(false) && c["isActive"] == json!(true))
            .flat_map(|c| c["positions"].as_array().unwrap())
            .filter(|p| p["isActive"] == json!(true) && !p["eramData"]["sectorId"].is_null())
            .count();
        let staffed: usize = parse_controllers(&raw)
            .staffed()
            .map(|(_, s)| s.len())
            .sum();
        assert_eq!(staffed, eram_positions);
    }
}
