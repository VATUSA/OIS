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

use crate::models::{RerouteAdvisory, RerouteRoutes, RerouteValidBasis};
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
