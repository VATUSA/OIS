-- @formatter:off
-- Persistent VATSIM stats collection (ported from the standalone `stats` system, adapted to plain
-- Postgres — no TimescaleDB). A background collector decomposes each ~15s feed snapshot into
-- flight/controller sessions + a high-frequency position time-series + a per-tick network snapshot.
-- Ambient collection is scoped to US/VATUSA-relevant traffic; a `stats.capture` window (tied to an
-- event) marks a time range as permanently retained and, while open, relaxes the US scope so the
-- event is captured in full. A manual compaction job (Rust) ages the position table, skipping any
-- rows inside a saved capture window. See DESIGN in ~/Programing/stats for the original rationale.

-- Latest known display name per member (names change; keep most recent).
create table if not exists stats.member (
    cid       integer primary key,
    name      text,
    last_seen timestamptz not null
);

-- One row per pilot connection. session_id = fnv1a(cid, logon_time); a reconnect gets a new
-- logon_time -> a new flight. The flight plan is stored once here (latest revision), not per tick.
create table if not exists stats.flight (
    session_id      bigint primary key,        -- hash(cid, logon_time)
    cid             integer not null,
    callsign        text    not null,
    server          text,
    logon_time      timestamptz not null,
    first_seen      timestamptz not null,
    last_seen       timestamptz not null,
    status          text not null default 'active',   -- prefiled | active | completed

    -- flight plan (latest revision); null if connected without a plan
    flight_rules    char(1),
    departure       text,
    arrival         text,
    alternate       text,
    aircraft_short  text,
    aircraft_faa    text,
    cruise_tas      integer,
    cruise_alt      integer,
    deptime         text,
    enroute_time    text,
    route           text,
    remarks         text,
    revision_id     integer,

    -- summary, computed when the flight closes
    duration_s      integer,
    distance_nm     real,
    max_altitude    integer,
    max_groundspeed integer,
    path_simplified jsonb                        -- Douglas-Peucker track [[ts,lat,lon,alt],...]
);

create index if not exists stats_flight_cid_idx       on stats.flight (cid, logon_time desc);
create index if not exists stats_flight_departure_idx on stats.flight (departure, first_seen desc);
create index if not exists stats_flight_arrival_idx   on stats.flight (arrival, first_seen desc);
create index if not exists stats_flight_callsign_idx  on stats.flight (callsign, first_seen desc);
create index if not exists stats_flight_active_idx    on stats.flight (status) where status = 'active';

-- The high-frequency position time-series (the only large table). A single rolling table with a
-- BRIN index on the (append-only, time-ordered) ts column; the compaction job downsamples/prunes it
-- by age while skipping rows inside a saved capture window.
create table if not exists stats.position (
    session_id  bigint      not null,
    ts          timestamptz not null,
    lat         real        not null,
    lon         real        not null,
    altitude    integer     not null,
    groundspeed smallint    not null,
    heading     smallint    not null,
    transponder char(4),
    qnh_mb      smallint
);

create index if not exists stats_position_brin_ts  on stats.position using brin (ts);
create index if not exists stats_position_session_idx on stats.position (session_id, ts desc);

-- Controller / ATIS sessions (no positions — they don't move).
create table if not exists stats.controller_session (
    session_id   bigint primary key,            -- hash(cid, logon_time)
    cid          integer not null,
    callsign     text    not null,
    frequency    text,
    facility     integer,
    rating       integer,
    server       text,
    visual_range integer,
    atis_code    text,
    logon_time   timestamptz not null,
    first_seen   timestamptz not null,
    last_seen    timestamptz not null,
    duration_s   integer,
    is_atis      boolean not null default false
);

create index if not exists stats_controller_cid_idx      on stats.controller_session (cid, logon_time desc);
create index if not exists stats_controller_callsign_idx on stats.controller_session (callsign, first_seen desc);

-- Network totals, one row per tick. connected_clients/unique_users are the feed's global numbers;
-- pilots/controllers/atis/prefiles are the US-scoped counts we actually store.
create table if not exists stats.snapshot (
    ts                timestamptz primary key,
    connected_clients integer,
    unique_users      integer,
    pilots            integer,
    controllers       integer,
    atis              integer,
    prefiles          integer
);

-- A "saved" time window: rows in stats.position whose ts falls inside a status='saved' window are
-- permanently retained (the compaction job skips them). While a window is 'open', the collector
-- relaxes the US scope filter so everything present is captured. Optionally tied to a VATUSA event.
create table if not exists stats.capture (
    id          text primary key default gen_random_uuid()::text,
    event_id    bigint references events.event(id) on delete set null,
    label       text not null default '',
    start_time  timestamptz not null,
    end_time    timestamptz,                      -- null while open
    status      text not null default 'open' check (status in ('open', 'saved', 'discarded')),
    relax_scope boolean not null default true,    -- bypass the US filter while open
    created_by  text references identity.users(id) on delete set null,
    created_at  timestamptz not null default now()
);

create index if not exists stats_capture_window_idx on stats.capture (start_time, end_time);
create index if not exists stats_capture_open_idx   on stats.capture (status) where status = 'open';
create index if not exists stats_capture_event_idx  on stats.capture (event_id);

insert into access.permissions (name, description) values
    ('stats.data.read',      'View collected network / airport / event statistics'),
    ('stats.capture.update', 'Create and manage saved stat-capture windows (and event capture)')
on conflict (name) do nothing;
