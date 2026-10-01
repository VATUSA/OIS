-- Bulk gate (parking stand) import from the X-Plane Scenery Gateway (#431). Gate data was seeded at
-- one airport of 185 — KDCA's 57 hand-committed osm rows from 0067 — so 184 airports could never
-- reach the taxi-estimate ladder's tier-1 gate_type_runway bucket. The FAA Aerodrome Mapping extract
-- that supplies taxiways/aprons/runways has no parking-stand layer (confirmed over all 106 published
-- services), so gates need their own source. The Gateway's licence is unstated; using it is a risk
-- the project owner accepted deliberately, recorded on #431.

-- 1. A new source value, following 0073's drop/add shape. Only the gate table gains it: the Gateway
--    supplies stands, not ramp or taxiway geometry.
alter table flow.airport_gate
    drop constraint if exists airport_gate_source_check;
alter table flow.airport_gate
    add constraint airport_gate_source_check
    check (source in ('manual', 'osm', 'crc', 'faa', 'xplane'));

-- 2. The stand type, verbatim from X-Plane row 1300: gate | tie_down | misc | hangar. Nullable on
--    purpose — manual, osm and crc rows carry no type, and inventing one would assert something
--    about data we never imported. An aircraft at a tie_down generally does not push back, so this
--    is what lets a future change tell a real zero-pushback measurement from a session that began
--    mid-departure (feed/taxi_observations.rs phases_at_stand).
alter table flow.airport_gate
    add column if not exists kind text;

comment on column flow.airport_gate.kind is
    'X-Plane stand type (gate | tie_down | misc | hangar); null for manual/osm/crc rows.';

-- 3. Seed-once marker, mirroring 0077's airport_runway_faa_seeded. Records that an airport has been
--    gate-seeded even if every xplane row is later deleted, so the boot job never re-populates an
--    airport a facility deliberately emptied. No backfill: no xplane rows exist yet.
create table if not exists flow.airport_gate_xplane_seeded (
    icao      text primary key,
    seeded_at timestamptz not null default now()
);
