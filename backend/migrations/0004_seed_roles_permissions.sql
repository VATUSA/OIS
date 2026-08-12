-- Seed the role list and a focused starter permission catalog. Kept in sync with
-- crates/ois-core/src/catalog.rs; expanded per feature spec in later migrations.
-- SERVER_ADMIN holds every permission implicitly via v_effective_user_permissions,
-- so it needs no role_permissions rows.

insert into access.roles (name, description)
values ('SERVER_ADMIN', 'Singleton server administrator (env-bootstrapped)'),
       ('USER', 'Baseline authenticated user'),
       ('VATUSA_STAFF', 'National division staff'),
       ('EVENTS_NATIONAL', 'National events staff'),
       ('TMU_NATIONAL', 'National traffic management staff'),
       ('ACE_NATIONAL', 'National ACE team staff'),
       ('WEB_TEAM', 'Web team'),
       ('ATM', 'Air Traffic Manager (facility-scoped)'),
       ('DATM', 'Deputy ATM (facility-scoped)'),
       ('TA', 'Training Administrator (facility-scoped)'),
       ('EC', 'Events Coordinator (facility-scoped)'),
       ('AEC', 'Assistant Events Coordinator (facility-scoped)'),
       ('WM', 'Webmaster (facility-scoped)'),
       ('FE', 'Facilities Engineer (facility-scoped)'),
       ('INS', 'Instructor (facility-scoped)'),
       ('MTR', 'Mentor (facility-scoped)'),
       ('BOT', 'Discord bot service role'),
       ('SERVICE_APP', 'Generic service application role') on conflict (name) do nothing;

insert into access.permissions (name, description)
values ('auth.profile.read', 'Read current user profile'),
       ('auth.profile.update', 'Update current user profile'),
       ('auth.sessions.delete', 'Delete current session (logout)'),
       ('access.self.read', 'Read own access state'),
       ('access.catalog.read', 'Read the role/permission catalog'),
       ('access.users.read', 'Read another user''s access state'),
       ('access.users.update', 'Update another user''s access state'),
       ('users.directory.read', 'Read the user directory'),
       ('audit.logs.read', 'Read audit logs') on conflict (name) do nothing;
