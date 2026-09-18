//! US winds aloft from the Aviation Weather Center FB (winds/temps) tables, for enroute
//! ETA correction. Fetched at runtime and hot-swapped; fails safe to still air whenever
//! data is missing. Ported from vatflow's `winds-aloft.js`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::airports::{Airport, AirportDb};

const R_NM: f64 = 3440.065;
/// AWC FB tables cover the CONUS in these six regions.
const REGIONS: [&str; 6] = ["bos", "mia", "chi", "dfw", "slc", "sfo"];
/// FB table sets: `low` is 3,000–39,000 ft, `high` adds 45,000 and 53,000 ft.
const TABLE_LEVELS: [&str; 2] = ["low", "high"];
/// A station further than this from the query point contributes no wind (nm).
const MAX_STATION_NM: f64 = 600.0;
/// The wind at a point is an inverse-distance-squared blend of at most this many nearest stations.
const BLEND_STATIONS: usize = 4;
/// A query point this close to a station (nm) takes that station's wind outright.
const ON_STATION_NM: f64 = 0.1;

/// Wind at one forecast level: direction (deg true, None when calm) and speed (kt).
#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct WindLevel {
    pub dir: Option<f64>,
    pub spd: f64,
}

impl WindLevel {
    /// The wind as a "from" vector `(east, north)` in kt, so winds can be averaged: calm is zero,
    /// and the headwind along a course `c` is `u·sin c + v·cos c`.
    fn uv(self) -> (f64, f64) {
        match self.dir {
            Some(dir) => {
                let r = dir.to_radians();
                (self.spd * r.sin(), self.spd * r.cos())
            }
            None => (0.0, 0.0),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Station {
    lat: f64,
    lon: f64,
    /// `(altitude_ft, wind)` forecast levels.
    levels: Vec<(i32, WindLevel)>,
}

impl Station {
    /// The wind vector at `alt_ft`, linearly interpolated between the bracketing forecast levels
    /// (in any order); held at the nearest level outside the forecast range. None with no levels.
    fn uv_at(&self, alt_ft: f64) -> Option<(f64, f64)> {
        let below = self
            .levels
            .iter()
            .filter(|(a, _)| *a as f64 <= alt_ft)
            .max_by_key(|(a, _)| *a);
        let above = self
            .levels
            .iter()
            .filter(|(a, _)| *a as f64 >= alt_ft)
            .min_by_key(|(a, _)| *a);
        match (below, above) {
            (Some(&(a0, w0)), Some(&(a1, w1))) if a1 > a0 => {
                let t = (alt_ft - a0 as f64) / (a1 - a0) as f64;
                let ((u0, v0), (u1, v1)) = (w0.uv(), w1.uv());
                Some((u0 + t * (u1 - u0), v0 + t * (v1 - v0)))
            }
            (Some(&(_, w)), _) | (None, Some(&(_, w))) => Some(w.uv()),
            (None, None) => None,
        }
    }
}

/// Headwind component (kt; `+` = headwind) of a `(u, v)` "from" wind along `course_deg`.
fn headwind_along((u, v): (f64, f64), course_deg: f64) -> f64 {
    let c = course_deg.to_radians();
    u * c.sin() + v * c.cos()
}

/// The winds-aloft picture. Serializable so the stats collector can snapshot it for historical
/// replay (`stats.winds`) and the reconstruction can load a past snapshot back.
#[derive(Default, Serialize, Deserialize)]
pub struct Winds {
    stations: Vec<Station>,
}

impl Winds {
    pub fn is_empty(&self) -> bool {
        self.stations.is_empty()
    }

    pub fn station_count(&self) -> usize {
        self.stations.len()
    }

    /// The wind vector at a point: each of the [`BLEND_STATIONS`] nearest stations within
    /// [`MAX_STATION_NM`] interpolated to `alt_ft`, blended by inverse distance squared. None when
    /// no station covers the point (the caller flies still air).
    fn wind_uv(&self, lat: f64, lon: f64, alt_ft: f64) -> Option<(f64, f64)> {
        let mut near: Vec<(f64, (f64, f64))> = self
            .stations
            .iter()
            .filter_map(|s| {
                let d = gc_dist(lat, lon, s.lat, s.lon);
                (d <= MAX_STATION_NM).then_some(d).zip(s.uv_at(alt_ft))
            })
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        near.truncate(BLEND_STATIONS);
        let &(nearest_d, nearest_uv) = near.first()?;
        if nearest_d <= ON_STATION_NM {
            return Some(nearest_uv);
        }
        let (mut u, mut v, mut wsum) = (0.0, 0.0, 0.0);
        for (d, (su, sv)) in near {
            let w = 1.0 / (d * d);
            u += w * su;
            v += w * sv;
            wsum += w;
        }
        Some((u / wsum, v / wsum))
    }

    /// Mean headwind (kt; `+` = headwind, `-` = tailwind) along a `[lat, lon]` polyline at
    /// `alt_ft`. None when no station covers the route (caller flies still air).
    pub fn route_headwind(&self, points: &[[f64; 2]], alt_ft: f64) -> Option<f64> {
        if points.len() < 2 || self.stations.is_empty() {
            return None;
        }
        let mut sum = 0.0;
        let mut cnt = 0;
        for seg in points.windows(2) {
            let (a, b) = (seg[0], seg[1]);
            let course = bearing_deg(a[0], a[1], b[0], b[1]);
            let (mlat, mlon) = ((a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0);
            // A calm wind (9900) is a zero vector, so it still counts as a sample.
            if let Some(uv) = self.wind_uv(mlat, mlon, alt_ft) {
                sum += headwind_along(uv, course);
                cnt += 1;
            }
        }
        (cnt > 0).then(|| sum / cnt as f64)
    }

    /// Headwind (kt) at a single point along `course_deg`.
    pub fn point_headwind(&self, lat: f64, lon: f64, course_deg: f64, alt_ft: f64) -> Option<f64> {
        self.wind_uv(lat, lon, alt_ft)
            .map(|uv| headwind_along(uv, course_deg))
    }
}

/// Fetch and assemble the current winds-aloft picture. Best-effort per region; an empty
/// result simply means the model flies still air until the next refresh.
pub async fn fetch(client: &reqwest::Client, airports: &AirportDb) -> Winds {
    let mut merged: HashMap<String, Vec<(i32, WindLevel)>> = HashMap::new();
    for level in TABLE_LEVELS {
        for reg in REGIONS {
            let url =
                format!("https://aviationweather.gov/api/data/windtemp?region={reg}&level={level}");
            let text = match client
                .get(&url)
                .send()
                .await
                .and_then(|r| r.error_for_status())
            {
                Ok(resp) => match resp.text().await {
                    Ok(t) => t,
                    Err(_) => continue,
                },
                Err(e) => {
                    tracing::debug!(region = reg, level, error = %e, "winds region fetch failed");
                    continue;
                }
            };
            if !text.contains("FT") {
                continue;
            }
            merge_table(&mut merged, &text);
        }
    }

    // Resolve each FB station id to airport coordinates (K.., P.., or bare).
    let mut stations = Vec::new();
    for (id, levels) in merged {
        let coord = airports
            .get(&format!("K{id}"))
            .or_else(|| airports.get(&format!("P{id}")))
            .or_else(|| airports.get(&id));
        if let Some(&Airport { lat, lon, .. }) = coord {
            stations.push(Station { lat, lon, levels });
        }
    }
    Winds { stations }
}

/// Add a table's station levels to `merged`, extending (never replacing) a station already seen —
/// the high table's FL450/FL530 join the same station's low levels.
fn merge_table(merged: &mut HashMap<String, Vec<(i32, WindLevel)>>, text: &str) {
    for (id, levels) in parse_windtemp(text) {
        merged.entry(id).or_default().extend(levels);
    }
}

/// Parse an AWC FB table into `(station_id, levels)`. Columns are matched to their nearest
/// header altitude by character position, tolerating irregular spacing.
pub fn parse_windtemp(text: &str) -> Vec<(String, Vec<(i32, WindLevel)>)> {
    let mut levels: Option<Vec<(i32, f64)>> = None;
    let mut out = Vec::new();

    for raw in text.lines() {
        let line = raw.replace('\t', " ");

        // Header row: its first token is exactly `FT`, followed by the altitude columns.
        // (Matching a bare `contains("FT")` would misread stations like `FTW`.)
        if line.split_whitespace().next() == Some("FT") {
            let cols = level_columns(&line);
            if !cols.is_empty() {
                levels = Some(cols);
                continue;
            }
        }
        let Some(cols) = &levels else { continue };

        // Station row: three alphanumerics followed by whitespace.
        let Some((id, scan_from)) = station_id(&line) else {
            continue;
        };
        let mut lev: Vec<(i32, WindLevel)> = Vec::new();
        for (center, tok) in tokens_with_centers(&line, scan_from) {
            let Some(w) = decode_fb(tok) else { continue };
            // Assign the token to the nearest altitude column.
            if let Some((alt, _)) = cols
                .iter()
                .min_by(|(_, ca), (_, cb)| (ca - center).abs().total_cmp(&(cb - center).abs()))
            {
                lev.push((*alt, w));
            }
        }
        if !lev.is_empty() {
            out.push((id, lev));
        }
    }
    out
}

/// Decode an FB group like `2427+21`, `9900`, or `274131` → wind. `9900` = light/variable.
fn decode_fb(g: &str) -> Option<WindLevel> {
    let b = g.as_bytes();
    if b.len() < 4 || !b[..4].iter().all(u8::is_ascii_digit) {
        return None;
    }
    if &g[..4] == "9900" {
        return Some(WindLevel {
            dir: None,
            spd: 0.0,
        });
    }
    let mut dd: i32 = g[0..2].parse().ok()?;
    let mut ss: i32 = g[2..4].parse().ok()?;
    if dd > 36 {
        // Codes 51–86 encode ≥100 kt (subtract 50, add 100 kt); 37–50 are invalid.
        if dd < 51 {
            return None;
        }
        dd -= 50;
        ss += 100;
    }
    Some(WindLevel {
        dir: Some(((dd * 10) % 360) as f64),
        spd: ss as f64,
    })
}

/// Altitude columns `(alt_ft, center_char)` from a header row's 4–5 digit runs.
fn level_columns(line: &str) -> Vec<(i32, f64)> {
    let b = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() {
            let start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            let len = i - start;
            if (4..=5).contains(&len)
                && let Ok(alt) = line[start..i].parse::<i32>()
            {
                out.push((alt, start as f64 + len as f64 / 2.0));
            }
        } else {
            i += 1;
        }
    }
    out
}

/// A station id is exactly three alphanumerics at the line start followed by whitespace;
/// returns `(id, byte_offset_after_id)`.
fn station_id(line: &str) -> Option<(String, usize)> {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    if i + 3 > b.len() || !line.is_char_boundary(i + 3) {
        return None;
    }
    let id = &line[i..i + 3];
    if !id.bytes().all(|c| c.is_ascii_alphanumeric())
        || !id.bytes().any(|c| c.is_ascii_alphabetic())
    {
        return None;
    }
    if i + 3 < b.len() && !b[i + 3].is_ascii_whitespace() {
        return None;
    }
    Some((id.to_string(), i + 3))
}

/// Whitespace-separated tokens with their center char position, from `from`.
fn tokens_with_centers(s: &str, from: usize) -> Vec<(f64, &str)> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = from;
    while i < b.len() {
        if b[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() {
            i += 1;
        }
        let tok = &s[start..i];
        out.push((start as f64 + tok.len() as f64 / 2.0, tok));
    }
    out
}

fn gc_dist(la1: f64, lo1: f64, la2: f64, lo2: f64) -> f64 {
    let (p1, p2) = (la1.to_radians(), la2.to_radians());
    let dla = (la2 - la1).to_radians();
    let dlo = (lo2 - lo1).to_radians();
    let a = (dla / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dlo / 2.0).sin().powi(2);
    2.0 * R_NM * a.sqrt().asin()
}

fn bearing_deg(la1: f64, lo1: f64, la2: f64, lo2: f64) -> f64 {
    let dlo = (lo2 - lo1).to_radians();
    let y = dlo.sin() * la2.to_radians().cos();
    let x = la1.to_radians().cos() * la2.to_radians().sin()
        - la1.to_radians().sin() * la2.to_radians().cos() * dlo.cos();
    (y.atan2(x).to_degrees() + 360.0) % 360.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
DATA BASED ON 131800Z
VALID 140000Z   FOR USE 2000-0300Z. TEMPS NEG ABV 24000

FT  3000    6000    9000   12000   18000   24000  30000  34000  39000
BRL 1817 2427+21 2727+14 2928+09 2933-05 2736-15 274131 264142 243352
FWA 9900 2605+16 2708+12 2818+07 3025-05 2740-15 265031 265541 256053
";

    #[test]
    fn decodes_fb_groups() {
        // 2427 → from 240°, 27 kt.
        let w = decode_fb("2427+21").unwrap();
        assert_eq!(w.dir, Some(240.0));
        assert_eq!(w.spd, 27.0);
        // 9900 → calm / light-variable.
        let calm = decode_fb("9900").unwrap();
        assert_eq!(calm.dir, None);
        assert_eq!(calm.spd, 0.0);
        // dd>36 encodes speed ≥ 100 kt: 7420 → dir 240°, 120 kt.
        let fast = decode_fb("7420").unwrap();
        assert_eq!(fast.dir, Some(240.0));
        assert_eq!(fast.spd, 120.0);
        assert!(decode_fb("+21").is_none());
    }

    #[test]
    fn parses_a_windtemp_table() {
        let stations = parse_windtemp(SAMPLE);
        let brl = stations
            .iter()
            .find(|(id, _)| id == "BRL")
            .expect("BRL parsed");
        // Nine levels, matching the nine header columns.
        assert_eq!(brl.1.len(), 9);
        // 3000 ft column: 1817 → 180°, 17 kt.
        let (_, low) = brl.1.iter().find(|(a, _)| *a == 3000).unwrap();
        assert_eq!(low.dir, Some(180.0));
        assert_eq!(low.spd, 17.0);
        // 39000 ft column: 2433 → 240°, 33+100 = ... actually 24 ≤ 36 so 240°, 33 kt.
        let (_, high) = brl.1.iter().find(|(a, _)| *a == 39000).unwrap();
        assert_eq!(high.dir, Some(240.0));
        // FWA at 3000 is calm (9900).
        let fwa = stations.iter().find(|(id, _)| id == "FWA").unwrap();
        let (_, fwa_low) = fwa.1.iter().find(|(a, _)| *a == 3000).unwrap();
        assert_eq!(fwa_low.spd, 0.0);
    }

    #[test]
    fn station_id_containing_ft_is_not_a_header() {
        // FTW (Fort Worth) contains "FT" — it must parse as a station, not a header, and
        // must not corrupt the altitude columns for stations that follow it.
        let sample = "\
FT  3000    6000    9000
FTW 1817 2427+21 2727+14
BRL 2013 2110+18 2308+12
";
        let stations = parse_windtemp(sample);
        let ftw = stations
            .iter()
            .find(|(id, _)| id == "FTW")
            .expect("FTW parses as a station");
        assert_eq!(
            ftw.1.iter().find(|(a, _)| *a == 3000).map(|(_, w)| w.dir),
            Some(Some(180.0)),
            "FTW 3000ft = 180°/17kt"
        );
        assert!(
            stations.iter().any(|(id, _)| id == "BRL"),
            "the station after FTW must still parse"
        );
    }

    #[test]
    fn headwind_projection() {
        // One station at (40,-90) with a due-west 240°/50kt-ish wind; flying east (090°)
        // into it should read as a headwind component.
        let winds = Winds {
            stations: vec![Station {
                lat: 40.0,
                lon: -90.0,
                levels: vec![(
                    35000,
                    WindLevel {
                        dir: Some(90.0),
                        spd: 50.0,
                    },
                )],
            }],
        };
        // Flying east (course ~090) directly into a 090° wind → +50 kt headwind.
        let hw = winds
            .route_headwind(&[[40.0, -90.5], [40.0, -89.5]], 35000.0)
            .unwrap();
        assert!(
            (hw - 50.0).abs() < 1.0,
            "expected ~+50kt headwind, got {hw}"
        );
        // Flying west → tailwind (negative).
        let tw = winds
            .route_headwind(&[[40.0, -89.5], [40.0, -90.5]], 35000.0)
            .unwrap();
        assert!(tw < -40.0, "expected strong tailwind, got {tw}");
    }

    // ---- interpolation + high-level merge (#314) ----

    fn wind(dir: f64, spd: f64) -> WindLevel {
        WindLevel {
            dir: Some(dir),
            spd,
        }
    }

    fn station(lat: f64, lon: f64, levels: Vec<(i32, WindLevel)>) -> Station {
        Station { lat, lon, levels }
    }

    /// Headwind at a point for a course of 360° (so a northerly wind reads as its full speed).
    fn north_hw(w: &Winds, lat: f64, lon: f64, alt: f64) -> f64 {
        w.point_headwind(lat, lon, 0.0, alt).unwrap()
    }

    #[test]
    fn wind_between_two_levels_is_interpolated_and_clamped_outside() {
        // 360° winds (straight headwinds on a 360° course): 20 kt at 34,000, 70 kt at 39,000.
        let w = Winds {
            stations: vec![station(
                40.0,
                -90.0,
                vec![(39000, wind(360.0, 70.0)), (34000, wind(360.0, 20.0))],
            )],
        };
        assert!((north_hw(&w, 40.0, -90.0, 37000.0) - 50.0).abs() < 1e-9);
        assert!((north_hw(&w, 40.0, -90.0, 45000.0) - 70.0).abs() < 1e-9);
        assert!((north_hw(&w, 40.0, -90.0, 10000.0) - 20.0).abs() < 1e-9);
    }

    #[test]
    fn wind_between_stations_is_blended_by_inverse_distance() {
        // Two stations 120 nm apart (2° of latitude), 10 kt and 50 kt northerlies.
        let w = Winds {
            stations: vec![
                station(40.0, -90.0, vec![(35000, wind(360.0, 10.0))]),
                station(42.0, -90.0, vec![(35000, wind(360.0, 50.0))]),
            ],
        };
        // Midway: equal weights.
        assert!((north_hw(&w, 41.0, -90.0, 35000.0) - 30.0).abs() < 1e-6);
        // A quarter of the way: 1/d² weights 9:1 toward the near station → 14 kt.
        assert!((north_hw(&w, 40.5, -90.0, 35000.0) - 14.0).abs() < 0.05);
        // On a station: exactly its wind.
        assert!((north_hw(&w, 42.0, -90.0, 35000.0) - 50.0).abs() < 1e-9);
        // Beyond the station radius: no wind (still air).
        assert!(w.point_headwind(60.0, -90.0, 0.0, 35000.0).is_none());
    }

    #[test]
    fn only_the_nearest_stations_are_blended() {
        // Four near 10 kt stations around the point and a fifth, further, 90 kt one: the fifth is
        // outside the blend, so the result is exactly 10 kt.
        let near = |lat, lon| station(lat, lon, vec![(35000, wind(360.0, 10.0))]);
        let w = Winds {
            stations: vec![
                near(40.5, -90.0),
                near(39.5, -90.0),
                near(40.0, -89.5),
                near(40.0, -90.5),
                station(42.0, -90.0, vec![(35000, wind(360.0, 90.0))]),
            ],
        };
        assert!((north_hw(&w, 40.0, -90.0, 35000.0) - 10.0).abs() < 1e-9);
    }

    #[test]
    fn direction_interpolates_through_a_shear_as_a_vector() {
        // 270°/50 kt at 30,000 and 360°/50 kt at 40,000: halfway is the vector mean — from 315° at
        // ~35.4 kt — not a 50 kt wind, and not a naive 315°/50.
        let w = Winds {
            stations: vec![station(
                40.0,
                -90.0,
                vec![(30000, wind(270.0, 50.0)), (40000, wind(360.0, 50.0))],
            )],
        };
        let along = w.point_headwind(40.0, -90.0, 315.0, 35000.0).unwrap();
        let across = w.point_headwind(40.0, -90.0, 45.0, 35000.0).unwrap();
        assert!((along - 50.0 / 2f64.sqrt()).abs() < 1e-9, "got {along}");
        assert!(across.abs() < 1e-9, "got {across}");
    }

    #[test]
    fn calm_levels_count_as_zero_wind() {
        let w = Winds {
            stations: vec![station(
                40.0,
                -90.0,
                vec![(
                    35000,
                    WindLevel {
                        dir: None,
                        spd: 0.0,
                    },
                )],
            )],
        };
        let hw = w.route_headwind(&[[40.0, -90.5], [40.0, -89.5]], 35000.0);
        assert_eq!(hw, Some(0.0));
    }

    #[test]
    fn high_table_extends_a_stations_low_levels() {
        let high = "\
FT   45000  53000
BRL 264663 263369
";
        let mut merged = HashMap::new();
        merge_table(&mut merged, SAMPLE);
        merge_table(&mut merged, high);
        let brl = &merged["BRL"];
        assert_eq!(brl.len(), 11, "nine low levels + two high levels");
        let at = |alt| {
            brl.iter()
                .find(|(a, _)| *a == alt)
                .map(|(_, w)| (w.dir, w.spd))
        };
        assert_eq!(at(3000), Some((Some(180.0), 17.0)), "low levels kept");
        assert_eq!(at(45000), Some((Some(260.0), 46.0)));
        assert_eq!(at(53000), Some((Some(260.0), 33.0)));
    }
}
