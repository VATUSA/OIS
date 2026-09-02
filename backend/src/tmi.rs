//! NTML restriction codec. Turns a structured [`NtmlRestriction`] (the form-built TMI) into the
//! canonical raw NTML line stored on `tmu.tmis.restriction`, and into the plain-English `decoded`
//! rendering pilots read. Modelled on the vATCSCC/FAA NTML entry breakdown (see docs/TMIs). The
//! requesting/providing facilities and the valid window live on the TMI row, not in the line here.

use crate::models::{Bound, NtmlRestriction};

fn clean(s: &str) -> String {
    s.trim().to_ascii_uppercase()
}

fn opt(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|t| !t.is_empty())
}

/// Encode the canonical raw NTML line, e.g.
/// `JFK arrivals via CAMRN 20MIT NO STACKS TYPE:ALL SPD:≤210 ALT:AOB090 VOLUME:VOLUME EXCL:PHL`.
pub fn encode(r: &NtmlRestriction) -> String {
    let mut parts: Vec<String> = vec![clean(&r.element)];

    match r.direction.trim().to_ascii_lowercase().as_str() {
        "arrivals" => parts.push("arrivals".into()),
        "departures" => parts.push("departures".into()),
        _ => {} // enroute — no direction word (e.g. "PHL via J152 STOP")
    }
    if let Some(via) = opt(&r.via) {
        parts.push(format!("via {}", clean(via)));
    }

    let kind = clean(&r.kind);
    match kind.as_str() {
        "MIT" | "MINIT" => parts.push(format!("{}{kind}", r.value.unwrap_or(0))),
        "TXT" => {
            if let Some(t) = opt(&r.text) {
                parts.push(t.to_string());
            }
        }
        other => parts.push(other.to_string()), // STOP, DSP, APREQ, TBM, CFR
    }

    if let Some(q) = opt(&r.qualifier) {
        parts.push(clean(q));
    }
    if let Some(a) = opt(&r.aircraft) {
        parts.push(format!("TYPE:{}", clean(a)));
    }
    if let Some(s) = &r.speed {
        parts.push(format!("SPD:{}{}", s.op.trim(), s.value));
    }
    if let Some(a) = &r.altitude {
        parts.push(format!("ALT:{}{:03}", clean(&a.op), a.value));
    }
    if let Some(c) = opt(&r.condition) {
        match opt(&r.condition_detail) {
            Some(d) => parts.push(format!("{}:{}", clean(c), clean(d))),
            None => parts.push(clean(c)),
        }
    }
    let excl: Vec<String> = r
        .exclude
        .iter()
        .map(|e| clean(e))
        .filter(|e| !e.is_empty())
        .collect();
    if !excl.is_empty() {
        parts.push(format!("EXCL:{}", excl.join(",")));
    }

    parts.join(" ")
}

fn speed_english(b: &Bound) -> String {
    let lead = match b.op.trim() {
        "≤" | "<=" => "at or below ",
        "≥" | ">=" => "at or above ",
        _ => "at ",
    };
    format!("{lead}{}kt", b.value)
}

fn altitude_english(b: &Bound) -> String {
    let lead = match clean(&b.op).as_str() {
        "AOB" => "at or below ",
        "AOA" => "at or above ",
        _ => "at ",
    };
    format!("{lead}FL{:03}", b.value)
}

fn condition_english(cat: &str, detail: Option<&str>) -> String {
    let base = cat.trim().to_ascii_lowercase();
    match detail {
        Some(d) if !d.eq_ignore_ascii_case(cat) => {
            format!("{base} ({})", d.trim().to_ascii_lowercase())
        }
        _ => base,
    }
}

/// Render the structured restriction to a plain-English sentence for pilots, e.g.
/// `JFK arrivals via CAMRN: 20 miles-in-trail (no stacks, all aircraft, at or below 210kt, at or
/// below FL090) — due to volume; excluding PHL`.
pub fn render_english(r: &NtmlRestriction) -> String {
    let mut s = clean(&r.element);
    match r.direction.trim().to_ascii_lowercase().as_str() {
        "arrivals" => s.push_str(" arrivals"),
        "departures" => s.push_str(" departures"),
        _ => {}
    }
    if let Some(via) = opt(&r.via) {
        s.push_str(&format!(" via {}", clean(via)));
    }
    s.push_str(": ");

    let kind = clean(&r.kind);
    s.push_str(&match kind.as_str() {
        "MIT" => format!("{} miles-in-trail", r.value.unwrap_or(0)),
        "MINIT" => format!("{} minutes-in-trail", r.value.unwrap_or(0)),
        "STOP" => "stop".into(),
        "APREQ" => "approval request (APREQ)".into(),
        "CFR" => "call-for-release (CFR)".into(),
        "DSP" => "departure spacing program (DSP)".into(),
        "TBM" => "time-based metering (TBM)".into(),
        "TXT" => opt(&r.text).unwrap_or("free-text restriction").to_string(),
        other => other.to_lowercase(),
    });

    let mut mods: Vec<String> = Vec::new();
    if let Some(q) = opt(&r.qualifier) {
        mods.push(q.to_lowercase());
    }
    if let Some(a) = opt(&r.aircraft) {
        mods.push(match clean(a).as_str() {
            "ALL" => "all aircraft".into(),
            "JET" => "jets only".into(),
            "PROP" => "props only".into(),
            "TURBOPROP" => "turboprops only".into(),
            other => other.to_lowercase(),
        });
    }
    if let Some(sp) = &r.speed {
        mods.push(speed_english(sp));
    }
    if let Some(al) = &r.altitude {
        mods.push(altitude_english(al));
    }
    if !mods.is_empty() {
        s.push_str(&format!(" ({})", mods.join(", ")));
    }

    if let Some(c) = opt(&r.condition) {
        s.push_str(&format!(
            " — due to {}",
            condition_english(c, opt(&r.condition_detail))
        ));
    }
    let excl: Vec<String> = r
        .exclude
        .iter()
        .map(|e| clean(e))
        .filter(|e| !e.is_empty())
        .collect();
    if !excl.is_empty() {
        s.push_str(&format!("; excluding {}", excl.join(", ")));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bound(op: &str, value: i32) -> Bound {
        Bound {
            op: op.into(),
            value,
        }
    }

    /// Base restriction with everything empty/none, for tests to tweak.
    fn base(element: &str, direction: &str, kind: &str) -> NtmlRestriction {
        NtmlRestriction {
            element: element.into(),
            direction: direction.into(),
            via: None,
            kind: kind.into(),
            value: None,
            text: None,
            qualifier: None,
            aircraft: None,
            speed: None,
            altitude: None,
            condition: None,
            condition_detail: None,
            exclude: Vec::new(),
        }
    }

    #[test]
    fn encodes_the_camrn_mit_example() {
        let r = NtmlRestriction {
            via: Some("CAMRN".into()),
            value: Some(20),
            qualifier: Some("NO STACKS".into()),
            aircraft: Some("ALL".into()),
            speed: Some(bound("≤", 210)),
            altitude: Some(bound("AOB", 90)),
            condition: Some("VOLUME".into()),
            condition_detail: Some("VOLUME".into()),
            exclude: vec!["PHL".into()],
            ..base("JFK", "arrivals", "MIT")
        };
        assert_eq!(
            encode(&r),
            "JFK arrivals via CAMRN 20MIT NO STACKS TYPE:ALL SPD:≤210 ALT:AOB090 VOLUME:VOLUME EXCL:PHL"
        );
    }

    #[test]
    fn encodes_the_enroute_stop_example() {
        let r = NtmlRestriction {
            via: Some("J152".into()),
            aircraft: Some("ALL".into()),
            condition: Some("WEATHER".into()),
            condition_detail: Some("THUNDERSTORMS".into()),
            exclude: vec!["PNE".into()],
            ..base("PHL", "enroute", "STOP")
        };
        assert_eq!(
            encode(&r),
            "PHL via J152 STOP TYPE:ALL WEATHER:THUNDERSTORMS EXCL:PNE"
        );
    }

    #[test]
    fn encodes_the_per_airport_departure_example() {
        let r = NtmlRestriction {
            via: Some("BIGGY".into()),
            value: Some(15),
            qualifier: Some("PER AIRPORT".into()),
            aircraft: Some("JET".into()),
            condition: Some("VOLUME".into()),
            condition_detail: Some("VOLUME".into()),
            ..base("EWR,LGA", "departures", "MIT")
        };
        assert_eq!(
            encode(&r),
            "EWR,LGA departures via BIGGY 15MIT PER AIRPORT TYPE:JET VOLUME:VOLUME"
        );
    }

    #[test]
    fn renders_readable_english() {
        let r = NtmlRestriction {
            via: Some("CAMRN".into()),
            value: Some(20),
            qualifier: Some("NO STACKS".into()),
            aircraft: Some("ALL".into()),
            speed: Some(bound("≤", 210)),
            altitude: Some(bound("AOB", 90)),
            condition: Some("VOLUME".into()),
            condition_detail: Some("VOLUME".into()),
            exclude: vec!["PHL".into()],
            ..base("JFK", "arrivals", "MIT")
        };
        assert_eq!(
            render_english(&r),
            "JFK arrivals via CAMRN: 20 miles-in-trail (no stacks, all aircraft, at or below 210kt, at or below FL090) — due to volume; excluding PHL"
        );
    }
}
