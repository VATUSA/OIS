-- Monitor Alert Parameters (#598, Airspace Monitor #593): the per-sector limit a sector's count is
-- coloured against. Shared, so everyone watching an ARTCC sees the same numbers and colours.
--
-- Overrides only: a sector with no row reads the default (10, `feed::sectors::DEFAULT_MAP`). No foreign
-- key to flow.airspace_sector — a sector is several volumes there, replaced wholesale per import — so
-- an override is keyed by the same (artcc, sector_id) pair the volumes carry. Writes are ARTCC-scoped
-- by `flow.monitor.update`; the handler is the only writer.

create table flow.sector_map (
    artcc      text not null,
    sector_id  text not null,
    map        integer not null check (map > 0),
    updated_by text references identity.users(id) on delete set null,
    updated_at timestamptz not null default now(),
    primary key (artcc, sector_id)
);

-- Three-in-sync: the markers live in backend/src/auth/permissions.rs, the names in
-- crates/ois-core/src/catalog.rs, and the rows are here.
insert into access.permissions (name, description) values
    ('flow.monitor.read', 'View Airspace Monitor sectors and their alert parameters'),
    ('flow.monitor.update', 'Set a sector''s Monitor Alert Parameter (facility-scoped)')
on conflict (name) do nothing;

-- Every group whose preset covers the flow domain carries them (0094; checked by
-- `seeded_roles_match_the_preset_domains`). Their facility grants supply the ARTCC scope.
insert into access.role_permissions (role_name, permission_name)
select r.role_name, p.name
from (values ('VATUSA_STAFF'), ('DCC_STAFF'), ('EC'), ('AEC'), ('NTMO')) as r(role_name)
cross join access.permissions p
where p.name in ('flow.monitor.read', 'flow.monitor.update')
on conflict do nothing;
