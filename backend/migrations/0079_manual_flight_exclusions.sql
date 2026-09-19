-- @formatter:off
-- Manually excluded ("bogus") flights: a controller dropping a single VATSIM aircraft whose data is
-- garbage — a teleporting position, a mis-parsed route, a stuck ground squawk, a duplicate — so it
-- stops polluting the map, the FCA crossing lists, metering, counts and AADC demand for everyone
-- (issue #342).
--
-- Distinct from `flow.tmu_program`'s exclude_wake/exclude_types (`feed::flow::is_excluded`), which is
-- program-wide by wake/type and keeps the flight *shown*. This removes one specific callsign.
--
-- Scoped per-ARTCC so a removal has an owner and an audit trail. Managed by staff holding
-- flow.fca.update for that ARTCC (no new permission — the FCA page's existing write gate).
--
-- Rows are short-lived by design: the refresh job clears an exclusion once the callsign leaves the
-- VATSIM feed, and `expires_at` is the backstop for a flight that never cleanly departs it. Readers
-- filter on `expires_at > now()`, so an expired row stops applying without needing a reaper.

create table if not exists flow.manual_flight_exclusion (
    id text primary key default gen_random_uuid()::text,
    callsign text not null,
    artcc text not null,                         -- owning ARTCC (scope + audit)
    reason text not null default '',             -- optional controller note
    created_by text references identity.users(id) on delete set null,
    created_at timestamptz not null default now(),
    expires_at timestamptz not null
);

-- One live exclusion per callsign per facility; re-removing refreshes it via upsert.
create unique index if not exists idx_manual_flight_exclusion_artcc_callsign
    on flow.manual_flight_exclusion(artcc, callsign);
-- The cache loader reads every unexpired row.
create index if not exists idx_manual_flight_exclusion_expires
    on flow.manual_flight_exclusion(expires_at);
