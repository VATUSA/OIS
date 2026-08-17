-- @formatter:off
-- Rename the stats view permission `stats.read` -> `stats.data.read`.
--
-- A bare one-segment `stats.read` collides with the two-segment `stats.capture.update` in the
-- permission tree (a domain node can't be both a leaf of actions and a branch of sub-resources), so
-- the client's permission check for `stats.read` never resolved. `stats.data.read` composes cleanly.
--
-- 0039 (which originally seeded `stats.read`) is left as-applied; this migration does the rename on
-- existing databases and converges fresh ones to the same end state. Grants are repointed before the
-- old permission row is removed (both role_permissions and user_permissions FK to permissions.name).

insert into access.permissions (name, description) values
    ('stats.data.read', 'View collected network / airport / event statistics')
on conflict (name) do nothing;

update access.role_permissions set permission_name = 'stats.data.read'
    where permission_name = 'stats.read';
update access.user_permissions set permission_name = 'stats.data.read'
    where permission_name = 'stats.read';

delete from access.permissions where name = 'stats.read';
