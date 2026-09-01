-- @formatter:off
-- Per-flight leg timings for the average-delay page. One row per completed leg, written by the
-- feed/delays collector:
--   * departure — taxi-out: start-of-taxi (~7 kt) → wheels-up. duration = taxi_out_sec.
--   * arrival   — transit: crossing a ~40 NM entry ring → touchdown. duration = transit_sec.
-- Tagged with the detected runway and the filed SID/STAR (base name) so the page can filter/normalize.

create table if not exists stats.flight_leg (
    id           bigint generated always as identity primary key,
    kind         text not null check (kind in ('departure', 'arrival')),
    airport      text not null,                   -- departure field (dep) / arrival field (arr), ICAO
    callsign     text not null,
    cid          integer not null default 0,
    aircraft     text,                            -- ICAO type (from the flight plan)
    runway       text,                            -- detected dep / landing runway id, if matched
    procedure    text,                            -- filed SID (dep) / STAR (arr) base name, if any
    start_time   timestamptz not null,            -- start of taxi (dep) / entry-ring crossing (arr)
    end_time     timestamptz not null,            -- wheels-up (dep) / touchdown (arr)
    duration_sec integer not null,
    created_at   timestamptz not null default now()
);

create index if not exists idx_stats_flight_leg_airport
    on stats.flight_leg (airport, kind, end_time);
create index if not exists idx_stats_flight_leg_end
    on stats.flight_leg (end_time);
