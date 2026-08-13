-- @formatter:off
-- Give ground stops the same draft→published lifecycle as TMIs (plus cancelled/expired),
-- so they can be staged and published/cancelled rather than only created and deleted.

alter table tmu.ground_stops
    add column if not exists status text not null default 'draft'
        check (status in ('draft', 'published', 'expired', 'cancelled')),
    add column if not exists published_by text references identity.users(id) on delete set null,
    add column if not exists published_at timestamptz;

create index if not exists idx_tmu_ground_stops_status on tmu.ground_stops(status);

insert into access.permissions (name, description) values
    ('tmu.groundstop.publish', 'Publish and cancel ground stops')
on conflict (name) do nothing;
