-- Read-only audit for the #805 deploy (VATUSA/OIS#805): who holds SERVER_ADMIN before and after
-- migration 0130, and who stops being server admin when the release starts.
--
-- 0130 itself changes nobody's access: it re-tags the `manual` USER and SERVER_ADMIN rows 0098
-- backfilled as `system`, and resolution ignores `source`. The effect lands right after it, when the
-- backend starts: demote_unconfigured_server_admins removes SERVER_ADMIN, of any source, from every
-- holder whose CID is not in OIS_SERVER_ADMIN_CID, and puts them on the baseline. Before #805 a row
-- 0098 backfilled outlived the list, so a server admin since removed from it kept the role.
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
--   * `configured` — the holder's CID is in `admin_cids`. Every holder with `false` is demoted when the
--     release starts, whatever their sources; check each is a former admin.
--
-- `repos::access::tests::the_0130_audit_lists_every_server_admin_before_and_after` runs the SETUP and
-- REPORT blocks below, verbatim, so this file cannot drift from what the test proves.

begin;

-- BEGIN SETUP
create temp table audit_configured_cids (cid bigint primary key);

create temp view audit_admin_rows as
select user_id,
       string_agg(source || coalesce('@' || artcc_id, ''), ',' order by source, artcc_id) as sources
from access.user_roles
where role_name = 'SERVER_ADMIN'
group by user_id;

create temp table audit_admins_before as
select * from audit_admin_rows;
-- END SETUP
;

-- Parsed as configured_server_admin_cids parses it: a part that isn't a positive integer is skipped.
insert into audit_configured_cids
select distinct cid
from (
    select case when trim(part) ~ '^[0-9]{1,18}$' then trim(part)::bigint end as cid
    from unnest(string_to_array(:'admin_cids', ',')) as part
) parsed
where cid > 0;

\i backend/migrations/0130_retag_system_groups.sql

-- BEGIN REPORT
select u.cid,
       u.display_name,
       coalesce(b.sources, '') as sources_before,
       coalesce(a.sources, '') as sources_after,
       c.cid is not null as configured
from audit_admins_before b
full join audit_admin_rows a on a.user_id = b.user_id
join identity.users u on u.id = coalesce(a.user_id, b.user_id)
left join audit_configured_cids c on c.cid = u.cid
order by u.cid nulls last, u.display_name
-- END REPORT
;

rollback;
