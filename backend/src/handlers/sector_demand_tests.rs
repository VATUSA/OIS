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
/// enroute volumes only, as in the real dataset. ZNY has `010` and `030` (high, elsewhere): a neighbour
/// with sectors of its own to refuse edits on. ZOB has none.
fn state(pool: PgPool) -> AppState {
    let state = test_state(pool, HashMap::new());
    state.airspace_sectors.store(Arc::new(SectorTable {
        volumes: vec![
            corridor("ZDC", "01001", "high"),
            elsewhere("ZDC", "02001", "low"),
            elsewhere("ZDC", "07001", "approach"),
            elsewhere("ZSE", "03001", "low"),
            elsewhere("ZSE", "04001", "ultra_high"),
            elsewhere("ZNY", "01001", "high"),
            elsewhere("ZNY", "03001", "high"),
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

/// #722's rule through the real router, at both sides of each boundary. Three flights airborne in
/// `010` read active 3: against limits 2, 3 and 4 that is `over`, `ok` (equal never alerts) and `ok`.
/// A fourth flight on the ground holding an issued CFR is proposed, so combined reads 4: against limits
/// 2, 3, 4 and 5 that is `over` (active alone exceeds), `watch` (only active + proposed does), `ok`
/// (equal) and `ok`. A manually excluded flight counts nowhere.
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

    for (limit, level) in [(2, "over"), (3, "ok"), (4, "ok")] {
        set_limit(limit);
        let body = get(&state, "ZDC", &cookie).await;
        assert_eq!(
            first_bin(&body),
            json!({ "active": 3, "proposed": 0, "combined": 3, "level": level }),
            "active 3 against {limit}, the excluded flight not counted"
        );
        assert_eq!(row(&body, "enroute", "010")["limit"], limit);
        assert_eq!(row(&body, "enroute", "010")["limit_overridden"], true);
    }

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
    for (limit, level) in [(2, "over"), (3, "watch"), (4, "ok"), (5, "ok")] {
        set_limit(limit);
        let body = get(&state, "ZDC", &cookie).await;
        assert_eq!(
            first_bin(&body),
            json!({ "active": 3, "proposed": 1, "combined": 4, "level": level }),
            "active 3, combined 4 against {limit}"
        );
    }
}

/// One volume, `050` (high), over KJFK and the first few minutes of the corridor (40.0–40.7N,
/// 73.6–74.15W). The airborne flight at 40.2N 74.0W heading for RBV leaves it within two minutes; a
/// departure from KJFK is inside it from wheels-up.
fn departure_box(pool: PgPool) -> AppState {
    let state = test_state(pool, HashMap::new());
    state.airspace_sectors.store(Arc::new(SectorTable {
        volumes: vec![SectorVolume {
            tier: "high".into(),
            base_alt_ft: 0,
            top_alt_ft: 60_000,
            rings: vec![vec![
                [40.0, -74.15],
                [40.0, -73.6],
                [40.7, -73.6],
                [40.7, -74.15],
                [40.0, -74.15],
            ]],
            ..volume("ZDC", "05001")
        }],
    }));
    state
}

/// Epic AC "the combined peak is computed minute by minute then maxed", through the real router: one
/// active flight is in `050` in the bin's first minutes and gone; one proposed flight departs KJFK into
/// it eight minutes later. Each population peaks at 1 and the combined peak is 1, not 2, so against a
/// limit of 1 the bin is green — the sum of the two peaks would read `watch`. The control, the same
/// departure with a wheels-up already passed, is in the sector in the same minute as the active flight
/// and reads combined 2, `watch`: the fixture can tell a sum from a per-minute max.
#[sqlx::test]
async fn the_combined_peak_is_maxed_per_minute_never_summed(pool: PgPool) {
    let state = departure_box(pool.clone());
    let cookie = user(&pool, &[]).await;
    state.sector_limits.store(Arc::new(HashMap::from([(
        ("ZDC".to_string(), "050".to_string()),
        1,
    )])));
    // 30 s into the 1400Z bin, so a wheels-up 8 minutes on is still inside it.
    let at: DateTime<Utc> = "2026-10-07T14:00:30Z".parse().unwrap();
    let mut data = VatsimData {
        pilots: vec![airborne("AAL1")],
        ..Default::default()
    };
    data.prefiles.push(Prefile {
        callsign: "DAL9".into(),
        flight_plan: Some(plan()),
        ..Default::default()
    });
    cycle(&state, data, at).await;

    for (wheels_up, combined, level) in [
        (at + chrono::Duration::minutes(8), 1, "ok"),
        (at - chrono::Duration::minutes(5), 2, "watch"),
    ] {
        sqlx::query(
            "insert into tmu.issued_cfrs (callsign, airport, wheels_up) values ('DAL9', 'KDCA', $1) \
             on conflict (callsign) do update set wheels_up = excluded.wheels_up",
        )
        .bind(wheels_up)
        .execute(&pool)
        .await
        .unwrap();
        let body = get(&state, "ZDC", &cookie).await;
        assert_eq!(
            row(&body, "enroute", "050")["bins"][0],
            json!({ "active": 1, "proposed": 1, "combined": combined, "level": level }),
            "wheels-up {wheels_up}"
        );
    }
}

/// The usual proposed flight is a pilot connected at the gate, not a prefile: the handler must ask for
/// a grounded pilot's locked wheels-up too, or `project_tracks` drops it and every proposed count reads
/// low. `DAL9` sits at KJFK with an issued CFR whose wheels-up has passed, so it departs into `010` now
/// and reads proposed 1. `UAL7` is beside it with no wheels-up and counts nowhere; `AAL1`, airborne,
/// is active as before.
#[sqlx::test]
async fn a_pilot_connected_on_the_ground_is_proposed_from_its_wheels_up(pool: PgPool) {
    let state = state(pool.clone());
    let cookie = user(&pool, &[]).await;
    let now = Utc::now();
    let at_the_gate = |callsign: &str| Pilot {
        callsign: callsign.into(),
        latitude: 40.64,
        longitude: -73.78,
        altitude: 0,
        groundspeed: 0,
        heading: 220,
        flight_plan: Some(plan()),
        ..Default::default()
    };
    let data = VatsimData {
        pilots: vec![airborne("AAL1"), at_the_gate("DAL9"), at_the_gate("UAL7")],
        ..Default::default()
    };
    cycle(&state, data, now).await;
    let first_bin = |body: &Value| row(body, "enroute", "010")["bins"][0].clone();

    let body = get(&state, "ZDC", &cookie).await;
    assert_eq!(
        first_bin(&body),
        json!({ "active": 1, "proposed": 0, "combined": 1, "level": "ok" }),
        "no wheels-up locked, so neither grounded pilot is proposed"
    );

    sqlx::query(
        "insert into tmu.issued_cfrs (callsign, airport, wheels_up) values ('DAL9', 'KDCA', $1)",
    )
    .bind(now - chrono::Duration::minutes(5))
    .execute(&pool)
    .await
    .unwrap();
    let body = get(&state, "ZDC", &cookie).await;
    assert_eq!(
        first_bin(&body),
        json!({ "active": 1, "proposed": 1, "combined": 2, "level": "ok" }),
        "DAL9 is proposed from its CFR; UAL7, holding nothing, is not"
    );
}

/// #723's hand-off: a consolidated sector has no row of its own; its target's row lists it and reads
/// the **target's** limit, never the source's, a sum or a maximum. The source's limit (14) is above the
/// target's (5), so a maximum would read 14 and a sum 19.
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
        (("ZDC".to_string(), "010".to_string()), 5),
        (("ZDC".to_string(), "020".to_string()), 14),
    ])));

    let body = get(&state, "ZDC", &cookie).await;
    let rows = body["enroute"]["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "020 is folded into 010: {body}");
    assert_eq!(rows[0]["sector_id"], "010");
    assert_eq!(rows[0]["consolidated"], json!(["020"]));
    assert_eq!(rows[0]["limit"], 5);
}

/// Epic AC "a combined row is a union of the polygons, never a sum of the rows", through the route.
/// `060` is a second high sector over the corridor, so the three FL240 flights are inside both `010` and
/// `060` in the same minutes: apart, each row reads 3. Worked at `010`, the combined row counts each
/// flight once — 3, not the 6 a sum of the rows would read — and against `010`'s limit of 4 it stays
/// green where the sum would be red. The control before the consolidation proves both rows carry the
/// traffic, so the 3 is a union and not a dropped source.
#[sqlx::test]
async fn a_combined_row_counts_a_flight_in_two_sources_once(pool: PgPool) {
    let state = state(pool.clone());
    let mut volumes = state.airspace_sectors.load().volumes.clone();
    volumes.push(corridor("ZDC", "06001", "high"));
    state
        .airspace_sectors
        .store(Arc::new(SectorTable { volumes }));
    state.sector_limits.store(Arc::new(HashMap::from([(
        ("ZDC".to_string(), "010".to_string()),
        4,
    )])));
    let cookie = user(&pool, &[]).await;
    let data = VatsimData {
        pilots: ["AAL1", "AAL2", "AAL3"].map(airborne).into(),
        ..Default::default()
    };
    cycle(&state, data, Utc::now()).await;
    let first_bin = |body: &Value, sector: &str| row(body, "enroute", sector)["bins"][0].clone();

    let apart = get(&state, "ZDC", &cookie).await;
    assert_eq!(first_bin(&apart, "010")["active"], 3);
    assert_eq!(first_bin(&apart, "060")["active"], 3);

    state.sector_consolidations.store(Arc::new(HashMap::from([(
        ("ZDC".to_string(), "060".to_string()),
        "010".to_string(),
    )])));
    let combined = get(&state, "ZDC", &cookie).await;
    let ids: Vec<&Value> = combined["enroute"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| &r["sector_id"])
        .collect();
    assert_eq!(
        ids,
        [&json!("010"), &json!("020")],
        "060 is folded into 010"
    );
    assert_eq!(
        row(&combined, "enroute", "010")["consolidated"],
        json!(["060"])
    );
    assert_eq!(
        first_bin(&combined, "010"),
        json!({ "active": 3, "proposed": 0, "combined": 3, "level": "ok" }),
        "each flight once, judged against the target's limit of 4"
    );
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

/// #726 through the route: the engine runs over the **whole** table, so 3D containment and TRACON
/// precedence hold across ARTCC lines. Under the corridor the neighbour ZNY has `080`, an approach
/// volume, and ZDC has `090`, an enroute low, both surface to 10,000 ft. The three flights at FL240
/// count in ZDC's enroute `010` above and in neither below. A flight at 5,000 ft is inside both `080`
/// and `090`: ZNY's approach claims it, so ZDC's `090` reads 0 and ZNY's TRACON `080` reads 1. Counting
/// ZDC against its own slice of the table would put it in `090`. The control without ZNY's approach
/// volume shows the same flight does count in `090`, so the 0 is precedence and not a missed fix.
#[sqlx::test]
async fn a_flight_counts_in_the_tracon_only_when_inside_it(pool: PgPool) {
    let state = state(pool.clone());
    let base = state.airspace_sectors.load().volumes.clone();
    let below = |artcc: &str, volume_id: &str, tier: &str| SectorVolume {
        top_alt_ft: 10_000,
        ..corridor(artcc, volume_id, tier)
    };
    let with_tracon = SectorTable {
        volumes: [
            base.clone(),
            vec![
                below("ZNY", "08001", "approach"),
                below("ZDC", "09001", "low"),
            ],
        ]
        .concat(),
    };
    state.airspace_sectors.store(Arc::new(with_tracon));
    let cookie = user(&pool, &[]).await;
    let first_active = |body: &Value, table: &str, sector: &str| {
        row(body, table, sector)["bins"][0]["active"].clone()
    };

    let data = VatsimData {
        pilots: ["AAL1", "AAL2", "AAL3"].map(airborne).into(),
        ..Default::default()
    };
    cycle(&state, data, Utc::now()).await;
    let zdc = get(&state, "ZDC", &cookie).await;
    assert_eq!(first_active(&zdc, "enroute", "010"), 3);
    assert_eq!(first_active(&zdc, "enroute", "090"), 0);
    let zny = get(&state, "ZNY", &cookie).await;
    assert_eq!(first_active(&zny, "tracon", "080"), 0);

    let low = Pilot {
        altitude: 5_000,
        groundspeed: 250,
        ..airborne("UAL5")
    };
    let data = VatsimData {
        pilots: vec![low],
        ..Default::default()
    };
    cycle(&state, data, Utc::now()).await;
    let zdc = get(&state, "ZDC", &cookie).await;
    assert_eq!(
        first_active(&zdc, "enroute", "090"),
        0,
        "a neighbour's approach volume claims the fix: {zdc}"
    );
    let zny = get(&state, "ZNY", &cookie).await;
    assert_eq!(first_active(&zny, "tracon", "080"), 1);

    state.airspace_sectors.store(Arc::new(SectorTable {
        volumes: [base, vec![below("ZDC", "09001", "low")]].concat(),
    }));
    let zdc = get(&state, "ZDC", &cookie).await;
    assert_eq!(
        first_active(&zdc, "enroute", "090"),
        1,
        "with no approach volume over it, the low counts the flight"
    );
}

/// A write refreshes the page at once and only at the writer's facility. A ZDC TMU's limit and
/// consolidation writes reload the caches in the request — the next demand read recolours and merges,
/// no job tick — and each publishes the topic the page refetches on. The same TMU's writes at the
/// neighbour ZNY are refused (403) by the real router, publish nothing and leave ZNY's demand as it
/// was, and ZNY's demand tells the page so (`*_editable` false). A reader scoped to ZDC alone still
/// reads ZNY's demand: neighbour tables are view-only, not hidden.
#[sqlx::test]
async fn writes_refresh_the_demand_and_a_neighbours_are_refused(pool: PgPool) {
    use crate::realtime::topic;

    let state = state(pool.clone());
    let tmu = user(
        &pool,
        &[
            ("flow.sector_limits.update", "ZDC"),
            ("flow.sector_consolidations.update", "ZDC"),
        ],
    )
    .await;
    let data = VatsimData {
        pilots: ["AAL1", "AAL2", "AAL3"].map(airborne).into(),
        ..Default::default()
    };
    cycle(&state, data, Utc::now()).await;
    let mut rx = state.events.subscribe();
    let mut topics = || {
        let mut seen = Vec::new();
        while let Ok(event) = rx.try_recv() {
            seen.push(event.topic);
        }
        seen
    };
    let limit = |artcc: &str, sector: &str, limit: i32| {
        (
            format!("/api/v1/flow/sector-limits/{artcc}/{sector}"),
            json!({ "limit": limit }),
        )
    };
    let consolidate = |artcc: &str, source: &str, target: &str| {
        (
            format!("/api/v1/flow/sector-consolidations/{artcc}/{source}"),
            json!({ "target_sector_id": target }),
        )
    };
    let put = |(uri, body): (String, Value)| {
        let (state, tmu) = (state.clone(), tmu.clone());
        async move { send(&state, Method::PUT, &uri, &tmu, Some(body)).await }
    };

    let zny_before = get(&state, "ZNY", &tmu).await;
    assert_eq!(zny_before["limits_editable"], false);
    assert_eq!(zny_before["consolidations_editable"], false);
    assert_eq!(put(limit("ZNY", "010", 2)).await, StatusCode::FORBIDDEN);
    assert_eq!(
        put(consolidate("ZNY", "030", "010")).await,
        StatusCode::FORBIDDEN
    );
    assert!(topics().is_empty(), "a refused write publishes nothing");
    let zny_after = get(&state, "ZNY", &tmu).await;
    assert_eq!(zny_after["enroute"], zny_before["enroute"]);
    assert_eq!(row(&zny_after, "enroute", "010")["limit"], 10);
    assert_eq!(row(&zny_after, "enroute", "030")["consolidated"], json!([]));

    assert_eq!(
        row(&get(&state, "ZDC", &tmu).await, "enroute", "010")["bins"][0]["level"],
        "ok"
    );
    assert_eq!(put(limit("ZDC", "010", 2)).await, StatusCode::OK);
    assert_eq!(topics(), [topic::SECTOR_LIMITS]);
    let body = get(&state, "ZDC", &tmu).await;
    assert_eq!(row(&body, "enroute", "010")["limit"], 2);
    assert_eq!(row(&body, "enroute", "010")["bins"][0]["level"], "over");

    assert_eq!(put(consolidate("ZDC", "020", "010")).await, StatusCode::OK);
    assert_eq!(topics(), [topic::SECTOR_CONSOLIDATIONS]);
    let body = get(&state, "ZDC", &tmu).await;
    let rows = body["enroute"]["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "020 is folded into 010: {body}");
    assert_eq!(rows[0]["consolidated"], json!(["020"]));
    assert_eq!(rows[0]["limit"], 2);

    let id = seed_user(&pool).await;
    grant(&pool, &id, READ, Some("ZDC")).await;
    let zdc_reader = session_cookie(&pool, &id).await;
    let zny = get(&state, "ZNY", &zdc_reader).await;
    assert_eq!(zny["status"], "ready");
    assert_eq!(zny["enroute"]["rows"].as_array().unwrap().len(), 2);
}
