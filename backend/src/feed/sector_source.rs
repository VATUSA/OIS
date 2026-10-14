//! Where ATC sector volumes (#594) come from: the pinned source file, fetched and parsed into
//! [`SectorVolume`]s. `repos::airspace_sector_seed` writes them to `flow.airspace_sector` (#821).
//!
//! No public FAA or vNAS source publishes altitude-bounded sector geometry (see #594 for the
//! search), so the dataset comes from the Virtual Traffic Situation Display project's
//! `Data/sectors.json`, used with its authors' permission. It is fetched at a **pinned commit**
//! ([`SOURCE_REF`]); the file is never committed to this repo (it is CC BY-NC-SA 4.0, OIS is MIT).
//! To move to a newer revision, change [`SOURCE_REF`] and deploy: the next start replaces every row.
//! The seed decides a table is current from `SOURCE_REF` alone, so a change to [`parse`] that alters
//! what is stored also has to move `SOURCE_REF`, or be followed by one manual
//! `airspace-sector-importer` run, or databases already at the pin keep the old rows.
//!
//! Volumes that fail validation (`feed::sectors::validate_volume`) are skipped and listed, never
//! repaired. The source has one, ZTL `07007`, whose floor is above its ceiling.

use std::time::Duration;

use serde::Deserialize;

use crate::feed::sectors::{SectorVolume, validate_volume};

/// The vtsd commit imported. `source_cycle` on every row.
pub const SOURCE_REF: &str = "f33ef73f21091e71456e45cfe9019ddf3ba76247";
/// `source` on every row.
pub const SOURCE: &str = "vtsd sectors.json";

/// The pinned source file.
pub fn source_url() -> String {
    format!(
        "https://raw.githubusercontent.com/Virtual-Traffic-Situation-Display/vtsd/{SOURCE_REF}/Data/sectors.json"
    )
}

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
                "polygon has {} rings; only a single outer ring is supported",
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

/// The source file as the volumes the table would store, plus the features it skips.
pub struct Parsed {
    pub volumes: Vec<SectorVolume>,
    /// `"<ARTCC> <full_id>: <reason>"` for every feature that failed conversion or validation.
    pub skipped: Vec<String>,
}

/// Parse `sectors.json`. A feature with an unknown tier, a polygon with other than one ring (a hole,
/// or none), a short position or a volume that fails `validate_volume` is skipped and listed. Anything else that doesn't match the expected
/// shape (another geometry type, a missing property, a non-integer altitude) fails the whole parse.
pub fn parse(bytes: &[u8]) -> Result<Parsed, String> {
    let collection: Collection =
        serde_json::from_slice(bytes).map_err(|e| format!("parsing {SOURCE}: {e}"))?;
    let mut volumes = Vec::new();
    let mut skipped = Vec::new();
    for f in collection.features {
        let id = format!("{} {}", f.properties.artcc, f.properties.full_id);
        match to_volume(f).and_then(|v| validate_volume(&v).map(|()| v)) {
            Ok(v) => volumes.push(v),
            Err(e) => skipped.push(format!("{id}: {e}")),
        }
    }
    Ok(Parsed { volumes, skipped })
}

/// Fetch the pinned `sectors.json`. The real network call; tests pass their own fetcher to the seed.
pub async fn fetch() -> Result<Vec<u8>, String> {
    let url = source_url();
    let body = reqwest::Client::builder()
        .user_agent("ois-airspace-sector-seed/0.1 (+https://vatusa.net)")
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?
        .get(&url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| format!("fetching {url}: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("reading {url}: {e}"))?;
    Ok(body.to_vec())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A hand-written document in the source's shape (the real file can't be committed, see the
    /// module docs). Positions are `[lon, lat]`, as in GeoJSON. Four features the table stores, one
    /// per tier: a ZDC High Polygon, a ZNY Approach MultiPolygon of two parts, and two ZAU volumes,
    /// Low and Ultra High. Six it must skip: ZTL `07007` (floor above ceiling, as in the real
    /// source), and ZOA features with an unknown tier, a hole, a position with one number, no ring
    /// at all, and a bridged ring (two lobes joined at a revisited vertex, #481).
    pub(crate) const FIXTURE: &str = r#"{
      "type": "FeatureCollection",
      "features": [
        {"type": "Feature",
         "properties": {"artcc": "zdc", "sector": "01", "tier": "High", "base_alt": 24000,
                        "max_alt": 60000, "full_id": "01001"},
         "geometry": {"type": "Polygon",
                      "coordinates": [[[-77, 38], [-76, 38], [-76, 39], [-77, 39], [-77, 38]]]}},
        {"type": "Feature",
         "properties": {"artcc": "ZNY", "sector": "70", "tier": "Approach Control", "base_alt": 0,
                        "max_alt": 4000, "full_id": "07001"},
         "geometry": {"type": "MultiPolygon",
                      "coordinates": [[[[-74, 40], [-73, 40], [-73, 41], [-74, 40]]],
                                      [[[-72, 40], [-71, 40], [-71, 41], [-72, 40]]]]}},
        {"type": "Feature",
         "properties": {"artcc": "ZAU", "sector": "02", "tier": "Low", "base_alt": 0,
                        "max_alt": 23000, "full_id": "02001"},
         "geometry": {"type": "Polygon",
                      "coordinates": [[[-88, 41], [-87, 41], [-87, 42], [-88, 42], [-88, 41]]]}},
        {"type": "Feature",
         "properties": {"artcc": "ZAU", "sector": "50", "tier": "Ultra High", "base_alt": 35000,
                        "max_alt": 60000, "full_id": "50001"},
         "geometry": {"type": "Polygon",
                      "coordinates": [[[-88, 41], [-87, 41], [-87, 42], [-88, 42], [-88, 41]]]}},
        {"type": "Feature",
         "properties": {"artcc": "ZTL", "sector": "07", "tier": "Low", "base_alt": 30000,
                        "max_alt": 20000, "full_id": "07007"},
         "geometry": {"type": "Polygon",
                      "coordinates": [[[-85, 33], [-84, 33], [-84, 34], [-85, 33]]]}},
        {"type": "Feature",
         "properties": {"artcc": "ZOA", "sector": "99", "tier": "Oceanic", "base_alt": 0,
                        "max_alt": 60000, "full_id": "99001"},
         "geometry": {"type": "Polygon",
                      "coordinates": [[[-123, 37], [-122, 37], [-122, 38], [-123, 37]]]}},
        {"type": "Feature",
         "properties": {"artcc": "ZOA", "sector": "99", "tier": "High", "base_alt": 24000,
                        "max_alt": 60000, "full_id": "99002"},
         "geometry": {"type": "Polygon",
                      "coordinates": [[[-123, 37], [-120, 37], [-120, 40], [-123, 37]],
                                      [[-122, 38], [-121, 38], [-121, 39], [-122, 38]]]}},
        {"type": "Feature",
         "properties": {"artcc": "ZOA", "sector": "99", "tier": "High", "base_alt": 24000,
                        "max_alt": 60000, "full_id": "99003"},
         "geometry": {"type": "Polygon",
                      "coordinates": [[[-123, 37], [-122], [-122, 38], [-123, 37]]]}},
        {"type": "Feature",
         "properties": {"artcc": "ZOA", "sector": "99", "tier": "High", "base_alt": 24000,
                        "max_alt": 60000, "full_id": "99004"},
         "geometry": {"type": "Polygon", "coordinates": []}},
        {"type": "Feature",
         "properties": {"artcc": "ZOA", "sector": "99", "tier": "High", "base_alt": 24000,
                        "max_alt": 60000, "full_id": "99005"},
         "geometry": {"type": "Polygon",
                      "coordinates": [[[-77, 38], [-76, 38], [-76, 39], [-77, 39], [-77, 38],
                                       [-77, 37], [-78, 37], [-78, 36], [-77, 36], [-77, 37],
                                       [-77, 38]]]}}
      ]
    }"#;

    #[test]
    fn parse_converts_tiers_flips_positions_and_lists_what_it_skips() {
        let parsed = parse(FIXTURE.as_bytes()).unwrap();
        let ids: Vec<(&str, &str, &str, &str)> = parsed
            .volumes
            .iter()
            .map(|v| {
                (
                    v.artcc.as_str(),
                    v.sector_id.as_str(),
                    v.volume_id.as_str(),
                    v.tier.as_str(),
                )
            })
            .collect();
        // Every source tier maps to its own stored tier.
        assert_eq!(
            ids,
            [
                ("ZDC", "01", "01001", "high"),
                ("ZNY", "70", "07001", "approach"),
                ("ZAU", "02", "02001", "low"),
                ("ZAU", "50", "50001", "ultra_high"),
            ]
        );
        // `[lon, lat]` in, `[lat, lon]` stored.
        assert_eq!(parsed.volumes[0].rings[0][1], [38.0, -76.0]);
        // One ring per MultiPolygon part.
        assert_eq!(parsed.volumes[1].rings.len(), 2);
        // Each refusal names the feature and why.
        assert_eq!(
            parsed.skipped,
            [
                "ZTL 07007: altitudes 30000..20000 ft are not a positive band",
                "ZOA 99001: unknown tier \"Oceanic\"",
                "ZOA 99002: polygon has 2 rings; only a single outer ring is supported",
                "ZOA 99003: position has fewer than two numbers",
                "ZOA 99004: polygon has 0 rings; only a single outer ring is supported",
                "ZOA 99005: ring revisits [38, -77] (bridged)",
            ]
        );
    }

    #[test]
    fn parse_refuses_a_document_that_is_not_the_expected_geojson() {
        let err = parse(b"<html>rate limited</html>").err().unwrap();
        assert!(err.starts_with("parsing vtsd sectors.json: "), "{err}");
    }

    /// A document of one feature, with `props` and `geometry` as raw JSON.
    fn one_feature(props: &str, geometry: &str) -> String {
        format!(r#"{{"features": [{{"properties": {props}, "geometry": {geometry}}}]}}"#)
    }

    const PROPS: &str = r#"{"artcc": "ZDC", "sector": "01", "tier": "High", "base_alt": 24000,
                            "max_alt": 60000, "full_id": "01001"}"#;
    const SQUARE: &str = r#"{"type": "Polygon", "coordinates": [[[-77, 38], [-76, 38], [-76, 39], [-77, 39], [-77, 38]]]}"#;

    /// A feature outside the expected shape fails the whole document rather than being skipped: a
    /// geometry type other than Polygon or MultiPolygon, a missing property, a non-integer altitude.
    #[test]
    fn parse_refuses_a_document_with_a_feature_outside_the_expected_shape() {
        // Control: the same feature in the expected shape parses.
        let ok = parse(one_feature(PROPS, SQUARE).as_bytes()).unwrap();
        assert_eq!((ok.volumes.len(), ok.skipped.len()), (1, 0));

        let collection = r#"{"type": "GeometryCollection", "geometries": []}"#;
        let no_top = PROPS.replace(r#""max_alt": 60000, "#, "");
        let fractional = PROPS.replace(r#""base_alt": 24000"#, r#""base_alt": 1.5"#);
        assert_ne!(no_top, PROPS);
        assert_ne!(fractional, PROPS);
        for doc in [
            one_feature(PROPS, collection),
            one_feature(&no_top, SQUARE),
            one_feature(&fractional, SQUARE),
        ] {
            let err = parse(doc.as_bytes()).err().unwrap();
            assert!(err.starts_with("parsing vtsd sectors.json: "), "{err}");
        }
    }

    /// The source URL is the pinned commit, never a branch, so a deploy can't pick up new data
    /// nobody reviewed.
    #[test]
    fn the_source_is_fetched_at_the_pinned_commit() {
        assert_eq!(
            source_url(),
            "https://raw.githubusercontent.com/Virtual-Traffic-Situation-Display/vtsd/\
             f33ef73f21091e71456e45cfe9019ddf3ba76247/Data/sectors.json"
        );
    }
}
