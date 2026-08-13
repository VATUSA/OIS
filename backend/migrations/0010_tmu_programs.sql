-- @formatter:off
-- TMU rate programs: one per airport (ICAO). AAR + an airport-wide spacing default
-- (route minutes-in-trail, or miles-in-trail when set), plus per-gate restrictions and
-- aircraft exclusions. Mirrors vatflow's TMU tab; drives metering/CFR once a traffic
-- feed consumes it. Live operational config — edited in place, no draft/publish lifecycle.

create table if not exists tmu.programs (
    icao text primary key,
    aar integer not null default 30 check (aar between 1 and 200),
    -- Airport-wide default spacing. trail = minutes-in-trail; mit = miles-in-trail and
    -- overrides trail when > 0. Mutually exclusive per vatflow (setting one clears the other).
    trail integer not null default 0 check (trail between 0 and 60),
    mit integer not null default 0 check (mit between 0 and 300),
    -- Per-gate restrictions: [{ "name": "JJEDI4", "trail": 0, "mit": 20 }, ...] (<= 10).
    gates jsonb not null default '[]'::jsonb,
    -- Aircraft exclusions from metering.
    exclude_wake text[] not null default '{}',
    exclude_types text[] not null default '{}',
    jets_only boolean not null default false,
    created_by text references identity.users(id) on delete set null,
    updated_by text references identity.users(id) on delete set null,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);

create trigger trg_tmu_programs_updated_at
before update on tmu.programs
for each row execute function platform.touch_updated_at();

insert into access.permissions (name, description) values
    ('tmu.program.read', 'Read TMU rate programs'),
    ('tmu.program.update', 'Create and edit TMU rate programs'),
    ('tmu.program.delete', 'Remove TMU rate programs')
on conflict (name) do nothing;
