-- @formatter:off
-- Give every outbound job a consumer, so two consumers can lease without stealing (#590).
--
-- `lease_jobs` selected on nothing but status and due time, so any second holder of
-- `integration.jobs.update` would lease the Discord bot's jobs, fail to route them and nack them. A lease
-- now names its consumer and sees only that consumer's jobs.
--
-- Every existing row, and every job enqueued today, is the Discord bot's — hence the default.
--
-- 0111 rather than 0103-0110: those are claimed by unmerged branches (#584, #585, #605 and others).

alter table integration.outbound_jobs
    add column if not exists consumer text not null default 'discord';

-- The lease query, now per consumer: due-pending jobs for one consumer, oldest first.
drop index if exists integration.idx_outbound_jobs_due;
create index if not exists idx_outbound_jobs_due
    on integration.outbound_jobs (consumer, next_attempt_at) where status = 'pending';
