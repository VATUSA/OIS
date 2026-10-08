-- Read-only audit for migration 0130 (VATUSA/OIS#805): who holds SERVER_ADMIN before and after it,
-- and who stops being server admin at their next sign-in because of it.
--
-- 0130 changes nobody's access when it runs: it re-tags the `manual` USER and SERVER_ADMIN rows 0098
-- backfilled as `system`, and resolution ignores `source`. The effect lands at sign-in, where
-- revoke_server_admin deletes the `system` SERVER_ADMIN of anyone not in OIS_SERVER_ADMIN_CID. Before
-- 0130 it could not see a backfilled row, so a server admin since removed from the list kept the role.
--
-- It does not re-implement the migration. It applies 0130 itself inside a transaction, lists the
-- holders before and after, and rolls back.
--
-- Run it from the repo root, against a production snapshot on which 0130 has NOT been applied, with
-- the production value of OIS_SERVER_ADMIN_CID:
--
--   psql "$SNAPSHOT_URL" -v ON_ERROR_STOP=1 -v admin_cids="$OIS_SERVER_ADMIN_CID" \
--     -f backend/audits/0130_retag_effect.sql
--
-- Nothing persists: the transaction always rolls back.
--
-- Reading the result, one row per SERVER_ADMIN holder:
--   * `sources_before` / `sources_after` — the holder's SERVER_ADMIN rows by source (and scope, if
--     any). Every `manual` should be gone from `sources_after`.
--   * `configured` — the holder's CID is in `admin_cids`.
--   * `admin_after_sign_in_today` / `..._with_0130` — whether they are still server admin after
--     their next sign-in, without and with 0130. Sign-in grants a configured CID the role and deletes
--     an unconfigured holder's `system` rows, so an unconfigured holder stays admin only through a
--     row of another source. A row where the two differ is someone 0130 demotes; there should be one
--     for each former admin and none for a configured one.
--
-- `repos::access::tests::the_0130_audit_lists_every_server_admin_before_and_after` runs the SETUP and
-- REPORT blocks below, verbatim, so this file cannot drift from what the test proves.

begin;

-- BEGIN SETUP
create temp table audit_configured_cids (cid bigint primary key);

create temp view audit_admin_rows as
select user_id,
       string_agg(source || coalesce('@' || artcc_id, ''), ',' order by source, artcc_id) as sources,
       bool_or(source <> 'system') as survives_revoke
from access.user_roles
where role_name = 'SERVER_ADMIN'
group by user_id;

create temp table audit_admins_before as
select * from audit_admin_rows;
-- END SETUP
;

insert into audit_configured_cids
select distinct trim(cid)::bigint
from unnest(string_to_array(:'admin_cids', ',')) as cid
where trim(cid) <> '';

\i backend/migrations/0130_retag_system_groups.sql

-- BEGIN REPORT
select u.cid,
       u.display_name,
       coalesce(b.sources, '') as sources_before,
       coalesce(a.sources, '') as sources_after,
       c.cid is not null as configured,
       c.cid is not null or coalesce(b.survives_revoke, false) as admin_after_sign_in_today,
       c.cid is not null or coalesce(a.survives_revoke, false) as admin_after_sign_in_with_0130
from audit_admins_before b
full join audit_admin_rows a on a.user_id = b.user_id
join identity.users u on u.id = coalesce(a.user_id, b.user_id)
left join audit_configured_cids c on c.cid = u.cid
order by u.cid nulls last, u.display_name
-- END REPORT
;

rollback;
