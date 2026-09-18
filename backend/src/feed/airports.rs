//! Airport coordinate database — the public mwgg/Airports dataset, fetched once and
//! cached in memory. Used to compute distance-to-destination and route length for ETAs.

use std::collections::HashMap;

const AIRPORTS_URL: &str = "https://raw.githubusercontent.com/mwgg/Airports/master/airports.json";

/// An airport's position and field elevation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Airport {
    pub lat: f64,
    pub lon: f64,
    /// Field elevation, feet MSL.
    pub elevation_ft: f64,
}

impl Airport {
    /// A sea-level airport at `lat`/`lon` — for test fixtures that only care about position.
    #[cfg(test)]
    pub fn at(lat: f64, lon: f64) -> Self {
        Self {
            lat,
            lon,
            elevation_ft: 0.0,
        }
    }
}

/// ICAO (uppercase) -> airport.
pub type AirportDb = HashMap<String, Airport>;
/// IATA (uppercase) -> ICAO, so a controller callsign prefix like `SFO` resolves to `KSFO`.
pub type IataMap = HashMap<String, String>;

#[derive(serde::Deserialize)]
struct Entry {
    lat: f64,
    lon: f64,
    #[serde(default)]
    iata: String,
    #[serde(default)]
    elevation: f64,
}

/// The field elevation (ft) of `icao`, or sea level when the airport is unknown — the descent
/// model's fallback, so an unlisted destination is timed exactly as before elevation was known.
pub fn field_elevation_ft(db: &AirportDb, icao: &str) -> f64 {
    db.get(&icao.to_ascii_uppercase())
        .map_or(0.0, |a| a.elevation_ft)
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
        db.insert(
            icao,
            Airport {
                lat: e.lat,
                lon: e.lon,
                elevation_ft: e.elevation,
            },
        );
    }
    Ok((db, iata))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_keeps_elevation_and_defaults_it_when_absent() {
        let with: Entry = serde_json::from_str(
            r#"{"lat": 39.86, "lon": -104.67, "iata": "DEN", "elevation": 5431}"#,
        )
        .unwrap();
        assert_eq!(with.elevation, 5431.0);
        let without: Entry = serde_json::from_str(r#"{"lat": 1.0, "lon": 2.0}"#).unwrap();
        assert_eq!(without.elevation, 0.0);
    }

    #[test]
    fn field_elevation_falls_back_to_sea_level_for_unknown_airports() {
        let db = AirportDb::from([(
            "KDEN".to_string(),
            Airport {
                lat: 39.86,
                lon: -104.67,
                elevation_ft: 5431.0,
            },
        )]);
        assert_eq!(field_elevation_ft(&db, "KDEN"), 5431.0);
        assert_eq!(field_elevation_ft(&db, "kden"), 5431.0);
        assert_eq!(field_elevation_ft(&db, "ZZZZ"), 0.0);
    }
}
