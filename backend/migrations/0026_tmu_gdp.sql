-- @formatter:off
-- Ground Delay Programs (GDP): meter inbound demand to a constrained arrival airport down
-- to its AAR by assigning frozen control times (CTA) + derived EDCTs (controlled wheels-up)
-- to not-yet-departed inbounds via Ration-By-Schedule. Draft→published→cancelled lifecycle
-- like ground stops; control times are frozen into tmu.gdp_slot at publish so an issued
-- EDCT doesn't drift as demand changes.

create table if not exists tmu.gdp (
    id text primary key default gen_random_uuid()::text,
    airport text not null,
    -- Airport Acceptance Rate (arrivals/hour) the program meters to.
    aar integer not null check (aar between 1 and 200),
    -- HHMM Zulu program window.
    start_time text not null,
    end_time text not null,
    -- Scope tier: only inbounds whose enroute estimate is <= this many minutes are
    -- controllable; farther flights are exempt. null = no distance limit.
    max_enroute_min integer,
    -- Airborne inbounds can't be held on the ground, so exempt them from control.
    exempt_airborne boolean not null default true,
    status text not null default 'draft'
        check (status in ('draft', 'published', 'expired', 'cancelled')),
    published_by text references identity.users(id) on delete set null,
    published_at timestamptz,
    created_by text references identity.users(id) on delete set null,
    updated_by text references identity.users(id) on delete set null,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);

create index if not exists idx_tmu_gdp_airport on tmu.gdp(airport);
create index if not exists idx_tmu_gdp_status on tmu.gdp(status);

create trigger trg_tmu_gdp_updated_at
before update on tmu.gdp
for each row execute function platform.touch_updated_at();

-- Frozen control times assigned by Ration-By-Schedule at publish (one row per controlled
-- flight). Cascades away with the program.
create table if not exists tmu.gdp_slot (
    gdp_id text not null references tmu.gdp(id) on delete cascade,
    callsign text not null,
    dep text not null default '',
    original_eta timestamptz not null,
    cta timestamptz not null,
    edct timestamptz,
    delay_min integer not null default 0,
    assigned_at timestamptz not null default now(),
    primary key (gdp_id, callsign)
);

insert into access.permissions (name, description) values
    ('tmu.gdp.read', 'Read ground delay programs'),
    ('tmu.gdp.create', 'Create ground delay programs'),
    ('tmu.gdp.publish', 'Publish and cancel ground delay programs'),
    ('tmu.gdp.delete', 'Delete ground delay programs')
on conflict (name) do nothing;
