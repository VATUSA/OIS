-- @formatter:off
-- Flow map routes: named polylines controllers draw on the FCA map to share a reference
-- track (a reroute, a preferred lateral path, etc.). Shared + server-side — one route set,
-- visible to everyone. Simpler than an FCA: just a name, color, and polyline geometry, with
-- no metering or membership filters.

create table if not exists flow.route (
    id text primary key default gen_random_uuid()::text,
    name text not null default '',
    color text not null default '#38bdf8',
    -- geometry: array of [lat, lon] vertices — a polyline (>= 2 pts)
    points jsonb not null default '[]'::jsonb,
    created_by text references identity.users(id) on delete set null,
    updated_by text references identity.users(id) on delete set null,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);

create trigger trg_flow_route_updated_at
before update on flow.route
for each row execute function platform.touch_updated_at();

-- Routes are visible to anyone with flow.fca.read (the flow map audience); only editing and
-- deleting need dedicated perms.
insert into access.permissions (name, description) values
    ('flow.route.update', 'Create and edit flow map routes'),
    ('flow.route.delete', 'Delete flow map routes')
on conflict (name) do nothing;
