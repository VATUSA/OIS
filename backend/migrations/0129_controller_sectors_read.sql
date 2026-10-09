-- @formatter:off
-- VATUSA/OIS#725: CONTROLLER reads sector demand.
--
-- The Sector Monitor (Operations -> Sector Monitor, GET /api/v1/flow/sector-demand/{artcc}) is gated on
-- flow.sectors.read (0120), which only the flow-domain groups held, so a rostered controller could not
-- open it. The owner's decision on #725 (2026-10-07, decision 4) grants it to CONTROLLER. Additive: one row,
-- nothing else in 0126's set changes.
--
-- Scope: the row is granted where CONTROLLER is (one access.user_roles row per home and visiting ARTCC,
-- source = 'vatusa'), but flow.sectors.read is a read gate (RequirePermission), which a grant at any scope
-- satisfies. So a ZDC controller reads every ARTCC's demand, which the page's view-only neighbour tables
-- need, and edits nothing: the limit and consolidation writes have their own scoped permissions, which
-- CONTROLLER does not hold. The same gate opens the admin sector viewer (Admin -> Flow -> Sectors).
--
-- No role is added, so crates/ois-core/src/catalog.rs (default_roles, role names only) and
-- ASSIGNABLE_USER_ROLES are unchanged. The set is pinned by
-- repos::access::tests::the_controller_group_is_exactly_the_operational_baseline.

insert into access.role_permissions (role_name, permission_name)
select 'CONTROLLER', p.name
from access.permissions p
where p.name in ('flow.sectors.read')
on conflict do nothing;
