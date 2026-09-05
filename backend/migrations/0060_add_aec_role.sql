-- @formatter:off
-- Reinstate AEC (Assistant Events Coordinator) as an assignable positional role. It was seeded in
-- 0004 and removed in 0007's role reconciliation; issue #33 brings it back. Facility-scopable via
-- artcc_id on the grant, like EC. Like EC/EVENTS_TEAM it carries no default role_permissions —
-- capabilities are granted explicitly (nothing is implied by role name). Kept in sync with
-- crates/ois-core/src/catalog.rs (default_roles) and backend ASSIGNABLE_USER_ROLES.

insert into access.roles (name, description) values
    ('AEC', 'Assistant Events Coordinator')
on conflict (name) do update set description = excluded.description;
