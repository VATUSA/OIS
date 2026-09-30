//! NTML restriction codec. Turns a structured [`NtmlRestriction`] (the form-built TMI) into the
//! canonical raw NTML line stored on `tmu.tmis.restriction`, and into the plain-English `decoded`
//! rendering pilots read. Modelled on the vATCSCC/FAA NTML entry breakdown (see docs/TMIs).
//!
//! [`encode`] deliberately covers only the **restriction** — the requesting/providing facilities and
//! the valid window live on the TMI row, not inside that clause. [`ntml_line`] is what assembles the
//! whole row into the line the NTML channel actually carries, so exactly one place knows the full
//! shape (#436). Before it existed the bot improvised a bold `**N90 → ZNY**` header and the log time
//! and valid window were simply absent.
//!
//! # The codec is deliberately one-way
//!
//! There is no `parse`/`from_raw`, so a raw-typed TMI keeps `structured` null and anything wanting
//! fields gets nothing rather than a guess. That is a decision, not an omission, and the reason is
//! in the grammar below: two positions emit bare, unprefixed words.
//!
//! - A `qualifier` (`NO STACKS`) and a `condition` carrying no detail (`VOLUME`) are both
//!   written as plain uppercase text, in that order. Given `20MIT NO STACKS VOLUME`, nothing in
//!   the line says whether the qualifier is `NO STACKS` and the condition `VOLUME`, or whether
//!   the qualifier is the whole `NO STACKS VOLUME`. Only a closed vocabulary of conditions could
//!   split it, and the vocabulary is open — controllers name the cause of the day.
//! - `TXT` writes its free text verbatim into the slot a kind keyword occupies, so `PHL STOP NOW`
//!   is either `kind: STOP` qualified `NOW`, or `kind: TXT` with the text `STOP NOW`.
//!
//! Round-tripping the *line* does not rule either out, which is the trap worth naming: a parser
//! that folded a detail-less `VOLUME` into `qualifier` would satisfy `encode(parse(line)) == line`
//! while putting the value in the wrong field. Anything built here has to round-trip the *struct* —
//! `parse(encode(r)) == r` over the reference examples in `tests` — and refuse a line it cannot
//! place rather than filling fields on a best guess.
//!
//! Raw entry exists precisely because the grammar cannot express everything a controller needs to
//! say. Inferring a breakdown for text chosen to escape the grammar is how a confidently wrong
//! breakdown ships, which is worse than admitting there isn't one. If a parser does become worth
//! building, the prefixed half of the grammar (`nMIT`/`nMINIT`, `via`, `TYPE:`, `SPD:`, `ALT:`,
//! `EXCL:`, and `CONDITION:DETAIL`) is unambiguous on its own; the two bare positions above are
//! what needs settling first.

use chrono::{DateTime, Utc};

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

/// The facility token that closes an NTML row: `REQUESTING:PROVIDING`, e.g. `N90:ZNY`.
///
/// Either side can be missing — `update_tmi`, unlike `create_tmi`, does not reject a blank facility —
/// so this degrades to whichever is known rather than emitting a stray colon.
fn facilities(requesting: Option<&str>, providing: Option<&str>) -> Option<String> {
    match (
        opt(&requesting.map(str::to_string)),
        opt(&providing.map(str::to_string)),
    ) {
        (Some(req), Some(prov)) => Some(format!("{}:{}", clean(req), clean(prov))),
        (Some(req), None) => Some(clean(req)),
        (None, Some(prov)) => Some(clean(prov)),
        (None, None) => None,
    }
}

/// `DDHHMM`-style log stamp: day of month and Zulu time, e.g. `14/1442`.
fn log_stamp(at: DateTime<Utc>) -> String {
    at.format("%d/%H%M").to_string()
}

/// The valid window, `HHMM-HHMM` Zulu. Omitted unless both ends are known — a half-open window is
/// more misleading in a log line than no window at all.
fn window(start: Option<DateTime<Utc>>, stop: Option<DateTime<Utc>>) -> Option<String> {
    match (start, stop) {
        (Some(a), Some(b)) => Some(format!("{}-{}", a.format("%H%M"), b.format("%H%M"))),
        _ => None,
    }
}

/// The complete NTML row as the channel carries it (#436):
///
/// ```text
/// 14/1442 JFK arrivals via CAMRN 20MIT NO STACKS TYPE:ALL SPD:≤210 ALT:AOB090 VOLUME:VOLUME EXCL:PHL 2015-2315 N90:ZNY
/// ```
///
/// `restriction` is whatever is stored on the row, which is [`encode`]'s output for a form-built TMI
/// and the typed text for a raw one — so both produce the same line, which is the point.
pub fn ntml_line(
    logged_at: DateTime<Utc>,
    restriction: &str,
    start: Option<DateTime<Utc>>,
    stop: Option<DateTime<Utc>>,
    requesting: Option<&str>,
    providing: Option<&str>,
) -> String {
    let mut parts = vec![log_stamp(logged_at), restriction.trim().to_string()];
    parts.extend(window(start, stop));
    parts.extend(facilities(requesting, providing));
    parts.join(" ")
}

/// The NTML row for a cancellation:
///
/// ```text
/// 25/0210 CVG via ALL CANCEL TMI ZID:ZTL
/// ```
///
/// A cancel is its own row rather than an edit of the original — the channel is a chronological log,
/// and the original entry did happen. No valid window: the restriction is ending, so a window would
/// describe something that is no longer true.
pub fn ntml_cancel_line(
    logged_at: DateTime<Utc>,
    restriction: &str,
    requesting: Option<&str>,
    providing: Option<&str>,
) -> String {
    let mut parts = vec![
        log_stamp(logged_at),
        restriction.trim().to_string(),
        "CANCEL TMI".to_string(),
    ];
    parts.extend(facilities(requesting, providing));
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(day: u32, hh: u32, mm: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, day, hh, mm, 0).unwrap()
    }

    /// Base restriction with everything empty/none, for tests to tweak.
    ///
    /// Restored here: #436 added it for the raw-vs-structured test below, and #455's fixture-driven
    /// rewrite of this module removed it. Neither change conflicted textually, so the merge kept the
    /// caller and dropped the helper — `cargo clippy --all-targets` then fails with E0425 while
    /// `cargo build` stays green, because nothing outside the test build references it.
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

    /// The reference row from the vATCSCC TMI material, assembled end to end (#436). Every section
    /// is load-bearing: drop the log stamp, the window or the facility token and this fails.
    #[test]
    fn assembles_the_reference_ntml_row() {
        let line = ntml_line(
            at(14, 14, 42),
            "JFK arrivals via CAMRN 20MIT NO STACKS TYPE:ALL SPD:≤210 ALT:AOB090 VOLUME:VOLUME EXCL:PHL",
            Some(at(14, 20, 15)),
            Some(at(14, 23, 15)),
            Some("N90"),
            Some("ZNY"),
        );
        assert_eq!(
            line,
            "14/1442 JFK arrivals via CAMRN 20MIT NO STACKS TYPE:ALL SPD:≤210 ALT:AOB090 VOLUME:VOLUME EXCL:PHL 2015-2315 N90:ZNY"
        );
    }

    /// A structured TMI and a raw-typed one of the same restriction must produce the same row —
    /// that equivalence is what lets the channel be read as a single log.
    ///
    /// The raw side is written out by hand rather than derived from `encode`, so this compares two
    /// independent things; deriving both from the same call would pass no matter what `encode` did.
    #[test]
    fn a_raw_tmi_and_a_structured_one_render_the_same_row() {
        let structured = NtmlRestriction {
            via: Some("CAMRN".into()),
            value: Some(20),
            ..base("JFK", "arrivals", "MIT")
        };
        let typed_by_hand = "JFK arrivals via CAMRN 20MIT";
        assert_eq!(encode(&structured), typed_by_hand);

        let from_structured = ntml_line(
            at(14, 14, 42),
            &encode(&structured),
            None,
            None,
            Some("N90"),
            Some("ZNY"),
        );
        let from_raw = ntml_line(
            at(14, 14, 42),
            typed_by_hand,
            None,
            None,
            Some("N90"),
            Some("ZNY"),
        );
        assert_eq!(from_structured, from_raw);
        assert_eq!(
            from_structured,
            "14/1442 JFK arrivals via CAMRN 20MIT N90:ZNY"
        );
    }

    /// A half-open window would describe a restriction that doesn't exist, so it is dropped rather
    /// than half-rendered.
    #[test]
    fn an_incomplete_window_is_omitted_entirely() {
        let line = ntml_line(
            at(1, 0, 5),
            "PHL via J152 STOP",
            Some(at(1, 1, 0)),
            None,
            Some("ZNY"),
            None,
        );
        assert_eq!(line, "01/0005 PHL via J152 STOP ZNY");
    }

    /// A cancel is its own row, carries no window, and still names the facilities.
    #[test]
    fn a_cancellation_is_its_own_row() {
        let line = ntml_cancel_line(at(25, 2, 10), "CVG via ALL", Some("ZID"), Some("ZTL"));
        assert_eq!(line, "25/0210 CVG via ALL CANCEL TMI ZID:ZTL");
    }

    /// Mid-edit a TMI can have a blank facility; that must not leave a dangling colon.
    #[test]
    fn a_missing_facility_does_not_leave_a_stray_colon() {
        assert_eq!(facilities(Some("N90"), None).as_deref(), Some("N90"));
        assert_eq!(facilities(None, Some("ZNY")).as_deref(), Some("ZNY"));
        assert_eq!(facilities(None, None), None);
        assert_eq!(
            facilities(Some(" n90 "), Some("zny")).as_deref(),
            Some("N90:ZNY")
        );
    }

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
