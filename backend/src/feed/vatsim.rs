//! VATSIM v3 datafeed — the subset of fields OIS needs for arrival metering.
//! Feed: <https://data.vatsim.net/v3/vatsim-data.json> (refreshes ~15s).

use serde::Deserialize;

const FEED_URL: &str = "https://data.vatsim.net/v3/vatsim-data.json";

#[derive(Debug, Default, Deserialize)]
pub struct VatsimData {
    #[serde(default)]
    pub general: General,
    #[serde(default)]
    pub pilots: Vec<Pilot>,
    #[serde(default)]
    pub prefiles: Vec<Prefile>,
}

#[derive(Debug, Default, Deserialize)]
pub struct General {
    #[serde(default)]
    pub update_timestamp: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Pilot {
    pub callsign: String,
    #[serde(default)]
    pub latitude: f64,
    #[serde(default)]
    pub longitude: f64,
    #[serde(default)]
    pub altitude: i64,
    #[serde(default)]
    pub groundspeed: i64,
    #[serde(default)]
    pub flight_plan: Option<FlightPlan>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Prefile {
    pub callsign: String,
    #[serde(default)]
    pub flight_plan: Option<FlightPlan>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FlightPlan {
    #[serde(default)]
    pub departure: String,
    #[serde(default)]
    pub arrival: String,
    /// Full equipment string, e.g. `H/B77W/L` — first `/`-segment carries the wake.
    #[serde(default)]
    pub aircraft: String,
    #[serde(default)]
    pub aircraft_short: String,
    #[serde(default)]
    pub aircraft_faa: String,
    #[serde(default)]
    pub flight_rules: String,
    #[serde(default)]
    pub route: String,
    #[serde(default)]
    pub cruise_tas: String,
    #[serde(default)]
    pub deptime: String,
}

impl FlightPlan {
    /// Aircraft type (ICAO) and wake category (`L/M/H/J`, empty if unfiled),
    /// mirroring vatflow's `parseAircraftFromFp`.
    pub fn aircraft_type_wake(&self) -> (String, String) {
        let src = if !self.aircraft_short.is_empty() {
            &self.aircraft_short
        } else if !self.aircraft_faa.is_empty() {
            &self.aircraft_faa
        } else {
            "ZZZZ"
        };
        let ty = src.split('/').next().unwrap_or("ZZZZ").to_ascii_uppercase();
        let wake = self
            .aircraft
            .split('/')
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        let wake = if matches!(wake.as_str(), "L" | "M" | "H" | "J") {
            wake
        } else {
            String::new()
        };
        (ty, wake)
    }

    pub fn is_vfr(&self) -> bool {
        self.flight_rules
            .chars()
            .next()
            .is_some_and(|c| c.eq_ignore_ascii_case(&'V'))
    }
}

pub async fn fetch(client: &reqwest::Client) -> Result<VatsimData, reqwest::Error> {
    client
        .get(FEED_URL)
        .send()
        .await?
        .error_for_status()?
        .json::<VatsimData>()
        .await
}
