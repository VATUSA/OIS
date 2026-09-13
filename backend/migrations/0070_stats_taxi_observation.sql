-- @formatter:off
-- Per-departure pushback+startup and taxi-out timings, matched to a gate/parking spot (from
-- flow.airport_gate, #164 sub-issue A), aircraft type, and departure runway — the raw observations
-- a later per-gate/type/runway estimator (#164 sub-issue D) learns from. Written by the
-- feed/taxi_observations collector, mirroring stats.flight_leg's shape (0057_stats_flight_leg.sql).
--
-- gate_id/aircraft/runway/pushback_sec are nullable: a departure with no matched gate (undefined
-- surface data at that airport) or first observed already rolling (pushback unmeasurable) still
-- records what it can rather than dropping the observation.

create table if not exists stats.taxi_observation (
    id           bigint generated always as identity primary key,
    airport      text not null,
    gate_id      text references flow.airport_gate(id) on delete set null,
    aircraft     text,
    runway       text,
    pushback_sec integer,
    taxi_sec     integer not null,
    observed_at  timestamptz not null,
    created_at   timestamptz not null default now()
);

create index if not exists idx_stats_taxi_observation_dims
    on stats.taxi_observation (airport, gate_id, aircraft, runway);
create index if not exists idx_stats_taxi_observation_observed
    on stats.taxi_observation (observed_at);
