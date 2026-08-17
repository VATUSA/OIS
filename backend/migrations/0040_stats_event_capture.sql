-- @formatter:off
-- Per-event stats-capture config. When `enabled`, the capture scheduler (jobs::spawn_capture_scheduler)
-- opens a stats.capture window tied to the event from `start_time - pre_minutes` and closes+saves it
-- at `end_time + post_minutes`, so the event's traffic is permanently retained at full fidelity and
-- tied to the event for stats generation (GET /api/v1/events/{id}/stats).

create table if not exists stats.event_capture (
    event_id     bigint primary key references events.event(id) on delete cascade,
    enabled      boolean not null default false,
    pre_minutes  integer not null default 30 check (pre_minutes between 0 and 720),
    post_minutes integer not null default 30 check (post_minutes between 0 and 720),
    updated_by   text references identity.users(id) on delete set null,
    updated_at   timestamptz not null default now()
);

create trigger trg_stats_event_capture_updated_at
before update on stats.event_capture
for each row execute function platform.touch_updated_at();
