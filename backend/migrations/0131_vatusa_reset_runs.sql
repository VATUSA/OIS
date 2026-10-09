-- @formatter:off
-- VATUSA/OIS#806: the access reset to VATUSA (#795) runs off the request. POST starts a run and answers
-- 202 with its id; the run (the division pull, then every member's reset) finishes in a background task
-- and leaves its result here, where any backend replica can answer the dialog's poll for it.
--
-- One run at a time, across replicas: a run holds a Postgres advisory lock for as long as it lives
-- (`repos::access_reset`), so a row still 'running' with no lock holder belongs to a process that
-- died, and is reported as interrupted.

create table access.vatusa_reset_runs (
    id          text primary key default gen_random_uuid()::text,
    started_by  text references identity.users(id) on delete set null,
    reason      text not null,
    started_at  timestamptz not null default now(),
    finished_at timestamptz,
    status      text not null default 'running' check (status in ('running', 'succeeded', 'failed')),
    -- The run's `AccessResetBody` when it succeeded, its `AccessResetFailure` when it failed.
    result      jsonb,
    failure     jsonb,
    check ((status = 'running') = (finished_at is null))
);
