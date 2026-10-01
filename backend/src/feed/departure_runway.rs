//! The departure-runway prediction ladder (VATUSA/OIS#511, sub-issue C of #434).
//!
//! Mirrors the arrival side's shape — [`crate::feed::runway::assign`] walks
//! `override → STAR rule → AUTO` and returns the chosen rung alongside the runway. This is the
//! departure mirror, and it is **pure**: it reads no database and holds no state, so the caller does
//! every bit of I/O. That is what lets each rung have a test of its own, and it keeps the module inside
//! the feed's no-DB-handle rule (`AGENTS.md`).
//!
//! # Why a prediction is needed at all
//!
//! A runway is inferred today only once an aircraft is *rolling*:
//! `feed::flow::resolve_ground_allowance` refuses a heading match below
//! [`crate::feed::flow::TAXI_ROLL_GS_KT`], because a parked aircraft's heading says nothing about which
//! runway it will use. So every parked and every prefiled departure falls to the airport/default taxi
//! tier — exactly the flights IDST sequences, and the taxi estimate is what the EDCT is built from.
//!
//! # `None` is an answer
//!
//! Every rung can decline, and declining is correct. A plausible-looking wrong runway would narrow the
//! learned taxi estimate to the wrong bucket and therefore move the EDCT; having no runway merely
//! leaves the estimate where it already is. So there is no guess of last resort here.

use crate::models::AirportConfigBody;
use crate::repos::departure_runway::RunwaySource;

/// Predict a departure runway, and say which rung produced it.
///
/// Rungs, in order: **SID rule → gate rule → the active config's `departure_runways` → `None`.**
///
/// # The manual rung is deliberately absent
///
/// #511's ladder begins with a controller's manual override, and that rung is **not** implemented here.
/// It lives in `repos::departure_runway::assign`'s `on conflict` guard, which compares ladder rank in
/// SQL — the only place a caller cannot forget it, and the only place that is safe against two derives
/// racing. So this function never proposes [`RunwaySource::Manual`]: it proposes the best *derived*
/// answer, and a manual override outranks it at the point of writing (#511 AC4).
///
/// `RunwaySource::Auto` is likewise never proposed. #511's ladder ends at "none", and inventing an
/// automatic pick would be adding behaviour the issue declined.
///
/// # Arguments
///
/// `gate_name` is the stand's **name** (`A1`), not its id: `gate_rules` is authored by a facility, who
/// types a stand name rather than a uuid, so the caller resolves `nearest_gate`'s id through the gate
/// catalog first. `None` for a prefile — which has no position at all, so no stand can be matched — and
/// for a parked aircraft that is not within tolerance of a known stand.
///
/// `sid` is the revision-stripped SID base from [`crate::feed::delays::sid_of`], so `CAMRN4` and
/// `CAMRN3` are one rule. Available for a prefile, which is what makes this rung the useful one for the
/// case #511 calls large.
///
/// `config` is the wind-favoured config for the airport, or `None` when the airport has none at all.
pub fn predict(
    gate_name: Option<&str>,
    sid: Option<&str>,
    config: Option<&AirportConfigBody>,
) -> Option<(String, RunwaySource)> {
    // No configuration means no rules and no default: nothing to predict from. This is the first of
    // #511's three mandatory fallbacks.
    let config = config?;

    // Rung 2a — the SID rule. Above the gate rule because a SID is a filed intention about where the
    // aircraft is going, while a stand is only where it happens to be parked; two aircraft on adjacent
    // stands can file different SIDs and should get different runways.
    if let Some(runway) = sid.and_then(|s| lookup(&config.sid_rules.0, s)) {
        return Some((runway, RunwaySource::Rule));
    }

    // Rung 2b — the gate rule.
    if let Some(runway) = gate_name.and_then(|g| lookup(&config.gate_rules.0, g)) {
        return Some((runway, RunwaySource::Rule));
    }

    // Rung 3 — the config's own departure runways. The first is the primary; this makes no attempt to
    // balance across them, which is what `Auto` would be for.
    if let Some(runway) = config.departure_runways.first() {
        return Some((runway.clone(), RunwaySource::Config));
    }

    // Rung 4 — a config that names no departure runways predicts nothing, rather than falling back to
    // `landing_runways` (which would point an aircraft at a runway configured for arrivals).
    None
}

/// A rule lookup that ignores case and surrounding space on both sides.
///
/// Keys arrive from two directions — a facility typing into the editor, and `sid_of`/the gate catalog —
/// and `handlers::airport_configs` already compares runways case-insensitively when it validates these
/// rules. Matching case-sensitively here would make a rule that *validated* fail to *fire*.
fn lookup(rules: &std::collections::HashMap<String, String>, key: &str) -> Option<String> {
    let want = key.trim().to_ascii_uppercase();
    rules
        .iter()
        .find(|(k, _)| k.trim().to_ascii_uppercase() == want)
        .map(|(_, v)| v.trim().to_ascii_uppercase())
}

/// One departure the ladder can be run against.
///
/// Carries only what [`predict`] needs, resolved from the live snapshot: the stand **name** (not id)
/// and the revision-stripped SID base. Produced by [`candidates`] so the job does no snapshot walking
/// of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub icao: String,
    pub callsign: String,
    pub gate_name: Option<String>,
    pub sid: Option<String>,
}

/// Every not-yet-departed flight in `data`, with the ladder's inputs resolved.
///
/// Selection mirrors [`super::flow::pending_departures`] exactly — a flight is still a departure until
/// `groundspeed > 60 && altitude > 300`, and a prefile is skipped when a connected pilot already holds
/// its callsign. Two different answers to "is this still a departure" would be a bug waiting to happen,
/// so the condition is copied deliberately and this comment is the pointer between them.
///
/// The gate is resolved here rather than by the caller because it needs two things at once: the
/// position, which only a connected pilot has, and the catalog, to turn `nearest_gate`'s id into the
/// **name** a facility's `gate_rules` are keyed on. A prefile yields `None` — it has no position at all
/// (`feed::vatsim::Prefile` has no coordinate fields), so no stand can be matched for one, ever.
pub fn candidates(
    data: &crate::feed::vatsim::VatsimData,
    gates: &std::collections::HashMap<String, Vec<crate::models::AirportGateBody>>,
) -> Vec<Candidate> {
    let empty: Vec<crate::models::AirportGateBody> = Vec::new();
    let mut out: Vec<Candidate> = Vec::new();

    for p in &data.pilots {
        let Some(fp) = &p.flight_plan else { continue };
        // Same airborne test as `pending_departures`.
        if p.groundspeed > 60 && p.altitude > 300 {
            continue;
        }
        let icao = fp.departure.to_ascii_uppercase();
        let at_field = gates.get(&icao).unwrap_or(&empty);
        let gate_name = super::flow::nearest_gate(at_field, p.latitude, p.longitude)
            .and_then(|id| at_field.iter().find(|g| g.id == id).map(|g| g.name.clone()));
        out.push(Candidate {
            icao,
            callsign: p.callsign.clone(),
            gate_name,
            sid: super::delays::sid_of(&fp.route),
        });
    }

    for pf in &data.prefiles {
        let Some(fp) = &pf.flight_plan else { continue };
        if out.iter().any(|c| c.callsign == pf.callsign) {
            continue;
        }
        out.push(Candidate {
            icao: fp.departure.to_ascii_uppercase(),
            callsign: pf.callsign.clone(),
            // A prefile has no position, so no stand. The SID rung is what serves it.
            gate_name: None,
            sid: super::delays::sid_of(&fp.route),
        });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn rules(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn config(
        dep_runways: &[&str],
        sid: &[(&str, &str)],
        gate: &[(&str, &str)],
    ) -> AirportConfigBody {
        AirportConfigBody {
            id: "c1".into(),
            icao: "KJFK".into(),
            name: "South".into(),
            aar: 30,
            adr: 30,
            landing_runways: vec!["22L".into()],
            departure_runways: dep_runways.iter().map(|r| r.to_string()).collect(),
            sid_rules: sqlx::types::Json(rules(sid)),
            gate_rules: sqlx::types::Json(rules(gate)),
            wind_from_deg: 0,
            wind_to_deg: 360,
            calm_default: true,
            artcc: "ZNY".into(),
            updated_at: chrono::Utc::now(),
            updated_by: None,
            editable: false,
        }
    }

    /// Rung 2a. Above the gate rule on purpose: a SID is a filed intention about where the aircraft is
    /// going, a stand is only where it is parked.
    #[test]
    fn a_sid_rule_wins_over_a_gate_rule() {
        let c = config(&["04L"], &[("CAMRN", "31L")], &[("A1", "22R")]);
        assert_eq!(
            predict(Some("A1"), Some("CAMRN"), Some(&c)),
            Some(("31L".to_string(), RunwaySource::Rule))
        );
    }

    /// Rung 2b, when no SID rule matches.
    #[test]
    fn a_gate_rule_wins_over_the_config_default() {
        let c = config(&["04L"], &[("CAMRN", "31L")], &[("A1", "22R")]);
        assert_eq!(
            predict(Some("A1"), Some("HAPIE"), Some(&c)),
            Some(("22R".to_string(), RunwaySource::Rule))
        );
    }

    /// Rung 3, when neither rule matches.
    #[test]
    fn the_config_default_applies_when_no_rule_matches() {
        let c = config(&["04L", "13R"], &[("CAMRN", "31L")], &[("A1", "22R")]);
        assert_eq!(
            predict(Some("B7"), Some("HAPIE"), Some(&c)),
            Some(("04L".to_string(), RunwaySource::Config)),
            "the first departure runway is the primary; balancing across them would be `Auto`"
        );
    }

    /// Keys are matched case- and space-insensitively, because the validator that *accepted* these rules
    /// compares that way too — a rule that validates but never fires would be worse than a rejection.
    #[test]
    fn rule_keys_and_runways_ignore_case_and_space() {
        let c = config(&[], &[(" camrn ", " 31l ")], &[]);
        assert_eq!(
            predict(None, Some("CAMRN"), Some(&c)),
            Some(("31L".to_string(), RunwaySource::Rule))
        );
    }

    /// `sid_of` strips the revision, so one rule covers every revision of a procedure.
    #[test]
    fn the_sid_key_is_the_revision_stripped_base() {
        use crate::feed::delays::sid_of;
        assert_eq!(sid_of("CAMRN4 J174 ABC").as_deref(), Some("CAMRN"));
        assert_eq!(sid_of("CAMRN3 J174 ABC").as_deref(), Some("CAMRN"));

        let c = config(&[], &[("CAMRN", "31L")], &[]);
        for route in ["CAMRN4 J174 ABC", "CAMRN3 J174 ABC"] {
            assert_eq!(
                predict(None, sid_of(route).as_deref(), Some(&c)),
                Some(("31L".to_string(), RunwaySource::Rule)),
                "every revision of CAMRN is one rule"
            );
        }
    }

    /// #511 AC2, fallback 1: an airport with no configuration at all.
    #[test]
    fn no_config_predicts_nothing() {
        assert_eq!(predict(Some("A1"), Some("CAMRN"), None), None);
    }

    /// #511 AC2, fallback 2: a gate that matches no rule, at a config with no departure runways. A wrong
    /// runway would move the EDCT; no runway leaves the taxi estimate where it already was.
    #[test]
    fn no_matching_rule_and_no_default_predicts_nothing() {
        let c = config(&[], &[("CAMRN", "31L")], &[("A1", "22R")]);
        assert_eq!(predict(Some("B7"), Some("HAPIE"), Some(&c)), None);
    }

    /// A config naming only landing runways must not point a departure at one of them.
    #[test]
    fn a_config_with_only_landing_runways_predicts_nothing() {
        let c = config(&[], &[], &[]);
        assert!(
            !c.landing_runways.is_empty(),
            "the fixture has a landing runway"
        );
        assert_eq!(predict(Some("A1"), Some("CAMRN"), Some(&c)), None);
    }

    /// #511 AC3 and AC2's third fallback: a prefile has no position, so no stand can be matched — but it
    /// does have a filed route, so the SID rung still answers. This is the case the issue calls large.
    #[test]
    fn a_prefile_with_no_position_still_gets_its_sid_rule() {
        let c = config(&["04L"], &[("CAMRN", "31L")], &[("A1", "22R")]);
        assert_eq!(
            predict(None, Some("CAMRN"), Some(&c)),
            Some(("31L".to_string(), RunwaySource::Rule)),
            "no gate, but the filed SID is enough"
        );
    }

    /// And a prefile whose SID matches nothing falls to the config default rather than nothing — still a
    /// defined result, which is what AC3 asks for.
    #[test]
    fn a_prefile_with_no_matching_sid_falls_to_the_config_default() {
        let c = config(&["04L"], &[("CAMRN", "31L")], &[("A1", "22R")]);
        assert_eq!(
            predict(None, Some("HAPIE"), Some(&c)),
            Some(("04L".to_string(), RunwaySource::Config))
        );
    }

    /// A prefile with neither a usable SID nor a config default: defined, and declining.
    #[test]
    fn a_prefile_with_nothing_to_go_on_predicts_nothing() {
        let c = config(&[], &[("CAMRN", "31L")], &[]);
        assert_eq!(predict(None, None, Some(&c)), None);
    }

    /// #511 AC4, the caller's half. The ladder proposes only derived rungs; `Manual` is never among them,
    /// so a controller's override cannot be displaced by a derive — and `Auto` is not proposed either,
    /// because #511's ladder ends at "none".
    #[test]
    fn the_ladder_never_proposes_manual_or_auto() {
        let c = config(&["04L"], &[("CAMRN", "31L")], &[("A1", "22R")]);
        let proposals = [
            predict(Some("A1"), Some("CAMRN"), Some(&c)),
            predict(Some("A1"), Some("HAPIE"), Some(&c)),
            predict(Some("B7"), Some("HAPIE"), Some(&c)),
            predict(None, None, Some(&c)),
        ];
        for p in proposals.into_iter().flatten() {
            assert!(
                matches!(p.1, RunwaySource::Rule | RunwaySource::Config),
                "a derive proposed {:?}, which would outrank or invent a rung it must not",
                p.1
            );
        }
    }
}
