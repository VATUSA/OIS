//! Offline aircraft-profile builder and backtest (#828). Measures how each aircraft type climbs,
//! cruises and descends on VATSIM from the stats history OIS already stores, and scores a profile set
//! against held-out flights through the live prediction path. See `ois_backend::profile_builder`.
//!
//! ```text
//! # Measure: write the profile file and the coverage report.
//! RUST_LOG=info cargo run -p ois-backend --release --bin aircraft-profile-builder -- build \
//!     --from 2026-09-20 --to 2026-10-11 \
//!     --out backend/data/aircraft_profiles.json --report coverage.md
//!
//! # Score: the stored catalog (set A) against it with the file's types applied (set B).
//! RUST_LOG=info cargo run -p ois-backend --release --bin aircraft-profile-builder -- backtest \
//!     --from 2026-09-20 --to 2026-10-11 \
//!     --profiles backend/data/aircraft_profiles.json --report backtest.md
//! ```
//!
//! `--from` / `--to` are UTC dates or RFC 3339 instants; flights wholly inside the window are read.
//! Both are required, so a rerun over the same data writes the same bytes. Needs `DATABASE_URL`
//! (read from `.env` too) pointing at a database holding the stats history. Read-only: nothing is
//! written to the database. `backtest` also fetches the airport database (positions and field
//! elevations) the server uses, and uses the bundled nav data.

use std::collections::BTreeSet;

use chrono::{DateTime, NaiveDate, Utc};
use ois_backend::{
    feed::{airports, nav::NavData},
    profile_builder::{self, backtest, stats::ProfileSet},
    repos::aircraft_profiles,
};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn required(args: &[String], name: &str) -> Result<String, String> {
    arg(args, name).ok_or_else(|| format!("{name} is required"))
}

/// A UTC date (`2026-10-11`, midnight) or an RFC 3339 instant.
fn instant(s: &str) -> Result<DateTime<Utc>, String> {
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(d.and_hms_opt(0, 0, 0).expect("midnight exists").and_utc());
    }
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| format!("{s:?} is neither a date nor an RFC 3339 instant: {e}"))
}

fn write(path: &str, body: &str) -> Result<(), String> {
    std::fs::write(path, body).map_err(|e| format!("writing {path}: {e}"))
}

#[tokio::main]
async fn main() -> Result<(), String> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(String::as_str).unwrap_or("");
    let from = instant(&required(&args, "--from")?)?;
    let to = instant(&required(&args, "--to")?)?;
    if from >= to {
        return Err("--from must be before --to".into());
    }

    let database_url = std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL must be set")?;
    let pool = sqlx::PgPool::connect(&database_url)
        .await
        .map_err(|e| format!("connecting: {e}"))?;

    match cmd {
        "build" => {
            let out = arg(&args, "--out").unwrap_or("backend/data/aircraft_profiles.json".into());
            let set = profile_builder::build(&pool, from, to)
                .await
                .map_err(|e| format!("building: {e:?}"))?;
            write(&out, &profile_builder::to_json(&set))?;
            let report = set.coverage_report();
            match arg(&args, "--report") {
                Some(path) => write(&path, &report)?,
                None => print!("{report}"),
            }
            tracing::info!(
                "{} flights read, {} types, wrote {out}",
                set.source.flights_read,
                set.types.len()
            );
        }
        "backtest" => {
            let path = required(&args, "--profiles")?;
            let raw = std::fs::read_to_string(&path).map_err(|e| format!("reading {path}: {e}"))?;
            let built: ProfileSet =
                serde_json::from_str(&raw).map_err(|e| format!("parsing {path}: {e}"))?;
            let rows = aircraft_profiles::list(&pool)
                .await
                .map_err(|e| format!("loading flow.aircraft_profile: {e:?}"))?;
            let (set_a, set_b, b_types): (_, _, BTreeSet<String>) =
                backtest::profile_sets(&rows, &built);
            let client = reqwest::Client::builder()
                .user_agent("ois-aircraft-profile-builder/0.1 (+https://vatusa.net)")
                .build()
                .map_err(|e| e.to_string())?;
            let (airports, _) = airports::fetch(&client)
                .await
                .map_err(|e| format!("fetching the airport database: {e}"))?;
            let nav = NavData::load();
            let (scored, held_out) =
                backtest::run(&pool, &nav, &airports, &set_a, &set_b, &b_types, from, to)
                    .await
                    .map_err(|e| format!("replaying: {e:?}"))?;
            let report =
                backtest::report(&scored, held_out, "A (stored catalog)", "B (with profiles)");
            match arg(&args, "--report") {
                Some(p) => write(&p, &report)?,
                None => print!("{report}"),
            }
            tracing::info!("{held_out} held-out flights, {} scored", scored.len());
        }
        _ => {
            return Err(
                "usage: aircraft-profile-builder build|backtest --from <date> --to <date> …".into(),
            );
        }
    }
    Ok(())
}
