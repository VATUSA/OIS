//! Route-coverage analysis — how much of live filed traffic the nav engine fully resolves,
//! and which tokens it still can't. Drives data-coverage work and surfaces real gaps.

use std::collections::HashMap;

use serde::Serialize;
use utoipa::ToSchema;

use super::airports::AirportDb;
use super::nav::NavData;
use super::vatsim::VatsimData;

#[derive(Debug, Serialize, ToSchema, Default)]
pub struct CoverageReport {
    /// Pilots with a non-empty filed route.
    pub pilots_with_route: usize,
    /// …of those, how many resolved with zero unresolved tokens.
    pub fully_resolved: usize,
    pub partial: usize,
    /// Percentage of routed pilots fully resolved.
    pub resolved_pct: f64,
    /// Most common unresolved tokens, most-frequent first (capped).
    pub top_unresolved: Vec<UnresolvedToken>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UnresolvedToken {
    pub token: String,
    pub count: usize,
}

/// Resolve every routed pilot and tally coverage + the most common unresolved tokens.
pub fn analyze(nav: &NavData, airports: &AirportDb, data: &VatsimData) -> CoverageReport {
    let mut with_route = 0usize;
    let mut full = 0usize;
    let mut tally: HashMap<String, usize> = HashMap::new();

    for p in &data.pilots {
        let Some(fp) = &p.flight_plan else { continue };
        if fp.route.trim().is_empty() {
            continue;
        }
        with_route += 1;
        let res = nav.build_anchors(airports, &fp.departure, &fp.arrival, &fp.route);
        if res.unresolved.is_empty() {
            full += 1;
        } else {
            for t in res.unresolved {
                *tally.entry(t).or_default() += 1;
            }
        }
    }

    let mut top: Vec<UnresolvedToken> = tally
        .into_iter()
        .map(|(token, count)| UnresolvedToken { token, count })
        .collect();
    top.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.token.cmp(&b.token)));
    top.truncate(40);

    CoverageReport {
        pilots_with_route: with_route,
        fully_resolved: full,
        partial: with_route - full,
        resolved_pct: if with_route > 0 {
            full as f64 / with_route as f64 * 100.0
        } else {
            0.0
        },
        top_unresolved: top,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "network: snapshots live VATSIM route coverage"]
    async fn live_coverage() {
        let nav = NavData::load();
        let client = reqwest::Client::builder()
            .user_agent("ois-coverage/1.0")
            .build()
            .unwrap();
        let (airports, _iata) = crate::feed::airports::fetch(&client).await.unwrap();
        let data = crate::feed::vatsim::fetch(&client).await.unwrap();
        let r = analyze(&nav, &airports, &data);
        println!(
            "\nrouted={} fully_resolved={} ({:.1}%) partial={}",
            r.pilots_with_route, r.fully_resolved, r.resolved_pct, r.partial
        );
        println!("top unresolved tokens:");
        for u in &r.top_unresolved {
            println!("  {:>4}  {}", u.count, u.token);
        }
    }
}
