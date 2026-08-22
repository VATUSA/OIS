-- @formatter:off
-- Flight-plan revision history for temporally-faithful replay. stats.flight keeps only the latest
-- plan (overwritten each tick), so a mid-route amendment retroactively rewrites the whole replayed
-- track. This table keeps every distinct revision with the time it took effect, so replay can show the
-- plan that was in force at each instant. Written only when the plan actually changes (rare), so it's
-- ~1 row per flight; kept as long as the flight (no compaction — the position table is the large one).

create table if not exists stats.flight_plan (
    session_id      bigint      not null,           -- stats.flight.session_id
    effective_from  timestamptz not null,           -- the tick at which this revision was first seen
    revision_id     integer,
    flight_rules    text,
    departure       text,
    arrival         text,
    alternate       text,
    aircraft_short  text,
    aircraft_faa    text,
    cruise_alt      integer,
    deptime         text,
    enroute_time    text,
    route           text,
    remarks         text,
    primary key (session_id, effective_from)
);

create index if not exists stats_flight_plan_session_idx
    on stats.flight_plan (session_id, effective_from desc);
