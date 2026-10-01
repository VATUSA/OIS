//! ADVZY advisory document renderer. Turns a structured [`RerouteAdvisory`] (the form-built
//! advisory) into the multi-line vATCSCC document stored on `tmu.advisories.body`. Modelled on the
//! vATCSCC TMI material cited by #437; the reference documents themselves live in
//! `fixtures/reroute-reference.json`, which the tests below assert against.
//!
//! Reroute is the first advisory type (#458). `tmu.advisories.kind` is free-form text with no check
//! constraint — migration `0085` says so outright — so a type is claimed by rendering it here, not
//! by extending an enum.
//!
//! Unlike an NTML line, an advisory is a *document*: alignment is part of its meaning, and a label
//! with no value still prints (`REMARKS:` appears bare in the reference). That is the opposite of
//! [`crate::tmi::encode`]'s omit-when-absent rule, and the reason the two do not share helpers
//! beyond `clean`/`opt`.
//!
//! # The renderer is one-way, for the same reason the NTML codec is
//!
//! There is no `parse`/`from_document`, so a raw-typed advisory keeps `structured` null and anything
//! wanting fields gets nothing rather than a guess. `tmi.rs`'s module doc sets out the argument at
//! length (#454); it applies here with more force, because a reroute's free-text lines — `REMARKS`,
//! `ASSOCIATED RESTRICTIONS`, `MODIFICATIONS` — can contain anything at all, including text that
//! looks like another label.
//!
//! # No line wrapping, deliberately
//!
//! The reference wraps a long route onto a continuation line indented to the ROUTE column, but by no
//! stated rule: measured inline widths are LAS 62 (not wrapped), SEA 50 (not wrapped), LAX 65
//! (wrapped) and SFO 65 (wrapped), so the threshold sits somewhere in 62..65 and is a column artifact
//! of the source PDF. Fitting a constant to two samples would bake a layout accident into every route
//! OIS ever renders, so each route prints on one line. `fixtures/reroute-reference.json` records the
//! deviation on the multi-segment case; if a real wrap width is ever documented, that is where it
//! gets pinned.

use chrono::{DateTime, NaiveDate, Utc};

use crate::models::{
    GdpAdvisory, GroundStopAdvisory, RerouteAdvisory, RerouteRoutes, RerouteValidBasis,
};

fn clean(s: &str) -> String {
    s.trim().to_ascii_uppercase()
}

fn opt(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|t| !t.is_empty())
}

/// Who the advisory is and when — the parts of the document that come from the row rather than from
/// the author's fields.
///
/// `number` and `issued_day` are **read off the row, never re-derived**: the number is allocated in
/// the same transaction as the insert, at draft time, and `repos::tmu` documents an open question
/// about whether a draft started before 00:00Z should carry the next day's number. Recomputing
/// either here would silently answer that question differently from the database.
pub struct AdvisoryIdent {
    pub facility: String,
    pub number: i32,
    pub issued_day: NaiveDate,
    /// The signature instant, printed `YY/MM/DD HH:MM`.
    pub signed_at: DateTime<Utc>,
}

/// A label line: `LABEL: value`, or a bare `LABEL:` when the value is absent.
///
/// The bare form is not a degenerate case to be tidied away — the reference prints `REMARKS:`,
/// `ASSOCIATED RESTRICTIONS:` and `MODIFICATIONS:` with nothing after them, and a reader uses their
/// presence to know the author had nothing to add rather than that the section is missing.
fn label(name: &str, value: Option<&str>) -> String {
    match value {
        Some(v) => format!("{name}: {v}"),
        None => format!("{name}:"),
    }
}

/// One fixed-width table, emitted as a heading row, a dashed rule matching each heading's width, and
/// the body rows.
///
/// Built here because the repo has no column-alignment helper of any kind — this is the part #458
/// calls out as resembling nothing already in the codebase. `widths` is the column start offsets'
/// implied padding: every cell but the last is padded to its column width, and the last is printed
/// unpadded so no line carries trailing whitespace.
fn table(headings: &[&str], widths: &[usize], rows: &[Vec<String>]) -> Vec<String> {
    let line = |cells: &[String]| -> String {
        let mut out = String::new();
        for (i, cell) in cells.iter().enumerate() {
            if i + 1 == cells.len() {
                out.push_str(cell);
            } else {
                out.push_str(&format!("{:width$}", cell, width = widths[i]));
            }
        }
        out
    };
    let heading_cells: Vec<String> = headings.iter().map(|h| (*h).to_string()).collect();
    let rule_cells: Vec<String> = headings.iter().map(|h| "-".repeat(h.len())).collect();

    let mut out = vec![line(&heading_cells), line(&rule_cells)];
    out.extend(rows.iter().map(|r| line(r)));
    out
}

/// The route section: `ROUTE:` followed by one table, or by the origin/destination pair.
fn route_section(routes: &RerouteRoutes) -> Vec<String> {
    let mut out = vec!["ROUTE:".to_string()];
    match routes {
        RerouteRoutes::Single { rows } => {
            let cells: Vec<Vec<String>> = rows
                .iter()
                .map(|r| vec![clean(&r.orig), clean(&r.dest), r.route.trim().to_string()])
                .collect();
            out.extend(table(&["ORIG", "DEST", "ROUTE"], &[9, 10], &cells));
        }
        RerouteRoutes::Segmented {
            origin,
            destination,
        } => {
            let seg = |rows: &[crate::models::RerouteSegment]| -> Vec<Vec<String>> {
                rows.iter()
                    .map(|r| vec![clean(&r.orig), r.route.trim().to_string()])
                    .collect()
            };
            out.extend(table(
                &["ORIG", "ROUTE - ORIGIN SEGMENTS"],
                &[12],
                &seg(origin),
            ));
            // A blank line separates the two tables, as in the reference.
            out.push(String::new());
            out.extend(table(
                &["ORIG", "ROUTE - DESTINATION SEGMENTS"],
                &[12],
                &seg(destination),
            ));
        }
    }
    out
}

/// Render a Reroute advisory to its vATCSCC document.
///
/// The document is the contract: see `fixtures/reroute-reference.json` for the two reference
/// examples this is pinned against.
pub fn render_reroute(a: &RerouteAdvisory, id: &AdvisoryIdent) -> String {
    let facility = clean(&id.facility);
    let valid = match a.valid.basis {
        RerouteValidBasis::FcaEntryTime => format!(
            "FCA ENTRY TIME FROM {} TO {}",
            a.valid.from.trim(),
            a.valid.to.trim()
        ),
        RerouteValidBasis::Etd => format!("ETD {} TO {}", a.valid.from.trim(), a.valid.to.trim()),
    };

    let mut lines: Vec<String> = vec![
        format!(
            "vATCSCC ADVZY {:03} {} {} {}",
            id.number,
            facility,
            id.issued_day.format("%m/%d/%Y"),
            clean(&a.header),
        ),
        label("NAME", Some(&clean(&a.name))),
        label("IMPACTED AREA", Some(&clean(&a.impacted_area))),
        label("REASON", opt(&a.reason)),
        label("INCLUDE TRAFFIC", opt(&a.include_traffic)),
        label("VALID", Some(&valid)),
        label("FACILITIES INCLUDED", opt(&a.facilities_included)),
        label("PROBABILITY OF EXTENSION", opt(&a.probability_of_extension)),
        label("REMARKS", opt(&a.remarks)),
        label("ASSOCIATED RESTRICTIONS", opt(&a.associated_restrictions)),
        label("MODIFICATIONS", opt(&a.modifications)),
    ];

    lines.extend(route_section(&a.routes));

    // The signature block, separated by a blank line. `RR` is the reroute TMI-ID prefix; the number
    // is the same one the header carries, so the two can never disagree.
    lines.push(String::new());
    lines.push(format!("TMI ID: RR{}{:03}", facility, id.number));
    lines.push(format!("{}-{}", a.valid.from.trim(), a.valid.to.trim()));
    lines.push(id.signed_at.format("%y/%m/%d %H:%M").to_string());

    lines.join("\n")
}

/// `FLT INCL` is printed once per entry, and once bare when there are none.
///
/// The GDP reference carries two (`1stTier`, then `CZY`); the Ground Stop reference carries one with
/// its mode inline. Values print as typed — `(Manual)` and `1stTier` are the only mixed-case values
/// in any of the four reference documents, so `clean` would corrupt them.
fn flt_incl(entries: &[String]) -> Vec<String> {
    let printed: Vec<String> = entries
        .iter()
        .map(|e| e.trim())
        .filter(|e| !e.is_empty())
        .map(|e| format!("FLT INCL: {e}"))
        .collect();
    if printed.is_empty() {
        vec![label("FLT INCL", None)]
    } else {
        printed
    }
}

/// Render a Ground Delay Program advisory to its vATCSCC document.
///
/// The document is the contract: `fixtures/gdp-reference.json` holds the reference example this is
/// pinned against, transcribed from the source PDF cited by #437.
///
/// # No `TMI ID:` line
///
/// Not an omission. Neither the GDP nor the Ground Stop reference carries one, and #437 attributes
/// the TMI ID to reroute specifically (`RRDCC004` = Reroute + facility + advisory number). A GDP is
/// identified by its advisory number in the header alone.
pub fn render_gdp(a: &GdpAdvisory, id: &AdvisoryIdent) -> String {
    let mut lines: Vec<String> = vec![
        format!(
            "vATCSCC ADVZY {:03} {} {} {}",
            id.number,
            clean(&a.element),
            id.issued_day.format("%m/%d/%Y"),
            clean(&a.header),
        ),
        label("CTL ELEMENT", Some(&clean(&a.control_element))),
        label("ELEMENT TYPE", Some(&clean(&a.element_type))),
        label("ADL TIME", Some(&clean(&a.adl_time))),
        label(
            "DELAY ASSIGNMENT MODE",
            Some(&clean(&a.delay_assignment_mode)),
        ),
        label(
            "ARRIVALS ESTIMATED FOR",
            Some(&clean(&a.arrivals_estimated_for)),
        ),
        label(
            "CUMULATIVE PROGRAM PERIOD",
            Some(&clean(&a.cumulative_program_period)),
        ),
        label("PROGRAM RATE", Some(&clean(&a.program_rate))),
        label("POP-UP FACTOR", opt(&a.pop_up_factor)),
    ];

    lines.extend(flt_incl(&a.flights_included));

    lines.extend([
        label("DEPARTURE SCOPE", opt(&a.departure_scope)),
        label(
            "ADDITIONAL DEP FACILITIES INCLUDED",
            opt(&a.additional_dep_facilities_included),
        ),
        label("EXEMPT DEP FACILITIES", opt(&a.exempt_dep_facilities)),
        label("CANADIAN ARPTS INCLUDED", opt(&a.canadian_arpts_included)),
        label(
            "DELAY ASSIGNMENT TABLE APPLIES TO",
            opt(&a.delay_assignment_table_applies_to),
        ),
        label("DELAY LIMIT", opt(&a.delay_limit)),
        label("MAXIMUM DELAY", opt(&a.maximum_delay)),
        label("AVERAGE DELAY", opt(&a.average_delay)),
        label("IMPACTING CONDITION", opt(&a.impacting_condition)),
        label("COMMENTS", opt(&a.comments)),
    ]);

    lines.push(String::new());
    lines.push(a.period.trim().to_string());
    lines.push(id.signed_at.format("%y/%m/%d %H:%M").to_string());

    lines.join("\n")
}

/// Render a Ground Stop advisory to its vATCSCC document.
///
/// The document is the contract: `fixtures/ground-stop-reference.json` holds the reference example.
///
/// Structurally a GDP with the metering fields swapped for the three delay triplets — same header,
/// same opening, same footer, and no `TMI ID:` line. The two renderers are kept separate for the
/// reason [`crate::models::GroundStopAdvisory`] gives: sharing one would make every field optional.
pub fn render_ground_stop(a: &GroundStopAdvisory, id: &AdvisoryIdent) -> String {
    let mut lines: Vec<String> = vec![
        format!(
            "vATCSCC ADVZY {:03} {} {} {}",
            id.number,
            clean(&a.element),
            id.issued_day.format("%m/%d/%Y"),
            clean(&a.header),
        ),
        label("CTL ELEMENT", Some(&clean(&a.control_element))),
        label("ELEMENT TYPE", Some(&clean(&a.element_type))),
        label("ADL TIME", Some(&clean(&a.adl_time))),
        label("GROUND STOP PERIOD", Some(&clean(&a.ground_stop_period))),
        label(
            "CUMULATIVE PROGRAM PERIOD",
            Some(&clean(&a.cumulative_program_period)),
        ),
    ];

    lines.extend(flt_incl(&a.flights_included));

    lines.extend([
        label(
            "ADDITIONAL DEP FACILITIES INCLUDED",
            opt(&a.additional_dep_facilities_included),
        ),
        label(
            "CURRENT TOTAL, MAXIMUM, AVERAGE DELAYS",
            opt(&a.current_delays),
        ),
        label(
            "PREVIOUS TOTAL, MAXIMUM, AVERAGE DELAYS",
            opt(&a.previous_delays),
        ),
        label("NEW TOTAL, MAXIMUM, AVERAGE DELAYS", opt(&a.new_delays)),
        label("PROBABILITY OF EXTENSION", opt(&a.probability_of_extension)),
        label("IMPACTING CONDITION", opt(&a.impacting_condition)),
        label("COMMENTS", opt(&a.comments)),
    ]);

    lines.push(String::new());
    lines.push(a.period.trim().to_string());
    lines.push(id.signed_at.format("%y/%m/%d %H:%M").to_string());

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirrors the envelope of `fixtures/reroute-reference.json`.
    #[derive(serde::Deserialize)]
    struct Ident {
        facility: String,
        number: i32,
        issued_day: NaiveDate,
        signed_at: DateTime<Utc>,
    }

    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        ident: Ident,
        structured: RerouteAdvisory,
        rendered: String,
    }

    #[derive(serde::Deserialize)]
    struct Fixtures {
        cases: Vec<Case>,
    }

    const FIXTURES: &str = include_str!("../../fixtures/reroute-reference.json");

    fn ident(i: &Ident) -> AdvisoryIdent {
        AdvisoryIdent {
            facility: i.facility.clone(),
            number: i.number,
            issued_day: i.issued_day,
            signed_at: i.signed_at,
        }
    }

    #[test]
    fn matches_every_shared_reference_case() {
        let fixtures: Fixtures = serde_json::from_str(FIXTURES).expect("fixtures parse");
        // A failed load must not make this vacuously green.
        assert!(!fixtures.cases.is_empty(), "no reference cases loaded");

        for case in &fixtures.cases {
            assert_eq!(
                render_reroute(&case.structured, &ident(&case.ident)),
                case.rendered,
                "case {}",
                case.name
            );
        }
    }

    /// The reference document, written out here as well as in the fixture.
    ///
    /// Every section is load-bearing: drop a label, the route table's alignment, the TMI ID, the
    /// valid window or the signature and this fails. The duplication with the fixture is the point —
    /// this one reads as the document a controller would recognise.
    #[test]
    fn assembles_the_reference_reroute_document() {
        let fixtures: Fixtures = serde_json::from_str(FIXTURES).expect("fixtures parse");
        let case = fixtures
            .cases
            .iter()
            .find(|c| c.name == "advzy-004-single-segment")
            .expect("the single-segment reference case");

        assert_eq!(
            render_reroute(&case.structured, &ident(&case.ident)),
            "vATCSCC ADVZY 004 DCC 04/14/2020 FCA RQD/FL\n\
             NAME: NO_J75_3_PARTIAL\n\
             IMPACTED AREA: ZDC\n\
             REASON: WEATHER / THUNDERSTORMS\n\
             INCLUDE TRAFFIC: KBOS DEPARTURES TO KMCO\n\
             VALID: FCA ENTRY TIME FROM 142030 TO 150230\n\
             FACILITIES INCLUDED: ALL_FLIGHTS\n\
             PROBABILITY OF EXTENSION: MEDIUM\n\
             REMARKS:\n\
             ASSOCIATED RESTRICTIONS:\n\
             MODIFICATIONS:\n\
             ROUTE:\n\
             ORIG     DEST      ROUTE\n\
             ----     ----      -----\n\
             ZBW      MCO       >GONZZ Q29 DORET DJB J84 SPA J85 TWINS JEFOI SHEMP< BUGGZ4\n\
             \n\
             TMI ID: RRDCC004\n\
             142030-150230\n\
             20/04/14 14:36"
        );
    }

    /// An absent optional and an empty one must print the same bare label — the document always
    /// shows the section, so `Some("")` must not become `REMARKS: `.
    #[test]
    fn an_empty_optional_prints_the_same_bare_label_as_an_absent_one() {
        assert_eq!(label("REMARKS", opt(&None)), "REMARKS:");
        assert_eq!(label("REMARKS", opt(&Some("   ".into()))), "REMARKS:");
        assert_eq!(label("REMARKS", opt(&Some("X".into()))), "REMARKS: X");
    }

    /// No rendered line may carry trailing whitespace: the last column is printed unpadded, so a
    /// short route can't leave the table ragged in a way a diff would show but an eye would not.
    #[test]
    fn no_line_has_trailing_whitespace() {
        let fixtures: Fixtures = serde_json::from_str(FIXTURES).expect("fixtures parse");
        for case in &fixtures.cases {
            let out = render_reroute(&case.structured, &ident(&case.ident));
            for (i, line) in out.lines().enumerate() {
                assert_eq!(line, line.trim_end(), "case {} line {}", case.name, i + 1);
            }
        }
    }
}

#[cfg(test)]
mod gdp_tests {
    use super::*;

    /// Mirrors the envelope of `fixtures/gdp-reference.json`.
    #[derive(serde::Deserialize)]
    struct Ident {
        facility: String,
        number: i32,
        issued_day: NaiveDate,
        signed_at: DateTime<Utc>,
    }

    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        ident: Ident,
        structured: GdpAdvisory,
        rendered: String,
    }

    #[derive(serde::Deserialize)]
    struct Fixtures {
        cases: Vec<Case>,
    }

    const FIXTURES: &str = include_str!("../../fixtures/gdp-reference.json");

    fn ident(i: &Ident) -> AdvisoryIdent {
        AdvisoryIdent {
            facility: i.facility.clone(),
            number: i.number,
            issued_day: i.issued_day,
            signed_at: i.signed_at,
        }
    }

    fn fixtures() -> Fixtures {
        let fixtures: Fixtures = serde_json::from_str(FIXTURES).expect("fixtures parse");
        // A failed load must not make these vacuously green.
        assert!(!fixtures.cases.is_empty(), "no reference cases loaded");
        fixtures
    }

    #[test]
    fn matches_every_shared_reference_case() {
        for case in &fixtures().cases {
            assert_eq!(
                render_gdp(&case.structured, &ident(&case.ident)),
                case.rendered,
                "case {}",
                case.name
            );
        }
    }

    /// The reference document, written out here as well as in the fixture.
    ///
    /// The duplication is the point — this one reads as the document a controller would recognise.
    /// Note what the header does *not* contain: `ident.facility` is `DCC`, the issuing facility, and
    /// a reroute would print it there. A GDP prints the control element instead.
    #[test]
    fn assembles_the_reference_gdp_document() {
        let fixtures = fixtures();
        let case = fixtures
            .cases
            .iter()
            .find(|c| c.name == "advzy-002-gdp")
            .expect("the reference GDP case");

        assert_eq!(
            render_gdp(&case.structured, &ident(&case.ident)),
            concat!(
                "vATCSCC ADVZY 002 JFK/ZNY 04/14/2020 CDM GROUND DELAY PROGRAM\n",
                "CTL ELEMENT: JFK\n",
                "ELEMENT TYPE: APT\n",
                "ADL TIME: 1349Z\n",
                "DELAY ASSIGNMENT MODE: DAS\n",
                "ARRIVALS ESTIMATED FOR: 14/1415Z - 14/2315Z\n",
                "CUMULATIVE PROGRAM PERIOD: 14/1415Z - 14/2315Z\n",
                "PROGRAM RATE: 40/40/40/30/25/20/20/36/54\n",
                "POP-UP FACTOR: MEDIUM\n",
                "FLT INCL: 1stTier\n",
                "FLT INCL: CZY\n",
                "DEPARTURE SCOPE: 1200\n",
                "ADDITIONAL DEP FACILITIES INCLUDED: KATL\n",
                "EXEMPT DEP FACILITIES: KORD\n",
                "CANADIAN ARPTS INCLUDED: CYYZ\n",
                "DELAY ASSIGNMENT TABLE APPLIES TO: ZNY\n",
                "DELAY LIMIT: 240\n",
                "MAXIMUM DELAY: 171\n",
                "AVERAGE DELAY: 38\n",
                "IMPACTING CONDITION: WEATHER / THUNDERSTORMS\n",
                "COMMENTS: COMMENTS COMMENTS COMMENTS\n",
                "\n",
                "141415-142315\n",
                "20/04/14 13:49",
            )
        );
    }

    /// `FLT INCL` repeats, which the reference shows and a single-string field could not express.
    /// With no entries it still prints once, bare, so the document never loses the line entirely.
    #[test]
    fn flt_incl_prints_once_per_entry_and_once_bare_when_empty() {
        let fixtures = fixtures();
        let case = &fixtures.cases[0];

        let out = render_gdp(&case.structured, &ident(&case.ident));
        assert_eq!(
            out.lines().filter(|l| l.starts_with("FLT INCL")).count(),
            2,
            "the reference carries two: {out}"
        );

        let mut empty = case.structured.clone();
        empty.flights_included = vec![];
        let out = render_gdp(&empty, &ident(&case.ident));
        assert_eq!(
            out.lines().filter(|l| l.starts_with("FLT INCL")).count(),
            1,
            "still one, bare: {out}"
        );
        assert!(out.contains("\nFLT INCL:\n"), "bare form: {out}");
    }

    /// An absent optional still prints its label, as reroute's do — a reader uses the bare label to
    /// know the author had nothing to add rather than that the section is missing.
    #[test]
    fn an_absent_optional_prints_a_bare_label() {
        let fixtures = fixtures();
        let case = &fixtures.cases[0];
        let mut a = case.structured.clone();
        a.delay_limit = None;
        a.comments = Some("   ".to_string());

        let out = render_gdp(&a, &ident(&case.ident));
        assert!(out.contains("\nDELAY LIMIT:\n"), "bare label: {out}");
        assert!(
            out.contains("\nCOMMENTS:\n"),
            "whitespace is absence: {out}"
        );
    }

    /// A GDP footer carries no `TMI ID:` line — #437 attributes that to reroute, whose own test
    /// asserts `RRDCC004`. If a future edit copies reroute's footer wholesale, this catches it.
    #[test]
    fn a_gdp_carries_no_tmi_id() {
        for case in &fixtures().cases {
            let out = render_gdp(&case.structured, &ident(&case.ident));
            assert!(!out.contains("TMI ID"), "case {}: {out}", case.name);
        }
    }

    /// Alignment is part of a document's meaning, so a stray trailing space is a real defect — the
    /// same invariant `tests::no_line_has_trailing_whitespace` holds for reroute.
    #[test]
    fn no_line_has_trailing_whitespace() {
        for case in &fixtures().cases {
            let out = render_gdp(&case.structured, &ident(&case.ident));
            for (i, line) in out.lines().enumerate() {
                assert_eq!(line, line.trim_end(), "case {} line {}", case.name, i + 1);
            }
        }
    }
}

#[cfg(test)]
mod ground_stop_tests {
    use super::*;

    /// Mirrors the envelope of `fixtures/ground-stop-reference.json`.
    #[derive(serde::Deserialize)]
    struct Ident {
        facility: String,
        number: i32,
        issued_day: NaiveDate,
        signed_at: DateTime<Utc>,
    }

    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        ident: Ident,
        structured: GroundStopAdvisory,
        rendered: String,
    }

    #[derive(serde::Deserialize)]
    struct Fixtures {
        cases: Vec<Case>,
    }

    const FIXTURES: &str = include_str!("../../fixtures/ground-stop-reference.json");

    fn ident(i: &Ident) -> AdvisoryIdent {
        AdvisoryIdent {
            facility: i.facility.clone(),
            number: i.number,
            issued_day: i.issued_day,
            signed_at: i.signed_at,
        }
    }

    fn fixtures() -> Fixtures {
        let fixtures: Fixtures = serde_json::from_str(FIXTURES).expect("fixtures parse");
        assert!(!fixtures.cases.is_empty(), "no reference cases loaded");
        fixtures
    }

    #[test]
    fn matches_every_shared_reference_case() {
        for case in &fixtures().cases {
            assert_eq!(
                render_ground_stop(&case.structured, &ident(&case.ident)),
                case.rendered,
                "case {}",
                case.name
            );
        }
    }

    /// The reference document, written out as a controller would recognise it.
    #[test]
    fn assembles_the_reference_ground_stop_document() {
        let fixtures = fixtures();
        let case = &fixtures.cases[0];

        assert_eq!(
            render_ground_stop(&case.structured, &ident(&case.ident)),
            concat!(
                "vATCSCC ADVZY 003 DFW/ZFW 04/14/2020 CDM GROUND STOP\n",
                "CTL ELEMENT: DFW\n",
                "ELEMENT TYPE: APT\n",
                "ADL TIME: 1354Z\n",
                "GROUND STOP PERIOD: 14/1430Z - 14/1630Z\n",
                "CUMULATIVE PROGRAM PERIOD: 14/1430Z - 14/1630Z\n",
                "FLT INCL: (Manual) ZHU ZJX ZMA ZME ZTL\n",
                "ADDITIONAL DEP FACILITIES INCLUDED: KDEN\n",
                "CURRENT TOTAL, MAXIMUM, AVERAGE DELAYS: 1240/414/81\n",
                "PREVIOUS TOTAL, MAXIMUM, AVERAGE DELAYS: 636/211/70\n",
                "NEW TOTAL, MAXIMUM, AVERAGE DELAYS: 1876/625/151\n",
                "PROBABILITY OF EXTENSION: MEDIUM\n",
                "IMPACTING CONDITION: EQUIPMENT / STARS\n",
                "COMMENTS: BLAH\n",
                "\n",
                "141430-141630\n",
                "20/04/14 13:54",
            )
        );
    }

    /// The three delay triplets are distinct lines in a fixed order. Swapping CURRENT for PREVIOUS
    /// would change what a controller reads off the document, so the order is asserted directly
    /// rather than left to the whole-document comparison alone.
    #[test]
    fn the_delay_triplets_keep_their_order() {
        let fixtures = fixtures();
        let case = &fixtures.cases[0];
        let out = render_ground_stop(&case.structured, &ident(&case.ident));

        let at = |needle: &str| {
            out.find(needle)
                .unwrap_or_else(|| panic!("missing {needle}: {out}"))
        };
        assert!(
            at("CURRENT TOTAL") < at("PREVIOUS TOTAL") && at("PREVIOUS TOTAL") < at("NEW TOTAL"),
            "CURRENT then PREVIOUS then NEW: {out}"
        );
        assert!(out.contains("CURRENT TOTAL, MAXIMUM, AVERAGE DELAYS: 1240/414/81"));
    }

    /// `(Manual)` is one of only two mixed-case values in the four reference documents, so a stray
    /// `clean` here would silently corrupt the document to `(MANUAL)`.
    #[test]
    fn flt_incl_keeps_its_case() {
        let fixtures = fixtures();
        let case = &fixtures.cases[0];
        let out = render_ground_stop(&case.structured, &ident(&case.ident));
        assert!(out.contains("(Manual)"), "case preserved: {out}");
    }

    #[test]
    fn a_ground_stop_carries_no_tmi_id() {
        for case in &fixtures().cases {
            let out = render_ground_stop(&case.structured, &ident(&case.ident));
            assert!(!out.contains("TMI ID"), "case {}: {out}", case.name);
        }
    }

    #[test]
    fn no_line_has_trailing_whitespace() {
        for case in &fixtures().cases {
            let out = render_ground_stop(&case.structured, &ident(&case.ident));
            for (i, line) in out.lines().enumerate() {
                assert_eq!(line, line.trim_end(), "case {} line {}", case.name, i + 1);
            }
        }
    }
}
