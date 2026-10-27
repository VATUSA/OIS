//! Offline aircraft-profile builder and backtest (#828). Measures how each aircraft type climbs,
//! cruises and descends on VATSIM from the stats history OIS already stores, and scores a profile set
//! against held-out flights through the live prediction path. See `ois_backend::profile_builder`.
//!
//! ```text
//! # Measure: write the profile file and the coverage report.
//! RUST_LOG=info cargo run -p ois-backend --release --bin aircraft-profile-builder -- build \
//!     --from 2026-09-20 --to 2026-10-11 \
//!     --out /tmp/aircraft_profiles.json --report coverage.md
//!
//! # Score: the stored catalog (set A) against it with the file's types applied (set B).
//! RUST_LOG=info cargo run -p ois-backend --release --bin aircraft-profile-builder -- backtest \
//!     --from 2026-09-20 --to 2026-10-11 \
//!     --profiles /tmp/aircraft_profiles.json --report backtest.md
//! ```
//!
//! `--from` / `--to` are UTC dates or RFC 3339 instants; flights wholly inside the window are read.
//! Both are required, so a rerun over the same data writes the same bytes. `build` requires `--out`:
//! the file is a measurement to review, not committed data (#818 applies it). Needs `DATABASE_URL`
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

/// One run, parsed from the command line before anything connects or reads the network.
#[derive(Debug, PartialEq)]
enum Command {
    Build {
        window: (DateTime<Utc>, DateTime<Utc>),
        out: String,
        report: Option<String>,
    },
    Backtest {
        window: (DateTime<Utc>, DateTime<Utc>),
        profiles: String,
        report: Option<String>,
    },
}

const USAGE: &str = "usage: aircraft-profile-builder build --from <date> --to <date> --out <file> \
     [--report <file>] | backtest --from <date> --to <date> --profiles <file> [--report <file>]";

/// Parse `args` (the program name first). `--out` has no default: the output file is written only
/// where the caller says.
fn parse(args: &[String]) -> Result<Command, String> {
    let cmd = args.get(1).map(String::as_str).unwrap_or("");
    if cmd != "build" && cmd != "backtest" {
        return Err(USAGE.into());
    }
    let from = instant(&required(args, "--from")?)?;
    let to = instant(&required(args, "--to")?)?;
    if from >= to {
        return Err("--from must be before --to".into());
    }
    let report = arg(args, "--report");
    Ok(if cmd == "build" {
        Command::Build {
            window: (from, to),
            out: required(args, "--out")?,
            report,
        }
    } else {
        Command::Backtest {
            window: (from, to),
            profiles: required(args, "--profiles")?,
            report,
        }
    })
}

/// Write `report` to `path`, or print it when no path was given.
fn emit(report: &str, path: Option<&str>) -> Result<(), String> {
    match path {
        Some(p) => write(p, report),
        None => {
            print!("{report}");
            Ok(())
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), String> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();
    let args: Vec<String> = std::env::args().collect();
    let command = parse(&args)?;

    let database_url = std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL must be set")?;
    let pool = sqlx::PgPool::connect(&database_url)
        .await
        .map_err(|e| format!("connecting: {e}"))?;

    match command {
        Command::Build {
            window: (from, to),
            out,
            report,
        } => {
            let set = profile_builder::build(&pool, from, to)
                .await
                .map_err(|e| format!("building: {e:?}"))?;
            write(&out, &profile_builder::to_json(&set))?;
            emit(&set.coverage_report(), report.as_deref())?;
            tracing::info!(
                "{} flights read, {} types, wrote {out}",
                set.source.flights_read,
                set.types.len()
            );
        }
        Command::Backtest {
            window: (from, to),
            profiles,
            report,
        } => {
            let raw = std::fs::read_to_string(&profiles)
                .map_err(|e| format!("reading {profiles}: {e}"))?;
            let built: ProfileSet =
                serde_json::from_str(&raw).map_err(|e| format!("parsing {profiles}: {e}"))?;
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
            let replay = backtest::Replay {
                nav: &nav,
                airports: &airports,
                set_a: &set_a,
                set_b: &set_b,
                b_types: &b_types,
            };
            let (scored, held_out) = backtest::run(&pool, &replay, from, to)
                .await
                .map_err(|e| format!("replaying: {e:?}"))?;
            let text =
                backtest::report(&scored, held_out, "A (stored catalog)", "B (with profiles)");
            emit(&text, report.as_deref())?;
            tracing::info!("{held_out} held-out flights, {} scored", scored.len());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        std::iter::once("aircraft-profile-builder")
            .chain(v.iter().copied())
            .map(String::from)
            .collect()
    }

    /// `build` has no default output path: without `--out` it is refused before anything
    /// connects; with it, the file is written where the caller said.
    #[test]
    fn build_without_out_is_refused_before_connecting() {
        let window = ["build", "--from", "2026-09-20", "--to", "2026-10-11"];
        assert_eq!(parse(&args(&window)), Err("--out is required".to_string()));
        let mut with = window.to_vec();
        with.extend(["--out", "x.json"]);
        match parse(&args(&with)) {
            Ok(Command::Build { out, report, .. }) => {
                assert_eq!(out, "x.json");
                assert_eq!(report, None);
            }
            other => panic!("{other:?}"),
        }
    }

    /// The documented `--from` / `--to` forms: a UTC date is its midnight, an RFC 3339 instant is
    /// that instant, anything else is refused naming the input.
    #[test]
    fn from_and_to_accept_a_date_or_an_rfc3339_instant() {
        assert_eq!(
            instant("2026-10-11").unwrap(),
            DateTime::parse_from_rfc3339("2026-10-11T00:00:00Z").unwrap()
        );
        assert_eq!(
            instant("2026-10-11T06:30:00+02:00").unwrap(),
            DateTime::parse_from_rfc3339("2026-10-11T04:30:00Z").unwrap()
        );
        assert!(
            instant("11/10/2026")
                .unwrap_err()
                .contains("\"11/10/2026\"")
        );
        let backwards = [
            "backtest",
            "--from",
            "2026-10-11",
            "--to",
            "2026-10-11",
            "--profiles",
            "p",
        ];
        assert_eq!(
            parse(&args(&backwards)),
            Err("--from must be before --to".to_string())
        );
    }
}
