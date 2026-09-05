-- @formatter:off
-- Permissions for the admin background-tasks viewer (issue #34): read job statuses, and
-- "update" = trigger an immediate run. National, admin-facing.

insert into access.permissions (name, description) values
    ('system.jobs.read',   'View background-job status'),
    ('system.jobs.update', 'Trigger a background job to run now')
on conflict (name) do nothing;
