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
    #[serde(default)]
    pub controllers: Vec<Controller>,
    #[serde(default)]
    pub atis: Vec<Atis>,
}

/// An online ATC position. `facility` is the datafeed enum:
/// 0 OBS · 1 FSS · 2 DEL · 3 GND · 4 TWR · 5 APP · 6 CTR.
#[derive(Debug, Clone, Deserialize)]
pub struct Controller {
    pub callsign: String,
    #[serde(default)]
    pub frequency: String,
    #[serde(default)]
    pub facility: i32,
    #[serde(default)]
    pub rating: i32,
}

/// An online ATIS (its own datafeed array). `atis_code` is the broadcast letter.
#[derive(Debug, Clone, Deserialize)]
pub struct Atis {
    pub callsign: String,
    #[serde(default)]
    pub frequency: String,
    #[serde(default)]
    pub atis_code: Option<String>,
    #[serde(default)]
    pub text_atis: Option<Vec<String>>,
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
    pub heading: i64,
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
    /// Filed cruise altitude, e.g. `"35000"` or `"FL350"`.
    #[serde(default)]
    pub altitude: String,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(aircraft: &str, short: &str, faa: &str) -> FlightPlan {
        FlightPlan {
            aircraft: aircraft.into(),
            aircraft_short: short.into(),
            aircraft_faa: faa.into(),
            ..Default::default()
        }
    }

    #[test]
    fn type_and_wake_from_icao_equipment() {
        let (ty, wake) = plan("H/B77W/L", "B77W", "").aircraft_type_wake();
        assert_eq!(ty, "B77W");
        assert_eq!(wake, "H");
    }

    #[test]
    fn wake_empty_when_leading_segment_is_not_a_category() {
        let (ty, wake) = plan("B738/L", "B738", "").aircraft_type_wake();
        assert_eq!(ty, "B738");
        assert_eq!(wake, "");
    }

    #[test]
    fn type_falls_back_to_faa_then_zzzz() {
        assert_eq!(plan("", "", "A320").aircraft_type_wake().0, "A320");
        assert_eq!(plan("", "", "").aircraft_type_wake().0, "ZZZZ");
    }

    #[test]
    fn vfr_detection() {
        let vfr = FlightPlan {
            flight_rules: "V".into(),
            ..Default::default()
        };
        assert!(vfr.is_vfr());
        let ifr = FlightPlan {
            flight_rules: "I".into(),
            ..Default::default()
        };
        assert!(!ifr.is_vfr());
        assert!(!FlightPlan::default().is_vfr());
    }

    #[test]
    fn deserializes_datafeed_subset() {
        let data: VatsimData = serde_json::from_value(serde_json::json!({
            "general": { "update_timestamp": "2026-01-01T00:00:00Z" },
            "pilots": [{
                "callsign": "AAL1",
                "latitude": 40.0, "longitude": -73.0,
                "altitude": 10000, "groundspeed": 300,
                "flight_plan": {
                    "departure": "KBOS", "arrival": "KJFK",
                    "route": "DCT CAMRN", "aircraft_short": "B738"
                }
            }],
            "prefiles": []
        }))
        .unwrap();
        assert_eq!(data.pilots.len(), 1);
        assert_eq!(data.pilots[0].callsign, "AAL1");
        let fp = data.pilots[0].flight_plan.as_ref().unwrap();
        assert_eq!(fp.arrival, "KJFK");
        assert_eq!(fp.route, "DCT CAMRN");
    }
}
