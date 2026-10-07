//! VATUSA/OIS#725: the sector demand route, through the real router — session resolution,
//! `RequirePermission`, the scope flags, the projection and the binning all on the path.
//!
//! Limits and counts are literals, never `DEFAULT_LIMIT`: a fixture derived from the constant under
//! test passes for every value of it.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use axum::http::{Method, StatusCode};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::{
    feed::{
        Snapshot,
        airports::Airport,
        sectors::{SectorTable, SectorVolume, tests::volume},
        vatsim::{FlightPlan, Pilot, Prefile, VatsimData},
    },
    scope_test_support::{grant, seed_user, send, send_json, session_cookie, test_state},
    state::AppState,
};

const READ: &str = "flow.sectors.read";

/// A volume over the whole KJFK–KDCA corridor (37–42N, 72–78W), every altitude.
fn corridor(artcc: &str, volume_id: &str, tier: &str) -> SectorVolume {
    SectorVolume {
        tier: tier.into(),
        base_alt_ft: 0,
        top_alt_ft: 60_000,
        rings: vec![vec![
            [37.0, -78.0],
            [37.0, -72.0],
            [42.0, -72.0],
            [42.0, -78.0],
            [37.0, -78.0],
        ]],
        ..volume(artcc, volume_id)
    }
}

/// A volume far from the corridor (30–31N, 90–91W), so it never counts the corridor's traffic.
fn elsewhere(artcc: &str, volume_id: &str, tier: &str) -> SectorVolume {
    SectorVolume {
        tier: tier.into(),
        rings: vec![vec![
            [30.0, -91.0],
            [30.0, -90.0],
            [31.0, -90.0],
            [31.0, -91.0],
            [30.0, -91.0],
        ]],
        ..volume(artcc, volume_id)
    }
}

/// ZDC: `010` (high, over the corridor), `020` (low, elsewhere) and `070` (approach, elsewhere). ZSE has
/// enroute volumes only, as in the real dataset. ZOB has none.
fn state(pool: PgPool) -> AppState {
    let state = test_state(pool, HashMap::new());
    state.airspace_sectors.store(Arc::new(SectorTable {
        volumes: vec![
            corridor("ZDC", "01001", "high"),
            elsewhere("ZDC", "02001", "low"),
            elsewhere("ZDC", "07001", "approach"),
            elsewhere("ZSE", "03001", "low"),
            elsewhere("ZSE", "04001", "ultra_high"),
        ],
    }));
    state
}

/// KJFK -> KDCA via a route the bundled nav db resolves (as `feed::sector_tracks`' tests).
fn plan() -> FlightPlan {
    FlightPlan {
        departure: "KJFK".into(),
        arrival: "KDCA".into(),
        route: "RBV WHITE SIE".into(),
        aircraft_short: "B738".into(),
        cruise_tas: "440".into(),
        altitude: "35000".into(),
        ..Default::default()
    }
}

/// Airborne just south of KJFK at FL240, tracking SW down the corridor.
fn airborne(callsign: &str) -> Pilot {
    Pilot {
        callsign: callsign.into(),
        latitude: 40.2,
        longitude: -74.0,
        altitude: 24_000,
        groundspeed: 420,
        heading: 220,
        flight_plan: Some(plan()),
        ..Default::default()
    }
}

/// Install a feed cycle at `fetched_at` holding `data`, with KJFK and KDCA known.
async fn cycle(state: &AppState, data: VatsimData, fetched_at: DateTime<Utc>) {
    let mut feed = state.feed.write().await;
    feed.airports = Arc::new(HashMap::from([
        ("KJFK".to_string(), Airport::at(40.64, -73.78)),
        ("KDCA".to_string(), Airport::at(38.85, -77.04)),
    ]));
    feed.snapshot = Some(Arc::new(Snapshot {
        fetched_at,
        source_timestamp: String::new(),
        data,
    }));
}

/// A signed-in user holding `flow.sectors.read` nationally, plus each of `grants` at its ARTCC.
async fn user(pool: &PgPool, grants: &[(&str, &str)]) -> String {
    let id = seed_user(pool).await;
    grant(pool, &id, READ, None).await;
    for (permission, artcc) in grants {
        grant(pool, &id, permission, Some(artcc)).await;
    }
    session_cookie(pool, &id).await
}

async fn get(state: &AppState, artcc: &str, cookie: &str) -> Value {
    let uri = format!("/api/v1/flow/sector-demand/{artcc}");
    let (status, body) = send_json(state, Method::GET, &uri, cookie).await;
    assert_eq!(status, StatusCode::OK, "{uri}: {body}");
    body
}

/// The row for `sector_id` in `table` (`enroute` or `tracon`).
fn row<'a>(body: &'a Value, table: &str, sector_id: &str) -> &'a Value {
    body[table]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["sector_id"] == sector_id)
        .unwrap_or_else(|| panic!("no {table} row {sector_id} in {body}"))
}

/// AC "a facility with no sector data is named": ZOB has no volumes, so it answers `no_sector_data`
/// under its own id with no rows in either table — before the first cycle and after it.
#[sqlx::test]
async fn an_artcc_with_no_sector_data_says_so_by_name(pool: PgPool) {
    let state = state(pool.clone());
    let cookie = user(&pool, &[]).await;

    for when in ["before the first cycle", "after it"] {
        let body = get(&state, "zob", &cookie).await;
        assert_eq!(body["artcc"], "ZOB", "{when}");
        assert_eq!(body["status"], "no_sector_data", "{when}");
        assert_eq!(
            body["enroute"],
            json!({ "has_sector_data": false, "rows": [] })
        );
        assert_eq!(
            body["tracon"],
            json!({ "has_sector_data": false, "rows": [] })
        );
        cycle(&state, VatsimData::default(), Utc::now()).await;
    }
}

/// Before the first feed cycle a facility with data says `pending`, not an empty `ready` grid.
#[sqlx::test]
async fn before_the_first_cycle_it_is_pending(pool: PgPool) {
    let state = state(pool.clone());
    let cookie = user(&pool, &[]).await;

    let body = get(&state, "ZDC", &cookie).await;
    assert_eq!(body["status"], "pending");
    assert_eq!(body["cycle_at"], Value::Null);
    assert_eq!(body["bin_starts_ms"], json!([]));
    assert_eq!(
        body["enroute"],
        json!({ "has_sector_data": true, "rows": [] })
    );
    assert_eq!(
        body["tracon"],
        json!({ "has_sector_data": true, "rows": [] })
    );
}

/// A quiet cycle is `ready` with a row of zeros per sector, enroute and TRACON apart, 24 bins on
/// absolute Zulu quarter-hours from the one containing the cycle. ZSE has enroute rows and no TRACON
/// data, which is not the same as a quiet TRACON.
#[sqlx::test]
async fn a_quiet_cycle_is_two_tables_of_zero_rows_on_zulu_quarter_hours(pool: PgPool) {
    let state = state(pool.clone());
    let cookie = user(&pool, &[]).await;
    let at: DateTime<Utc> = "2026-10-07T14:07:31Z".parse().unwrap();
    cycle(&state, VatsimData::default(), at).await;

    let body = get(&state, "ZDC", &cookie).await;
    assert_eq!(body["status"], "ready");
    assert_eq!(body["cycle_at"], "2026-10-07T14:07:31Z");
    assert_eq!(body["bin_minutes"], 15);
    let starts: Vec<i64> = serde_json::from_value(body["bin_starts_ms"].clone()).unwrap();
    let first: DateTime<Utc> = "2026-10-07T14:00:00Z".parse().unwrap();
    assert_eq!(starts.len(), 24);
    assert_eq!(starts[0], first.timestamp_millis(), "1407Z starts at 1400");
    assert_eq!(starts[23] - starts[0], 23 * 15 * 60_000);

    let ids = |table: &str| -> Vec<Value> {
        body[table]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| json!([r["sector_id"], r["tier"]]))
            .collect()
    };
    assert_eq!(
        ids("enroute"),
        [json!(["010", "high"]), json!(["020", "low"])]
    );
    assert_eq!(ids("tracon"), [json!(["070", "approach"])]);
    let quiet = json!({ "active": 0, "proposed": 0, "combined": 0, "level": "ok" });
    let r = row(&body, "enroute", "010");
    assert_eq!(r["bins"].as_array().unwrap().len(), 24);
    assert!(r["bins"].as_array().unwrap().iter().all(|b| *b == quiet));
    assert_eq!(r["limit"], 10);
    assert_eq!(r["limit_overridden"], false);
    assert_eq!(r["consolidated"], json!([]));

    let zse = get(&state, "ZSE", &cookie).await;
    assert_eq!(zse["status"], "ready");
    assert_eq!(zse["enroute"]["has_sector_data"], true);
    assert_eq!(zse["enroute"]["rows"].as_array().unwrap().len(), 2);
    assert_eq!(
        zse["tracon"],
        json!({ "has_sector_data": false, "rows": [] })
    );
}

/// #722's rule through the real router: three flights airborne in `010` read active 3. A limit of 3
/// (equal) is `ok`, a limit of 2 is `over`. A fourth flight on the ground holding an issued CFR is
/// proposed: at a limit of 3 only active + proposed (4) exceeds it, so the bin is `watch`. A manually
/// excluded flight counts nowhere.
#[sqlx::test]
async fn each_bin_is_judged_against_the_rows_limit_strictly(pool: PgPool) {
    let state = state(pool.clone());
    let cookie = user(&pool, &[]).await;
    let now = Utc::now();
    let data = VatsimData {
        pilots: ["AAL1", "AAL2", "AAL3", "BOGUS1"].map(airborne).into(),
        ..Default::default()
    };
    cycle(&state, data, now).await;
    state.flight_exclusions.store(Arc::new(HashMap::from([(
        "ZNY".to_string(),
        HashSet::from(["BOGUS1".to_string()]),
    )])));
    let first_bin = |body: &Value| row(body, "enroute", "010")["bins"][0].clone();
    let set_limit = |limit: i32| {
        state.sector_limits.store(Arc::new(HashMap::from([(
            ("ZDC".to_string(), "010".to_string()),
            limit,
        )])));
    };

    set_limit(3);
    let body = get(&state, "ZDC", &cookie).await;
    assert_eq!(
        first_bin(&body),
        json!({ "active": 3, "proposed": 0, "combined": 3, "level": "ok" }),
        "a peak equal to the limit is ok, and the excluded flight is not counted"
    );
    assert_eq!(row(&body, "enroute", "010")["limit"], 3);
    assert_eq!(row(&body, "enroute", "010")["limit_overridden"], true);

    set_limit(2);
    let body = get(&state, "ZDC", &cookie).await;
    assert_eq!(first_bin(&body)["level"], "over", "active alone exceeds 2");

    // A prefile out of KJFK holding a wheels-up that has already passed departs now, inside `010`.
    sqlx::query(
        "insert into tmu.issued_cfrs (callsign, airport, wheels_up) values ('DAL9', 'KDCA', $1)",
    )
    .bind(now - chrono::Duration::minutes(5))
    .execute(&pool)
    .await
    .unwrap();
    let mut data = VatsimData {
        pilots: ["AAL1", "AAL2", "AAL3"].map(airborne).into(),
        ..Default::default()
    };
    data.prefiles.push(Prefile {
        callsign: "DAL9".into(),
        flight_plan: Some(plan()),
        ..Default::default()
    });
    cycle(&state, data, now).await;
    set_limit(3);
    let body = get(&state, "ZDC", &cookie).await;
    assert_eq!(
        first_bin(&body),
        json!({ "active": 3, "proposed": 1, "combined": 4, "level": "watch" }),
        "only active + proposed exceeds 3"
    );
}

/// #723's hand-off: a consolidated sector has no row of its own; its target's row lists it and reads
/// the **target's** limit, never the source's or a sum.
#[sqlx::test]
async fn a_combined_row_lists_its_sources_and_reads_the_targets_limit(pool: PgPool) {
    let state = state(pool.clone());
    let cookie = user(&pool, &[]).await;
    cycle(&state, VatsimData::default(), Utc::now()).await;
    state.sector_consolidations.store(Arc::new(HashMap::from([(
        ("ZDC".to_string(), "020".to_string()),
        "010".to_string(),
    )])));
    state.sector_limits.store(Arc::new(HashMap::from([
        (("ZDC".to_string(), "010".to_string()), 14),
        (("ZDC".to_string(), "020".to_string()), 5),
    ])));

    let body = get(&state, "ZDC", &cookie).await;
    let rows = body["enroute"]["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "020 is folded into 010: {body}");
    assert_eq!(rows[0]["sector_id"], "010");
    assert_eq!(rows[0]["consolidated"], json!(["020"]));
    assert_eq!(rows[0]["limit"], 14);
}

/// The edit flags follow each permission's own scope: a ZDC TMU holding only the limit grant may set
/// ZDC's limits and nothing else, and nothing at ZNY.
#[sqlx::test]
async fn the_edit_flags_follow_each_permissions_scope(pool: PgPool) {
    let state = state(pool.clone());
    let reader = user(&pool, &[]).await;
    let limits_tmu = user(&pool, &[("flow.sector_limits.update", "ZDC")]).await;
    let both_tmu = user(
        &pool,
        &[
            ("flow.sector_limits.update", "ZDC"),
            ("flow.sector_consolidations.update", "ZDC"),
        ],
    )
    .await;

    let flags = |body: Value| json!([body["limits_editable"], body["consolidations_editable"]]);
    assert_eq!(
        flags(get(&state, "ZDC", &reader).await),
        json!([false, false])
    );
    assert_eq!(
        flags(get(&state, "ZDC", &limits_tmu).await),
        json!([true, false])
    );
    assert_eq!(
        flags(get(&state, "ZDC", &both_tmu).await),
        json!([true, true])
    );
    assert_eq!(
        flags(get(&state, "ZNY", &both_tmu).await),
        json!([false, false])
    );
}

/// The neighbours are the Tier-1 ARTCCs OIS knows, sorted, never the facility itself — for an ARTCC
/// with no sector data too.
#[sqlx::test]
async fn the_neighbours_are_the_bordering_ois_artccs(pool: PgPool) {
    let state = state(pool.clone());
    let cookie = user(&pool, &[]).await;

    let body = get(&state, "ZDC", &cookie).await;
    assert_eq!(
        body["neighbours"],
        json!(["ZBW", "ZID", "ZJX", "ZNY", "ZOB", "ZTL"])
    );
    let zob = get(&state, "ZOB", &cookie).await;
    assert!(
        zob["neighbours"]
            .as_array()
            .unwrap()
            .contains(&json!("ZDC"))
    );
    assert!(
        !zob["neighbours"]
            .as_array()
            .unwrap()
            .contains(&json!("ZOB"))
    );
}

/// Demand is read with the sector data: no `flow.sectors.read`, no demand.
#[sqlx::test]
async fn reading_demand_needs_flow_sectors_read(pool: PgPool) {
    let state = state(pool.clone());
    let id = seed_user(&pool).await;
    let cookie = session_cookie(&pool, &id).await;
    let uri = "/api/v1/flow/sector-demand/ZDC";
    assert_eq!(
        send(&state, Method::GET, uri, &cookie, None).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(&state, Method::GET, uri, "", None).await,
        StatusCode::UNAUTHORIZED
    );
}
