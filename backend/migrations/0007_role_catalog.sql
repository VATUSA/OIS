-- @formatter:off
-- Reconcile the role catalog to VATUSA positional roles. Remove the facility/training
-- roles seeded in 0004, rename the national roles, and add NTMO + DCC_STAFF. Assignable
-- roles are now: EC, ACE, EVENTS_TEAM, VATUSA_STAFF, NTMO, DCC_STAFF. SERVER_ADMIN is
-- env-only; USER / BOT / SERVICE_APP are system roles. Deletes cascade to any grants of
-- the removed roles (dev only — no production data yet).

delete from access.roles where name in (
    'ATM', 'DATM', 'TA', 'WM', 'FE', 'INS', 'MTR', 'AEC', 'WEB_TEAM',
    'EVENTS_NATIONAL', 'TMU_NATIONAL', 'ACE_NATIONAL'
);

insert into access.roles (name, description) values
    ('VATUSA_STAFF', 'VATUSA division staff'),
    ('EVENTS_TEAM', 'Events Team'),
    ('EC', 'Events Coordinator'),
    ('ACE', 'ACE Team'),
    ('NTMO', 'National Traffic Management Officer'),
    ('DCC_STAFF', 'DCC staff')
on conflict (name) do update set description = excluded.description;
