//! NOAA Rapid Refresh (RAP) winds and temperatures aloft, the primary wind field (#836).
//!
//! RAP runs hourly on a 13 km Lambert conformal grid over North America (NCEP grid 130, product
//! `awp130pgrb`). The refresh reads the file's `.idx` index, range-fetches only the wind (UGRD and
//! VGRD share one message) and temperature messages at [`LEVELS_HPA`], decodes them with the
//! `grib` crate, turns the grid-relative winds to true north, and averages the 13 km points into
//! the 1° lat/lon [`WindGrid`] the rest of OIS samples.

use chrono::{DateTime, Duration, DurationRound, Utc};
use grib::{Grib2SubmessageDecoder, GridDefinitionTemplateValues, LatLons};

use super::winds::{GRID_MISSING, WindGrid, Winds};

/// Isobaric levels fetched (hPa), roughly 5,000 ft (850) to FL450 (150): every 25–50 hPa through
/// the cruise band.
pub const LEVELS_HPA: [u16; 14] = [
    850, 700, 600, 500, 450, 400, 350, 300, 275, 250, 225, 200, 175, 150,
];
/// Where the files are published: the AWS Open Data bucket first, NOMADS as the mirror. Both use
/// the same `rap.YYYYMMDD/rap.tHHz.awp130pgrbfFF.grib2` layout.
const SOURCES: [&str; 2] = [
    "https://noaa-rap-pds.s3.amazonaws.com",
    "https://nomads.ncep.noaa.gov/pub/data/nccf/com/rap/prod",
];
/// How many cycles back from the current hour to look for the newest published one. A cycle lands
/// about 50–80 minutes after its hour.
const CYCLES_BACK: i64 = 4;
/// Spacing (deg) of the lat/lon grid the RAP points are averaged onto.
pub const GRID_STEP_DEG: f64 = 1.0;
/// GRIB2 parameters (discipline 0, meteorological): category 0 number 0 is temperature (K);
/// category 2 numbers 2 and 3 are the u and v wind components (m/s).
const TMP: (u8, u8) = (0, 0);
const UGRD: (u8, u8) = (2, 2);
const VGRD: (u8, u8) = (2, 3);
/// Code table 4.5: fixed surface type 100 is an isobaric surface, valued in Pa.
const ISOBARIC: u8 = 100;
/// Flag table 3.3 bit 5: u/v are relative to the grid's x/y axes rather than east/north.
const GRID_RELATIVE_WINDS: u8 = 0b0000_1000;
const MS_TO_KT: f64 = 1.943_844;
const K_TO_C: f64 = 273.15;

/// Pressure altitude (ft) of an isobaric level in the ICAO standard atmosphere: the troposphere
/// (lapse 6.5 K/km, 44,330.8 m × (1 − (p/p₀)^0.190263)) to 226.32 hPa (36,089 ft), the isothermal
/// stratosphere (scale height 6,341.6 m) above it. The two meet at the tropopause.
pub fn pressure_alt_ft(hpa: f64) -> f64 {
    if hpa >= 226.32 {
        145_442.16 * (1.0 - (hpa / 1013.25).powf(0.190_263))
    } else {
        36_089.24 + 20_805.8 * (226.32 / hpa).ln()
    }
}

/// One line of a wgrib2 `.idx` index: `msg:offset:d=YYYYMMDDHH:VAR:level:ftime:`. A message holding
/// two fields lists them as `N.1` and `N.2` at the same offset.
#[derive(Debug, Clone, PartialEq)]
pub struct IdxEntry {
    pub offset: u64,
    pub var: String,
    pub level: String,
}

/// Parse a `.idx` index. Fails closed on any line that does not have the expected shape.
pub fn parse_idx(text: &str) -> Result<Vec<IdxEntry>, String> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            if f.len() < 6 || !f[2].starts_with("d=") {
                return Err(format!("unexpected idx line: {line}"));
            }
            let offset = f[1]
                .parse()
                .map_err(|_| format!("bad idx offset: {line}"))?;
            Ok(IdxEntry {
                offset,
                var: f[3].to_string(),
                level: f[4].to_string(),
            })
        })
        .collect()
}

/// The byte range (`start`, inclusive `end`, None for "to the end of the file") of the message
/// holding `var` at `level`: from its offset to just before the next message with a higher offset.
pub fn message_range(entries: &[IdxEntry], var: &str, level: &str) -> Option<(u64, Option<u64>)> {
    let start = entries
        .iter()
        .find(|e| e.var == var && e.level == level)?
        .offset;
    let end = entries
        .iter()
        .map(|e| e.offset)
        .filter(|&o| o > start)
        .min()
        .map(|next| next - 1);
    Some((start, end))
}

/// The hour a fetch at `now` wants a field for (the nearest one), and the files that hold it:
/// the newest cycle first, back [`CYCLES_BACK`] hours, each on every source in turn.
pub fn candidates(now: DateTime<Utc>) -> Result<(DateTime<Utc>, Vec<String>), String> {
    let hour = now
        .duration_trunc(Duration::hours(1))
        .map_err(|e| e.to_string())?;
    let valid = if now - hour >= Duration::minutes(30) {
        hour + Duration::hours(1)
    } else {
        hour
    };
    let mut urls = Vec::new();
    for back in 1..=CYCLES_BACK {
        let cycle = hour - Duration::hours(back);
        let fhour = (valid - cycle).num_hours();
        for base in SOURCES {
            urls.push(format!(
                "{base}/rap.{}/rap.t{}z.awp130pgrbf{fhour:02}.grib2",
                cycle.format("%Y%m%d"),
                cycle.format("%H"),
            ));
        }
    }
    Ok((valid, urls))
}

/// Fetch the newest published RAP field valid at the hour nearest `now`. Errors name every file
/// tried.
pub async fn fetch(client: &reqwest::Client, now: DateTime<Utc>) -> Result<Winds, String> {
    let (valid, urls) = candidates(now)?;
    let mut errors = Vec::new();
    for url in urls {
        match fetch_file(client, &url).await {
            Ok(grid) => return Ok(Winds::from_grid(grid, valid)),
            Err(e) => errors.push(format!("{url}: {e}")),
        }
    }
    Err(errors.join("; "))
}

/// Read one file's index, range-fetch its wind and temperature messages, and build the grid.
async fn fetch_file(client: &reqwest::Client, url: &str) -> Result<WindGrid, String> {
    let idx = get(client, &format!("{url}.idx"), None).await?;
    let entries = parse_idx(&String::from_utf8_lossy(&idx))?;
    let mut messages = Vec::new();
    for hpa in LEVELS_HPA {
        let level = format!("{hpa} mb");
        for var in ["UGRD", "TMP"] {
            let range = message_range(&entries, var, &level)
                .ok_or_else(|| format!("{var} at {level} is not in the index"))?;
            messages.push(get(client, url, Some(range)).await?);
        }
    }
    build_grid(messages)
}

async fn get(
    client: &reqwest::Client,
    url: &str,
    range: Option<(u64, Option<u64>)>,
) -> Result<Vec<u8>, String> {
    let mut req = client.get(url);
    if let Some((start, end)) = range {
        let end = end.map(|e| e.to_string()).unwrap_or_default();
        req = req.header(reqwest::header::RANGE, format!("bytes={start}-{end}"));
    }
    let resp = req
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.without_url().to_string())?;
    // A server that ignores Range answers 200 with the whole ~17 MB file; refuse it.
    if range.is_some() && resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(format!("range request answered {}", resp.status()));
    }
    let body = resp
        .bytes()
        .await
        .map_err(|e| e.without_url().to_string())?;
    Ok(body.to_vec())
}

/// One decoded field: its parameter, its isobaric level (hPa), and its values in grid order.
struct Field {
    param: (u8, u8),
    hpa: f64,
    values: Vec<f32>,
}

/// Where each source point lands on the lat/lon grid, and the turn from grid-relative to
/// true-north winds there.
struct Resampler {
    shape: (usize, usize),
    lat0: f64,
    lon0: f64,
    nlat: usize,
    nlon: usize,
    /// Target cell of each source point, in value order.
    cell: Vec<usize>,
    /// `(sin, cos)` of the angle from true north to the grid's +y axis at each point, or `(0, 1)`
    /// when the winds are already earth-relative.
    turn: Vec<(f64, f64)>,
}

/// Decode GRIB2 messages holding UGRD+VGRD or TMP on isobaric levels into the 1° grid. Every
/// message must share one grid; a level needs its wind to appear, and its temperature is optional.
pub fn build_grid(messages: Vec<Vec<u8>>) -> Result<WindGrid, String> {
    let mut resampler: Option<Resampler> = None;
    let mut fields: Vec<Field> = Vec::new();
    for bytes in messages {
        let grib = grib::from_bytes(bytes).map_err(|e| e.to_string())?;
        for (_, sub) in grib.iter() {
            let r = match &resampler {
                Some(r) => r,
                None => resampler.insert(Resampler::new(&sub)?),
            };
            if sub.grid_shape().map_err(|e| e.to_string())? != r.shape {
                return Err("messages are on different grids".into());
            }
            let prod = sub.prod_def();
            let param = (
                prod.parameter_category()
                    .ok_or("unsupported product template")?,
                prod.parameter_number()
                    .ok_or("unsupported product template")?,
            );
            let surface = prod.fixed_surfaces().ok_or("no fixed surface")?.0;
            if surface.surface_type != ISOBARIC {
                return Err(format!(
                    "surface type {} is not isobaric",
                    surface.surface_type
                ));
            }
            let hpa = surface.value() / 100.0;
            let values: Vec<f32> = Grib2SubmessageDecoder::from(sub)
                .map_err(|e| e.to_string())?
                .dispatch()
                .map_err(|e| e.to_string())?
                .collect();
            if values.len() != r.cell.len() {
                return Err("decoded value count does not match the grid".into());
            }
            fields.push(Field { param, hpa, values });
        }
    }
    let r = resampler.ok_or("no messages to decode")?;
    r.grid(&fields)
}

impl Resampler {
    fn new<R: grib::Grib2Read>(sub: &grib::SubMessage<'_, R>) -> Result<Self, String> {
        let GridDefinitionTemplateValues::Template30(lambert) =
            GridDefinitionTemplateValues::try_from(sub.grid_def()).map_err(|e| e.to_string())?
        else {
            return Err("RAP grid is not Lambert conformal (template 3.30)".into());
        };
        let grid_relative = lambert.resolution_and_component_flags.0 & GRID_RELATIVE_WINDS != 0;
        let (ni, nj) = sub.grid_shape().map_err(|e| e.to_string())?;
        let latlons: Vec<(f64, f64)> = sub.latlons().map_err(|e| e.to_string())?.collect();
        let ij: Vec<(usize, usize)> = sub.ij().map_err(|e| e.to_string())?.collect();
        if latlons.len() != ni * nj || ij.len() != ni * nj {
            return Err("grid point count does not match the grid shape".into());
        }
        let lon = |l: f64| if l > 180.0 { l - 360.0 } else { l };
        let mut at = vec![usize::MAX; ni * nj];
        for (k, &(i, j)) in ij.iter().enumerate() {
            at[j * ni + i] = k;
        }
        let up = lambert.scanning_mode.scans_positively_for_j();
        let turn = ij
            .iter()
            .map(|&(i, j)| {
                if !grid_relative {
                    return (0.0, 1.0);
                }
                // The +y axis's true bearing: toward the next point up the grid, or from the one
                // below it on the top row.
                let step = |jj: usize| latlons[at[jj * ni + i]];
                let (here, j_up, j_down) = (step(j), j + 1 < nj, j > 0);
                let (from, to) = match (up, j_up, j_down) {
                    (true, true, _) => (here, step(j + 1)),
                    (true, false, _) => (step(j - 1), here),
                    (false, _, true) => (here, step(j - 1)),
                    (false, _, false) => (step(j + 1), here),
                };
                let b = bearing_deg(from.0, lon(from.1), to.0, lon(to.1)).to_radians();
                (b.sin(), b.cos())
            })
            .collect();
        let (mut lat_min, mut lat_max, mut lon_min, mut lon_max) =
            (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for &(la, lo) in &latlons {
            let lo = lon(lo);
            lat_min = lat_min.min(la);
            lat_max = lat_max.max(la);
            lon_min = lon_min.min(lo);
            lon_max = lon_max.max(lo);
        }
        let snap = |v: f64| (v / GRID_STEP_DEG).round() * GRID_STEP_DEG;
        let (lat0, lon0) = (snap(lat_min), snap(lon_min));
        let nlat = ((snap(lat_max) - lat0) / GRID_STEP_DEG).round() as usize + 1;
        let nlon = ((snap(lon_max) - lon0) / GRID_STEP_DEG).round() as usize + 1;
        let cell = latlons
            .iter()
            .map(|&(la, lo)| {
                let i = ((la - lat0) / GRID_STEP_DEG).round() as usize;
                let j = ((lon(lo) - lon0) / GRID_STEP_DEG).round() as usize;
                i.min(nlat - 1) * nlon + j.min(nlon - 1)
            })
            .collect();
        Ok(Self {
            shape: (ni, nj),
            lat0,
            lon0,
            nlat,
            nlon,
            cell,
            turn,
        })
    }

    /// Average every level's points into the grid. Wind is turned to true north first and stored as
    /// the "from" vector in kt; temperature in °C.
    fn grid(&self, fields: &[Field]) -> Result<WindGrid, String> {
        let find = |param: (u8, u8), hpa: f64| {
            fields
                .iter()
                .find(|f| f.param == param && (f.hpa - hpa).abs() < 0.5)
        };
        let mut levels: Vec<f64> = fields
            .iter()
            .filter(|f| f.param == UGRD)
            .map(|f| f.hpa)
            .collect();
        // Highest pressure first is lowest altitude first.
        levels.sort_by(|a, b| b.total_cmp(a));
        levels.dedup_by(|a, b| (*a - *b).abs() < 0.5);
        if levels.is_empty() {
            return Err("no wind fields decoded".into());
        }
        let plane = self.nlat * self.nlon;
        let mut grid = WindGrid {
            lat0: self.lat0,
            lon0: self.lon0,
            step_deg: GRID_STEP_DEG,
            nlat: self.nlat,
            nlon: self.nlon,
            levels_ft: levels.iter().map(|&p| pressure_alt_ft(p)).collect(),
            u: Vec::with_capacity(plane * levels.len()),
            v: Vec::with_capacity(plane * levels.len()),
            t: Vec::with_capacity(plane * levels.len()),
        };
        for &hpa in &levels {
            let (u, v) = (
                find(UGRD, hpa).ok_or("UGRD missing")?,
                find(VGRD, hpa).ok_or_else(|| format!("VGRD missing at {hpa} hPa"))?,
            );
            let (mut su, mut sv, mut st) = (vec![0.0; plane], vec![0.0; plane], vec![0.0; plane]);
            let (mut nw, mut nt) = (vec![0u32; plane], vec![0u32; plane]);
            for (k, &c) in self.cell.iter().enumerate() {
                let (gu, gv) = (u.values[k] as f64, v.values[k] as f64);
                if gu.is_finite() && gv.is_finite() {
                    let (s, co) = self.turn[k];
                    // Grid-relative to earth-relative "toward" components, then the "from" vector.
                    su[c] -= (gu * co + gv * s) * MS_TO_KT;
                    sv[c] -= (gv * co - gu * s) * MS_TO_KT;
                    nw[c] += 1;
                }
            }
            if let Some(t) = find(TMP, hpa) {
                for (k, &c) in self.cell.iter().enumerate() {
                    let tk = t.values[k] as f64;
                    if tk.is_finite() {
                        st[c] += tk - K_TO_C;
                        nt[c] += 1;
                    }
                }
            }
            let mean = |sum: f64, n: u32| {
                if n == 0 {
                    GRID_MISSING
                } else {
                    (sum / n as f64).round() as i16
                }
            };
            for c in 0..plane {
                grid.u.push(mean(su[c], nw[c]));
                grid.v.push(mean(sv[c], nw[c]));
                grid.t.push(mean(st[c], nt[c]));
            }
        }
        Ok(grid)
    }
}

fn bearing_deg(la1: f64, lo1: f64, la2: f64, lo2: f64) -> f64 {
    let dlo = (lo2 - lo1).to_radians();
    let y = dlo.sin() * la2.to_radians().cos();
    let x = la1.to_radians().cos() * la2.to_radians().sin()
        - la1.to_radians().sin() * la2.to_radians().cos() * dlo.cos();
    (y.atan2(x).to_degrees() + 360.0) % 360.0
}

// Coverage matrix (VATUSA/OIS#836): RAP decode, index parsing, rotation.
//
// The GRIB2 and `.idx` bytes are captured from NOMADS (`rap.t00z.awp130pgrbf00`, 2026-10-16 00Z),
// never a live fetch (AC5): `fixtures/rap/uv250.grib2` is message 42 (UGRD+VGRD at 250 mb, idx
// offset 2091693, 77,351 bytes), `fixtures/rap/tmp250.grib2` is message 39 (TMP at 250 mb, offset
// 1978615, 29,869 bytes), `fixtures/rap/rap.t00z.awp130pgrbf00.grib2.idx` is the file's real index.
//
// Requirement                                  | Test                                                                       | Mutation that must turn it red
// ---------------------------------------------|----------------------------------------------------------------------------|---------------------------------------------------------------
// AC5 decode captured RAP: grid shape          | the_captured_250_mb_messages_decode_onto_a_one_degree_grid                 | `GRID_STEP_DEG` 1.0 -> 0.5; `snap` round -> floor
// AC5 level altitude 250 hPa                   | the_250_mb_level_sits_at_its_standard_atmosphere_altitude                  | swap the `pressure_alt_ft` branches; 0.190284 -> 0.19
// pressure_alt_ft both sides of 226.32 hPa     | pressure_altitude_matches_the_standard_atmosphere_table                    | `>= 226.32` -> `>= 200.0`; 20_805.8 -> 20_000.0
// AC5 plausible kt / degC ranges               | decoded_winds_and_temperatures_are_in_plausible_upper_air_ranges           | drop `* MS_TO_KT`; drop `- K_TO_C`
// AC5 cell value vs independent raw-grid calc  | a_cells_temperature_is_the_mean_of_the_raw_points_inside_it                | `st[c] +=` -> `st[c] =`; `cell` lat/lon index swapped
// AC5 cell wind speed vs raw grid              | a_cells_wind_speed_matches_the_raw_points_inside_it                        | drop `* MS_TO_KT`; drop `nw[c] += 1`
// rotation sign pinned far from LoV            | grid_relative_winds_are_turned_to_true_north_far_from_the_central_meridian | `(gu * co + gv * s)` -> `(gu * co - gv * s)`; `turn` forced (0, 1)
// rotation positive control on LoV             | winds_on_the_central_meridian_are_left_unturned                            | `turn` from longitude instead of the grid's +y bearing
// stored vector is the "from" wind             | the_grid_stores_the_wind_as_the_from_vector                                | `su[c] -=` -> `su[c] +=`
// wind without temperature                     | a_level_with_wind_but_no_temperature_has_missing_temperatures              | `t.push(mean(st[c], nt[c]))` -> `t.push(0)`
// fail closed: no messages                     | build_grid_refuses_an_empty_message_list                                   | `ok_or("no messages to decode")` -> an empty grid
// fail closed: temperature only, no wind       | build_grid_refuses_messages_with_no_wind_field                             | delete the `levels.is_empty()` check
// fail closed: non-isobaric surface            | build_grid_refuses_a_message_on_a_non_isobaric_surface                     | delete the `surface_type != ISOBARIC` check
// fail closed: bytes that are not GRIB2        | build_grid_refuses_bytes_that_are_not_grib                                 | `from_bytes` error -> `continue`
// parse_idx on the captured index              | parse_idx_reads_every_line_of_the_captured_index                           | `f.len() < 6` -> `f.len() < 7`; var/level fields swapped
// N.1/N.2 share an offset                      | the_u_and_v_submessages_share_one_offset_and_one_byte_range                | `o > start` -> `o >= start`
// range end = next distinct offset - 1         | message_ranges_match_the_captured_message_sizes                            | `next - 1` -> `next`; `min()` -> `max()`
// last message has an open end                 | the_last_message_in_the_index_runs_to_the_end_of_the_file                  | `.map(|next| next - 1)` -> `.or(Some(start))`
// absent var/level                             | message_range_is_none_for_a_field_the_index_does_not_list                  | `find` falls back to the first entry
// fail closed: malformed idx lines             | parse_idx_refuses_a_line_without_the_expected_shape                        | delete the `f.len() < 6 || !starts_with("d=")` guard
// fail closed: non-numeric offset              | parse_idx_refuses_a_non_numeric_offset                                     | parse error mapped to offset 0
//
// No seam (flagged): `fetch`'s valid-hour rounding and cycle/forecast-hour URL list, and `get`'s
// refusal of a 200 answer to a Range request, live inside network code.
#[cfg(test)]
mod tests {
    use super::*;

    const UV: &[u8] = include_bytes!("../../../fixtures/rap/uv250.grib2");
    const TMP_250: &[u8] = include_bytes!("../../../fixtures/rap/tmp250.grib2");
    const IDX: &str = include_str!("../../../fixtures/rap/rap.t00z.awp130pgrbf00.grib2.idx");

    /// NCEP grid 130 (ON388 Table B): a tangent Lambert cone at 25°N about 95°W. Taken from the
    /// published grid definition, not from the decoder, so the rotation tests do not reuse the
    /// production geometry.
    const LATIN1_DEG: f64 = 25.0;
    const LOV_DEG: f64 = -95.0;

    /// Every raw point of one field in a captured message, decoded here with the grib crate:
    /// `(lat, lon east-negative, value)`.
    fn raw(bytes: &[u8], param: (u8, u8)) -> Vec<(f64, f64, f64)> {
        let g = grib::from_bytes(bytes.to_vec()).unwrap();
        for (_, sub) in g.iter() {
            let p = sub.prod_def();
            if (
                p.parameter_category().unwrap(),
                p.parameter_number().unwrap(),
            ) != param
            {
                continue;
            }
            let ll: Vec<(f64, f64)> = sub.latlons().unwrap().collect();
            let vals: Vec<f32> = Grib2SubmessageDecoder::from(sub)
                .unwrap()
                .dispatch()
                .unwrap()
                .collect();
            return ll
                .into_iter()
                .zip(vals)
                .map(|((la, lo), v)| (la, if lo > 180.0 { lo - 360.0 } else { lo }, v as f64))
                .collect();
        }
        panic!("parameter {param:?} not in the message");
    }

    /// The raw points that average into the 1° cell at `(lat, lon)`.
    fn in_cell(points: &[(f64, f64, f64)], lat: f64, lon: f64) -> Vec<(f64, f64, f64)> {
        points
            .iter()
            .copied()
            .filter(|&(la, lo, _)| la.round() == lat && lo.round() == lon)
            .collect()
    }

    fn at(grid: &WindGrid, field: &[i16], lat: f64, lon: f64) -> i16 {
        let i = ((lat - grid.lat0) / grid.step_deg).round() as usize;
        let j = ((lon - grid.lon0) / grid.step_deg).round() as usize;
        field[i * grid.nlon + j]
    }

    /// The independent earth-relative "from" wind (kt) of a cell: each raw grid-relative point
    /// turned by α = sin(latin1)·(λ − LoV), meaned, negated.
    fn earth_from_wind(lat: f64, lon: f64) -> (f64, f64, usize) {
        let (u, v) = (raw(UV, UGRD), raw(UV, VGRD));
        let n = LATIN1_DEG.to_radians().sin();
        let (mut su, mut sv, mut k) = (0.0, 0.0, 0usize);
        for (pu, pv) in u.iter().zip(&v) {
            if pu.0.round() != lat || pu.1.round() != lon {
                continue;
            }
            let a = (n * (pu.1 - LOV_DEG)).to_radians();
            su += pu.2 * a.cos() + pv.2 * a.sin();
            sv += -pu.2 * a.sin() + pv.2 * a.cos();
            k += 1;
        }
        let kt = MS_TO_KT / k as f64;
        (-su * kt, -sv * kt, k)
    }

    // AC5: the captured messages decode; the grid is 1° and spans the RAP domain.
    #[test]
    fn the_captured_250_mb_messages_decode_onto_a_one_degree_grid() {
        let grid = build_grid(vec![UV.to_vec(), TMP_250.to_vec()]).unwrap();
        let pts = raw(TMP_250, TMP);
        let lat_min = pts.iter().map(|p| p.0).fold(f64::MAX, f64::min).round();
        let lat_max = pts.iter().map(|p| p.0).fold(f64::MIN, f64::max).round();
        let lon_min = pts.iter().map(|p| p.1).fold(f64::MAX, f64::min).round();
        let lon_max = pts.iter().map(|p| p.1).fold(f64::MIN, f64::max).round();
        assert_eq!(grid.step_deg, 1.0);
        assert_eq!(grid.levels_ft.len(), 1);
        assert_eq!((grid.lat0, grid.lon0), (lat_min, lon_min));
        assert_eq!(grid.nlat, (lat_max - lat_min) as usize + 1);
        assert_eq!(grid.nlon, (lon_max - lon_min) as usize + 1);
        let n = grid.nlat * grid.nlon;
        assert_eq!((grid.u.len(), grid.v.len(), grid.t.len()), (n, n, n));
        assert!(grid.lat0 + grid.nlat as f64 - 1.0 >= 50.0 && grid.lon0 <= -130.0);
    }

    // AC5: 250 hPa in the ICAO standard atmosphere is 10,363 m = 33,999 ft.
    #[test]
    fn the_250_mb_level_sits_at_its_standard_atmosphere_altitude() {
        let grid = build_grid(vec![UV.to_vec()]).unwrap();
        assert_eq!(grid.levels_ft.len(), 1);
        assert!(
            (grid.levels_ft[0] - 33_999.0).abs() < 20.0,
            "{}",
            grid.levels_ft[0]
        );
    }

    // Both branches of `pressure_alt_ft`, and continuity at the 226.32 hPa tropopause.
    #[test]
    fn pressure_altitude_matches_the_standard_atmosphere_table() {
        for (hpa, ft) in [
            (850.0, 4_781.0),
            (500.0, 18_289.0),
            (300.0, 30_065.0),
            (200.0, 38_662.0),
            (150.0, 44_647.0),
        ] {
            let got = pressure_alt_ft(hpa);
            assert!((got - ft).abs() < 20.0, "{hpa} hPa: {got} ft, table {ft}");
        }
        for hpa in [226.33, 226.31] {
            assert!((pressure_alt_ft(hpa) - 36_089.0).abs() < 2.0, "{hpa}");
        }
    }

    // AC5: plausible upper-air values (catches a dropped unit conversion).
    #[test]
    fn decoded_winds_and_temperatures_are_in_plausible_upper_air_ranges() {
        let grid = build_grid(vec![UV.to_vec(), TMP_250.to_vec()]).unwrap();
        let mut fastest = 0.0_f64;
        let mut missing = 0;
        for c in 0..grid.u.len() {
            if grid.u[c] == GRID_MISSING {
                missing += 1;
                continue;
            }
            let (u, v, t) = (grid.u[c] as f64, grid.v[c] as f64, grid.t[c]);
            assert!(u.abs() <= 250.0 && v.abs() <= 250.0, "cell {c}: {u},{v}");
            assert!((-75..=-25).contains(&t), "cell {c}: {t} °C");
            fastest = fastest.max(u.hypot(v));
        }
        assert!(fastest > 60.0, "fastest {fastest} kt");
        assert!(
            missing > 0,
            "the lat/lon box reaches outside the Lambert footprint"
        );
    }

    // AC5: the value at a known cell against an independent calculation from the raw grid.
    #[test]
    fn a_cells_temperature_is_the_mean_of_the_raw_points_inside_it() {
        let cell = in_cell(&raw(TMP_250, TMP), 40.0, -90.0);
        assert!(cell.len() >= 30, "{} raw points", cell.len());
        let mean_c = cell.iter().map(|p| p.2).sum::<f64>() / cell.len() as f64 - 273.15;
        let grid = build_grid(vec![UV.to_vec(), TMP_250.to_vec()]).unwrap();
        assert_eq!(at(&grid, &grid.t, 40.0, -90.0), mean_c.round() as i16);
    }

    // AC5: a cell's wind speed against the raw points (rotation-invariant).
    #[test]
    fn a_cells_wind_speed_matches_the_raw_points_inside_it() {
        let (u, v) = (
            in_cell(&raw(UV, UGRD), 40.0, -90.0),
            in_cell(&raw(UV, VGRD), 40.0, -90.0),
        );
        let n = u.len() as f64;
        let speed = (u.iter().map(|p| p.2).sum::<f64>() / n)
            .hypot(v.iter().map(|p| p.2).sum::<f64>() / n)
            * MS_TO_KT;
        let grid = build_grid(vec![UV.to_vec()]).unwrap();
        let got =
            (at(&grid, &grid.u, 40.0, -90.0) as f64).hypot(at(&grid, &grid.v, 40.0, -90.0) as f64);
        assert!((got - speed).abs() < 1.5, "grid {got} kt, raw {speed} kt");
    }

    // Rotation sign: far from LoV the earth- and grid-relative winds differ by
    // α = sin(latin1)·(λ − LoV).
    #[test]
    fn grid_relative_winds_are_turned_to_true_north_far_from_the_central_meridian() {
        let grid = build_grid(vec![UV.to_vec()]).unwrap();
        let mut turned = false;
        for (lat, lon) in [(45.0, -70.0), (45.0, -120.0)] {
            let (eu, ev, k) = earth_from_wind(lat, lon);
            assert!(k > 0);
            let (gu, gv) = (
                at(&grid, &grid.u, lat, lon) as f64,
                at(&grid, &grid.v, lat, lon) as f64,
            );
            assert!(
                (gu - eu).abs() < 1.5 && (gv - ev).abs() < 1.5,
                "({lat},{lon}): grid ({gu},{gv}) vs independent ({eu:.1},{ev:.1})"
            );
            let (ru, rv) = (
                in_cell(&raw(UV, UGRD), lat, lon),
                in_cell(&raw(UV, VGRD), lat, lon),
            );
            let m = ru.len() as f64;
            let un_u = -ru.iter().map(|p| p.2).sum::<f64>() / m * MS_TO_KT;
            let un_v = -rv.iter().map(|p| p.2).sum::<f64>() / m * MS_TO_KT;
            turned |= (gu - un_u).abs() > 3.0 || (gv - un_v).abs() > 3.0;
        }
        assert!(turned, "the fixture must exercise the turn");
    }

    // Positive control for the rotation test: on the central meridian α ≈ 0.
    #[test]
    fn winds_on_the_central_meridian_are_left_unturned() {
        let grid = build_grid(vec![UV.to_vec()]).unwrap();
        let (ru, rv) = (
            in_cell(&raw(UV, UGRD), 40.0, -95.0),
            in_cell(&raw(UV, VGRD), 40.0, -95.0),
        );
        let m = ru.len() as f64;
        let un_u = -ru.iter().map(|p| p.2).sum::<f64>() / m * MS_TO_KT;
        let un_v = -rv.iter().map(|p| p.2).sum::<f64>() / m * MS_TO_KT;
        let (gu, gv) = (
            at(&grid, &grid.u, 40.0, -95.0) as f64,
            at(&grid, &grid.v, 40.0, -95.0) as f64,
        );
        assert!(
            (gu - un_u).abs() < 1.5 && (gv - un_v).abs() < 1.5,
            "({gu},{gv}) vs ({un_u},{un_v})"
        );
    }

    // The grid holds the "from" vector `WindLevel::uv` uses, so a headwind reads positive.
    #[test]
    fn the_grid_stores_the_wind_as_the_from_vector() {
        let grid = build_grid(vec![UV.to_vec()]).unwrap();
        let alt = grid.levels_ft[0];
        // A cell whose independent wind blows toward the east (its "from" u is negative).
        let (lat, lon) = (25..=50)
            .flat_map(|la| (-120..=-70).map(move |lo| (la as f64, lo as f64)))
            .find(|&(la, lo)| earth_from_wind(la, lo).0 < -40.0)
            .expect("a westerly of 40 kt or more somewhere over the CONUS at 250 mb");
        assert!(at(&grid, &grid.u, lat, lon) < -40);
        let winds = Winds::from_grid(grid, chrono::Utc::now());
        let hw = winds.point_headwind(lat, lon, 270.0, alt).unwrap();
        assert!(hw > 40.0, "flying west into a westerly reads {hw} kt");
    }

    // A level whose TMP message is absent still yields wind; temperature is GRID_MISSING.
    #[test]
    fn a_level_with_wind_but_no_temperature_has_missing_temperatures() {
        let grid = build_grid(vec![UV.to_vec()]).unwrap();
        assert!(grid.t.iter().all(|&t| t == GRID_MISSING));
        assert!(grid.u.iter().any(|&u| u != GRID_MISSING));
        assert!(grid.v.iter().any(|&v| v != GRID_MISSING));
    }

    // Fail closed: nothing to decode.
    #[test]
    fn build_grid_refuses_an_empty_message_list() {
        let err = build_grid(vec![]).err().unwrap();
        assert!(err.contains("no messages"), "{err}");
    }

    // Fail closed: temperature only is not a wind field.
    #[test]
    fn build_grid_refuses_messages_with_no_wind_field() {
        let err = build_grid(vec![TMP_250.to_vec()]).err().unwrap();
        assert!(err.contains("no wind fields"), "{err}");
    }

    /// The captured TMP message with section 4's "type of first fixed surface" set to `surface`,
    /// found by walking the section lengths.
    fn with_surface(surface: u8) -> Vec<u8> {
        let mut b = TMP_250.to_vec();
        let mut at = 16; // section 0, the indicator
        loop {
            let len = u32::from_be_bytes(b[at..at + 4].try_into().unwrap()) as usize;
            if b[at + 4] == 4 {
                // Template 4.0: octet 23 of the section is the first surface's type.
                assert_eq!(
                    b[at + 22],
                    ISOBARIC,
                    "the octet located is the surface type"
                );
                b[at + 22] = surface;
                return b;
            }
            at += len;
        }
    }

    // Fail closed: a non-isobaric surface.
    #[test]
    fn build_grid_refuses_a_message_on_a_non_isobaric_surface() {
        let err = build_grid(vec![UV.to_vec(), with_surface(1)])
            .err()
            .unwrap();
        assert!(err.contains("not isobaric"), "{err}");
        assert!(build_grid(vec![UV.to_vec(), with_surface(ISOBARIC)]).is_ok());
    }

    // Fail closed: an HTML error page, a truncated range.
    #[test]
    fn build_grid_refuses_bytes_that_are_not_grib() {
        assert!(build_grid(vec![b"<html>Not Found</html>".to_vec()]).is_err());
        assert!(build_grid(vec![UV[..UV.len() / 2].to_vec()]).is_err());
    }

    // parse_idx on the real index.
    #[test]
    fn parse_idx_reads_every_line_of_the_captured_index() {
        let entries = parse_idx(IDX).unwrap();
        assert_eq!(entries.len(), 355);
        assert_eq!(
            entries[0],
            IdxEntry {
                offset: 0,
                var: "REFC".into(),
                level: "entire atmosphere".into(),
            }
        );
        let tmp = entries
            .iter()
            .find(|e| e.var == "TMP" && e.level == "250 mb")
            .unwrap();
        assert_eq!(tmp.offset, 1_978_615);
        let crlf = IDX.replace("\r\n", "\n").replace('\n', "\r\n");
        assert_eq!(parse_idx(&crlf).unwrap(), entries);
    }

    // N.1/N.2: UGRD and VGRD share message 42's offset and byte range.
    #[test]
    fn the_u_and_v_submessages_share_one_offset_and_one_byte_range() {
        let e = parse_idx(IDX).unwrap();
        let want = Some((2_091_693, Some(2_169_043)));
        assert_eq!(message_range(&e, "UGRD", "250 mb"), want);
        assert_eq!(message_range(&e, "VGRD", "250 mb"), want);
    }

    // message_range end = next distinct offset − 1, tied to the captured bytes.
    #[test]
    fn message_ranges_match_the_captured_message_sizes() {
        let e = parse_idx(IDX).unwrap();
        let (s, end) = message_range(&e, "UGRD", "250 mb").unwrap();
        assert_eq!((end.unwrap() - s + 1) as usize, UV.len());
        let tmp = message_range(&e, "TMP", "250 mb").unwrap();
        assert_eq!(tmp, (1_978_615, Some(2_008_483)));
        assert_eq!((tmp.1.unwrap() - tmp.0 + 1) as usize, TMP_250.len());
        for hpa in LEVELS_HPA {
            let level = format!("{hpa} mb");
            assert!(message_range(&e, "UGRD", &level).is_some(), "UGRD {level}");
            assert!(message_range(&e, "TMP", &level).is_some(), "TMP {level}");
        }
    }

    // The last message has no next offset.
    #[test]
    fn the_last_message_in_the_index_runs_to_the_end_of_the_file() {
        let e = parse_idx(IDX).unwrap();
        let last = e.iter().max_by_key(|x| x.offset).unwrap();
        assert_eq!(last.offset, 17_247_593);
        assert_eq!(
            message_range(&e, &last.var, &last.level),
            Some((17_247_593, None))
        );
    }

    // A field the index does not list.
    #[test]
    fn message_range_is_none_for_a_field_the_index_does_not_list() {
        let e = parse_idx(IDX).unwrap();
        assert_eq!(message_range(&e, "UGRD", "137 mb"), None);
        assert_eq!(message_range(&e, "XXXX", "250 mb"), None);
    }

    // Fail closed: malformed lines.
    #[test]
    fn parse_idx_refuses_a_line_without_the_expected_shape() {
        for bad in [
            "42:2091693:d=2026101600:UGRD",
            "42:2091693:2026101600:UGRD:250 mb:anl:",
            "<html>",
        ] {
            let err = parse_idx(bad).err().unwrap();
            assert!(err.contains(bad), "{err}");
        }
        let good = "42.1:2091693:d=2026101600:UGRD:250 mb:anl:";
        assert!(parse_idx(&format!("{good}\n<html>")).is_err());
        assert_eq!(parse_idx(&format!("\n\n{good}\n")).unwrap().len(), 1);
    }

    // The hour asked for is the nearest one, and each cycle's forecast hour reaches it: the newest
    // cycle first (a cycle publishes 50-80 min after its hour), AWS before NOMADS, back 4 cycles,
    // across midnight.
    #[test]
    fn candidates_ask_for_the_nearest_hour_from_the_newest_cycle_first() {
        let at = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
        let (valid, urls) = candidates(at("2026-10-16T01:20:00Z")).unwrap();
        assert_eq!(valid, at("2026-10-16T01:00:00Z"));
        assert_eq!(urls.len(), 8);
        assert_eq!(
            urls[0],
            "https://noaa-rap-pds.s3.amazonaws.com/rap.20261016/rap.t00z.awp130pgrbf01.grib2"
        );
        assert_eq!(
            urls[1],
            "https://nomads.ncep.noaa.gov/pub/data/nccf/com/rap/prod/rap.20261016/rap.t00z.awp130pgrbf01.grib2"
        );
        assert_eq!(
            urls[2],
            "https://noaa-rap-pds.s3.amazonaws.com/rap.20261015/rap.t23z.awp130pgrbf02.grib2"
        );
        assert!(
            urls[7].ends_with("rap.20261015/rap.t21z.awp130pgrbf04.grib2"),
            "{}",
            urls[7]
        );

        let (valid, urls) = candidates(at("2026-10-16T01:30:00Z")).unwrap();
        assert_eq!(valid, at("2026-10-16T02:00:00Z"), "half past rounds up");
        assert!(
            urls[0].ends_with("rap.20261016/rap.t00z.awp130pgrbf02.grib2"),
            "{}",
            urls[0]
        );
    }

    // Fail closed: an offset that is not a number.
    #[test]
    fn parse_idx_refuses_a_non_numeric_offset() {
        assert!(parse_idx("42:abc:d=2026101600:UGRD:250 mb:anl:").is_err());
        assert!(parse_idx("42:-5:d=2026101600:UGRD:250 mb:anl:").is_err());
    }
}
