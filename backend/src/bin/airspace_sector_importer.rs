//! Offline importer: loads ATC sector volumes (#594) into `flow.airspace_sector`.
//!
//! No public FAA or vNAS source publishes altitude-bounded sector geometry (see #594 for the
//! search), so the dataset comes from the Virtual Traffic Situation Display project's
//! `Data/sectors.json`, used with its authors' permission. It is fetched at a **pinned commit** and
//! written to the database; the file is never committed to this repo (it is CC BY-NC-SA 4.0, OIS is
//! MIT). Each row records `source` and `source_cycle` (the commit), so the data can be traced and
//! regenerated. To move to a newer revision, change [`SOURCE_REF`] and re-run.
//!
//! ```text
//! RUST_LOG=info cargo run -p ois-backend --bin airspace-sector-importer                 # every ARTCC
//! RUST_LOG=info cargo run -p ois-backend --bin airspace-sector-importer -- --artcc ZDC  # just one
//! ```
//! (Without `RUST_LOG=info` only errors print, so a successful run is silent.)
//! Needs `DATABASE_URL` (read from `.env` too) and a migrated database. Each ARTCC is replaced
//! atomically. A running server picks the new data up on its next `airspace_sectors_refresh` tick
//! (5 minutes). Volumes that fail validation (`feed::sectors::validate_volume`) are skipped and
//! listed, never repaired — the source has one, ZTL `07007`, whose floor is above its ceiling.

use std::{collections::BTreeMap, time::Duration};

use ois_backend::{feed::sectors::SectorVolume, repos::airspace_sectors};
use serde::Deserialize;

/// The vtsd commit imported. `source_cycle` on every row.
const SOURCE_REF: &str = "f33ef73f21091e71456e45cfe9019ddf3ba76247";
const SOURCE: &str = "vtsd sectors.json";

#[derive(Deserialize)]
struct Collection {
    features: Vec<Feature>,
}

#[derive(Deserialize)]
struct Feature {
    properties: Props,
    geometry: Geometry,
}

#[derive(Deserialize)]
struct Props {
    artcc: String,
    sector: String,
    tier: String,
    base_alt: i32,
    max_alt: i32,
    full_id: String,
}

/// GeoJSON positions are `[lon, lat, ...]`.
#[derive(Deserialize)]
#[serde(tag = "type", content = "coordinates")]
enum Geometry {
    Polygon(Vec<Vec<Vec<f64>>>),
    MultiPolygon(Vec<Vec<Vec<Vec<f64>>>>),
}

/// A source feature as a stored volume: tier mapped, positions flipped to `[lat, lon]`, one ring
/// per polygon part. Refuses what the table can't represent (holes, unknown tiers).
fn to_volume(f: Feature) -> Result<SectorVolume, String> {
    let p = f.properties;
    let tier = match p.tier.as_str() {
        "Low" => "low",
        "High" => "high",
        "Ultra High" => "ultra_high",
        "Approach Control" => "approach",
        other => return Err(format!("unknown tier {other:?}")),
    };
    let polygons = match f.geometry {
        Geometry::Polygon(rings) => vec![rings],
        Geometry::MultiPolygon(polys) => polys,
    };
    let mut rings = Vec::new();
    for poly in polygons {
        if poly.len() != 1 {
            return Err(format!(
                "polygon has {} rings; holes aren't supported",
                poly.len()
            ));
        }
        let ring = poly[0]
            .iter()
            .map(|pos| match pos.as_slice() {
                [lon, lat, ..] => Ok([*lat, *lon]),
                _ => Err("position has fewer than two numbers".to_string()),
            })
            .collect::<Result<Vec<_>, _>>()?;
        rings.push(ring);
    }
    Ok(SectorVolume {
        artcc: p.artcc.to_ascii_uppercase(),
        sector_id: p.sector,
        volume_id: p.full_id,
        name: None,
        tier: tier.into(),
        base_alt_ft: p.base_alt,
        top_alt_ft: p.max_alt,
        rings,
    })
}

#[tokio::main]
async fn main() -> Result<(), String> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();
    let args: Vec<String> = std::env::args().collect();
    let only = args
        .iter()
        .position(|a| a == "--artcc")
        .map(|i| {
            args.get(i + 1)
                .map(|a| a.to_ascii_uppercase())
                .ok_or("--artcc needs a value")
        })
        .transpose()?;

    let url = format!(
        "https://raw.githubusercontent.com/Virtual-Traffic-Situation-Display/vtsd/{SOURCE_REF}/Data/sectors.json"
    );
    let collection: Collection = reqwest::Client::builder()
        .user_agent("ois-airspace-sector-importer/0.1 (+https://vatusa.net)")
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?
        .get(&url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| format!("fetching {url}: {e}"))?
        .json()
        .await
        .map_err(|e| format!("parsing {url}: {e}"))?;

    let mut by_artcc: BTreeMap<String, Vec<SectorVolume>> = BTreeMap::new();
    let mut skipped = 0;
    for f in collection.features {
        let id = format!("{} {}", f.properties.artcc, f.properties.full_id);
        match to_volume(f).and_then(|v| ois_backend::feed::sectors::validate_volume(&v).map(|()| v))
        {
            Ok(v) if only.as_ref().is_none_or(|a| *a == v.artcc) => {
                by_artcc.entry(v.artcc.clone()).or_default().push(v)
            }
            Ok(_) => {}
            Err(e) => {
                skipped += 1;
                tracing::warn!("skipping {id}: {e}");
            }
        }
    }
    if let Some(a) = &only
        && !by_artcc.contains_key(a)
    {
        return Err(format!("the source has no volumes for {a}"));
    }

    let database_url = std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL must be set")?;
    let pool = sqlx::PgPool::connect(&database_url)
        .await
        .map_err(|e| format!("connecting: {e}"))?;
    let mut total = 0;
    for (artcc, volumes) in &by_artcc {
        airspace_sectors::replace_artcc(&pool, artcc, volumes, SOURCE, SOURCE_REF).await?;
        tracing::info!("{artcc}: {} volumes", volumes.len());
        total += volumes.len();
    }
    tracing::info!(
        "imported {total} volumes across {} ARTCCs from {SOURCE}@{SOURCE_REF}; skipped {skipped}",
        by_artcc.len()
    );
    Ok(())
}
