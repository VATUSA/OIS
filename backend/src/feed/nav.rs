//! Navigation database — full US enroute route expansion: fixes, navaids, airways,
//! SID/STAR procedures, and preferred routes. This is the single source of truth for
//! turning a filed route string into an accurate great-circle track; every consumer
//! (FCA matching, crossing detection, the aircraft-route popup, metering ETAs) resolves
//! through [`NavData::build_anchors`].
//!
//! Ported from vatflow's `route-engine.js`. Data is bundled at compile time from the FAA
//! NASR / CIFP export (`data/nav/*.json`); the cycle date lives in `meta.json`.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;

use super::airports::{Airport, AirportDb};

pub type Ll = [f64; 2];
pub type CoordList = Vec<Ll>;

/// A single procedure/airway leg as `(name, lat, lon)`.
type Leg = (String, f64, f64);

const R_NM: f64 = 3440.065;

/// What a resolved anchor represents. Retained for future labeled rendering; only the
/// coordinate is consumed by the geometry today.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)]
pub enum Kind {
    Apt,
    Nav,
    Fix,
    Sid,
    Star,
    Awy,
    /// An explicit lat/lon waypoint — an unambiguous coordinate (no duplicate-name risk).
    Coord,
}

/// One resolved point along a route.
#[derive(Clone, Debug)]
pub struct Anchor {
    pub name: String,
    pub ll: Ll,
    #[allow(dead_code)]
    pub kind: Kind,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ProcType {
    Sid,
    Star,
}

struct Procedure {
    ptype: ProcType,
    common: Vec<Leg>,
    transitions: HashMap<String, Vec<Leg>>,
}

struct Airway {
    w: Vec<Leg>,
}

/// The result of expanding a filed route.
#[derive(Default, Debug)]
pub struct RouteResult {
    pub anchors: Vec<Anchor>,
    /// Tokens inside US coverage that couldn't be resolved to a coordinate.
    pub unresolved: Vec<String>,
    /// Tokens deliberately dropped past an international/oceanic truncation.
    pub oceanic_skipped: Vec<String>,
    /// True when the route was truncated at the edge of US nav coverage.
    pub truncated_international: bool,
}

// --- raw JSON shapes (bundled files) ---

#[derive(Deserialize)]
struct RawAirway {
    #[serde(default)]
    w: Vec<Leg>,
}

#[derive(Deserialize)]
struct RawProc {
    #[serde(rename = "type")]
    ptype: String,
    #[serde(default)]
    common: Vec<Leg>,
    #[serde(default)]
    transitions: HashMap<String, Vec<Leg>>,
}

#[derive(Deserialize, Default)]
struct RawMeta {
    #[serde(default)]
    bbox: Option<[f64; 4]>,
    #[serde(rename = "nasrCycleDate", default)]
    nasr_cycle_date: Option<String>,
    #[serde(default)]
    source: Option<String>,
}

#[derive(Default)]
pub struct NavData {
    navaids: HashMap<String, CoordList>,
    fixes: HashMap<String, CoordList>,
    airways: HashMap<String, Airway>,
    procedures: HashMap<String, Procedure>,
    /// `DEP|ARR` → canonical route string.
    preferred: HashMap<String, String>,
    /// Shortest procedure key for a bare letter prefix (e.g. `DOTSS` → `DOTSS2`).
    proc_by_prefix: HashMap<String, String>,
    /// Navaid id → magnetic variation (deg, East positive), for fix-radial-distance points.
    nav_magvar: HashMap<String, f64>,
    bbox: [f64; 4],
    cycle: String,
    source: String,
}

impl NavData {
    /// Parse the bundled nav JSON (embedded at compile time). Used to seed the in-memory
    /// database at boot; the refresh job later hot-swaps in freshly fetched data.
    pub fn load() -> Self {
        Self::from_json(
            include_str!("../../data/nav/navaids.json"),
            include_str!("../../data/nav/fixes.json"),
            include_str!("../../data/nav/airways.json"),
            include_str!("../../data/nav/procedures.json"),
            include_str!("../../data/nav/preferred.json"),
            include_str!("../../data/nav/meta.json"),
            include_str!("../../data/nav/navvar.json"),
        )
    }

    /// Build a [`NavData`] from the six nav JSON blobs (bundled schema). Both the
    /// compile-time bundle and the runtime fetcher (which re-serializes fetched data into
    /// this same schema) go through here, so indexing/expansion behaviour is identical.
    #[allow(clippy::too_many_arguments)]
    pub fn from_json(
        navaids: &str,
        fixes: &str,
        airways: &str,
        procedures: &str,
        preferred: &str,
        meta: &str,
        navvar: &str,
    ) -> Self {
        let navaids: HashMap<String, CoordList> = serde_json::from_str(navaids).unwrap_or_default();
        let nav_magvar: HashMap<String, f64> = serde_json::from_str(navvar).unwrap_or_default();
        let fixes: HashMap<String, CoordList> = serde_json::from_str(fixes).unwrap_or_default();
        let raw_airways: HashMap<String, RawAirway> =
            serde_json::from_str(airways).unwrap_or_default();
        let raw_procs: HashMap<String, RawProc> =
            serde_json::from_str(procedures).unwrap_or_default();
        let preferred: HashMap<String, String> =
            serde_json::from_str(preferred).unwrap_or_default();
        let meta: RawMeta = serde_json::from_str(meta).unwrap_or_default();

        let airways = raw_airways
            .into_iter()
            .map(|(k, v)| (k, Airway { w: v.w }))
            .collect();

        let procedures: HashMap<String, Procedure> = raw_procs
            .into_iter()
            .map(|(k, v)| {
                let ptype = if v.ptype.eq_ignore_ascii_case("STAR") {
                    ProcType::Star
                } else {
                    ProcType::Sid
                };
                (
                    k,
                    Procedure {
                        ptype,
                        common: v.common,
                        transitions: v.transitions,
                    },
                )
            })
            .collect();

        // Index procedures by their bare letter prefix, keeping the shortest key so a
        // route filed without the revision digit (e.g. `DOTSS`) still resolves.
        let mut proc_by_prefix: HashMap<String, String> = HashMap::new();
        for key in procedures.keys() {
            if let Some(pfx) = proc_prefix(key) {
                proc_by_prefix
                    .entry(pfx)
                    .and_modify(|cur| {
                        if key.len() < cur.len() {
                            *cur = key.clone();
                        }
                    })
                    .or_insert_with(|| key.clone());
            }
        }

        Self {
            navaids,
            fixes,
            airways,
            procedures,
            preferred,
            proc_by_prefix,
            nav_magvar,
            bbox: meta.bbox.unwrap_or([-90.0, -180.0, 90.0, 180.0]),
            cycle: meta.nasr_cycle_date.unwrap_or_default(),
            source: meta.source.unwrap_or_default(),
        }
    }

    pub fn len(&self) -> usize {
        self.fixes.len() + self.navaids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// FAA NASR cycle date (e.g. `2026-07-09`), for display.
    pub fn cycle(&self) -> &str {
        &self.cycle
    }

    /// Human-readable provenance of the loaded data (e.g. `runtime fetch (faa)`).
    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn fix_count(&self) -> usize {
        self.fixes.len()
    }

    pub fn navaid_count(&self) -> usize {
        self.navaids.len()
    }

    pub fn airway_count(&self) -> usize {
        self.airways.len()
    }

    pub fn procedure_count(&self) -> usize {
        self.procedures.len()
    }

    /// Whether `token` names a known nav element (fix, navaid, airway, or procedure). Used to flag
    /// typos in an FCA's route-fix filter. Mirrors the trailing-digit stripping that `route_has_fix`
    /// applies to filed tokens, so a SID/STAR name like `MLLET5` is recognized via its fix `MLLET`.
    pub fn knows(&self, token: &str) -> bool {
        let t = token.split('/').next().unwrap_or("").to_ascii_uppercase();
        if t.is_empty() {
            return false;
        }
        if self.fixes.contains_key(&t)
            || self.navaids.contains_key(&t)
            || self.airways.contains_key(&t)
            || self.procedures.contains_key(&t)
        {
            return true;
        }
        let stripped = t.trim_end_matches(|c: char| c.is_ascii_digit());
        stripped != t && (self.fixes.contains_key(stripped) || self.navaids.contains_key(stripped))
    }

    /// Split a filed route into cleaned tokens (uppercased; `DCT` and flight-rule noise
    /// like `VFR`/`IFR` removed).
    pub fn parse_tokens(route: &str) -> Vec<String> {
        route
            .replace(['\n', '\r'], " ")
            .split_whitespace()
            .map(|t| t.to_ascii_uppercase())
            .filter(|t| !matches!(t.as_str(), "DCT" | "VFR" | "IFR" | "SVFR" | "DVFR"))
            .collect()
    }

    /// Whether a coordinate lies within the modeled US nav coverage box.
    pub fn in_nav_coverage(&self, lat: f64, lon: f64) -> bool {
        lat >= self.bbox[0] && lat <= self.bbox[2] && lon >= self.bbox[1] && lon <= self.bbox[3]
    }

    /// True when the filed destination is outside the US airport system we model.
    fn is_international_route(&self, arr: &str, destination: Option<Ll>) -> bool {
        let code = arr.to_ascii_uppercase();
        if code.len() >= 3 {
            let b = code.as_bytes();
            if b[0] == b'K' {
                return false;
            }
            if b[0] == b'P' && matches!(b[1], b'A' | b'H' | b'T' | b'G' | b'J' | b'F') {
                return false;
            }
            if b[0] == b'T' && matches!(b[1], b'J' | b'I' | b'P') {
                return false;
            }
            if b[0] == b'M' && b[1] == b'D' {
                return false;
            }
            return true;
        }
        destination.is_some_and(|d| !self.in_nav_coverage(d[0], d[1]))
    }

    /// If the pilot filed a bare `DEP..ARR` (≤2 airport tokens) and a preferred route
    /// exists, substitute it. Otherwise keep the filed route.
    fn maybe_preferred_route(
        &self,
        airports: &AirportDb,
        dep: &str,
        arr: &str,
        route: &str,
    ) -> String {
        if dep.is_empty() || arr.is_empty() {
            return route.to_string();
        }
        let Some(pr) = self.preferred.get(&format!("{dep}|{arr}")) else {
            return route.to_string();
        };
        let toks = Self::parse_tokens(route);
        if toks.len() <= 2 && toks.iter().all(|t| airports.get(&clean_token(t)).is_some()) {
            return pr.clone();
        }
        route.to_string()
    }

    fn find_procedure(&self, id: &str) -> Option<&Procedure> {
        let key = clean_token(id);
        if let Some(p) = self.procedures.get(&key) {
            return Some(p);
        }
        let pfx = proc_prefix(&key)?;
        self.proc_by_prefix
            .get(&pfx)
            .and_then(|k| self.procedures.get(k))
    }

    /// Resolve a single token to a point. Priority: airport → navaid → fix → procedure
    /// (first leg). Duplicate names are disambiguated by nearest to `ref_ll`.
    fn resolve_token(
        &self,
        name: &str,
        airports: &AirportDb,
        ref_ll: Option<Ll>,
        dep: &str,
        arr: &str,
    ) -> Option<Anchor> {
        let id = clean_token(name);
        if id.len() < 2 || id == dep || id == arr {
            return None;
        }
        if let Some(&Airport { lat, lon, .. }) = airports.get(&id) {
            return Some(Anchor {
                name: id,
                ll: [lat, lon],
                kind: Kind::Apt,
            });
        }
        // A navaid *name* can collide with a distant same-named fix (e.g. the "PARIS" VORTAC
        // in Texas vs the PARIS fix in Hawaii). Pool navaid + fix candidates and pick the one
        // nearest the route, rather than letting the navaid-before-fix priority pick a far match.
        let nav = self.navaids.get(&id);
        let fix = self.fixes.get(&id);
        if nav.is_some() || fix.is_some() {
            let mut cands: Vec<Ll> = Vec::new();
            if let Some(c) = nav {
                cands.extend_from_slice(c);
            }
            if let Some(c) = fix {
                cands.extend_from_slice(c);
            }
            return Some(Anchor {
                name: id,
                ll: nearest(&cands, ref_ll),
                kind: if nav.is_some() { Kind::Nav } else { Kind::Fix },
            });
        }
        if let Some((first, kind)) = self
            .find_procedure(&id)
            .and_then(|p| proc_first_leg(p).map(|f| ([f.1, f.2], proc_kind(p))))
        {
            return Some(Anchor {
                name: id,
                ll: first,
                kind,
            });
        }
        // Explicit lat/lon waypoint (e.g. 34N150E), common on oceanic segments.
        if let Some(ll) = decode_latlon(&id) {
            return Some(Anchor {
                name: id,
                ll,
                kind: Kind::Coord,
            });
        }
        // Fix-radial-distance (e.g. DAN060013 = 13 nm on DAN's 060° radial).
        self.resolve_frd(&id, ref_ll)
    }

    /// Resolve a fix-radial-distance token: a 2–5 char navaid/fix identifier followed by a
    /// 3-digit magnetic radial and a 2–3 digit distance in nm. The point is projected along
    /// the great circle from the station, correcting the (magnetic) radial to true with the
    /// navaid's published variation when known.
    fn resolve_frd(&self, id: &str, ref_ll: Option<Ll>) -> Option<Anchor> {
        let letters = id.bytes().take_while(u8::is_ascii_uppercase).count();
        if !(2..=5).contains(&letters) {
            return None;
        }
        let digits = &id[letters..];
        if !(5..=6).contains(&digits.len()) || !digits.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let radial: f64 = digits[..3].parse().ok()?;
        let dist: f64 = digits[3..].parse().ok()?;
        if radial > 360.0 || dist <= 0.0 {
            return None;
        }
        let base = &id[..letters];
        let (ll, magvar) = if let Some(cands) = self.navaids.get(base) {
            (nearest(cands, ref_ll), self.nav_magvar.get(base).copied())
        } else {
            let cands = self.fixes.get(base)?;
            (nearest(cands, ref_ll), None)
        };
        // Radials are magnetic; true = radial + declination (East positive).
        let true_brg = radial + magvar.unwrap_or(0.0);
        Some(Anchor {
            name: id.to_string(),
            ll: project(ll, true_brg, dist),
            kind: Kind::Fix,
        })
    }

    /// Magnetic variation (deg, East positive) published for a navaid, if known.
    #[allow(dead_code)]
    pub fn magvar(&self, id: &str) -> Option<f64> {
        self.nav_magvar.get(&id.to_ascii_uppercase()).copied()
    }

    /// Expand an airway between the waypoints nearest `from`/`to` (shorter direction).
    fn expand_airway(&self, id: &str, from: Option<Ll>, to: Option<Ll>) -> Vec<Anchor> {
        let awy = match self.airways.get(&clean_token(id)) {
            Some(a) if a.w.len() >= 2 => a,
            _ => return Vec::new(),
        };
        let wps = &awy.w;
        let from = from.unwrap_or([wps[0].1, wps[0].2]);
        let to = to.unwrap_or([wps[wps.len() - 1].1, wps[wps.len() - 1].2]);
        let i0 = nearest_wp_index(wps, from);
        let i1 = nearest_wp_index(wps, to);
        let to_anchor = |leg: &Leg| Anchor {
            name: leg.0.clone(),
            ll: [leg.1, leg.2],
            kind: Kind::Awy,
        };
        if i0 == i1 {
            return vec![to_anchor(&wps[i0])];
        }
        let (lo, hi) = (i0.min(i1), i0.max(i1));
        let slice: Vec<&Leg> = wps[lo..=hi].iter().collect();
        // Emit in travel order (reverse when the route runs against the airway's storage).
        let ordered: Vec<&Leg> = if i0 <= i1 {
            slice
        } else {
            slice.into_iter().rev().collect()
        };
        ordered.into_iter().map(to_anchor).collect()
    }

    /// Expand a SID/STAR into its leg sequence, splicing the named transition when given.
    fn expand_procedure(&self, proc: &Procedure, transition: Option<&str>) -> Vec<Anchor> {
        let kind = proc_kind(proc);
        let legs: Vec<Leg> = match transition.and_then(|t| proc.transitions.get(t)) {
            Some(trans) => merge_procedure_legs(trans, &proc.common),
            None if proc.common.len() >= 2 => proc.common.clone(),
            None => return Vec::new(),
        };
        legs.into_iter()
            .map(|leg| Anchor {
                name: leg.0,
                ll: [leg.1, leg.2],
                kind,
            })
            .collect()
    }

    /// Resolve a filed route to ordered lat/lon anchors: departure → expanded enroute
    /// (fixes/navaids/airways/SID/STAR) → arrival. Handles preferred-route substitution
    /// and truncates cleanly at the edge of US coverage for international routes.
    pub fn build_anchors(
        &self,
        airports: &AirportDb,
        dep: &str,
        arr: &str,
        route: &str,
    ) -> RouteResult {
        let dep = dep.to_ascii_uppercase();
        let arr = arr.to_ascii_uppercase();
        let origin = airports
            .get(&dep)
            .map(|&Airport { lat: a, lon: b, .. }| [a, b]);
        let destination = airports
            .get(&arr)
            .map(|&Airport { lat: a, lon: b, .. }| [a, b]);
        let route_str = self.maybe_preferred_route(airports, &dep, &arr, route);
        let tokens = Self::parse_tokens(&route_str);
        let intl = self.is_international_route(&arr, destination);

        let mut anchors: Vec<Anchor> = Vec::new();
        let mut unresolved: Vec<String> = Vec::new();
        let mut oceanic: Vec<String> = Vec::new();
        let mut ref_ll = origin;
        let mut truncated = false;

        if let Some(o) = origin {
            anchors.push(Anchor {
                name: if dep.is_empty() {
                    "DEP".into()
                } else {
                    dep.clone()
                },
                ll: o,
                kind: Kind::Apt,
            });
        }

        let drain_oceanic = |oceanic: &mut Vec<String>, from: usize| {
            for t in tokens.iter().skip(from) {
                oceanic.push(clean_token(t));
            }
        };

        let mut i = 0;
        while i < tokens.len() {
            let tok = clean_token(&tokens[i]);

            // A transition fix immediately preceding its STAR/SID is consumed by the
            // procedure expansion, not emitted as a standalone point.
            if let Some(next) = tokens.get(i + 1)
                && let Some(np) = self.find_procedure(&clean_token(next))
                && np.transitions.contains_key(&tok)
            {
                i += 1;
                continue;
            }

            if is_airway_token(&tok) {
                // Look ahead for the airway's exit fix (first resolvable, stop at next airway).
                let mut to_ll = None;
                for t in tokens.iter().skip(i + 1) {
                    if let Some(nx) = self.resolve_token(t, airports, ref_ll, &dep, &arr) {
                        to_ll = Some(nx.ll);
                        break;
                    }
                    if is_airway_token(&clean_token(t)) {
                        break;
                    }
                }
                if ref_ll.is_none() {
                    if !intl {
                        unresolved.push(tok);
                    }
                    truncated = intl;
                    i += 1;
                    continue;
                }
                let expanded = self.expand_airway(&tok, ref_ll, to_ll);
                if expanded.is_empty() {
                    if !intl {
                        unresolved.push(tok);
                    }
                    truncated = intl;
                    i += 1;
                    continue;
                }
                for pt in expanded {
                    if intl && !self.in_nav_coverage(pt.ll[0], pt.ll[1]) {
                        truncated = true;
                        break;
                    }
                    ref_ll = Some(push_anchor(&mut anchors, pt));
                }
                if truncated {
                    drain_oceanic(&mut oceanic, i + 1);
                    break;
                }
                i += 1;
                continue;
            }

            // Procedure? Only where a SID/STAR is plausible: a token with a digit, or a
            // bare name at the route edges that isn't itself a known point.
            let has_digit = tok.bytes().any(|c| c.is_ascii_digit());
            let at_edge = i <= 1 || i + 2 >= tokens.len();
            let at_arrival_edge = i + 2 >= tokens.len();
            let known_point = self.navaids.contains_key(&tok)
                || self.fixes.contains_key(&tok)
                || airports.get(&tok).is_some();
            let proc = if has_digit || (at_edge && !known_point) {
                self.find_procedure(&tok)
            } else {
                None
            };
            if let Some(proc) = proc {
                if intl && at_arrival_edge && proc.ptype == ProcType::Star {
                    truncated = true;
                    oceanic.push(tok);
                    drain_oceanic(&mut oceanic, i + 1);
                    break;
                }
                let prev_tok = if i > 0 {
                    clean_token(&tokens[i - 1])
                } else {
                    String::new()
                };
                let transition = if proc.transitions.contains_key(&prev_tok) {
                    Some(prev_tok.as_str())
                } else {
                    None
                };
                for pt in self.expand_procedure(proc, transition) {
                    if intl && !self.in_nav_coverage(pt.ll[0], pt.ll[1]) {
                        truncated = true;
                        break;
                    }
                    ref_ll = Some(push_anchor(&mut anchors, pt));
                }
                if truncated {
                    drain_oceanic(&mut oceanic, i + 1);
                    break;
                }
                i += 1;
                continue;
            }

            // Plain fix/navaid/airport.
            match self.resolve_token(&tok, airports, ref_ll, &dep, &arr) {
                None => {
                    if intl {
                        truncated = true;
                        oceanic.push(tok);
                        drain_oceanic(&mut oceanic, i + 1);
                        break;
                    }
                    if airports.get(&tok).is_none() && tok.len() >= 2 {
                        unresolved.push(tok);
                    }
                }
                Some(resolved) => {
                    if intl && !self.in_nav_coverage(resolved.ll[0], resolved.ll[1]) {
                        truncated = true;
                        oceanic.push(tok);
                        drain_oceanic(&mut oceanic, i + 1);
                        break;
                    }
                    // Guard against a bad duplicate-name pick jumping across the country.
                    // Exempt explicit coordinates — long oceanic legs are legitimate.
                    if let Some(r) = ref_ll
                        && resolved.kind != Kind::Coord
                        && haversine_nm(r, resolved.ll) > 900.0
                    {
                        if intl {
                            truncated = true;
                            oceanic.push(tok);
                            drain_oceanic(&mut oceanic, i + 1);
                            break;
                        }
                        unresolved.push(tok);
                        i += 1;
                        continue;
                    }
                    ref_ll = Some(push_anchor(&mut anchors, resolved));
                }
            }
            i += 1;
        }

        if let Some(d) = destination
            && anchors.last().is_none_or(|l| l.ll != d)
        {
            anchors.push(Anchor {
                name: if arr.is_empty() {
                    "ARR".into()
                } else {
                    arr.clone()
                },
                ll: d,
                kind: Kind::Apt,
            });
        }

        // De-duplicate the unresolved list, preserving first-seen order.
        let mut seen = HashSet::new();
        unresolved.retain(|t| seen.insert(t.clone()));

        RouteResult {
            anchors,
            unresolved,
            oceanic_skipped: oceanic,
            truncated_international: truncated,
        }
    }

    /// Back-compat convenience: resolve a single token to a coordinate (airport → navaid
    /// → fix → procedure), nearest to `prev`.
    pub fn resolve(&self, token: &str, airports: &AirportDb, prev: Option<Ll>) -> Option<Ll> {
        self.resolve_token(token, airports, prev, "", "")
            .map(|a| a.ll)
    }
}

// --- helpers ---

fn clean_token(t: &str) -> String {
    t.split('/').next().unwrap_or("").to_ascii_uppercase()
}

fn proc_kind(proc: &Procedure) -> Kind {
    match proc.ptype {
        ProcType::Star => Kind::Star,
        ProcType::Sid => Kind::Sid,
    }
}

/// First usable leg of a procedure: common if it has ≥2 legs, else the first transition.
fn proc_first_leg(proc: &Procedure) -> Option<&Leg> {
    if proc.common.len() >= 2 {
        proc.common.first()
    } else {
        proc.transitions.values().next().and_then(|t| t.first())
    }
}

/// Concatenate a transition's legs with the procedure's common legs, avoiding a duplicate
/// join fix.
fn merge_procedure_legs(trans: &[Leg], common: &[Leg]) -> Vec<Leg> {
    if trans.is_empty() {
        return common.to_vec();
    }
    if common.is_empty() {
        return trans.to_vec();
    }
    let mut out = trans.to_vec();
    let last_fix = &trans[trans.len() - 1].0;
    let start = if &common[0].0 == last_fix { 1 } else { 0 };
    out.extend_from_slice(&common[start..]);
    out
}

fn push_anchor(anchors: &mut Vec<Anchor>, pt: Anchor) -> Ll {
    match anchors.last() {
        Some(l) if l.ll == pt.ll => l.ll,
        _ => {
            let ll = pt.ll;
            anchors.push(pt);
            ll
        }
    }
}

fn nearest(cands: &[Ll], refll: Option<Ll>) -> Ll {
    match refll {
        Some(r) if cands.len() > 1 => *cands
            .iter()
            .min_by(|a, b| {
                haversine_nm(r, **a)
                    .partial_cmp(&haversine_nm(r, **b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(&cands[0]),
        _ => cands[0],
    }
}

fn nearest_wp_index(wps: &[Leg], ll: Ll) -> usize {
    let mut best = 0;
    let mut bd = f64::MAX;
    for (i, w) in wps.iter().enumerate() {
        let d = haversine_nm(ll, [w.1, w.2]);
        if d < bd {
            bd = d;
            best = i;
        }
    }
    best
}

/// Decode an explicit lat/lon waypoint: `<lat><N|S><lon><E|W>` where each coordinate is
/// whole degrees or degrees-minutes (`34N150E`, `4130N07000W`). Returns `[lat, lon]`.
fn decode_latlon(id: &str) -> Option<Ll> {
    let b = id.as_bytes();
    let ns = b.iter().position(|&c| c == b'N' || c == b'S')?;
    let ew = b.iter().rposition(|&c| c == b'E' || c == b'W')?;
    // Structure: digits, N/S, digits, E/W at the very end.
    if ns == 0 || ew != b.len() - 1 || ew <= ns + 1 {
        return None;
    }
    let lat_s = &id[..ns];
    let lon_s = &id[ns + 1..ew];
    if !lat_s.bytes().all(|c| c.is_ascii_digit()) || !lon_s.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let mut lat = parse_deg_min(lat_s)?;
    let mut lon = parse_deg_min(lon_s)?;
    if b[ns] == b'S' {
        lat = -lat;
    }
    if b[ew] == b'W' {
        lon = -lon;
    }
    (lat.abs() <= 90.0 && lon.abs() <= 180.0).then_some([lat, lon])
}

/// Whole degrees (2–3 digits) or degrees+minutes (4–5 digits, `DDMM`/`DDDMM`).
fn parse_deg_min(s: &str) -> Option<f64> {
    match s.len() {
        2 | 3 => s.parse().ok(),
        4 => {
            let d: f64 = s[..2].parse().ok()?;
            let m: f64 = s[2..].parse().ok()?;
            Some(d + m / 60.0)
        }
        5 => {
            let d: f64 = s[..3].parse().ok()?;
            let m: f64 = s[3..].parse().ok()?;
            Some(d + m / 60.0)
        }
        _ => None,
    }
}

/// Great-circle destination `dist_nm` from `from` along `bearing_deg` (true).
fn project(from: Ll, bearing_deg: f64, dist_nm: f64) -> Ll {
    let ang = dist_nm / R_NM;
    let brg = bearing_deg.to_radians();
    let (lat1, lon1) = (from[0].to_radians(), from[1].to_radians());
    let lat2 = (lat1.sin() * ang.cos() + lat1.cos() * ang.sin() * brg.cos()).asin();
    let lon2 =
        lon1 + (brg.sin() * ang.sin() * lat1.cos()).atan2(ang.cos() - lat1.sin() * lat2.sin());
    [
        lat2.to_degrees(),
        (lon2.to_degrees() + 540.0) % 360.0 - 180.0,
    ]
}

fn haversine_nm(a: Ll, b: Ll) -> f64 {
    let (la1, lo1) = (a[0].to_radians(), a[1].to_radians());
    let (la2, lo2) = (b[0].to_radians(), b[1].to_radians());
    let dla = la2 - la1;
    let dlo = lo2 - lo1;
    let h = (dla / 2.0).sin().powi(2) + la1.cos() * la2.cos() * (dlo / 2.0).sin().powi(2);
    2.0 * R_NM * h.sqrt().asin()
}

/// Airway designators: US `J/Q/V/T`, oceanic/international `A/B/G/R/L/M/N/P/W/Y`, and
/// European upper `U[LMNPQT]`. Digits must follow the prefix immediately so 5-letter
/// fixes, procedures (`DOTSS2`) and NRS waypoints (`KD60U`) never match.
fn is_airway_token(id: &str) -> bool {
    let b = id.as_bytes();
    if b.len() < 2 {
        return false;
    }
    let mut k = match b[0] {
        b'A' | b'B' | b'G' | b'J' | b'L' | b'M' | b'N' | b'P' | b'Q' | b'R' | b'T' | b'V'
        | b'W' | b'Y' => 1,
        b'U' if matches!(b[1], b'L' | b'M' | b'N' | b'P' | b'Q' | b'T') => 2,
        _ => return false,
    };
    let mut digits = 0;
    while k < b.len() && b[k].is_ascii_digit() {
        k += 1;
        digits += 1;
    }
    if !(1..=4).contains(&digits) {
        return false;
    }
    if k < b.len() {
        if b[k].is_ascii_uppercase() {
            k += 1;
        } else {
            return false;
        }
    }
    k == b.len()
}

/// The letter prefix of a procedure token matching `^[A-Z]{3,6}\d[A-Z]?$` (e.g. `DOTSS2`
/// → `DOTSS`), or None.
fn proc_prefix(id: &str) -> Option<String> {
    let b = id.as_bytes();
    let mut k = 0;
    while k < b.len() && b[k].is_ascii_uppercase() {
        k += 1;
    }
    let letters = k;
    if !(3..=6).contains(&letters) || k >= b.len() || !b[k].is_ascii_digit() {
        return None;
    }
    k += 1;
    if k < b.len() {
        if b[k].is_ascii_uppercase() {
            k += 1;
        } else {
            return None;
        }
    }
    (k == b.len()).then(|| id[..letters].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[tokio::test]
    #[ignore = "network: fetches live NASR and resolves Hawaii route tokens (worldwide coverage)"]
    async fn resolves_hawaii_route_tokens() {
        let nav = crate::feed::nav_source::fetch_latest()
            .await
            .expect("fetch_latest");
        let ap: AirportDb = HashMap::new();
        let hilo = Some([19.72, -155.05]); // reference so duplicate names disambiguate to Hawaii
        for tok in ["KOA031037", "KENNZ", "MKK257006", "PARIS", "LYCHI1"] {
            let ll = nav.resolve(tok, &ap, hilo);
            assert!(ll.is_some(), "{tok} should resolve now");
            let [lat, lon] = ll.unwrap();
            assert!(
                (17.0..24.0).contains(&lat) && (-162.0..-153.0).contains(&lon),
                "{tok} resolved outside Hawaii: {lat},{lon}"
            );
        }
    }

    #[test]
    fn knows_flags_unknown_route_fixes() {
        let navaids = r#"{"RDU":[[35.9,-78.8]]}"#;
        let fixes = r#"{"MLLET":[[35.0,-80.0]],"STOCR":[[34.0,-81.0]]}"#;
        let nav = NavData::from_json(navaids, fixes, "{}", "{}", "{}", "{}", "{}");

        assert!(nav.knows("MLLET"), "real fix");
        assert!(nav.knows("mllet"), "case-insensitive");
        assert!(
            nav.knows("MLLET5"),
            "STAR name maps to its fix via digit strip"
        );
        assert!(nav.knows("STOCR"));
        assert!(nav.knows("RDU"), "navaid");
        assert!(!nav.knows("MLLETT"), "the typo from the bug report");
        assert!(!nav.knows("ZZZZZ"), "not a fix");
        assert!(!nav.knows(""), "empty");
    }

    #[test]
    fn airway_and_proc_token_classification() {
        for t in ["J121", "Q22", "V16", "T449", "UL10", "A509"] {
            assert!(is_airway_token(t), "{t} should be an airway");
        }
        for t in ["DOTSS2", "LUCIT3", "KD60U", "MERIT", "SIE", "ULM", "ATA315"] {
            assert!(!is_airway_token(t), "{t} should NOT be an airway");
        }
        assert_eq!(proc_prefix("DOTSS2").as_deref(), Some("DOTSS"));
        assert_eq!(proc_prefix("LUCIT3"), Some("LUCIT".to_string()));
        assert_eq!(proc_prefix("SIE"), None);
        assert_eq!(proc_prefix("J121"), None);
    }

    #[test]
    fn expands_a_real_airway() {
        let nav = NavData::load();
        // J10: LAX → JUGLI → CIVET → RUSTT → PIONE → TNP → ... (US high airway).
        let full = nav.expand_airway("J10", None, None);
        assert!(full.len() >= 6, "full airway returns its waypoint chain");
        assert_eq!(full[0].name, "LAX");
        assert_eq!(full[1].name, "JUGLI");
        // A sub-span between the 2nd and 5th waypoints keeps only that inclusive slice.
        let from = [full[1].ll[0], full[1].ll[1]];
        let to = [full[4].ll[0], full[4].ll[1]];
        let span = nav.expand_airway("J10", Some(from), Some(to));
        assert_eq!(span.len(), 4);
        assert_eq!(span.first().unwrap().name, "JUGLI");
        assert_eq!(span.last().unwrap().name, "PIONE");
    }

    #[test]
    fn expands_a_real_star_with_transition() {
        let nav = NavData::load();
        let proc = nav.find_procedure("LUCIT3").expect("LUCIT3 STAR resolves");
        assert_eq!(proc.ptype, ProcType::Star);
        // COOKS transition then common legs, ending at the runway-side common fix.
        let legs = nav.expand_procedure(proc, Some("COOKS"));
        assert!(legs.len() >= 3);
        assert_eq!(legs.first().unwrap().name, "COOKS");
        assert!(legs.iter().any(|l| l.name == "LUCIT"));
    }

    #[test]
    fn build_anchors_expands_airway_between_fixes() {
        let nav = NavData::load();
        let ap: AirportDb = HashMap::new();
        // Bare navaid→navaid vs. via the airway should differ: the airway inserts its
        // intermediate waypoints between the endpoints.
        let direct = nav.build_anchors(&ap, "", "", "LAX TNP");
        let via = nav.build_anchors(&ap, "", "", "LAX J10 TNP");
        assert!(
            via.anchors.len() > direct.anchors.len(),
            "airway expansion should add intermediate waypoints ({} vs {})",
            via.anchors.len(),
            direct.anchors.len()
        );
        assert!(via.unresolved.is_empty(), "airway must not be unresolved");
    }

    #[test]
    fn build_anchors_never_panics_on_garbage() {
        let nav = NavData::load();
        let ap: AirportDb = HashMap::new();
        let garbage = [
            "",
            "     ",
            "DCT DCT DCT",
            "AAAAAAAAAAAAAAAAAAAA",
            "123456789012345",
            "N",
            "999999999",
            "//// / /..//",
            "A1 B22 Q99999",
            "RBV999999 RBV000000 RBV060000",
            "34N 150E N150E 91N200E",
            "😀FIX RBV\u{0301}",
            "RBV060013 34N150E VFR IFR SVFR DOTSS2 J121 LUCIT3",
            "J Q V T A B G R Y",
        ];
        for r in garbage {
            // Must not panic for any dep/arr combination.
            let _ = nav.build_anchors(&ap, "KJFK", "KLAX", r);
            let _ = nav.build_anchors(&ap, "", "", r);
            let _ = nav.build_anchors(&ap, "RJAA", "PHNL", r);
        }
    }

    #[test]
    fn resolves_fix_radial_distance() {
        let nav = NavData::load();
        let empty = HashMap::new();
        // Robbinsville VOR carries a published 10°W variation.
        assert_eq!(nav.magvar("RBV"), Some(-10.0));
        let rbv = nav.resolve("RBV", &empty, None).expect("RBV resolves");
        // RBV060013 = 13 nm out on the (magnetic) 060 radial.
        let frd = nav
            .resolve("RBV060013", &empty, None)
            .expect("FRD resolves");
        let d = haversine_nm(rbv, frd);
        assert!((d - 13.0).abs() < 0.6, "expected ~13 nm from RBV, got {d}");
        // Variation is applied: true bearing = 060 + (-10) = 050°, not 060°.
        let brg = bearing_deg(rbv[0], rbv[1], frd[0], frd[1]);
        assert!((brg - 50.0).abs() < 2.0, "expected ~050° true, got {brg}");
        // A radial > 360 is not a valid FRD.
        assert!(nav.resolve("RBV999013", &empty, None).is_none());
        // A plain 5-letter fix is untouched.
        assert!(nav.resolve("MERIT", &empty, None).is_some());
    }

    #[test]
    fn decodes_latlon_waypoints() {
        assert_eq!(decode_latlon("34N150E"), Some([34.0, 150.0]));
        assert_eq!(decode_latlon("41N140W"), Some([41.0, -140.0]));
        assert_eq!(decode_latlon("30S170W"), Some([-30.0, -170.0]));
        // Degrees-minutes form.
        let dm = decode_latlon("4130N07000W").unwrap();
        assert!((dm[0] - 41.5).abs() < 1e-9 && (dm[1] + 70.0).abs() < 1e-9);
        // Not coordinates.
        assert_eq!(decode_latlon("MERIT"), None);
        assert_eq!(decode_latlon("RBV060013"), None);
        assert_eq!(decode_latlon("91N010E"), None); // lat out of range
    }

    /// A local bearing helper for the FRD test (nav.rs keeps geometry in fca.rs/winds.rs).
    fn bearing_deg(la1: f64, lo1: f64, la2: f64, lo2: f64) -> f64 {
        let dlo = (lo2 - lo1).to_radians();
        let y = dlo.sin() * la2.to_radians().cos();
        let x = la1.to_radians().cos() * la2.to_radians().sin()
            - la1.to_radians().sin() * la2.to_radians().cos() * dlo.cos();
        (y.atan2(x).to_degrees() + 360.0) % 360.0
    }

    #[test]
    fn preferred_route_substituted_for_bare_filing() {
        let nav = NavData::load();
        // ABE|ACY has a preferred route "FJC ARD CYN"; a bare airport-to-airport filing
        // should pick it up (all three fixes resolve).
        let ap = HashMap::from([
            ("KABE".to_string(), Airport::at(40.65, -75.44)),
            ("KACY".to_string(), Airport::at(39.46, -74.58)),
        ]);
        // Preferred keys are 3-letter (FAA) codes; emulate with matching dep/arr.
        let res = nav.build_anchors(&ap, "ABE", "ACY", "");
        // With no origin/destination airports in `ap` for 3-letter codes, anchors come
        // purely from the substituted preferred route fixes.
        assert!(
            res.anchors.len() >= 3,
            "preferred route should expand to its fixes, got {}",
            res.anchors.len()
        );
    }
}
