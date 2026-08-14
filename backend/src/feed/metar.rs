//! METAR fetch + parse for the Runway Balancer's wind/flight-category banner. Fetched
//! server-side (the AWC API sends no CORS header) and cached per airport. Ported from
//! vatflow's `flightCategory`/`fetchMetar`.

/// Parsed METAR essentials.
#[derive(Debug, Clone)]
pub struct MetarInfo {
    pub raw: String,
    /// `VFR` | `MVFR` | `IFR` | `LIFR`.
    pub category: String,
    /// Human wind, e.g. `270@15G25kt`, if present.
    pub wind: Option<String>,
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
    Some(MetarInfo {
        category: flight_category(&raw).to_string(),
        wind: parse_wind(&raw),
        raw,
    })
}

/// Flight category from ceiling + visibility (LIFR/IFR/MVFR/VFR).
pub fn flight_category(metar: &str) -> &'static str {
    let mut vis = 10.0_f64;
    let mut ceil = 99_999_i32;
    for tok in metar.split_whitespace() {
        if let Some(v) = parse_vis(tok) {
            vis = v;
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
fn parse_wind(metar: &str) -> Option<String> {
    for tok in metar.split_whitespace() {
        let Some(w) = tok.strip_suffix("KT") else {
            continue;
        };
        let (dir, rest) = if let Some(r) = w.strip_prefix("VRB") {
            ("VRB", r)
        } else if w.len() >= 3 && w[..3].bytes().all(|b| b.is_ascii_digit()) {
            (&w[..3], &w[3..])
        } else {
            continue;
        };
        let (spd, gust) = match rest.split_once('G') {
            Some((s, g)) => (s, g.parse::<i32>().ok()),
            None => (rest, None),
        };
        let spd: i32 = spd.parse().ok()?;
        return Some(match gust {
            Some(g) => format!("{dir}@{spd}G{g}kt"),
            None => format!("{dir}@{spd}kt"),
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

    #[test]
    fn winds_parse() {
        assert_eq!(parse_wind("KJFK 10004KT 10SM").as_deref(), Some("100@4kt"));
        assert_eq!(
            parse_wind("KJFK 27015G25KT").as_deref(),
            Some("270@15G25kt")
        );
        assert_eq!(parse_wind("KJFK VRB03KT").as_deref(), Some("VRB@3kt"));
        assert_eq!(parse_wind("KJFK 10SM CLR"), None);
    }
}
