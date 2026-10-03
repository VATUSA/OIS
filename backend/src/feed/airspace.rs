//! ARTCC boundary polygons, for FCA scope filtering. Bundled GeoJSON (a FeatureCollection
//! of Polygon/MultiPolygon features keyed by `properties.id`); coordinates are `[lon, lat]`.
//! Used to test whether a route's FCA crossing falls inside an FCA's scoped ARTCCs.

use std::collections::HashMap;

use serde::Deserialize;

/// A closed ring of `[lat, lon]` points.
type Ring = Vec<[f64; 2]>;

#[derive(Default)]
pub struct Boundaries {
    /// ARTCC code → its outer rings (an ARTCC may span several disjoint areas).
    polys: HashMap<String, Vec<Ring>>,
}

impl Boundaries {
    /// Parse the bundled boundary GeoJSON (embedded at compile time).
    pub fn load() -> Self {
        parse(include_str!("../../data/artcc-boundaries.json"))
    }

    pub fn is_empty(&self) -> bool {
        self.polys.is_empty()
    }

    pub fn len(&self) -> usize {
        self.polys.len()
    }

    /// Whether we hold boundary geometry for ARTCC `code` at all.
    ///
    /// Distinct from "is this a real US ARTCC": `ZAK` (Oakland Oceanic) and `ZSU` (San Juan) are
    /// both real and neither has a polygon in the bundled set. Callers that need to *draw* a
    /// facility must ask this; callers deciding whether a controller is American must not
    /// (VATUSA/OIS#482).
    pub fn has(&self, code: &str) -> bool {
        self.polys.contains_key(&code.to_ascii_uppercase())
    }

    /// Whether `(lat, lon)` lies inside any polygon of ARTCC `code`.
    pub fn contains(&self, code: &str, lat: f64, lon: f64) -> bool {
        self.polys
            .get(&code.to_ascii_uppercase())
            .is_some_and(|rings| rings.iter().any(|r| point_in_ring(r, lat, lon)))
    }

    /// Whether `(lat, lon)` lies inside **any** loaded US ARTCC — a cheap "over US airspace" test
    /// for scoping stats collection to US-relevant traffic.
    pub fn any_contains(&self, lat: f64, lon: f64) -> bool {
        self.polys
            .values()
            .any(|rings| rings.iter().any(|r| point_in_ring(r, lat, lon)))
    }
}

#[derive(Deserialize)]
struct FeatureCollection {
    #[serde(default)]
    features: Vec<Feature>,
}

#[derive(Deserialize)]
struct Feature {
    properties: Props,
    geometry: Geometry,
}

#[derive(Deserialize)]
struct Props {
    id: Option<String>,
}

#[derive(Deserialize)]
struct Geometry {
    #[serde(rename = "type")]
    gtype: String,
    #[serde(default)]
    coordinates: serde_json::Value,
}

fn parse(src: &str) -> Boundaries {
    let fc: FeatureCollection = serde_json::from_str(src).unwrap_or(FeatureCollection {
        features: Vec::new(),
    });
    let mut polys: HashMap<String, Vec<Ring>> = HashMap::new();
    for f in fc.features {
        let Some(id) = f.properties.id else { continue };
        let rings = outer_rings(&f.geometry);
        if !rings.is_empty() {
            polys
                .entry(id.to_ascii_uppercase())
                .or_default()
                .extend(rings);
        }
    }
    Boundaries { polys }
}

/// Extract each polygon's outer ring (converted to `[lat, lon]`) from a Polygon or
/// MultiPolygon geometry. Holes are ignored — ARTCC boundaries don't use them.
fn outer_rings(geom: &Geometry) -> Vec<Ring> {
    match geom.gtype.as_str() {
        "Polygon" => serde_json::from_value::<Vec<Vec<[f64; 2]>>>(geom.coordinates.clone())
            .ok()
            .and_then(|rings| rings.into_iter().next())
            .map(|outer| vec![to_latlon(outer)])
            .unwrap_or_default(),
        "MultiPolygon" => {
            serde_json::from_value::<Vec<Vec<Vec<[f64; 2]>>>>(geom.coordinates.clone())
                .ok()
                .map(|polys| {
                    polys
                        .into_iter()
                        .filter_map(|rings| rings.into_iter().next())
                        .map(to_latlon)
                        .collect()
                })
                .unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

/// GeoJSON stores `[lon, lat]`; we work in `[lat, lon]`.
fn to_latlon(ring: Vec<[f64; 2]>) -> Ring {
    ring.into_iter().map(|[lon, lat]| [lat, lon]).collect()
}

/// Ray-casting point-in-polygon on a `[lat, lon]` ring.
fn point_in_ring(ring: &[[f64; 2]], lat: f64, lon: f64) -> bool {
    if ring.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = ring.len() - 1;
    for i in 0..ring.len() {
        let (lat_i, lon_i) = (ring[i][0], ring[i][1]);
        let (lat_j, lon_j) = (ring[j][0], ring[j][1]);
        if (lat_i > lat) != (lat_j > lat)
            && lon < (lon_j - lon_i) * (lat - lat_i) / (lat_j - lat_i) + lon_i
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_in_a_square() {
        let sq: Ring = vec![
            [0.0, 0.0],
            [0.0, 10.0],
            [10.0, 10.0],
            [10.0, 0.0],
            [0.0, 0.0],
        ];
        assert!(point_in_ring(&sq, 5.0, 5.0));
        assert!(!point_in_ring(&sq, 15.0, 5.0));
        assert!(!point_in_ring(&sq, 5.0, -1.0));
    }

    #[test]
    fn loads_bundled_boundaries_and_locates_artccs() {
        let b = Boundaries::load();
        assert!(
            b.len() >= 20,
            "should load the CONUS ARTCCs, got {}",
            b.len()
        );
        // Chicago Center (ZAU) contains O'Hare (~41.98, -87.90).
        assert!(b.contains("ZAU", 41.98, -87.90), "ORD should be inside ZAU");
        // ...and does not contain a point over Los Angeles.
        assert!(!b.contains("ZAU", 33.94, -118.4), "LAX is not inside ZAU");
    }
}

#[cfg(test)]
mod boundary_identity_tests {
    //! VATUSA/OIS#556: every bundled boundary must be a facility OIS knows, or a deliberate exception.
    //! Honolulu was split — `HCF` in `org.facilities`, `ZHN` in the boundaries — and nothing noticed.

    use std::collections::BTreeSet;

    /// Polygons OIS draws but has no facility for. Each needs a reason, so adding one is a decision.
    const NOT_OIS_FACILITIES: &[(&str, &str)] = &[(
        "ZUA",
        "Guam CERAP: not a VATUSA facility, kept for the map only (#556)",
    )];

    fn bundled_ids() -> BTreeSet<String> {
        let json: serde_json::Value =
            serde_json::from_str(include_str!("../../data/artcc-boundaries.json")).unwrap();
        json["features"]
            .as_array()
            .expect("a FeatureCollection")
            .iter()
            .map(|f| {
                f["properties"]["id"]
                    .as_str()
                    .expect("every feature has an id")
                    .to_string()
            })
            .collect()
    }

    #[sqlx::test]
    async fn every_bundled_boundary_is_a_known_facility_or_a_listed_exception(pool: sqlx::PgPool) {
        let facilities: BTreeSet<String> =
            sqlx::query_scalar::<_, String>("select id from org.facilities")
                .fetch_all(&pool)
                .await
                .unwrap()
                .into_iter()
                .collect();
        let exceptions: BTreeSet<&str> = NOT_OIS_FACILITIES.iter().map(|(id, _)| *id).collect();

        let unknown: Vec<String> = bundled_ids()
            .into_iter()
            .filter(|id| !facilities.contains(id) && !exceptions.contains(id.as_str()))
            .collect();

        assert!(
            unknown.is_empty(),
            "boundary ids with no org.facilities row and no listed reason: {unknown:?} — rename the \
             feature to the facility's OIS id, or add it to NOT_OIS_FACILITIES with why"
        );
    }

    /// An exception that has since become a facility would hide a real split, so the list must not go
    /// stale in that direction either.
    #[sqlx::test]
    async fn no_listed_exception_is_actually_a_facility(pool: sqlx::PgPool) {
        for (id, _) in NOT_OIS_FACILITIES {
            let is_facility: bool =
                sqlx::query_scalar("select exists (select 1 from org.facilities where id = $1)")
                    .bind(id)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert!(
                !is_facility,
                "{id} is a facility now; drop it from NOT_OIS_FACILITIES"
            );
        }
    }

    /// The web map and the backend carry their own copy. They must stay byte-identical, or a rename
    /// in one silently leaves the other drawing — or filtering on — the old id.
    #[test]
    fn the_web_and_backend_boundary_copies_are_identical() {
        assert!(
            include_bytes!("../../data/artcc-boundaries.json")[..]
                == include_bytes!("../../../web/src/assets/artcc-boundaries.json")[..],
            "backend/data and web/src/assets artcc-boundaries.json have diverged"
        );
    }
}
