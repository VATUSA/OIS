-- @formatter:off
-- A member an admin has hand-edited is off VATUSA role sync until an explicit Resync (#549).
--
-- `vatusa_roles_detached_at` null = synced. While it's set, the VATUSA reconciler leaves the member's
-- group grants alone; their name, rating, facility and stored VATUSA roles keep syncing, so the admin
-- view stays current and a Resync reconciles from fresh data. `_by` records who first detached them.

alter table identity.users
    add column if not exists vatusa_roles_detached_at timestamptz,
    add column if not exists vatusa_roles_detached_by text references identity.users(id) on delete set null;
