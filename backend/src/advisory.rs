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

use crate::feed::airports::IataMap;
use crate::feed::gdp::GdpStats;
use crate::models::{
    AarStep, GdpAdvisory, GdpBody, GroundStopAdvisory, GroundStopBody, PublishGdpRequest,
    PublishGroundStopRequest, RerouteAdvisory, RerouteRoutes, RerouteValidBasis,
};
use crate::text::collapse;

/// Upper-cased and whitespace-collapsed. The collapse is what stops an author-supplied value
/// from introducing a line break into the document (#498) — see [`crate::text::collapse`].
fn clean(s: &str) -> String {
    crate::text::collapse(s).to_ascii_uppercase()
}

/// A present, non-blank value, collapsed.
///
/// Returns an owned `String` rather than a borrow because collapsing allocates. That is the
/// deliberate cost of collapsing here, in the helper that *produces* the value, instead of at each
/// place one is printed — a print site can then do anything it likes with the string without
/// reopening #498.
fn opt(s: &Option<String>) -> Option<String> {
    s.as_deref()
        .map(crate::text::collapse)
        .filter(|t| !t.is_empty())
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
            // Collapsed again here, not only at the callers: a cell carrying a newline or a run of
            // tabs would shift every column to its right, so the table's own invariant is defended
            // where the padding is computed (#498).
            let cell = collapse(cell);
            if i + 1 == cells.len() {
                out.push_str(&cell);
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
                .map(|r| vec![clean(&r.orig), clean(&r.dest), collapse(&r.route)])
                .collect();
            out.extend(table(&["ORIG", "DEST", "ROUTE"], &[9, 10], &cells));
        }
        RerouteRoutes::Segmented {
            origin,
            destination,
        } => {
            let seg = |rows: &[crate::models::RerouteSegment]| -> Vec<Vec<String>> {
                rows.iter()
                    .map(|r| vec![clean(&r.orig), collapse(&r.route)])
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
            collapse(&a.valid.from),
            collapse(&a.valid.to)
        ),
        RerouteValidBasis::Etd => {
            format!(
                "ETD {} TO {}",
                collapse(&a.valid.from),
                collapse(&a.valid.to)
            )
        }
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
        label("REASON", opt(&a.reason).as_deref()),
        label("INCLUDE TRAFFIC", opt(&a.include_traffic).as_deref()),
        label("VALID", Some(&valid)),
        label(
            "FACILITIES INCLUDED",
            opt(&a.facilities_included).as_deref(),
        ),
        label(
            "PROBABILITY OF EXTENSION",
            opt(&a.probability_of_extension).as_deref(),
        ),
        label("REMARKS", opt(&a.remarks).as_deref()),
        label(
            "ASSOCIATED RESTRICTIONS",
            opt(&a.associated_restrictions).as_deref(),
        ),
        label("MODIFICATIONS", opt(&a.modifications).as_deref()),
    ];

    lines.extend(route_section(&a.routes));

    // The signature block, separated by a blank line. `RR` is the reroute TMI-ID prefix; the number
    // is the same one the header carries, so the two can never disagree.
    lines.push(String::new());
    lines.push(format!("TMI ID: RR{}{:03}", facility, id.number));
    lines.push(format!(
        "{}-{}",
        collapse(&a.valid.from),
        collapse(&a.valid.to)
    ));
    lines.push(id.signed_at.format("%y/%m/%d %H:%M").to_string());

    lines.join("\n")
}

/// `FLT INCL` is printed once per entry, and once bare when there are none.
///
/// The GDP reference carries two (`1stTier`, then `CZY`); the Ground Stop reference carries one with
/// its mode inline. Values print as typed — `(Manual)` and `1stTier` are the only mixed-case values
/// in any of the four reference documents, so `clean` would corrupt them. `collapse` is applied
/// instead (#498): it cannot introduce or remove a line, and it leaves case alone.
fn flt_incl(entries: &[String]) -> Vec<String> {
    let printed: Vec<String> = entries
        .iter()
        .map(|e| collapse(e))
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
        label("POP-UP FACTOR", opt(&a.pop_up_factor).as_deref()),
    ];

    lines.extend(flt_incl(&a.flights_included));

    lines.extend([
        label("DEPARTURE SCOPE", opt(&a.departure_scope).as_deref()),
        label(
            "ADDITIONAL DEP FACILITIES INCLUDED",
            opt(&a.additional_dep_facilities_included).as_deref(),
        ),
        label(
            "EXEMPT DEP FACILITIES",
            opt(&a.exempt_dep_facilities).as_deref(),
        ),
        label(
            "CANADIAN ARPTS INCLUDED",
            opt(&a.canadian_arpts_included).as_deref(),
        ),
        label(
            "DELAY ASSIGNMENT TABLE APPLIES TO",
            opt(&a.delay_assignment_table_applies_to).as_deref(),
        ),
        label("DELAY LIMIT", opt(&a.delay_limit).as_deref()),
        label("MAXIMUM DELAY", opt(&a.maximum_delay).as_deref()),
        label("AVERAGE DELAY", opt(&a.average_delay).as_deref()),
        label(
            "IMPACTING CONDITION",
            opt(&a.impacting_condition).as_deref(),
        ),
        label("COMMENTS", opt(&a.comments).as_deref()),
    ]);

    lines.push(String::new());
    lines.push(collapse(&a.period));
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
            opt(&a.additional_dep_facilities_included).as_deref(),
        ),
        label(
            "CURRENT TOTAL, MAXIMUM, AVERAGE DELAYS",
            opt(&a.current_delays).as_deref(),
        ),
        label(
            "PREVIOUS TOTAL, MAXIMUM, AVERAGE DELAYS",
            opt(&a.previous_delays).as_deref(),
        ),
        label(
            "NEW TOTAL, MAXIMUM, AVERAGE DELAYS",
            opt(&a.new_delays).as_deref(),
        ),
        label(
            "PROBABILITY OF EXTENSION",
            opt(&a.probability_of_extension).as_deref(),
        ),
        label(
            "IMPACTING CONDITION",
            opt(&a.impacting_condition).as_deref(),
        ),
        label("COMMENTS", opt(&a.comments).as_deref()),
    ]);

    lines.push(String::new());
    lines.push(collapse(&a.period));
    lines.push(id.signed_at.format("%y/%m/%d %H:%M").to_string());

    lines.join("\n")
}

/// The correction posted when an advisory is cancelled (VATUSA/OIS#459).
///
/// A short line, not a re-render of the document. The channel is a chronological log and the original
/// advisory did go out, so the cancellation sits beside it as its own entry rather than replacing it —
/// the shape #436 chose for TMI cancellations, for the same reason.
///
/// It repeats the TMI ID, which is what ties the correction to the document above it. Deliberately
/// **not** the valid period: that lives in `structured`, which a raw-typed advisory does not have, so
/// including it would make the correction's shape depend on how the advisory happened to be entered.
/// The ID identifies the document on its own.
pub fn render_cancellation(id: &AdvisoryIdent) -> String {
    let facility = clean(&id.facility);
    [
        format!(
            "vATCSCC ADVZY {:03} {} {} CANCELLED",
            id.number,
            facility,
            id.issued_day.format("%m/%d/%Y"),
        ),
        format!("TMI ID: RR{}{:03}", facility, id.number),
        id.signed_at.format("%y/%m/%d %H:%M").to_string(),
    ]
    .join("\n")
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

    // --- the cancellation correction (VATUSA/OIS#459) ---

    /// A short correction, not a re-render. It ties itself to the document by TMI ID; the channel is
    /// a chronological log, so the cancellation sits beside the original rather than replacing it.
    #[test]
    fn a_cancellation_names_the_document_it_corrects() {
        let fixtures: Fixtures = serde_json::from_str(FIXTURES).expect("fixtures parse");
        let case = fixtures
            .cases
            .iter()
            .find(|c| c.name == "advzy-004-single-segment")
            .expect("the single-segment reference case");

        assert_eq!(
            render_cancellation(&ident(&case.ident)),
            "vATCSCC ADVZY 004 DCC 04/14/2020 CANCELLED\n\
             TMI ID: RRDCC004\n\
             20/04/14 14:36"
        );
    }

    /// The number in the header and in the TMI ID are the same one, so a reader cannot be told two
    /// different things about which advisory was cancelled.
    #[test]
    fn a_cancellation_cannot_disagree_with_itself_about_the_number() {
        let fixtures: Fixtures = serde_json::from_str(FIXTURES).expect("fixtures parse");
        for case in &fixtures.cases {
            let out = render_cancellation(&ident(&case.ident));
            let n = case.ident.number;
            assert!(out.contains(&format!("ADVZY {n:03} ")), "{out}");
            assert!(out.contains(&format!("{:03}", n)), "{out}");
            assert_eq!(
                out.matches(&format!("{n:03}")).count(),
                2,
                "the number appears exactly twice — header and TMI ID: {out}"
            );
        }
    }

    /// An absent optional and an empty one must print the same bare label — the document always
    /// shows the section, so `Some("")` must not become `REMARKS: `.
    #[test]
    fn an_empty_optional_prints_the_same_bare_label_as_an_absent_one() {
        assert_eq!(label("REMARKS", opt(&None).as_deref()), "REMARKS:");
        assert_eq!(
            label("REMARKS", opt(&Some("   ".into())).as_deref()),
            "REMARKS:"
        );
        assert_eq!(
            label("REMARKS", opt(&Some("X".into())).as_deref()),
            "REMARKS: X"
        );
        // Whitespace-only is absence however it is spelled, now that `opt` collapses (#498).
        assert_eq!(
            label("REMARKS", opt(&Some("\n\t ".into())).as_deref()),
            "REMARKS:"
        );
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

// ---- generating a document from a program (#508) ----------------------------------------------
//
// #461 settled that a GDP advisory and its `tmu.gdp` row are the same event, so the document is
// derived here rather than retyped by the author. These functions are the mapping, and they live
// beside the renderers because this is document construction — they touch no database.

/// The three-letter form the documents use for an airport — `KJFK` → `JFK`, `PHNL` → `HNL`,
/// `TJSJ` → `SJU`.
///
/// Looked up in the feed's IATA index rather than derived by stripping a character, because the US is
/// not all `K`: `feed::stats::US_ICAO_PREFIXES` lists eight prefixes, and `data/facilities.json` carries
/// PANC, PHNL, TJSJ, PAFA and PHOG. No string rule can do it either — `PHNL` → `HNL` drops two
/// characters and `TJSJ` → `SJU` is not a substring of its ICAO at all.
///
/// `IataMap` is IATA → ICAO, so this is a reverse scan. Linear over ~28k entries and deliberately not
/// indexed: it runs twice per *publish*, an operator action measured in a handful per hour, and adding a
/// third airport map would reach the fetch, `FeedInner` and the refresh job for no measurable gain.
///
/// Falls back to the old `K`-strip when the index has no entry — an empty map in tests, or a feed that
/// has not loaded yet. A wrong-looking element beats an empty one in a published document, and for the
/// contiguous US the fallback is already correct.
fn element_airport(iata: &IataMap, icao: &str) -> String {
    let t = icao.trim().to_ascii_uppercase();
    if let Some((code, _)) = iata.iter().find(|(_, mapped)| **mapped == t) {
        return code.clone();
    }
    match t.strip_prefix('K') {
        Some(rest) if t.len() == 4 => rest.to_string(),
        _ => t,
    }
}

/// `JFK/ZNY` — the header's element slot. Falls back to the airport alone when the program has no
/// ARTCC stamped (`GdpBody::artcc` is filled from the live facility map, not a column, so it can be
/// absent).
fn element_of(iata: &IataMap, airport: &str, artcc: Option<&str>) -> String {
    let apt = element_airport(iata, airport);
    match artcc.map(str::trim).filter(|a| !a.is_empty()) {
        Some(a) => format!("{apt}/{}", a.to_ascii_uppercase()),
        None => apt,
    }
}

/// `14/1415Z - 14/2315Z` — the form `ARRIVALS ESTIMATED FOR` and `CUMULATIVE PROGRAM PERIOD` take.
fn day_window(from: DateTime<Utc>, to: DateTime<Utc>) -> String {
    format!("{} - {}", from.format("%d/%H%MZ"), to.format("%d/%H%MZ"))
}

/// `141415-142315` — the compact form the footer `PERIOD` takes.
fn compact_window(from: DateTime<Utc>, to: DateTime<Utc>) -> String {
    format!("{}-{}", from.format("%d%H%M"), to.format("%d%H%M"))
}

/// `40/40/30` — the per-hour rate profile.
///
/// The reference shows nine values for a nine-hour program. We emit one per configured step, which is
/// the same information in the program's own terms; expanding a stepped rate into one value per clock
/// hour would require inventing how a step that starts mid-hour is reported. A program with no steps
/// emits its single AAR.
fn program_rate(aar: i32, steps: &[AarStep]) -> String {
    if steps.is_empty() {
        return aar.to_string();
    }
    steps
        .iter()
        .map(|s| s.aar.to_string())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod derivation_tests {
    use super::*;
    use chrono::TimeZone;

    /// The feed's index, as `feed::airports::fetch` builds it: IATA → ICAO.
    fn iata() -> IataMap {
        ["JFK:KJFK", "HNL:PHNL", "SJU:TJSJ", "ANC:PANC", "YYZ:CYYZ"]
            .iter()
            .map(|e| {
                let (code, icao) = e.split_once(':').unwrap();
                (code.to_string(), icao.to_string())
            })
            .collect()
    }

    fn at(day: u32, hour: u32, min: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2020, 4, day, hour, min, 0).unwrap()
    }

    /// The documents use the three-letter form, and the US is not all `K`: PANC, PHNL, TJSJ, PAFA and
    /// PHOG are all in `data/facilities.json`. A `K`-strip gets `KJFK` right and every one of those
    /// wrong — `PHNL` drops two characters and `TJSJ → SJU` shares no substring with its ICAO.
    #[test]
    fn the_element_airport_is_looked_up_not_stripped() {
        let m = iata();
        assert_eq!(element_airport(&m, "KJFK"), "JFK");
        assert_eq!(element_airport(&m, "PHNL"), "HNL", "Honolulu is VATUSA");
        assert_eq!(
            element_airport(&m, "TJSJ"),
            "SJU",
            "no string rule produces this"
        );
        assert_eq!(element_airport(&m, "PANC"), "ANC", "Anchorage is VATUSA");
        assert_eq!(element_airport(&m, "CYYZ"), "YYZ");
        assert_eq!(
            element_airport(&m, " kjfk "),
            "JFK",
            "trimmed and upper-cased"
        );
    }

    /// With no index — an empty map in a test, or a feed that has not loaded — the old `K`-strip
    /// stands, because a wrong-looking element beats an empty one in a published document. It must not
    /// mangle a non-`K` code into nonsense while doing so.
    #[test]
    fn an_unknown_airport_falls_back_without_mangling() {
        let empty = IataMap::new();
        assert_eq!(element_airport(&empty, "KJFK"), "JFK");
        assert_eq!(
            element_airport(&empty, "PHNL"),
            "PHNL",
            "not \"HNL\", and not \"HNL\"-by-luck"
        );
        assert_eq!(element_airport(&empty, "TJSJ"), "TJSJ");
        assert_eq!(
            element_airport(&empty, "JFK"),
            "JFK",
            "already three letters"
        );
        assert_eq!(element_airport(&empty, "KSFO"), "SFO");
    }

    #[test]
    fn the_element_slot_carries_the_artcc_when_there_is_one() {
        let m = iata();
        assert_eq!(element_of(&m, "KJFK", Some("ZNY")), "JFK/ZNY");
        assert_eq!(element_of(&m, "PHNL", Some("zak")), "HNL/ZAK");
        assert_eq!(element_of(&m, "KJFK", None), "JFK");
        assert_eq!(element_of(&m, "KJFK", Some("  ")), "JFK", "blank is absent");
    }

    /// **Order is the point.** A window printed end-first reads as a program that finishes before it
    /// starts, and an assertion that each timestamp merely *appears* cannot tell the two apart — which
    /// is how a swapped `from`/`to` survived the handler test it was supposed to be caught by.
    #[test]
    fn the_day_window_prints_start_then_end() {
        assert_eq!(
            day_window(at(14, 14, 15), at(14, 23, 15)),
            "14/1415Z - 14/2315Z"
        );
        assert_ne!(
            day_window(at(14, 14, 15), at(14, 23, 15)),
            day_window(at(14, 23, 15), at(14, 14, 15)),
            "a swapped window must not render identically"
        );
    }

    /// The footer `PERIOD` is day-hour-minute, not any other arrangement of the same digits.
    #[test]
    fn the_compact_window_is_day_then_time() {
        assert_eq!(
            compact_window(at(14, 14, 15), at(15, 2, 30)),
            "141415-150230"
        );
    }

    /// Open-ended forms: a ground stop with no resolved end is `UFN`, not a blank or a guess.
    #[test]
    fn an_open_ended_window_renders_ufn() {
        let from = at(14, 14, 15);
        assert_eq!(
            day_window_open(from, Some(at(14, 23, 15))),
            "14/1415Z - 14/2315Z"
        );
        assert!(day_window_open(from, None).contains("UFN"));
        assert!(compact_window_open(from, None).contains("UFN"));
    }

    /// One value per configured step, in order, falling back to the single AAR when there are none.
    #[test]
    fn the_program_rate_lists_each_step_in_order() {
        let step = |aar| AarStep {
            start_time: "1400".to_string(),
            aar,
        };
        assert_eq!(
            program_rate(40, &[step(40), step(30), step(25)]),
            "40/30/25"
        );
        assert_ne!(
            program_rate(40, &[step(40), step(30), step(25)]),
            program_rate(40, &[step(25), step(30), step(40)]),
            "reversing the steps must not render identically"
        );
        assert_eq!(program_rate(44, &[]), "44", "no steps means the single AAR");
    }
}

/// Build a GDP advisory document from the program, its frozen-slot statistics, and the author's
/// editorial fields.
///
/// `window` is the program's resolved start/end — passed in rather than recomputed here, because the
/// caller has already resolved it to freeze the slots and two answers to the same question is how the
/// document and the program drift.
///
/// `stats` comes from [`crate::feed::gdp::program_stats`] over the same assignments that were frozen,
/// so `MAXIMUM`/`AVERAGE DELAY` describe exactly the rows in `tmu.gdp_slot`. Note that
/// `repos::public` aggregates the same two figures in SQL with `round(avg(...))` where `program_stats`
/// uses integer division, so the public board and this document can differ by a minute on the average.
/// Not reconciled here — that is a visible decision of its own.
pub fn gdp_advisory_from(
    iata: &IataMap,
    gdp: &GdpBody,
    stats: &GdpStats,
    window: (DateTime<Utc>, DateTime<Utc>),
    ed: &PublishGdpRequest,
    now: DateTime<Utc>,
) -> GdpAdvisory {
    let (from, to) = window;
    GdpAdvisory {
        // Constant per kind, not editorial: the reference's header line for every GDP.
        header: "CDM GROUND DELAY PROGRAM".to_string(),
        element: element_of(iata, &gdp.airport, gdp.artcc.as_deref()),
        control_element: element_airport(iata, &gdp.airport),
        // A GDP in OIS always meters an airport's arrivals, so the element is always an airport.
        element_type: "APT".to_string(),
        adl_time: now.format("%H%MZ").to_string(),
        delay_assignment_mode: ed
            .delay_assignment_mode
            .clone()
            .unwrap_or_else(|| "DAS".to_string()),
        arrivals_estimated_for: day_window(from, to),
        cumulative_program_period: day_window(from, to),
        program_rate: program_rate(gdp.aar, &gdp.aar_steps),
        pop_up_factor: ed.pop_up_factor.clone(),
        flights_included: ed.flights_included.clone().unwrap_or_default(),
        departure_scope: ed.departure_scope.clone(),
        additional_dep_facilities_included: ed.additional_dep_facilities_included.clone(),
        exempt_dep_facilities: ed.exempt_dep_facilities.clone(),
        canadian_arpts_included: ed.canadian_arpts_included.clone(),
        delay_assignment_table_applies_to: ed.delay_assignment_table_applies_to.clone(),
        delay_limit: ed.delay_limit.clone(),
        maximum_delay: Some(stats.max_delay_min.to_string()),
        average_delay: Some(stats.avg_delay_min.to_string()),
        impacting_condition: ed.impacting_condition.clone(),
        comments: ed.comments.clone(),
        period: compact_window(from, to),
    }
}

/// `14/1430Z - 14/1630Z`, or `14/1430Z - UFN` for a stop with no stated end.
///
/// `UFN` ("until further notice") is the term the documents use for an open-ended stop, which is what a
/// null `until` means on `tmu.ground_stops`. Printing a fabricated end time instead would be worse.
fn day_window_open(from: DateTime<Utc>, to: Option<DateTime<Utc>>) -> String {
    match to {
        Some(t) => day_window(from, t),
        None => format!("{} - UFN", from.format("%d/%H%MZ")),
    }
}

/// `141430-141630`, or `141430-UFN`.
fn compact_window_open(from: DateTime<Utc>, to: Option<DateTime<Utc>>) -> String {
    match to {
        Some(t) => compact_window(from, t),
        None => format!("{}-UFN", from.format("%d%H%M")),
    }
}

/// Build a Ground Stop advisory document from the program and the author's editorial fields.
///
/// Far less derives here than for a GDP, and that is the data's fault rather than an omission: a ground
/// stop has no slot table and no delay computation anywhere, so every delay figure is author-supplied.
/// `window` is the stop's resolved period.
pub fn ground_stop_advisory_from(
    iata: &IataMap,
    gs: &GroundStopBody,
    window: (DateTime<Utc>, Option<DateTime<Utc>>),
    ed: &PublishGroundStopRequest,
    now: DateTime<Utc>,
) -> GroundStopAdvisory {
    let (from, to) = window;
    GroundStopAdvisory {
        header: "CDM GROUND STOP".to_string(),
        element: element_of(iata, &gs.airport, gs.artcc.as_deref()),
        control_element: element_airport(iata, &gs.airport),
        element_type: "APT".to_string(),
        adl_time: now.format("%H%MZ").to_string(),
        ground_stop_period: day_window_open(from, to),
        cumulative_program_period: day_window_open(from, to),
        flights_included: ed.flights_included.clone().unwrap_or_default(),
        additional_dep_facilities_included: ed.additional_dep_facilities_included.clone(),
        current_delays: ed.current_delays.clone(),
        previous_delays: ed.previous_delays.clone(),
        new_delays: ed.new_delays.clone(),
        probability_of_extension: ed.probability_of_extension.clone(),
        impacting_condition: ed.impacting_condition.clone(),
        comments: ed.comments.clone(),
        period: compact_window_open(from, to),
    }
}

/// #498 — no author-supplied value may introduce a line or shift a column.
#[cfg(test)]
mod whitespace_tests {
    use super::*;
    use crate::models::{RerouteRow, RerouteSegment, RerouteValid};
    use chrono::TimeZone;

    /// Interior newline, interior tab, and a run of spaces, in one value. Collapsed, it is `A B C D`.
    const POISON: &str = "A\nB\tC  D";

    fn ident() -> AdvisoryIdent {
        AdvisoryIdent {
            facility: "DCC".to_string(),
            number: 4,
            issued_day: NaiveDate::from_ymd_opt(2020, 4, 14).unwrap(),
            signed_at: Utc.with_ymd_and_hms(2020, 4, 14, 14, 36, 0).unwrap(),
        }
    }

    /// Every field set to `value`.
    ///
    /// **Spelled out as a full struct literal on purpose.** A literal must name every field, so
    /// adding one to `RerouteAdvisory` breaks this test's *compilation* and whoever adds it has to
    /// decide whether it needs poisoning. Building it from the fixture with `..` or from JSON would
    /// leave a new field silently untested — which is how #498 survived #458 in the first place.
    fn advisory(value: &str) -> RerouteAdvisory {
        RerouteAdvisory {
            header: value.to_string(),
            name: value.to_string(),
            impacted_area: value.to_string(),
            reason: Some(value.to_string()),
            include_traffic: Some(value.to_string()),
            valid: RerouteValid {
                // Not poisoned: an enum is not author-supplied text, and the two bases render
                // different line shapes, so varying it would change the expected line count.
                basis: RerouteValidBasis::FcaEntryTime,
                from: value.to_string(),
                to: value.to_string(),
            },
            facilities_included: Some(value.to_string()),
            probability_of_extension: Some(value.to_string()),
            remarks: Some(value.to_string()),
            associated_restrictions: Some(value.to_string()),
            modifications: Some(value.to_string()),
            routes: RerouteRoutes::Single {
                rows: vec![RerouteRow {
                    orig: value.to_string(),
                    dest: value.to_string(),
                    route: value.to_string(),
                }],
            },
        }
    }

    fn segmented(value: &str) -> RerouteAdvisory {
        RerouteAdvisory {
            routes: RerouteRoutes::Segmented {
                origin: vec![RerouteSegment {
                    orig: value.to_string(),
                    route: value.to_string(),
                }],
                destination: vec![RerouteSegment {
                    orig: value.to_string(),
                    route: value.to_string(),
                }],
            },
            ..advisory(value)
        }
    }

    /// The core assertion, and the reason it is a line *count* rather than a list of fields: a
    /// poisoned document must have exactly as many lines as a clean one. Any field that leaks a
    /// newline adds a line, whichever field it is and whatever the print site does with it.
    #[test]
    fn a_poisoned_document_has_the_same_line_count_as_a_clean_one() {
        for (what, poisoned, clean_doc) in [
            ("single", advisory(POISON), advisory("A B C D")),
            ("segmented", segmented(POISON), segmented("A B C D")),
        ] {
            let out = render_reroute(&poisoned, &ident());
            assert_eq!(
                out.lines().count(),
                render_reroute(&clean_doc, &ident()).lines().count(),
                "{what}: a field introduced a line:\n{out}"
            );
            // Byte-identical, in fact — collapsing is the only difference the poison can make.
            assert_eq!(
                out,
                render_reroute(&clean_doc, &ident()),
                "{what}: collapsed output must match the already-clean value"
            );
        }
    }

    /// No tab may survive into a document: it would shift a table column by a terminal-dependent
    /// amount, so alignment could not be reasoned about at all.
    #[test]
    fn no_tab_survives_into_the_document() {
        let out = render_reroute(&advisory(POISON), &ident());
        assert!(!out.contains('\t'), "tab reached the document:\n{out}");
    }

    /// The issue's own payload: a `remarks` value crafted to look like a signature block must not
    /// produce a second one inside the document.
    #[test]
    fn a_crafted_remarks_value_cannot_forge_a_signature_block() {
        let mut a = advisory("X");
        a.remarks = Some("NONE\nTMI ID: RRDCC001\n142030-150230".to_string());
        let out = render_reroute(&a, &ident());

        assert_eq!(
            out.lines().filter(|l| l.starts_with("TMI ID:")).count(),
            1,
            "exactly one TMI ID line:\n{out}"
        );
        assert!(
            out.contains("REMARKS: NONE TMI ID: RRDCC001 142030-150230"),
            "the value survives, on one line:\n{out}"
        );
        // The real footer is still the last three lines, not something a field invented.
        let tail: Vec<&str> = out.lines().rev().take(3).collect();
        assert_eq!(tail[0], "20/04/14 14:36", "signature is last:\n{out}");
    }

    /// `table` collapses each cell itself, which the renderers make redundant — `route_section`
    /// already collapses what it passes in. Tested directly because otherwise that collapse is
    /// unreachable defence that no test would notice the loss of: removing it leaves every
    /// document-level case green.
    ///
    /// It is kept rather than deleted because `table` is where the padding is computed, and a
    /// leaked cell there does not merely add a line — it shifts every column to its right. AC3 is
    /// specifically about alignment, so the guarantee belongs next to the arithmetic.
    #[test]
    fn table_collapses_a_cell_so_padding_cannot_be_shifted() {
        let rows = vec![
            vec!["A\nB".to_string(), "X\tY".to_string(), "Z  Z".to_string()],
            vec!["CD".to_string(), "EF".to_string(), "GH".to_string()],
        ];
        let out = table(&["ONE", "TWO", "THREE"], &[6, 6], &rows);

        assert_eq!(out.len(), 4, "heading, rule, two rows: {out:?}");
        for line in &out {
            assert_eq!(line.lines().count(), 1, "a cell added a line: {line:?}");
            assert!(!line.contains('\t'), "a tab survived: {line:?}");
        }
        // Both data rows must put the third column at the same offset.
        assert_eq!(
            out[2].find("Z Z"),
            out[3].find("GH"),
            "third column misaligned: {out:?}"
        );
    }

    /// A route cell is the one value that also carries column alignment, so it gets its own case:
    /// the ROUTE column must start at the same offset on every row.
    #[test]
    fn a_poisoned_route_cell_keeps_the_table_aligned() {
        let a = RerouteAdvisory {
            routes: RerouteRoutes::Single {
                rows: vec![
                    RerouteRow {
                        orig: "EWR".to_string(),
                        dest: "ORD".to_string(),
                        route: "J80\nJ146".to_string(),
                    },
                    RerouteRow {
                        orig: "JFK".to_string(),
                        dest: "BOS".to_string(),
                        route: "J75".to_string(),
                    },
                ],
            },
            ..advisory("X")
        };
        let out = render_reroute(&a, &ident());

        let rows: Vec<&str> = out
            .lines()
            .filter(|l| l.starts_with("EWR") || l.starts_with("JFK"))
            .collect();
        assert_eq!(rows.len(), 2, "both rows on one line each:\n{out}");
        assert_eq!(
            rows[0].find("J80"),
            rows[1].find("J75"),
            "the ROUTE column must start at the same offset:\n{out}"
        );
        assert!(
            out.contains("J80 J146"),
            "the route survives collapsed:\n{out}"
        );
    }
}
