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

    /// One reference case from `fixtures/ntml-reference.json`. The same file drives
    /// `web/src/lib/ntml.test.ts`, so a case is written once and the two implementations of this
    /// grammar cannot drift apart without a test failing on one side or the other.
    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        restriction: NtmlRestriction,
        raw: String,
        english: String,
    }

    #[derive(serde::Deserialize)]
    struct Fixtures {
        cases: Vec<Case>,
    }

    /// Baked in at compile time, so there is no runtime path to get wrong and no way to run the
    /// suite against a fixture file that isn't there.
    const FIXTURES: &str = include_str!("../../fixtures/ntml-reference.json");

    #[test]
    fn matches_every_shared_reference_case() {
        let fixtures: Fixtures =
            serde_json::from_str(FIXTURES).expect("fixtures/ntml-reference.json is not valid JSON");
        assert!(
            !fixtures.cases.is_empty(),
            "the fixture file parsed but held no cases, so this test would pass by asserting nothing"
        );

        for case in &fixtures.cases {
            assert_eq!(
                encode(&case.restriction),
                case.raw,
                "encoded line disagrees with the shared fixture for case '{}'",
                case.name
            );
            assert_eq!(
                render_english(&case.restriction),
                case.english,
                "English rendering disagrees with the shared fixture for case '{}'",
                case.name
            );
        }
    }
}
