//! Live VATSIM feed: a background poller keeps an in-memory snapshot of network
//! traffic (plus a cached airport-coordinate database) that the TMU flow endpoints
//! read to meter arrivals against rate programs.

pub mod airports;
pub mod airspace;
pub mod coverage;
pub mod events;
pub mod facilities;
pub mod fca;
pub mod flow;
pub mod gdp;
pub mod metar;
pub mod nav;
pub mod nav_source;
pub mod runway;
pub mod runway_db;
pub mod taxi;
pub mod tracon;
pub mod trajectory;
pub mod vatsim;
pub mod winds;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::sync::RwLock;

use airports::AirportDb;
use vatsim::VatsimData;

const POLL_SECS: u64 = 15;

pub struct Snapshot {
    pub fetched_at: DateTime<Utc>,
    pub source_timestamp: String,
    pub data: VatsimData,
}

#[derive(Clone, Default)]
pub struct FeedStatus {
    pub healthy: bool,
    pub last_ok: Option<DateTime<Utc>>,
    pub source_timestamp: Option<String>,
    pub last_error: Option<String>,
    pub pilots: usize,
    pub prefiles: usize,
    pub airports_loaded: usize,
}

#[derive(Default)]
pub struct FeedInner {
    /// Behind `Arc` so read handlers can clone it and drop the feed lock before doing the
    /// heavy per-request CPU (route resolution / metering), instead of holding the read
    /// guard across it and stalling the poller's writes.
    pub snapshot: Option<Arc<Snapshot>>,
    pub airports: Arc<AirportDb>,
    /// IATA → ICAO, for resolving US-style ATC callsign prefixes (`SFO_TWR` → `KSFO`).
    pub iata: Arc<airports::IataMap>,
    pub status: FeedStatus,
    /// Departures currently being timed (callsign -> session).
    pub taxi_sessions: HashMap<String, taxi::TaxiSession>,
    /// Completed taxi samples per airport (rolling 3h).
    pub taxi_samples: HashMap<String, Vec<taxi::TaxiSample>>,
}

/// Shared, cheaply-cloneable handle to the feed state.
pub type FeedState = Arc<RwLock<FeedInner>>;

pub fn new_state() -> FeedState {
    Arc::new(RwLock::new(FeedInner::default()))
}

/// Spawn the background poller. Safe to call once at startup; it loads the airport
/// database, then refreshes the traffic snapshot every `POLL_SECS`.
pub fn spawn_poller(state: FeedState) {
    tokio::spawn(async move { poller(state).await });
}

async fn poller(state: FeedState) {
    let client = match reqwest::Client::builder()
        .user_agent("ois-backend/0.1 (+https://vatusa.net)")
        .timeout(Duration::from_secs(20))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "feed poller: failed to build HTTP client");
            return;
        }
    };

    match airports::fetch(&client).await {
        Ok((db, iata)) => {
            let n = db.len();
            let mut guard = state.write().await;
            guard.status.airports_loaded = n;
            guard.airports = Arc::new(db);
            guard.iata = Arc::new(iata);
            tracing::info!(airports = n, "feed: airport database loaded");
        }
        Err(e) => {
            tracing::warn!(error = %e, "feed: airport database load failed; ETAs degraded");
        }
    }

    let mut ticker = tokio::time::interval(Duration::from_secs(POLL_SECS));
    loop {
        ticker.tick().await;
        match vatsim::fetch(&client).await {
            Ok(data) => {
                let now = Utc::now();
                let pilots = data.pilots.len();
                let prefiles = data.prefiles.len();
                let source_timestamp = data.general.update_timestamp.clone();
                let mut guard = state.write().await;
                guard.status.healthy = true;
                guard.status.last_ok = Some(now);
                guard.status.source_timestamp = Some(source_timestamp.clone());
                guard.status.last_error = None;
                guard.status.pilots = pilots;
                guard.status.prefiles = prefiles;
                // Advance the taxi state machine before the data is moved into the snapshot.
                let FeedInner {
                    taxi_sessions,
                    taxi_samples,
                    airports,
                    ..
                } = &mut *guard;
                taxi::process(taxi_sessions, taxi_samples, airports, &data, now);
                guard.snapshot = Some(Arc::new(Snapshot {
                    fetched_at: now,
                    source_timestamp,
                    data,
                }));
            }
            Err(e) => {
                let mut guard = state.write().await;
                guard.status.healthy = false;
                guard.status.last_error = Some(e.to_string());
                tracing::warn!(error = %e, "feed: vatsim fetch failed");
            }
        }
    }
}
