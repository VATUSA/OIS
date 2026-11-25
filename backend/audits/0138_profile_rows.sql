-- Read-only audit for the #985 deploy (VATUSA/OIS#985): which aircraft-profile type rows migration 0138
-- removes, and which stay. A type whose row goes flies ERAM's tables once the release starts.
--
-- It does not re-implement the migration. It applies 0138 itself inside a transaction, compares the rows
-- before and after, and rolls back.
--
-- Run it from the repo root, against a production snapshot on which 0138 has NOT been applied:
--
--   psql "$SNAPSHOT_URL" -v ON_ERROR_STOP=1 -f backend/audits/0138_profile_rows.sql
--
-- Nothing persists: the transaction always rolls back.
--
-- Reading the result, one row per stored type row:
--   * `removed` — true for a seeded row nobody changed. Anything else stays and keeps beating ERAM.
--   * `updated_at` — a row that stays with a later date than the others was edited on the page.
--
-- `repos::aircraft_profiles::eram_tests::the_0138_audit_names_the_rows_that_go` runs the SETUP and REPORT
-- blocks below, verbatim.

begin;

-- BEGIN SETUP
create temp table audit_types_before as
select key, updated_at from flow.aircraft_profile where kind = 'type';
-- END SETUP
;

\i backend/migrations/0138_aircraft_profile_eram.sql

-- BEGIN REPORT
select b.key,
       not exists (
           select 1 from flow.aircraft_profile p where p.kind = 'type' and p.key = b.key
       ) as removed,
       b.updated_at
from audit_types_before b
order by b.key
-- END REPORT
;

rollback;
