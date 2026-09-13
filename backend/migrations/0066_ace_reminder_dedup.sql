-- Closes a TOCTOU race in the ACE-claim reminder scheduler (backend/src/jobs.rs): the scheduler
-- SELECTs claims due for a reminder, then INSERTs an outbound_jobs row to both dispatch and
-- de-duplicate future ticks. Without a DB-level constraint, two backend processes briefly running
-- against the same database during a rolling deploy could both SELECT the same due claim before
-- either commits its INSERT, sending a duplicate reminder DM.
--
-- Scoped to just the two reminder job types via a partial index — NOT a blanket constraint on
-- (job_type, subject_type, subject_id), because other job types sharing this table (e.g.
-- ace_request_notify) legitimately enqueue multiple jobs for the same subject over time (once per
-- claim/release), which a general uniqueness constraint would silently break.
create unique index if not exists idx_outbound_jobs_ace_reminder_dedup
    on integration.outbound_jobs (subject_id, job_type)
    where job_type in ('ace_claim_reminder_24h', 'ace_claim_reminder_6h');
