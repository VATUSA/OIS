-- @formatter:off
-- VATUSA/OIS#723 (epic #720): sectors worked at another sector's position. A consolidated sector has no
-- occupancy row of its own: its volumes are filed under its target's row before counting
-- (`feed::sector_load::sector_loads`), so the combined row is a union of the airspace, never a sum of
-- the rows, and is judged against the target's limit (0127).
--
-- Shared, not per-browser. Always flat — a target is never itself a source — which the one writer
-- (`repos::sector_consolidations::consolidate`) keeps true in a single transaction. Same ARTCC only, by
-- construction: one artcc column. No foreign key to flow.airspace_sector, for the reason 0127 gives.
-- Rebuilds the table 0118 created and 0125 dropped; nothing of that data survives to migrate.

create table flow.sector_consolidation (
    artcc            text not null,
    sector_id        text not null,
    target_sector_id text not null,
    updated_by       text references identity.users(id) on delete set null,
    updated_at       timestamptz not null default now(),
    primary key (artcc, sector_id),
    check (sector_id <> target_sector_id)
);

-- Three-in-sync: the marker lives in backend/src/auth/permissions.rs, the name in
-- crates/ois-core/src/catalog.rs, and the row is here. Reading rides on flow.sectors.read (0120).
insert into access.permissions (name, description) values
    ('flow.sector_consolidations.update', 'Work a sector at another sector''s position (facility-scoped)')
on conflict (name) do nothing;

-- Owner decision on #723: its own permission, so combining positions can be granted apart from limit
-- editing, seeded to exactly the groups that hold flow.sector_limits.update (0127). CONTROLLER is left to
-- the owner, not decided here.
insert into access.role_permissions (role_name, permission_name)
select r.role_name, 'flow.sector_consolidations.update'
from (values ('VATUSA_STAFF'), ('DCC_STAFF'), ('EC'), ('AEC'), ('NTMO')) as r(role_name)
on conflict do nothing;
