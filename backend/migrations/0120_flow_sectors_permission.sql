-- VATUSA/OIS#602: the admin sector map's own permission. Sector volumes (#594) are internal traffic
-- monitoring data, shown only on that admin page, so viewing them is granted on its own rather than
-- riding on a planning permission.

insert into access.permissions (name, description) values
    ('flow.sectors.read', 'View ATC sector volumes on the admin sector map')
on conflict (name) do nothing;

-- Seeded like every other `flow` permission (0094): the groups whose presets carry the `flow` domain.
-- `access::tests::seeded_roles_match_the_preset_domains` fails until a new permission is placed.
insert into access.role_permissions (role_name, permission_name)
select r.role_name, 'flow.sectors.read'
from (values ('VATUSA_STAFF'), ('DCC_STAFF'), ('EC'), ('AEC'), ('NTMO')) as r(role_name)
on conflict do nothing;
