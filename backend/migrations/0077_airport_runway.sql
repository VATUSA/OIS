-- @formatter:off
-- #279: runway pavement polygons for the map, from the FAA AM_Runway layer (bundled extract) or
-- drawn in the surface editor. Display geometry only — independent of data/runways.json, which stays
-- the Runway Balancer's heading/length source. Same shape as flow.airport_taxiway after 0076.

create table if not exists flow.airport_runway (
    id         text primary key default gen_random_uuid()::text,
    icao       text not null,
    name       text not null,                  -- designator, e.g. 01/19
    rings      jsonb not null,                 -- array of rings, each an array of [lat, lon]
    source     text not null default 'manual' check (source in ('manual', 'osm', 'crc', 'faa')),
    updated_by text references identity.users(id) on delete set null,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);
create index if not exists idx_airport_runway_icao on flow.airport_runway(icao);
create trigger trg_airport_runway_updated_at
before update on flow.airport_runway
for each row execute function platform.touch_updated_at();

-- Airports the FAA seed has populated with runways — the runway counterpart of 0074's
-- flow.airport_surface_faa_seeded. Separate because every airport seeded before runways existed is
-- already in that table, yet still needs its runways seeded once; a facility that deletes an
-- airport's faa runways doesn't get them back on the next boot (re-pull restores them).
create table if not exists flow.airport_runway_faa_seeded (
    icao      text primary key,
    seeded_at timestamptz not null default now()
);
