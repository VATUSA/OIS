-- @formatter:off
-- Issued CFRs (Call-For-Release / EDCT): a controller-locked wheels-up time for a ground
-- departure into a metered field. Once issued, the metering scheduler reserves the slot
-- so the flight's release no longer drifts with each recompute.

create table if not exists tmu.issued_cfrs (
    callsign text primary key,          -- one active CFR per flight
    airport text not null,              -- the metered arrival field
    wheels_up timestamptz not null,     -- locked release time
    issued_by text references identity.users(id) on delete set null,
    issued_at timestamptz not null default now()
);

create index if not exists idx_tmu_issued_cfrs_airport on tmu.issued_cfrs(airport);

insert into access.permissions (name, description) values
    ('tmu.cfr.assign', 'Issue and release Call-For-Release (CFR) times')
on conflict (name) do nothing;
