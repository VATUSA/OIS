-- @formatter:off
-- The permission that gates group (role) management (VATUSA/OIS#545).
--
-- Until now nothing could read or write access.role_permissions: GET /access/catalog returned role
-- names with no contents, so an admin could not see what a group granted, let alone change it, and
-- adding a role meant a migration plus a deploy.
--
-- Three-in-sync: the marker lives in backend/src/auth/permissions.rs, the name in
-- crates/ois-core/src/catalog.rs, and the row is here. auth::permissions::sync_tests fails if any
-- one of the three is missing.

insert into access.permissions (name, description) values
    ('access.groups.read', 'Read groups and the permissions they grant'),
    ('access.groups.update', 'Create, edit and delete groups')
on conflict (name) do nothing;

-- VATUSA_STAFF is seeded as "every permission" (0092), so it must carry these too — otherwise the
-- group's meaning quietly narrows the moment a permission is added. This is exactly the drift
-- 0092's `seeded_roles_match_the_preset_domains` test exists to catch, and it catches it here.
insert into access.role_permissions (role_name, permission_name)
select 'VATUSA_STAFF', name
from access.permissions
where name in ('access.groups.read', 'access.groups.update')
on conflict do nothing;
