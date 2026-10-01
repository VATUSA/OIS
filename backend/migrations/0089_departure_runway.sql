-- @formatter:off
-- Which runway a departure will use (VATUSA/OIS#509, sub-issue A of #434).
--
-- Keyed `(icao, callsign)` — the physical departure — and NOT by the metering initiative, which is
-- how the two existing per-flight tables are keyed:
--
--   flow.fca_release  primary key (fca_id, callsign)   -- 0023
--   tmu.gdp_slot      primary key (gdp_id, callsign)   -- 0026
--
-- That is right for those facts: a frozen CTA belongs to the FCA that metered it. A runway does not.
-- `IdstFlight` is one row per (FCA, departure), so a flight metered by two FCAs appears twice; a
-- runway column on fca_release would let one aircraft hold two different runways with nothing to
-- reconcile them, and would cover only *released* flights when #511 needs a runway for parked and
-- prefiled ones too.
--
-- Three consequences of this key, recorded because the rest of #434 depends on them:
--
--   * A flight metered by two FCAs has ONE assignment, shared by both. Unrepresentable otherwise.
--   * A cross-FCA swap of release times (#514) does not touch this table at all — the times move
--     between aircraft, each runway stays with its own aircraft.
--   * Nothing cascades. This is the first callsign-keyed table with no parent row, so a prune job
--     on a retention horizon is what removes stale assignments (see jobs.rs). Without one the table
--     would grow for the life of the deployment, which is exactly what #444 had to fix for audit.

create table if not exists flow.departure_runway_assignment (
    icao text not null,
    callsign text not null,
    runway text not null,
    -- Which rung of #511's ladder produced this: a manual override, a facility rule, the active
    -- airport config's departure_runways, or an automatic pick. The ordering between them lives in
    -- repos::departure_runway::assign, not here — it is a property of the ladder, not of the row.
    source text not null check (source in ('manual', 'rule', 'config', 'auto')),
    updated_by text references identity.users(id) on delete set null,
    updated_at timestamptz not null default now(),
    primary key (icao, callsign)
);

-- For the prune pass only; reads go through the primary key.
create index if not exists idx_dep_runway_updated_at
    on flow.departure_runway_assignment (updated_at);

-- The airport-level default #511's third rung reads. Mirrors landing_runways (0036), which is the
-- arrival side of the same idea.
alter table flow.airport_config
    add column if not exists departure_runways text[] not null default '{}';
