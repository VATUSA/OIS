-- @formatter:off
-- Index the reaper's scan over leased outbound jobs (#446 review).
--
-- `reap_stranded_jobs` asks for `status = 'in_progress' and last_attempt_at < $1`. The only index on
-- `integration.outbound_jobs` is `idx_outbound_jobs_due` — `(next_attempt_at) where status = 'pending'`
-- (`0048_integration_discord.sql:26`) — which serves `lease_jobs` and is no use here, so the reaper
-- sequentially scanned the table on every `CLEANUP_INTERVAL` tick.
--
-- That matters more than the reaper's own cost, because nothing prunes this table: there is no
-- `delete from integration.outbound_jobs` anywhere, so `succeeded` rows accumulate for the life of the
-- deployment and the scan grows with them.
--
-- Partial on `status = 'in_progress'`, which is the point: in normal operation that set is the handful of
-- jobs currently leased, usually empty, so the index stays tiny and the reaper's lookup is bounded by the
-- number of in-flight jobs rather than by the table's history.
--
-- 0086 rather than 0081-0085: those are claimed by the unmerged #433, #432, #436, #444 and #457 branches.

create index if not exists idx_outbound_jobs_leased
    on integration.outbound_jobs (last_attempt_at) where status = 'in_progress';
