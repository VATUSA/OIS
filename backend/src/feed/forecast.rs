//! Airport surface-wind forecast via Open-Meteo (free, no API key). Used to predict an event-day
//! runway configuration + arrival rate. We fetch the full hourly series once per airport (cached ~2h)
//! and answer any requested time from it. Airport coordinates come from the feed's airport DB.

use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::Duration,
};

use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;

use super::airports::AirportDb;

/// A single forecast hour's surface wind.
#[derive(Debug, Clone)]
pub struct HourWind {
    pub time: DateTime<Utc>,
    /// Direction (degrees true), or None when calm/variable.
    pub dir: Option<i32>,
    pub spd_kt: i32,
    pub gust_kt: Option<i32>,
}

type Series = Vec<HourWind>;

const TTL_MS: i64 = 2 * 60 * 60 * 1000;
/// Below this we treat the wind as calm/variable and report no direction.
const CALM_KT: i32 = 3;

static CACHE: LazyLock<Mutex<HashMap<String, (Series, i64)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .user_agent("ois-forecast/1.0 (+https://vatusa.net)")
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap_or_default()
});

#[derive(Deserialize)]
struct OpenMeteoResp {
    hourly: Hourly,
}

#[derive(Deserialize)]
struct Hourly {
    time: Vec<i64>,
    wind_speed_10m: Vec<f64>,
    wind_direction_10m: Vec<f64>,
    wind_gusts_10m: Vec<f64>,
}

async fn fetch(lat: f64, lon: f64) -> Option<Series> {
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat:.4}&longitude={lon:.4}\
         &hourly=wind_speed_10m,wind_direction_10m,wind_gusts_10m\
         &forecast_days=16&wind_speed_unit=kn&timeformat=unixtime"
    );
    let resp = CLIENT.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let data: OpenMeteoResp = resp.json().await.ok()?;
    let h = data.hourly;
    let n = h.time.len();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let spd = *h.wind_speed_10m.get(i)? as i32;
        let dir = if spd < CALM_KT {
            None
        } else {
            Some((h.wind_direction_10m.get(i).copied()?.round() as i32).rem_euclid(360))
        };
        let gust = h.wind_gusts_10m.get(i).map(|g| g.round() as i32);
        out.push(HourWind {
            time: Utc.timestamp_opt(*h.time.get(i)?, 0).single()?,
            dir,
            spd_kt: spd,
            gust_kt: gust,
        });
    }
    Some(out)
}

/// The forecast wind for `icao` nearest to `at`. Returns None when the airport isn't in the DB, the
/// fetch fails, or `at` is outside the forecast window (caller can fall back to live METAR).
pub async fn wind_at(airports: &AirportDb, icao: &str, at: DateTime<Utc>) -> Option<HourWind> {
    let &(lat, lon) = airports.get(icao)?;
    let now = Utc::now().timestamp_millis();

    // Serve from cache when fresh.
    let cached: Option<Series> = {
        let guard = CACHE.lock().ok()?;
        match guard.get(icao) {
            Some((s, ts)) if now - ts < TTL_MS => Some(s.clone()),
            _ => None,
        }
    };
    let series = match cached {
        Some(s) => s,
        None => {
            let fresh = fetch(lat, lon).await?;
            if let Ok(mut guard) = CACHE.lock() {
                guard.insert(icao.to_string(), (fresh.clone(), now));
            }
            fresh
        }
    };

    // Nearest hour to `at`, but only if within ~90 min (else `at` is outside the forecast window).
    let target = at.timestamp();
    series
        .into_iter()
        .min_by_key(|h| (h.time.timestamp() - target).abs())
        .filter(|h| (h.time.timestamp() - target).abs() <= 90 * 60)
}
