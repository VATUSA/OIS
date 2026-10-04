-- @formatter:off
-- VATUSA/OIS#699: ship default VATUSA role → OIS group mappings.
--
-- 0100 shipped the mapping table deliberately empty, so the sync stored every member's VATUSA roles
-- and granted nobody anything. The owner decided on #699 to ship defaults instead.
--
-- 0100's comment lists the wrong vocabulary. The division pull (`GET /v3/division/controllers`,
-- VATUSA's acl_user_role table) sends LONG-form roles — EVENT_COORDINATOR, FACILITY_ACADEMY_EDITOR,
-- INSTRUCTOR, WEB_MAINTAINER, DIVISION_TECH_TEAM — not the short codes (EC, INS, WM, …) VATUSA's
-- per-facility endpoint shows. `AEC` is not a VATUSA role at all; it's an OIS group. See
-- repos::vatusa::DOCUMENTED_VATUSA_ROLES.
--
-- The defaults:
--   * EVENT_COORDINATOR, at any facility → EC, scoped to the ARTCC the holder holds it at. VATUSA has
--     no assistant role (its "AEC" is a holder who isn't the facility's point of contact, which a
--     mapping can't see), so every holder gets EC and OIS's AEC group stays hand-assigned. EC and AEC
--     seed the same domains, so the access is the same.
--   * DIVISION_TECH_TEAM, at ZHQ (VATUSA's `*`, the division) → VATUSA_STAFF, nationally.
--
-- Nothing is granted here: grants arrive at the next reconcile of each member (the daily division
-- pull, a roster webhook, or their next sign-in), carrying source = 'vatusa' like any sync grant. An
-- admin can remove either mapping in the Groups editor, which revokes its grants the same way.

insert into access.vatusa_role_mappings (vatusa_role, facility, role_name)
values ('EVENT_COORDINATOR', null, 'EC'),
       ('DIVISION_TECH_TEAM', 'ZHQ', 'VATUSA_STAFF')
on conflict (vatusa_role, (coalesce(facility, '')), role_name) do nothing;
