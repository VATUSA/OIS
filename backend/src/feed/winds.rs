//! US winds aloft for enroute ETA correction, from NOAA's Rapid Refresh (RAP) model decoded by
//! [`crate::feed::rap`] onto a regular lat/lon grid (#836). An empty field is still air. Fields
//! snapshotted before RAP hold AWC FB station winds; those still load and blend for historical
//! replay. Fetched at runtime and hot-swapped; fails safe to still air whenever data is missing.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::trajectory::{RouteWind, WindKnot};

const R_NM: f64 = 3440.065;
/// A station further than this from the query point contributes no wind (nm).
const MAX_STATION_NM: f64 = 600.0;
/// The wind at a point is an inverse-distance-squared blend of at most this many nearest stations.
const BLEND_STATIONS: usize = 4;
/// A query point this close to a station (nm) takes that station's wind outright.
const ON_STATION_NM: f64 = 0.1;
/// Longest gap (nm) between wind samples along a route: close to one 1° grid cell (60 nm north-south,
/// 46 nm east-west at 40°N), so the samples follow the field at about its own resolution.
const KNOT_SPACING_NM: f64 = 50.0;
/// Celsius to kelvin.
const C_TO_K: f64 = 273.15;
/// A grid value that is not defined (outside the source model's footprint).
pub const GRID_MISSING: i16 = i16::MIN;

/// Wind at one FB forecast level, as snapshots taken before RAP store it: direction (deg true,
/// None when calm) and speed (kt).
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

/// One vertical column of the field at a point: `(altitude_ft, (u, v) "from" kt, temp_k)` in
/// increasing altitude.
type Column = Vec<(f64, (f64, f64), Option<f64>)>;

/// A model wind field on a regular lat/lon grid, by pressure altitude. Values are stored as whole
/// knots and whole °C to keep the `stats.winds` snapshot small; [`GRID_MISSING`] marks a point
/// outside the source model's footprint.
#[derive(Clone, Serialize, Deserialize)]
pub struct WindGrid {
    /// Latitude and longitude (deg) of grid index `[0][0]`, and the spacing (deg) of both axes.
    pub lat0: f64,
    pub lon0: f64,
    pub step_deg: f64,
    pub nlat: usize,
    pub nlon: usize,
    /// Pressure altitude (ft) of each level, in increasing altitude.
    pub levels_ft: Vec<f64>,
    /// The "from" wind vector `(east, north)` (kt), as [`WindLevel`] uses, and the temperature
    /// (°C), each laid out `[level][lat][lon]`.
    pub u: Vec<i16>,
    pub v: Vec<i16>,
    pub t: Vec<i16>,
}

impl WindGrid {
    fn len_ok(&self) -> bool {
        let n = self.levels_ft.len() * self.nlat * self.nlon;
        n > 0 && self.u.len() == n && self.v.len() == n && self.t.len() == n
    }

    /// The column at a point, bilinear between the four surrounding grid points that are defined.
    /// None outside the grid or where no surrounding point is defined.
    fn column(&self, lat: f64, lon: f64) -> Option<Column> {
        if !self.len_ok() {
            return None;
        }
        let fi = (lat - self.lat0) / self.step_deg;
        let fj = (lon - self.lon0) / self.step_deg;
        let (imax, jmax) = ((self.nlat - 1) as f64, (self.nlon - 1) as f64);
        if !(0.0..=imax).contains(&fi) || !(0.0..=jmax).contains(&fj) {
            return None;
        }
        let (i0, j0) = (fi.floor() as usize, fj.floor() as usize);
        let (i1, j1) = ((i0 + 1).min(self.nlat - 1), (j0 + 1).min(self.nlon - 1));
        let (ti, tj) = (fi - i0 as f64, fj - j0 as f64);
        let corners = [
            (i0, j0, (1.0 - ti) * (1.0 - tj)),
            (i0, j1, (1.0 - ti) * tj),
            (i1, j0, ti * (1.0 - tj)),
            (i1, j1, ti * tj),
        ];
        let plane = self.nlat * self.nlon;
        let mut col = Column::new();
        for (k, &alt) in self.levels_ft.iter().enumerate() {
            let (mut u, mut v, mut wsum, mut t, mut tsum) = (0.0, 0.0, 0.0, 0.0, 0.0);
            for &(i, j, w) in &corners {
                let at = k * plane + i * self.nlon + j;
                if self.u[at] != GRID_MISSING && self.v[at] != GRID_MISSING {
                    u += w * self.u[at] as f64;
                    v += w * self.v[at] as f64;
                    wsum += w;
                }
                if self.t[at] != GRID_MISSING {
                    t += w * self.t[at] as f64;
                    tsum += w;
                }
            }
            if wsum > 1e-9 {
                let temp_k = (tsum > 1e-9).then(|| t / tsum + C_TO_K);
                col.push((alt, (u / wsum, v / wsum), temp_k));
            }
        }
        (!col.is_empty()).then_some(col)
    }
}

/// Where the live wind field came from, for the data-status view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum WindsSource {
    /// NOAA Rapid Refresh.
    Rap,
    /// No field: every ETA flies still air.
    None,
}

/// The winds-aloft picture. Serializable so the stats collector can snapshot it for historical
/// replay (`stats.winds`) and the reconstruction can load a past snapshot back. The fields added
/// since are `Option`s, which serde reads as `None` when absent, so an older snapshot still loads.
#[derive(Default, Serialize, Deserialize)]
pub struct Winds {
    stations: Vec<Station>,
    #[serde(skip_serializing_if = "Option::is_none")]
    grid: Option<WindGrid>,
    /// The time the field is valid for (RAP); None for FB, whose tables carry a forecast window.
    #[serde(skip_serializing_if = "Option::is_none")]
    valid_at: Option<DateTime<Utc>>,
}

impl Winds {
    /// A RAP field.
    pub fn from_grid(grid: WindGrid, valid_at: DateTime<Utc>) -> Self {
        Self {
            stations: Vec::new(),
            grid: Some(grid),
            valid_at: Some(valid_at),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.stations.is_empty() && self.grid.is_none()
    }

    pub fn valid_at(&self) -> Option<DateTime<Utc>> {
        self.valid_at
    }

    /// The source of a live field. Only replayed snapshots from before RAP hold stations, and
    /// data-status never reports one of those.
    pub fn source(&self) -> WindsSource {
        if self.grid.is_some() {
            WindsSource::Rap
        } else {
            WindsSource::None
        }
    }

    /// The vertical column at a point. None when the field does not cover it (still air there).
    fn column(&self, lat: f64, lon: f64) -> Option<Column> {
        match &self.grid {
            Some(grid) => grid.column(lat, lon),
            None => self.station_column(lat, lon),
        }
    }

    /// A pre-RAP snapshot's column: each of the [`BLEND_STATIONS`] nearest FB stations within [`MAX_STATION_NM`],
    /// blended by inverse distance squared at every level any of them reports.
    fn station_column(&self, lat: f64, lon: f64) -> Option<Column> {
        let mut near: Vec<(f64, &Station)> = self
            .stations
            .iter()
            .map(|s| (gc_dist(lat, lon, s.lat, s.lon), s))
            .filter(|(d, s)| *d <= MAX_STATION_NM && !s.levels.is_empty())
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        near.truncate(BLEND_STATIONS);
        if near.first()?.0 <= ON_STATION_NM {
            near.truncate(1);
        }
        let mut alts: Vec<i32> = near
            .iter()
            .flat_map(|(_, s)| s.levels.iter().map(|(a, _)| *a))
            .collect();
        alts.sort_unstable();
        alts.dedup();
        let single = near.len() == 1;
        let col: Column = alts
            .into_iter()
            .map(|a| {
                let alt = a as f64;
                let (mut u, mut v, mut wsum) = (0.0, 0.0, 0.0);
                for &(d, s) in &near {
                    let w = if single { 1.0 } else { 1.0 / (d * d) };
                    if let Some((su, sv)) = s.uv_at(alt) {
                        u += w * su;
                        v += w * sv;
                        wsum += w;
                    }
                }
                (alt, (u / wsum, v / wsum), None)
            })
            .collect();
        (!col.is_empty()).then_some(col)
    }

    /// Headwind (kt) at a single point along `course_deg`, interpolated to `alt_ft`.
    #[cfg(test)]
    pub fn point_headwind(&self, lat: f64, lon: f64, course_deg: f64, alt_ft: f64) -> Option<f64> {
        let col = self.column(lat, lon)?;
        let levels = col
            .iter()
            .map(|&(a, uv, t)| (a, headwind_along(uv, course_deg), t))
            .collect();
        RouteWind::new(vec![WindKnot { d_nm: 0.0, levels }]).headwind(0.0, alt_ft)
    }

    /// The wind along a `[lat, lon]` route path, for the trajectory model to read at each point's
    /// own distance and altitude (#836). `route_len_nm` is the distance-to-destination the profile
    /// starts at; the path's own length is scaled onto it, so a padded ground route
    /// (`GROUND_ROUTE_FACTOR`) still spans the whole profile. Each sample carries the headwind along
    /// the route's course at that point; samples are at most [`KNOT_SPACING_NM`] apart, on the great
    /// circle of each leg. A point the field does not cover is a gap: the wind of the last covered
    /// point holds up to it, at most one sample spacing, and is still air beyond. A route the field
    /// does not cover at all is still air.
    pub fn along(&self, path: &[[f64; 2]], route_len_nm: f64) -> RouteWind {
        if self.is_empty() || path.len() < 2 {
            return RouteWind::still();
        }
        let legs: Vec<f64> = path
            .windows(2)
            .map(|w| gc_dist(w[0][0], w[0][1], w[1][0], w[1][1]))
            .collect();
        let total: f64 = legs.iter().sum();
        if total <= 0.0 {
            return RouteWind::still();
        }
        let mut knots = Vec::new();
        let mut knot = |point: [f64; 2], flown: f64, course: f64| {
            let levels = self
                .column(point[0], point[1])
                .map_or_else(Vec::new, |col| {
                    col.iter()
                        .map(|&(a, uv, t)| (a, headwind_along(uv, course), t))
                        .collect()
                });
            knots.push(WindKnot {
                d_nm: route_len_nm * (1.0 - flown / total),
                levels,
            });
        };
        let (mut flown, mut last_course) = (0.0, 0.0);
        for (seg, &len) in path.windows(2).zip(&legs) {
            let (a, b) = (seg[0], seg[1]);
            let n = (len / KNOT_SPACING_NM).ceil().max(1.0) as usize;
            for k in 0..n {
                let f = k as f64 / n as f64;
                let point = great_circle_point(a, b, f);
                knot(
                    point,
                    flown + f * len,
                    bearing_deg(point[0], point[1], b[0], b[1]),
                );
            }
            flown += len;
            // The course arriving at `b`: the reverse of the bearing from `b` back to `a`.
            last_course = (bearing_deg(b[0], b[1], a[0], a[1]) + 180.0) % 360.0;
        }
        knot(path[path.len() - 1], total, last_course);
        RouteWind::new(knots)
    }
}

fn gc_dist(la1: f64, lo1: f64, la2: f64, lo2: f64) -> f64 {
    let (p1, p2) = (la1.to_radians(), la2.to_radians());
    let dla = (la2 - la1).to_radians();
    let dlo = (lo2 - lo1).to_radians();
    let a = (dla / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dlo / 2.0).sin().powi(2);
    2.0 * R_NM * a.sqrt().asin()
}

/// The point a fraction `f` of the way from `a` to `b` along the great circle between them.
fn great_circle_point(a: [f64; 2], b: [f64; 2], f: f64) -> [f64; 2] {
    let (p1, l1, p2, l2) = (
        a[0].to_radians(),
        a[1].to_radians(),
        b[0].to_radians(),
        b[1].to_radians(),
    );
    let delta = gc_dist(a[0], a[1], b[0], b[1]) / R_NM;
    if delta < 1e-12 {
        return a;
    }
    let (ka, kb) = (
        ((1.0 - f) * delta).sin() / delta.sin(),
        (f * delta).sin() / delta.sin(),
    );
    let x = ka * p1.cos() * l1.cos() + kb * p2.cos() * l2.cos();
    let y = ka * p1.cos() * l1.sin() + kb * p2.cos() * l2.sin();
    let z = ka * p1.sin() + kb * p2.sin();
    [
        z.atan2((x * x + y * y).sqrt()).to_degrees(),
        y.atan2(x).to_degrees(),
    ]
}

fn bearing_deg(la1: f64, lo1: f64, la2: f64, lo2: f64) -> f64 {
    let dlo = (lo2 - lo1).to_radians();
    let y = dlo.sin() * la2.to_radians().cos();
    let x = la1.to_radians().cos() * la2.to_radians().sin()
        - la1.to_radians().sin() * la2.to_radians().cos() * dlo.cos();
    (y.atan2(x).to_degrees() + 360.0) % 360.0
}

/// Test-only constructor for a RAP-shaped field, for tests outside this module (the fields are
/// private). A 1° grid with index `[0][0]` at `(lat0, lon0)`, `nlat × nlon` points, at `levels_ft`
/// (increasing); each point's `(u, v, t)` (the "from" wind in kt and °C, or [`GRID_MISSING`]) is
/// `at(alt_ft, lat, lon)`.
#[cfg(test)]
impl Winds {
    pub(crate) fn test_grid(
        (lat0, lon0): (f64, f64),
        (nlat, nlon): (usize, usize),
        levels_ft: &[f64],
        valid_at: DateTime<Utc>,
        at: impl Fn(f64, f64, f64) -> (i16, i16, i16),
    ) -> Self {
        let mut grid = WindGrid {
            lat0,
            lon0,
            step_deg: 1.0,
            nlat,
            nlon,
            levels_ft: levels_ft.to_vec(),
            u: Vec::new(),
            v: Vec::new(),
            t: Vec::new(),
        };
        for &alt in levels_ft {
            for i in 0..nlat {
                for j in 0..nlon {
                    let (u, v, t) = at(alt, lat0 + i as f64, lon0 + j as f64);
                    grid.u.push(u);
                    grid.v.push(v);
                    grid.t.push(t);
                }
            }
        }
        Self::from_grid(grid, valid_at)
    }

    /// The field every #836 caller test seeds: over 37–42°N, 79–72°W, 80 kt from 225° (a headwind
    /// on the south-west KJFK→KDCA course) at and above 30,000 ft east of 75.5°W, and calm
    /// elsewhere. It shears with altitude and changes along the route, so both still air and any
    /// single route-mean value time a flight through it differently.
    pub(crate) fn test_shear() -> Self {
        Self::test_grid(
            (37.0, -79.0),
            (6, 8),
            &[5_000.0, 18_000.0, 30_000.0, 39_000.0],
            DateTime::from_timestamp(1_792_112_400, 0).expect("2026-10-16T01:00Z"),
            |alt, _, lon| {
                if alt >= 30_000.0 && lon > -75.5 {
                    (-57, -57, -45)
                } else {
                    (0, 0, -45)
                }
            },
        )
    }

    /// An FB field of one station with one level.
    pub(crate) fn test_station(lat: f64, lon: f64, alt_ft: i32, dir: f64, spd: f64) -> Self {
        Self {
            stations: vec![Station {
                lat,
                lon,
                levels: vec![(
                    alt_ft,
                    WindLevel {
                        dir: Some(dir),
                        spd,
                    },
                )],
            }],
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            ..Winds::default()
        };
        // Flying east (course ~090) directly into a 090° wind → +50 kt headwind.
        let hw = winds
            .along(&[[40.0, -90.5], [40.0, -89.5]], 60.0)
            .headwind(30.0, 35000.0)
            .unwrap();
        assert!(
            (hw - 50.0).abs() < 1.0,
            "expected ~+50kt headwind, got {hw}"
        );
        // Flying west → tailwind (negative).
        let tw = winds
            .along(&[[40.0, -89.5], [40.0, -90.5]], 60.0)
            .headwind(30.0, 35000.0)
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
            ..Winds::default()
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
            ..Winds::default()
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
            ..Winds::default()
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
            ..Winds::default()
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
            ..Winds::default()
        };
        let hw = w
            .along(&[[40.0, -90.5], [40.0, -89.5]], 60.0)
            .headwind(30.0, 35000.0);
        assert_eq!(hw, Some(0.0));
    }

    // ---- #836: snapshot compatibility, the grid column and `along` ----

    fn at_time(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn fb(stations: Vec<Station>) -> Winds {
        Winds {
            stations,
            ..Winds::default()
        }
    }

    /// The 2 x 2 grid at (40..41, -90..-89), one level at 34,000 ft, u 10/20/30/50 kt, v 0, -40 °C.
    fn square(missing_ne: bool) -> Winds {
        Winds::test_grid(
            (40.0, -90.0),
            (2, 2),
            &[34_000.0],
            at_time("2026-10-16T01:00:00Z"),
            |_, lat, lon| {
                let u = match (lat as i32, lon as i32) {
                    (40, -90) => 10,
                    (40, -89) => 20,
                    (41, -90) => 30,
                    _ if missing_ne => GRID_MISSING,
                    _ => 50,
                };
                (u, if u == GRID_MISSING { GRID_MISSING } else { 0 }, -40)
            },
        )
    }

    fn east_u(w: &Winds, lat: f64, lon: f64) -> Option<f64> {
        w.column(lat, lon)
            .map(|c| c.iter().find(|l| l.0 == 34_000.0).unwrap().1.0)
    }

    // AC4: a `stats.winds` row written before #836 (FB stations, no grid, no valid_at) still loads
    // and reads the same headwind.
    #[test]
    fn a_pre_change_fb_snapshot_decodes_and_gives_the_same_headwind() {
        let json = r#"{"stations":[{"lat":40.0,"lon":-90.0,"levels":[[3000,{"dir":180.0,"spd":17.0}],[35000,{"dir":90.0,"spd":50.0}]]}]}"#;
        let w: Winds = serde_json::from_str(json).unwrap();
        assert_eq!(w.valid_at(), None);
        assert_eq!(w.stations.len(), 1);
        let east = w.point_headwind(40.0, -90.0, 90.0, 35_000.0).unwrap();
        assert!((east - 50.0).abs() < 1e-9, "{east}");
        let north = w.point_headwind(40.0, -90.0, 0.0, 3_000.0).unwrap();
        assert!((north + 17.0).abs() < 1e-9, "{north}");
    }

    // AC4 edge: an old snapshot's calm level (`dir: null`).
    #[test]
    fn a_pre_change_snapshot_with_a_calm_level_decodes_as_zero_wind() {
        let json =
            r#"{"stations":[{"lat":40.0,"lon":-90.0,"levels":[[35000,{"dir":null,"spd":0.0}]]}]}"#;
        let w: Winds = serde_json::from_str(json).unwrap();
        assert_eq!(w.point_headwind(40.0, -90.0, 90.0, 35_000.0), Some(0.0));
    }

    // A RAP field written to `stats.winds` reads back identical (replay of post-#836 snapshots).
    #[test]
    fn a_rap_field_round_trips_through_its_snapshot_json() {
        let valid = at_time("2026-10-16T01:00:00Z");
        let w = Winds::test_grid(
            (38.0, -78.0),
            (3, 4),
            &[18_000.0, 34_000.0],
            valid,
            |alt, lat, lon| {
                let t = if lat == 40.0 && lon == -75.0 {
                    GRID_MISSING
                } else {
                    -30 - (alt / 1000.0) as i16
                };
                ((lon + 80.0) as i16 * 10, (lat - 37.0) as i16 * 5, t)
            },
        );
        let json = serde_json::to_string(&w).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v.get("grid").is_some() && v.get("valid_at").is_some());
        let back: Winds = serde_json::from_str(&json).unwrap();
        assert_eq!(back.source(), WindsSource::Rap);
        assert_eq!(back.valid_at(), Some(valid));
        assert_eq!(
            back.point_headwind(39.5, -76.5, 45.0, 34_000.0),
            w.point_headwind(39.5, -76.5, 45.0, 34_000.0)
        );
        assert_eq!(back.column(39.5, -76.5), w.column(39.5, -76.5));
        // The missing temperature stays missing: its grid point alone reads no temperature.
        assert_eq!(back.column(40.0, -75.0).unwrap()[1].2, None);
    }

    // data-status's `winds_source` comes from here.
    #[test]
    fn the_source_is_rap_for_a_grid_and_none_without_one() {
        assert_eq!(Winds::default().source(), WindsSource::None);
        assert_eq!(square(false).source(), WindsSource::Rap);
    }

    // Grid column: bilinear between the four surrounding points.
    #[test]
    fn the_grid_column_is_bilinear_between_the_four_surrounding_points() {
        let w = square(false);
        assert!((east_u(&w, 40.25, -89.5).unwrap() - 21.25).abs() < 1e-9);
        assert_eq!(east_u(&w, 40.0, -90.0), Some(10.0));
        let col = w.column(40.25, -89.5).unwrap();
        assert!((col[0].2.unwrap() - 233.15).abs() < 1e-9);
    }

    // Grid column: a GRID_MISSING corner is left out and the others reweighted.
    #[test]
    fn a_missing_grid_corner_is_left_out_and_the_rest_reweighted() {
        let w = square(true);
        assert!((east_u(&w, 40.5, -89.5).unwrap() - 20.0).abs() < 1e-9);
        // Only the missing corner surrounds this point: no wind there.
        assert_eq!(w.column(41.0, -89.0), None);
    }

    // Grid column: a corner missing only its v is left out too, not averaged in with u alone.
    #[test]
    fn a_grid_corner_missing_only_its_v_is_left_out() {
        let w = Winds::test_grid(
            (40.0, -90.0),
            (2, 2),
            &[34_000.0],
            at_time("2026-10-16T01:00:00Z"),
            |_, lat, lon| match (lat as i32, lon as i32) {
                (40, -90) => (10, 0, -40),
                (40, -89) => (20, 0, -40),
                (41, -90) => (30, 0, -40),
                _ => (50, GRID_MISSING, -40),
            },
        );
        assert!((east_u(&w, 40.5, -89.5).unwrap() - 20.0).abs() < 1e-9);
    }

    // A replayed station with no levels is not blended, even when it sits on the point asked about.
    #[test]
    fn a_station_with_no_levels_is_skipped() {
        let w = Winds {
            stations: vec![
                station(40.0, -90.0, vec![]),
                station(41.0, -90.0, vec![(35000, wind(360.0, 10.0))]),
            ],
            ..Winds::default()
        };
        assert!((north_hw(&w, 40.0, -90.0, 35000.0) - 10.0).abs() < 1e-9);
    }

    // Grid column: outside the grid there is no wind (still air), not a clamped edge value.
    #[test]
    fn a_point_outside_the_grid_has_no_column() {
        let w = square(false);
        for (lat, lon) in [
            (39.99, -89.5),
            (41.01, -89.5),
            (40.5, -90.01),
            (40.5, -88.99),
        ] {
            assert_eq!(w.column(lat, lon), None, "({lat},{lon})");
        }
        assert!(w.column(41.0, -89.0).is_some());
    }

    // Grid column: wind present, temperature missing -> temp None, so the profile uses ISA.
    #[test]
    fn a_grid_point_with_wind_but_no_temperature_reads_isa_downstream() {
        let w = Winds::test_grid(
            (40.0, -90.0),
            (2, 2),
            &[34_000.0],
            at_time("2026-10-16T01:00:00Z"),
            |_, _, _| (30, 0, GRID_MISSING),
        );
        let rw = w.along(&[[40.2, -89.8], [40.2, -89.2]], 27.0);
        assert_eq!(rw.temp_k(10.0, 34_000.0), None);
        assert!(rw.headwind(10.0, 34_000.0).is_some());
    }

    // Grid column: arrays that do not match the declared shape are refused (a corrupt snapshot).
    #[test]
    fn a_grid_whose_arrays_do_not_match_its_shape_has_no_column() {
        let grid = WindGrid {
            lat0: 40.0,
            lon0: -90.0,
            step_deg: 1.0,
            nlat: 2,
            nlon: 2,
            levels_ft: vec![34_000.0],
            u: vec![10, 20, 30],
            v: vec![0, 0, 0, 0],
            t: vec![-40, -40, -40, -40],
        };
        assert_eq!(grid.column(40.5, -89.5), None);
        let w = Winds::from_grid(grid, at_time("2026-10-16T01:00:00Z"));
        assert!(w.along(&[[40.2, -89.8], [40.2, -89.2]], 27.0).is_still());
    }

    // along: a sample at most every 50 nm.
    #[test]
    fn along_samples_a_long_leg_at_most_every_fifty_nm() {
        let w = fb(vec![station(40.0, -88.0, vec![(34000, wind(270.0, 40.0))])]);
        let path = [[40.0, -90.0], [40.0, -85.5]];
        let len = gc_dist(40.0, -90.0, 40.0, -85.5);
        let ds = w.along(&path, len).knot_distances();
        assert_eq!(ds.len(), (len / 50.0).ceil() as usize + 1, "{ds:?}");
        assert!(ds.windows(2).all(|p| p[1] - p[0] <= 50.0 + 1e-9), "{ds:?}");
        assert!(ds[0].abs() < 1e-9 && (ds[ds.len() - 1] - len).abs() < 1e-9);
    }

    // along: each leg's headwind is along that leg's own course.
    #[test]
    fn along_reads_each_legs_headwind_along_that_legs_course() {
        let w = fb(vec![station(41.0, -89.0, vec![(34000, wind(360.0, 40.0))])]);
        let path = [[40.0, -90.0], [40.0, -88.0], [42.0, -88.0]];
        let (east, north) = (
            gc_dist(40.0, -90.0, 40.0, -88.0),
            gc_dist(40.0, -88.0, 42.0, -88.0),
        );
        let rw = w.along(&path, east + north);
        let on_east = rw.headwind(north + east * 0.75, 34_000.0).unwrap();
        let on_north = rw.headwind(north * 0.5, 34_000.0).unwrap();
        assert!(
            on_east.abs() < 0.5,
            "crosswind on the east leg reads {on_east}"
        );
        assert!((on_north - 40.0).abs() < 0.5, "north leg reads {on_north}");
    }

    // along: the path's length is scaled onto the distance the profile starts at.
    #[test]
    fn along_scales_the_paths_length_onto_the_profiles_distance() {
        // 60 kt from the east at and east of -87°, calm west of -88°.
        let w = Winds::test_grid(
            (39.0, -91.0),
            (3, 7),
            &[34_000.0],
            at_time("2026-10-16T01:00:00Z"),
            |_, _, lon| (if lon >= -87.0 { 60 } else { 0 }, 0, -40),
        );
        let path = [[40.0, -90.0], [40.0, -86.0]];
        let route_len = 1.12 * gc_dist(40.0, -90.0, 40.0, -86.0);
        let rw = w.along(&path, route_len);
        assert_eq!(rw.headwind(route_len, 34_000.0), Some(0.0));
        assert_eq!(rw.headwind(0.5 * route_len, 34_000.0), Some(0.0));
        let east = rw.headwind(0.25 * route_len, 34_000.0).unwrap();
        assert!(
            (east - 60.0).abs() < 0.1,
            "flying east into an easterly reads {east}"
        );
    }

    // along: a route the field does not reach is still air (no error, no zero-wind knots).
    #[test]
    fn along_a_route_the_field_does_not_cover_is_still_air() {
        let w = fb(vec![station(40.0, -90.0, vec![(34000, wind(270.0, 40.0))])]);
        assert!(w.along(&[[60.0, -150.0], [61.0, -150.0]], 60.0).is_still());
        assert!(
            square(false)
                .along(&[[60.0, -150.0], [61.0, -150.0]], 60.0)
                .is_still()
        );
    }

    // along: a point outside the field is a gap knot, so the covered wind stops one sample past
    // the edge.
    #[test]
    fn along_marks_the_points_outside_the_field_as_gaps() {
        let w = Winds::test_grid(
            (40.0, -90.0),
            (2, 2),
            &[34_000.0],
            at_time("2026-10-16T01:00:00Z"),
            |_, _, _| (-50, 0, -40),
        );
        let path = [[40.5, -89.5], [40.5, -85.0]];
        let len = gc_dist(40.5, -89.5, 40.5, -85.0);
        let rw = w.along(&path, len);
        // Samples every 41 nm (205 nm in 5); only the first is inside the grid.
        let ds = rw.knot_distances();
        assert_eq!(ds.len(), 6, "{ds:?}");
        // An eastbound great circle runs a hair off 090°, so a 50 kt westerly reads just under 50 kt
        // of tailwind.
        let start = rw.headwind(len, 34_000.0).unwrap();
        assert!((start + 50.0).abs() < 0.1, "{start}");
        // Up to the first uncovered sample the covered wind holds; past it, still air.
        assert_eq!(rw.headwind(ds[4] + 1.0, 34_000.0), Some(start));
        assert_eq!(rw.headwind(ds[3] + 1.0, 34_000.0), None);
        assert_eq!(rw.headwind(0.0, 34_000.0), None);
    }

    // along: degenerate inputs.
    #[test]
    fn along_a_degenerate_path_is_still_air() {
        let w = square(false);
        assert!(w.along(&[], 10.0).is_still());
        assert!(w.along(&[[40.5, -89.5]], 10.0).is_still());
        assert!(w.along(&[[40.5, -89.5], [40.5, -89.5]], 10.0).is_still());
        assert!(
            Winds::default()
                .along(&[[40.2, -89.8], [40.8, -89.2]], 40.0)
                .is_still()
        );
    }

    // along on a RAP grid carries every pressure level with its temperature, so the profile can
    // read wind and temperature at its own altitude.
    #[test]
    fn along_on_a_grid_carries_every_level_with_its_temperature() {
        let w = Winds::test_grid(
            (38.0, -78.0),
            (3, 4),
            &[5_000.0, 18_000.0, 34_000.0],
            at_time("2026-10-16T01:00:00Z"),
            |alt, _, _| match alt as i32 {
                5_000 => (0, 0, 5),
                18_000 => (0, 20, -20),
                _ => (0, 80, -50),
            },
        );
        let path = [[38.2, -76.5], [39.8, -76.5]];
        let len = gc_dist(38.2, -76.5, 39.8, -76.5);
        let rw = w.along(&path, len);
        let d = len / 2.0;
        let hw = |alt| rw.headwind(d, alt).unwrap();
        assert!(hw(5_000.0).abs() < 1e-6);
        assert!((hw(18_000.0) - 20.0).abs() < 1e-6);
        assert!((hw(26_000.0) - 50.0).abs() < 1e-6);
        assert!((hw(34_000.0) - 80.0).abs() < 1e-6);
        assert!((rw.temp_k(d, 34_000.0).unwrap() - 223.15).abs() < 1e-9);
    }

    // Long legs are sampled on the great circle, not a straight lat/lon line.
    #[test]
    fn great_circle_points_follow_the_great_circle() {
        let mid = great_circle_point([0.0, 0.0], [0.0, 90.0], 0.5);
        assert!(
            mid[0].abs() < 1e-9 && (mid[1] - 45.0).abs() < 1e-9,
            "{mid:?}"
        );
        // Halfway between two points on 45°N, 180° apart, the great circle crosses the pole.
        let pole = great_circle_point([45.0, -90.0], [45.0, 90.0], 0.5);
        assert!((pole[0] - 90.0).abs() < 1e-9, "{pole:?}");
        assert_eq!(
            great_circle_point([40.0, -90.0], [40.0, -90.0], 0.5),
            [40.0, -90.0]
        );
    }

    // Each sample reads the headwind on the route's course where it sits: on a KJFK→KLAX direct
    // leg the course turns from 274° to 246°, so a 50 kt northerly goes from a crosswind to a
    // 20 kt tailwind (50 kt × cos 246°).
    #[test]
    fn along_reads_the_course_at_each_point_of_a_long_leg() {
        let w = Winds::test_grid(
            (30.0, -120.0),
            (15, 50),
            &[34_000.0],
            at_time("2026-10-16T01:00:00Z"),
            |_, _, _| (0, 50, -45),
        );
        let (jfk, lax) = ([40.64, -73.78], [33.94, -118.41]);
        let len = gc_dist(jfk[0], jfk[1], lax[0], lax[1]);
        let rw = w.along(&[jfk, lax], len);
        let start = rw.headwind(len, 34_000.0).unwrap();
        let end = rw.headwind(0.0, 34_000.0).unwrap();
        assert!((start - 3.3).abs() < 1.5, "start {start}");
        assert!((end + 20.4).abs() < 1.5, "end {end}");
    }

    // Interior samples sit on the great circle and read the course there. Halfway along KJFK→KLAX
    // (sample 21 of 43, 1,098 nm from LAX) the great circle is at 39.54°N on a 259.0° course; a
    // straight lat/lon line would be at 37.37°N. A 50 kt northerly blowing only north of 39°N
    // reads 50 × cos 259.0° = -9.5 kt there, and nothing on the straight line.
    #[test]
    fn along_samples_a_long_leg_on_the_great_circle_with_the_course_there() {
        let w = Winds::test_grid(
            (30.0, -120.0),
            (15, 50),
            &[34_000.0],
            at_time("2026-10-16T01:00:00Z"),
            |_, lat, _| (0, if lat >= 39.0 { 50 } else { 0 }, -45),
        );
        let (jfk, lax) = ([40.64, -73.78], [33.94, -118.41]);
        let len = gc_dist(jfk[0], jfk[1], lax[0], lax[1]);
        let rw = w.along(&[jfk, lax], len);
        let mid = rw.headwind(1_097.95, 34_000.0).unwrap();
        assert!((mid + 9.5).abs() < 1.0, "halfway reads {mid} kt");
    }

    // A grid with no rows has no column rather than underflowing its index arithmetic.
    #[test]
    fn a_grid_with_no_rows_has_no_column() {
        let grid = WindGrid {
            lat0: 40.0,
            lon0: -90.0,
            step_deg: 1.0,
            nlat: 0,
            nlon: 2,
            levels_ft: vec![34_000.0],
            u: vec![],
            v: vec![],
            t: vec![],
        };
        assert_eq!(grid.column(40.0, -90.0), None);
    }
}
