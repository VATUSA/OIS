-- @formatter:off
-- Index `access.audit_logs.created_at` so retention can prune by age (#444).
--
-- The table had only `(resource_type, resource_id)`, which a `where created_at < $1` cannot use — so
-- every prune pass would sequential-scan a table that has been growing since the deployment started
-- (most recently at the Discord bot's poll rate, ~17k rows a day per process, until #430 stopped it).
--
-- Also the index the admin audit view wants: it lists newest-first, and every filter is opt-in, so
-- the unfiltered default ordering is exactly this.
--
-- 0084 rather than 0081-0083: those are claimed by the unmerged #433, #432 and #436 branches.

create index if not exists idx_access_audit_logs_created_at
    on access.audit_logs (created_at desc);
