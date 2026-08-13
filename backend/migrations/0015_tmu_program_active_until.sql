-- @formatter:off
-- Optional scheduled end for a rate program. Null = indefinite (persistent config). When
-- set, the cleanup job removes the program an hour after this time (see 0014).

alter table tmu.programs
    add column if not exists active_until timestamptz;
