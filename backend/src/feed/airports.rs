//! Airport coordinate database — the public mwgg/Airports dataset, fetched once and
//! cached in memory. Used to compute distance-to-destination and route length for ETAs.

use std::collections::HashMap;

const AIRPORTS_URL: &str = "https://raw.githubusercontent.com/mwgg/Airports/master/airports.json";

/// ICAO (uppercase) -> (latitude, longitude).
pub type AirportDb = HashMap<String, (f64, f64)>;
/// IATA (uppercase) -> ICAO, so a controller callsign prefix like `SFO` resolves to `KSFO`.
pub type IataMap = HashMap<String, String>;

#[derive(serde::Deserialize)]
struct Entry {
    lat: f64,
    lon: f64,
    #[serde(default)]
    iata: String,
}

pub async fn fetch(client: &reqwest::Client) -> Result<(AirportDb, IataMap), reqwest::Error> {
    let raw: HashMap<String, Entry> = client
        .get(AIRPORTS_URL)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let mut db = AirportDb::with_capacity(raw.len());
    let mut iata = IataMap::new();
    for (icao, e) in raw {
        let icao = icao.to_ascii_uppercase();
        if !e.iata.is_empty() {
            iata.insert(e.iata.to_ascii_uppercase(), icao.clone());
        }
        db.insert(icao, (e.lat, e.lon));
    }
    Ok((db, iata))
}
