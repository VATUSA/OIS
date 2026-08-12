-- @formatter:off
-- Permissions for managing service accounts (the Discord bot's credentials + roles).
-- audit.logs.read is already seeded (0004). SERVER_ADMIN holds all of these implicitly.
insert into access.permissions (name, description) values
    ('service_accounts.read', 'List service accounts'),
    ('service_accounts.create', 'Create a service account and issue credentials'),
    ('service_accounts.update', 'Update service account roles / rotate credentials'),
    ('service_accounts.delete', 'Disable a service account')
on conflict (name) do nothing;
