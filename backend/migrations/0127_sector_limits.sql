-- @formatter:off
-- VATUSA/OIS#722 (epic #720): the per-sector limit a sector's occupancy is judged against. Shared, so
-- everyone watching an ARTCC sees the same numbers and therefore the same colours.
--
-- Overrides only: a sector with no row reads the default (10, `feed::sector_limits::DEFAULT_LIMIT`), and
-- setting a sector back to the default deletes its row rather than storing 10. Keyed per
-- (artcc, sector_id), not per volume — a sector is one workload however many pieces its airspace is in.
-- No foreign key to flow.airspace_sector: a sector is several volumes there, replaced wholesale on every
-- import. `limit_value` because `limit` is reserved. The handler is the only writer.

create table flow.sector_limit (
    artcc       text not null,
    sector_id   text not null,
    limit_value integer not null check (limit_value > 0),
    updated_by  text references identity.users(id) on delete set null,
    updated_at  timestamptz not null default now(),
    primary key (artcc, sector_id)
);

-- Three-in-sync: the marker lives in backend/src/auth/permissions.rs, the name in
-- crates/ois-core/src/catalog.rs, and the row is here. Reading limits rides on flow.sectors.read (0120).
insert into access.permissions (name, description) values
    ('flow.sector_limits.update', 'Set a sector''s occupancy limit (facility-scoped)')
on conflict (name) do nothing;

-- Seeded like every other `flow` permission (0094): the groups whose presets carry the `flow` domain,
-- whose facility grants supply the ARTCC scope. Not CONTROLLER: setting a limit is a TMU act, and #730
-- fixed that group's set exactly. `access::tests::seeded_roles_match_the_preset_domains` fails until a
-- new permission is placed.
insert into access.role_permissions (role_name, permission_name)
select r.role_name, 'flow.sector_limits.update'
from (values ('VATUSA_STAFF'), ('DCC_STAFF'), ('EC'), ('AEC'), ('NTMO')) as r(role_name)
on conflict do nothing;
