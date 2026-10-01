//! Text hygiene shared by the single-line document renderers.
//!
//! [`crate::tmi`] renders an NTML restriction to one line; [`crate::advisory`] renders an ADVZY
//! advisory to a document whose every field is one line. Both are formats where a line break *means*
//! something, so an author-supplied value must not be able to introduce one.

/// Collapse every run of whitespace — including newlines and tabs — to a single space.
///
/// Subsumes `trim`: `split_whitespace` ignores leading and trailing whitespace as well as interior
/// runs, so a caller needs this and not both.
///
/// # Why this exists rather than a `trim` at each print site
///
/// `trim` removes whitespace at the ends and does nothing to a `\n` in the middle, so a field value
/// containing one became extra lines in the rendered output (#498). For a structured advisory that
/// broke the invariant the structured path exists to provide: the `body` is re-derived from the
/// fields so the two cannot disagree (#458, #488), but a field carrying newlines means the
/// document's *shape* is no longer determined by the model. The issue's own example,
/// `remarks = "NONE\nTMI ID: RRDCC001\n142030-150230"`, renders a second signature block inside the
/// document.
///
/// It lives here, applied by the helpers that *produce* values, rather than at the places that print
/// them — because the print sites are exactly what went wrong. `tmi::encode`'s `"TXT"` arm and
/// `tmi::english` both reach output without calling `clean` at all, so a fix applied per print site
/// misses them, and misses whatever the next renderer adds. Collapsing at the source means a new
/// output path cannot bypass it.
///
/// Collapsing is lossless for every legitimate value in these formats: a single-line document field
/// has no use for an interior newline, and a route is already a space-separated fix list. Rejecting
/// the input instead would tell the author rather than silently reformatting, but it is a behaviour
/// change on endpoints that accept these values today — so that is a validation concern, filed
/// separately if wanted, not this fix.
pub(crate) fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::collapse;

    #[test]
    fn collapses_newlines_tabs_and_runs_to_single_spaces() {
        assert_eq!(collapse("A\nB\tC  D"), "A B C D");
        assert_eq!(collapse("A\r\nB"), "A B");
        assert_eq!(collapse("A\n\n\nB"), "A B");
    }

    /// It replaces `trim` rather than complementing it, so the ends must come out clean too.
    #[test]
    fn also_trims_the_ends() {
        assert_eq!(collapse("  A B  "), "A B");
        assert_eq!(collapse("\n\tA\n"), "A");
    }

    /// An all-whitespace value collapses to empty, which is what lets the callers' "absent and
    /// empty behave alike" rule keep working.
    #[test]
    fn whitespace_only_becomes_empty() {
        assert_eq!(collapse("   "), "");
        assert_eq!(collapse("\n\t\n"), "");
        assert_eq!(collapse(""), "");
    }

    /// Neither renderer may call `.trim()`, because [`collapse`] subsumes it and a `trim` is
    /// precisely the half-measure #498 was filed about.
    ///
    /// A source-level assertion rather than a behavioural one, following
    /// `jobs::registration_tests`' use of `include_str!`: the failure this guards against is a
    /// *new* field or print site added later that trims instead of collapsing, and no behavioural
    /// test can cover code that does not exist yet. Concretely, PR #507 adds two renderers whose
    /// `flt_incl` and `period` paths trim — when those branches meet, this fails and names the file
    /// instead of shipping a quiet regression.
    ///
    /// Comment lines are stripped first, since the prose here and in both renderers discusses
    /// `trim` deliberately.
    #[test]
    fn neither_renderer_calls_trim() {
        for (name, src) in [
            ("advisory.rs", include_str!("advisory.rs")),
            ("tmi.rs", include_str!("tmi.rs")),
        ] {
            let offenders: Vec<(usize, &str)> = src
                .lines()
                .enumerate()
                .filter(|(_, l)| !l.trim_start().starts_with("//"))
                .filter(|(_, l)| l.contains(".trim()"))
                .map(|(i, l)| (i + 1, l.trim()))
                .collect();
            assert!(
                offenders.is_empty(),
                "{name} must collapse, not trim (#498): {offenders:?}"
            );
        }
    }

    /// A value with nothing to fix must come back byte-identical — the fix has to be invisible to
    /// every legitimate document field, which is why the reference fixtures do not move.
    #[test]
    fn a_clean_value_is_unchanged() {
        assert_eq!(collapse("JFK arrivals via CAMRN"), "JFK arrivals via CAMRN");
        assert_eq!(collapse("14/1415Z - 14/2315Z"), "14/1415Z - 14/2315Z");
        assert_eq!(
            collapse("40/40/40/30/25/20/20/36/54"),
            "40/40/40/30/25/20/20/36/54"
        );
    }
}
