-- Dedicated permission for the airport surface-data editor (#164's sub-issue B), replacing the
-- borrowed events.config.update placeholder that #177's CRUD handlers shipped with.

insert into access.permissions (name, description) values
    ('flow.surface_data.update', 'Manage an airport''s gates/ramp areas/taxiways (facility-scoped)')
on conflict (name) do nothing;

-- Additive, not a rename: unlike a true permission rename (e.g. 0041_stats_perm_rename.sql, which
-- deletes the old permission and moves every grant 1:1), events.config.update keeps its own,
-- unrelated meaning (airport_configs.rs) — it isn't going away. So existing holders of
-- events.config.update also get the new, more narrowly-scoped flow.surface_data.update grant
-- (matching whatever access they already had for the placeholder), rather than losing surface-data
-- write access the moment this migration ships with no explanation. This is a bootstrap-only step —
-- it does not run again for grants made after this migration.
insert into access.role_permissions (role_name, permission_name)
select role_name, 'flow.surface_data.update' from access.role_permissions
where permission_name = 'events.config.update'
on conflict (role_name, permission_name) do nothing;

insert into access.user_permissions (user_id, permission_name, granted, artcc_id)
select user_id, 'flow.surface_data.update', granted, artcc_id
from access.user_permissions
where permission_name = 'events.config.update'
on conflict (user_id, permission_name, (coalesce(artcc_id, ''))) do nothing;
