-- @formatter:off
-- Carry ARTCC scope through the effective-permissions view, so there is one resolver
-- instead of two that disagree (VATUSA/OIS#543).
--
-- 0003_access.sql's header and its view comment say scope "is ignored here for now" and
-- that "the effective-permissions view treats all grants as applicable". That is no longer
-- true, and migrations are append-only, so the correction lives here.
--
-- What was wrong: the old view projected only (user_id, permission_name). It dropped
-- artcc_id on every arm, including the deny side, and anti-joined on the name alone — so a
-- deny scoped to one ARTCC revoked the permission *nationally*, and a grant scoped to ZDC
-- read as unscoped. Meanwhile access.permission_scope() in Rust honoured scope but never
-- read granted = false at all. Each resolver was blind to exactly what the other saw.
--
-- This view now emits one row per (permission, scope) fact and carries `granted`, leaving
-- the composition to a single Rust resolver (repos::access::fetch_effective_permissions).
-- Composing in Rust rather than SQL is deliberate: the rule "a national allow minus a
-- scoped deny" has no row representation — it is "everywhere except ZDC", which the
-- PermissionScope type expresses directly and a row set cannot without enumerating every
-- facility.
--
-- The deny rule, which was previously unstated and is now settled policy: a deny removes
-- the permission at its own scope, and a national deny (artcc_id is null) removes it
-- everywhere, even where a scoped allow exists. This matches what
-- docs/architecture/permissions.md already promised — "an explicit deny beats any allow" —
-- and it fails closed.
--
-- `drop view` rather than `create or replace`: the column list changes, and replace cannot
-- do that. Nothing else in SQL depends on this view (only one Rust reader), so dropping is
-- safe.

drop view if exists access.v_effective_user_permissions;

create view access.v_effective_user_permissions as
-- Role-derived grants keep the scope of the *membership* that confers them: user_roles
-- carries artcc_id, role_permissions deliberately does not, so one EC role can mean "EC at
-- ZDC" for one person and national for another.
select
    ur.user_id,
    rp.permission_name,
    ur.artcc_id,
    true as granted
from access.user_roles ur
join access.role_permissions rp on rp.role_name = ur.role_name

union all

-- SERVER_ADMIN holds the whole catalogue, nationally and unconditionally. Its artcc_id is
-- forced to null rather than read from the membership: the role is national by definition,
-- and it must stay env-bootstrapped and un-narrowable.
select
    sau.user_id,
    p.name as permission_name,
    null::text as artcc_id,
    true as granted
from (select distinct user_id from access.user_roles where role_name = 'SERVER_ADMIN') sau
cross join access.permissions p

union all

-- Direct grants and denies alike, each keeping its own scope. The deny rows are emitted
-- rather than subtracted here; the resolver applies them.
select
    up.user_id,
    up.permission_name,
    up.artcc_id,
    up.granted
from access.user_permissions up;
