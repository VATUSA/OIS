//! METAR fetch + parse for the Runway Balancer's wind/flight-category banner. Fetched
//! server-side (the AWC API sends no CORS header) and cached per airport. Ported from
//! vatflow's `flightCategory`/`fetchMetar`.

/// Parsed METAR essentials.
#[derive(Debug, Clone)]
pub struct MetarInfo {
    pub raw: String,
    /// `VFR` | `MVFR` | `IFR` | `LIFR`.
    pub category: String,
    /// Human wind, e.g. `270@15G25kt`, if present. Rendered verbatim by the Runway Balancer banner, so
    /// its formatting is a wire format in practice — derived from [`MetarInfo::wind_obs`] rather than
    /// parsed separately, so the two can never disagree.
    pub wind: Option<String>,
    /// The same wind as numbers, for rules that have to compute with it (#510). Separate from the
    /// display string because `RunwayBoard` exposes only the string, and this stays internal.
    pub wind_obs: Option<MetarWind>,
}

/// An observed surface wind, in the units the METAR states it: degrees true and knots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetarWind {
    /// Degrees true, or `None` for a variable (`VRB`) wind — which has no single direction to match a
    /// runway configuration against, so a caller must treat it like calm rather than guess one.
    pub dir: Option<i32>,
    pub spd_kt: i32,
    pub gust_kt: Option<i32>,
}

impl MetarWind {
    /// The banner string, e.g. `270@15G25kt` or `VRB@3kt`.
    fn display(&self) -> String {
        let dir = match self.dir {
            Some(d) => format!("{d:03}"),
            None => "VRB".to_string(),
        };
        match self.gust_kt {
            Some(g) => format!("{dir}@{}G{g}kt", self.spd_kt),
            None => format!("{dir}@{}kt", self.spd_kt),
        }
    }
}

/// Fetch the raw METAR for `icao` and parse category + wind.
pub async fn fetch_one(client: &reqwest::Client, icao: &str) -> Option<MetarInfo> {
    let url = format!("https://aviationweather.gov/api/data/metar?ids={icao}&format=raw");
    let text = client
        .get(&url)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .text()
        .await
        .ok()?;
    let raw = text
        .lines()
        .find(|l| l.trim_start().starts_with(icao))
        .or_else(|| text.lines().next())?
        .trim()
        .to_string();
    if raw.len() < 4 {
        return None;
    }
    let wind_obs = parse_wind(&raw);
    Some(MetarInfo {
        category: flight_category(&raw).to_string(),
        wind: wind_obs.as_ref().map(MetarWind::display),
        wind_obs,
        raw,
    })
}

/// Flight category from ceiling + visibility (LIFR/IFR/MVFR/VFR).
pub fn flight_category(metar: &str) -> &'static str {
    let mut vis = 10.0_f64;
    let mut ceil = 99_999_i32;
    let toks: Vec<&str> = metar.split_whitespace().collect();
    for (i, tok) in toks.iter().enumerate() {
        if let Some(v) = parse_vis(tok) {
            // US METARs split mixed fractions across two tokens: `1 1/2SM`. When this
            // fraction token is preceded by a bare integer, add the whole part back in.
            let whole = if tok.contains('/') && i > 0 {
                toks[i - 1].parse::<f64>().ok()
            } else {
                None
            };
            vis = v + whole.unwrap_or(0.0);
        }
        if let Some(c) = parse_ceiling(tok) {
            ceil = ceil.min(c);
        }
    }
    if ceil < 500 || vis < 1.0 {
        "LIFR"
    } else if ceil < 1000 || vis < 3.0 {
        "IFR"
    } else if ceil <= 3000 || vis <= 5.0 {
        "MVFR"
    } else {
        "VFR"
    }
}

/// Visibility in statute miles from a `…SM` token (`10SM`, `1/2SM`, `M1/4SM`).
fn parse_vis(tok: &str) -> Option<f64> {
    let t = tok.strip_suffix("SM")?;
    let t = t.strip_prefix('M').unwrap_or(t); // "M" = less than
    match t.split_once('/') {
        Some((a, b)) => Some(a.parse::<f64>().ok()? / b.parse::<f64>().ok()?),
        None => t.parse().ok(),
    }
}

/// Ceiling in feet from a `BKN###`/`OVC###`/`VV###` token.
fn parse_ceiling(tok: &str) -> Option<i32> {
    for pfx in ["BKN", "OVC", "VV"] {
        if let Some(rest) = tok.strip_prefix(pfx) {
            let d = rest.get(..3)?;
            if d.bytes().all(|b| b.is_ascii_digit()) {
                return Some(d.parse::<i32>().ok()? * 100);
            }
        }
    }
    None
}

/// Human wind from a `…KT` token (`10004KT`, `27015G25KT`, `VRB03KT`).
/// The observed wind group, as numbers.
///
/// Returned structured rather than pre-formatted (#510): the direction is needed to match an airport
/// configuration, and this function already had it — the previous version computed `dir`, `spd` and
/// `gust` and then discarded them into a display string.
fn parse_wind(metar: &str) -> Option<MetarWind> {
    for tok in metar.split_whitespace() {
        let Some(w) = tok.strip_suffix("KT") else {
            continue;
        };
        // `w.get(..3)` (not `w[..3]`) so a non-ASCII byte boundary yields None, not a panic.
        let (dir, rest) = if let Some(r) = w.strip_prefix("VRB") {
            ("VRB", r)
        } else if let (Some(d), Some(r)) = (w.get(..3), w.get(3..))
            && d.bytes().all(|b| b.is_ascii_digit())
        {
            (d, r)
        } else {
            continue;
        };
        let (spd, gust) = match rest.split_once('G') {
            Some((s, g)) => (s, g.parse::<i32>().ok()),
            None => (rest, None),
        };
        // A malformed speed skips this token rather than abandoning the whole search.
        let Ok(spd) = spd.parse::<i32>() else {
            continue;
        };
        return Some(MetarWind {
            // `VRB` is a real reading, not a parse failure: the wind is variable, so there is no
            // direction. Kept as `None` so a caller cannot mistake a placeholder for a bearing.
            dir: dir.parse::<i32>().ok(),
            spd_kt: spd,
            gust_kt: gust,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_from_ceiling_and_vis() {
        assert_eq!(flight_category("KJFK 10SM BKN250 27/16"), "VFR");
        assert_eq!(flight_category("KJFK 4SM BKN050"), "MVFR");
        assert_eq!(flight_category("KJFK 2SM OVC008"), "IFR");
        assert_eq!(flight_category("KJFK 1/2SM OVC003"), "LIFR");
        assert_eq!(flight_category("KJFK 10SM OVC008"), "IFR"); // ceiling-only (800 ft), good vis
    }

    /// The display string every one of these used to assert is now derived from the numbers, so both
    /// are pinned: a change to one that does not reach the other fails here.
    #[test]
    fn winds_parse() {
        let w = parse_wind("KJFK 10004KT 10SM").expect("a wind group");
        assert_eq!((w.dir, w.spd_kt, w.gust_kt), (Some(100), 4, None));
        assert_eq!(w.display(), "100@4kt");

        let g = parse_wind("KJFK 27015G25KT").expect("a gusting wind");
        assert_eq!((g.dir, g.spd_kt, g.gust_kt), (Some(270), 15, Some(25)));
        assert_eq!(g.display(), "270@15G25kt");

        // `VRB` is a reading, not a failure: variable wind has no direction to match a configuration
        // against, so `dir` is None and a caller must treat it as it treats calm.
        let v = parse_wind("KJFK VRB03KT").expect("a variable wind");
        assert_eq!((v.dir, v.spd_kt, v.gust_kt), (None, 3, None));
        assert_eq!(v.display(), "VRB@3kt");

        assert_eq!(parse_wind("KJFK 10SM CLR"), None, "no wind group at all");
    }

    /// AC2's remaining cases for #510.
    #[test]
    fn winds_parse_calm_and_malformed() {
        // Calm is reported as a real group with zero speed, not as an absent one.
        let calm = parse_wind("KJFK 00000KT 10SM").expect("calm is still a wind group");
        assert_eq!((calm.dir, calm.spd_kt, calm.gust_kt), (Some(0), 0, None));
        assert_eq!(calm.display(), "000@0kt");

        // A malformed group is skipped, and a later well-formed one still parses — a junk token must
        // not cost us the observation.
        let after_junk = parse_wind("KJFK ///KT 27010KT").expect("the valid group after the junk");
        assert_eq!(after_junk.dir, Some(270));
        assert_eq!(
            parse_wind("KJFK 270XXKT"),
            None,
            "unparseable speed yields nothing"
        );
        assert_eq!(
            parse_wind("KJFK 27015G/KT").map(|w| w.gust_kt),
            Some(None),
            "an unreadable gust drops the gust, keeping the direction and speed"
        );
    }

    #[test]
    fn mixed_fraction_visibility() {
        // `1 1/2SM` is two tokens; the whole part must be added back (1.5 SM → IFR, not LIFR).
        assert_eq!(flight_category("KJFK 1 1/2SM OVC020"), "IFR");
        // A bare fraction stays as-is (0.5 SM → LIFR).
        assert_eq!(flight_category("KJFK 1/2SM OVC020"), "LIFR");
        // `3 1/2SM` → 3.5 SM (no ceiling layer) → MVFR.
        assert_eq!(flight_category("KJFK 3 1/2SM SCT030"), "MVFR");
    }

    #[test]
    fn wind_parser_is_panic_free_and_skips_junk() {
        // A `KT` token split on a non-ASCII byte boundary must not panic (get(), not [..]).
        assert_eq!(parse_wind("KJFK 12€KT 10SM"), None);
        // A malformed leading wind token must not abort the search for a later valid one.
        let w = parse_wind("KJFK 100XXKT 27015KT").expect("the later valid group");
        assert_eq!((w.dir, w.spd_kt), (Some(270), 15));
        assert_eq!(w.display(), "270@15kt");
    }
}
