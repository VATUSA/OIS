//! Airport coordinate database — the public mwgg/Airports dataset, fetched once and
//! cached in memory. Used to compute distance-to-destination and route length for ETAs.

use std::collections::HashMap;

const AIRPORTS_URL: &str = "https://raw.githubusercontent.com/mwgg/Airports/master/airports.json";

/// ICAO (uppercase) -> (latitude, longitude).
pub type AirportDb = HashMap<String, (f64, f64)>;

#[derive(serde::Deserialize)]
struct Entry {
    lat: f64,
    lon: f64,
}

pub async fn fetch(client: &reqwest::Client) -> Result<AirportDb, reqwest::Error> {
    let raw: HashMap<String, Entry> = client
        .get(AIRPORTS_URL)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(raw
        .into_iter()
        .map(|(icao, e)| (icao.to_ascii_uppercase(), (e.lat, e.lon)))
        .collect())
}
