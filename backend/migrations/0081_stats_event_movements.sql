-- @formatter:off
-- Per-event movement counts, frozen when the event's capture closes (#433).
--
-- Movements are counted from `stats.flight_leg`, which is pruned at DELAY_LEG_RETAIN_DAYS (30) by
-- `jobs.rs`. Without a snapshot, every event's numbers would silently fall to zero a month after it
-- ran — the counts would be correct and then quietly disappear, which is worse than being wrong.
--
-- So the read path prefers a row here and only computes from legs when there isn't one: live and
-- recently-finished events compute, everything older serves what was frozen at close. That also
-- keeps leg retention a storage decision rather than a reporting one.
--
-- `unique_pilots` is frozen alongside the movements even though it is sourced from `stats.flight`,
-- so one event's breakdown is a single consistent snapshot rather than two halves with different
-- lifetimes.

create table if not exists stats.event_movements (
    event_id      bigint      not null references events.event (id) on delete cascade,
    icao          text        not null,
    arrivals      bigint      not null default 0,
    departures    bigint      not null default 0,
    unique_pilots bigint      not null default 0,
    -- The window the counts were taken over, so a later reader can tell what they mean rather than
    -- having to re-derive it from the event (which may since have been rescheduled).
    window_start  timestamptz not null,
    window_end    timestamptz not null,
    captured_at   timestamptz not null default now(),
    primary key (event_id, icao)
);
