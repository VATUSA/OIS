-- Airports the FAA surface seed (#231) has populated. The seed skips any airport listed here, so a
-- facility that deletes all of an airport's faa ramps/taxiways (e.g. to redraw it by hand) doesn't
-- get the FAA set re-inserted on the next boot. Re-populating a listed airport is the explicit
-- per-airport re-pull (#232).

create table if not exists flow.airport_surface_faa_seeded (
    icao      text primary key,
    seeded_at timestamptz not null default now()
);

-- Airports already carrying faa rows from a seed run before this table existed.
insert into flow.airport_surface_faa_seeded (icao)
select icao from flow.airport_taxiway where source = 'faa'
union
select icao from flow.airport_ramp_area where source = 'faa'
on conflict (icao) do nothing;
