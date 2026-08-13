-- @formatter:off
-- Ground stops: hold GROUND departures into a named airport that originate inside the
-- scoped ARTCC/FIR(s). Blank scope = field-wide stop. Mirrors vatflow's Ground stops tab.

create table if not exists tmu.ground_stops (
    id text primary key default gen_random_uuid()::text,
    airport text not null,
    -- Space-separated ARTCC/FIR codes the stop applies to; '' = every departure.
    scope text not null default '',
    -- HHMM Zulu clock time the stop runs until; null = until further notice.
    until text,
    created_by text references identity.users(id) on delete set null,
    updated_by text references identity.users(id) on delete set null,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);

create index if not exists idx_tmu_ground_stops_airport on tmu.ground_stops(airport);

create trigger trg_tmu_ground_stops_updated_at
before update on tmu.ground_stops
for each row execute function platform.touch_updated_at();

insert into access.permissions (name, description) values
    ('tmu.groundstop.read', 'Read ground stops'),
    ('tmu.groundstop.create', 'Issue ground stops'),
    ('tmu.groundstop.delete', 'Cancel ground stops')
on conflict (name) do nothing;
