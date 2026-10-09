-- @formatter:off
-- VATUSA/OIS#730: CONTROLLER, the baseline operational group for rostered controllers.
--
-- Every signed-in user is in USER, which is unscoped and can't open FCA, IDST, Runway, Airport or AADC.
-- CONTROLLER is granted by the VATUSA sync, never nationally: one access.user_roles row at the member's
-- home ARTCC and one at each visiting ARTCC, with source = 'vatusa' (repos::vatusa::desired_vatusa_grants),
-- so the reconciler adds and removes only its own rows. It stays hand-assignable like any positional
-- group. Kept in sync with crates/ois-core/src/catalog.rs (default_roles) and ASSIGNABLE_USER_ROLES.
--
-- The set is the owner's decision on #730: work FCAs, routes, releases and the runway configuration, and
-- read the TMU picture that binds them. Deliberately absent: events.plan.*, stats.*, every admin domain,
-- and every *.publish (publishing a TMI or ground stop is a TMU act). (#730 also proposed flow.data.read
-- and flow.programs.read; those are catalog names with no permission row and nothing checks them, so
-- they're left out.) Each write it grants is
-- ARTCC-scoped in its handler; flow.runway.update became so in this change.

insert into access.roles (name, description) values
    ('CONTROLLER', 'Rostered controller: baseline operational access, granted per facility by VATUSA sync')
on conflict (name) do update set description = excluded.description;

insert into access.role_permissions (role_name, permission_name)
select 'CONTROLLER', p.name
from access.permissions p
where p.name in (
    'flow.fca.read', 'flow.fca.update', 'flow.fca.delete',
    'flow.route.update',
    'tmu.cfr.assign',
    'flow.runway.read', 'flow.runway.update',
    'tmu.program.read',
    'tmu.tmi.read', 'tmu.adv.read', 'tmu.ntml.read', 'tmu.gdp.read', 'tmu.groundstop.read',
    'tmu.delays.read'
)
on conflict do nothing;
