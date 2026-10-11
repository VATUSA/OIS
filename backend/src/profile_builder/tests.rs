//! The builder end to end over seeded stats rows (#828 AC1), and the split it shares with the
//! backtest.

use chrono::{DateTime, Duration, Utc};
use sqlx::{PgPool, Postgres, QueryBuilder};

use super::samples::Field;
use super::stats::{Accumulator, MIN_FLIGHTS_PER_FIELD, Source};
use super::*;

fn t0() -> DateTime<Utc> {
    DateTime::from_timestamp(1_790_000_000, 0).unwrap()
}

#[test]
fn the_split_is_one_in_five_and_stable() {
    let held: Vec<i64> = (-10..10).filter(|s| is_held_out(*s)).collect();
    assert_eq!(held, vec![-10, -5, 0, 5]);
    // Negative session ids (an FNV hash cast to bigint) split the same way.
    assert!(is_held_out(-1_234_567_890));
    assert!(!is_held_out(-1_234_567_891));
}

#[test]
fn the_wake_comes_from_the_faa_equipment_prefix() {
    let f = |short: Option<&str>, faa: Option<&str>| HistFlight {
        session_id: 1,
        departure: "KJFK".into(),
        arrival: "KDCA".into(),
        aircraft_short: short.map(str::to_string),
        aircraft_faa: faa.map(str::to_string),
        cruise_tas: None,
        cruise_alt: None,
        route: None,
        first_seen: t0(),
        last_seen: t0(),
    };
    assert_eq!(
        type_and_wake(&f(Some("B77W"), Some("H/B77W/L"))),
        ("B77W".to_string(), "H".to_string())
    );
    assert_eq!(
        type_and_wake(&f(Some("B738"), Some("B738/L"))),
        ("B738".to_string(), String::new())
    );
}

/// Feeding the same flights in another order gives the same bytes.
#[test]
fn accumulation_order_does_not_change_the_output() {
    let flights: Vec<(&str, Vec<(Field, f64)>)> = (0..25)
        .map(|k| {
            let ty = if k % 3 == 0 { "A320" } else { "B738" };
            let v = vec![
                (Field::ClimbFpmLo, 1_800.0 + (k * 37 % 11) as f64 * 10.0),
                (Field::CruiseTas, 430.0 + (k * 13 % 7) as f64),
            ];
            (ty, v)
        })
        .collect();
    let source = || Source {
        from: "a".into(),
        to: "b".into(),
        flights_read: 25,
        training_flights: 25,
        held_out_flights: 0,
        held_out_rule: HELD_OUT_RULE.into(),
    };
    let mut fwd = Accumulator::default();
    for (ty, v) in &flights {
        fwd.add_flight(ty, "", v);
    }
    let mut rev = Accumulator::default();
    for (ty, v) in flights.iter().rev() {
        rev.add_flight(ty, "", v);
    }
    assert_eq!(
        to_json(&fwd.finish(source(), |_| None)),
        to_json(&rev.finish(source(), |_| None))
    );
}

/// One synthetic B738 flight starting at `start`: a 15-second-tick climb at 2,000 fpm to 10,000 ft
/// and 1,500 fpm to 12,000 ft, ten minutes level, then a 1,800 fpm descent.
fn track(start: DateTime<Utc>) -> Vec<(DateTime<Utc>, f32, f32, i32, i16)> {
    let mut v = vec![];
    let (mut t, mut lon, mut alt) = (start, -100.0_f32, 1_000);
    let mut push = |t: DateTime<Utc>, lon: f32, alt: i32, gs: i16| v.push((t, 40.0, lon, alt, gs));
    while alt < 12_000 {
        push(t, lon, alt, 250);
        alt += if alt < 10_000 { 500 } else { 375 };
        t += Duration::seconds(15);
        lon += 0.02;
    }
    for _ in 0..10 {
        push(t, lon, 12_000, 300);
        t += Duration::seconds(60);
        lon += 0.12;
    }
    while alt > 1_000 {
        alt -= 450;
        t += Duration::seconds(15);
        lon += 0.02;
        push(t, lon, alt, 240);
    }
    v
}

async fn seed_flight(pool: &PgPool, session_id: i64, start: DateTime<Utc>) {
    let pts = track(start);
    let end = pts.last().unwrap().0;
    sqlx::query(
        "insert into stats.flight (session_id, cid, callsign, logon_time, first_seen, last_seen, \
            status, departure, arrival, aircraft_short, aircraft_faa, cruise_tas, cruise_alt, route) \
         values ($1, 1, 'TEST', $2, $2, $3, 'completed', 'KAAA', 'KBBB', 'B738', 'B738/L', 450, \
            12000, 'DCT')",
    )
    .bind(session_id)
    .bind(start)
    .bind(end)
    .execute(pool)
    .await
    .unwrap();
    let mut qb: QueryBuilder<Postgres> = QueryBuilder::new(
        "insert into stats.position (session_id, ts, lat, lon, altitude, groundspeed, heading) ",
    );
    qb.push_values(pts, |mut b, (ts, lat, lon, alt, gs)| {
        b.push_bind(session_id)
            .push_bind(ts)
            .push_bind(lat)
            .push_bind(lon)
            .push_bind(alt)
            .push_bind(gs)
            .push_bind(90_i16);
    });
    qb.build().execute(pool).await.unwrap();
}

/// AC1: the builder reads the history, profiles a type with enough training flights, leaves the
/// held-out flight out, and gives byte-identical output on a second run over the same data.
#[sqlx::test]
async fn the_builder_is_repeatable_and_skips_held_out_flights(pool: PgPool) {
    // A calm-wind snapshot covering every flight, so airspeeds are measured too.
    sqlx::query("insert into stats.winds (ts, data) values ($1, $2)")
        .bind(t0() - Duration::minutes(30))
        .bind(serde_json::json!({"stations": [{"lat": 40.0, "lon": -99.0,
            "levels": [[3000, {"dir": null, "spd": 0.0}], [39000, {"dir": null, "spd": 0.0}]]}]}))
        .execute(&pool)
        .await
        .unwrap();
    // 20 training flights (session ids not divisible by 5), each starting a minute apart so the one
    // snapshot stays fresh, plus one held-out flight (session id 5).
    let training: Vec<i64> = (1..).filter(|s| !is_held_out(*s)).take(20).collect();
    for (k, s) in training.iter().enumerate() {
        seed_flight(&pool, *s, t0() + Duration::minutes(k as i64)).await;
    }
    seed_flight(&pool, 5, t0()).await;

    let (from, to) = (t0() - Duration::hours(1), t0() + Duration::hours(3));
    let first = build(&pool, from, to).await.unwrap();
    let again = build(&pool, from, to).await.unwrap();
    assert_eq!(to_json(&first), to_json(&again), "same data, same bytes");

    assert_eq!(first.source.flights_read, 21);
    assert_eq!(first.source.training_flights, 20);
    assert_eq!(first.source.held_out_flights, 1);
    let b738 = &first.types["B738"];
    assert_eq!(b738.flights, MIN_FLIGHTS_PER_FIELD);
    assert!(b738.profiled);
    assert_eq!(b738.engine.as_deref(), Some("jet"));
    assert_eq!(b738.fields["climb_fpm_lo"].median, 2_000.0);
    assert_eq!(b738.fields["climb_fpm_hi"].median, 1_500.0);
    assert_eq!(b738.fields["desc_fpm"].median, 1_800.0);
    // Calm air: true airspeed is the groundspeed.
    assert_eq!(b738.fields["cruise_tas"].median, 300.0);

    // A window that ends before the flights land reads none of them.
    let none = build(&pool, from, t0() + Duration::minutes(20))
        .await
        .unwrap();
    assert_eq!(none.source.flights_read, 0);
}
