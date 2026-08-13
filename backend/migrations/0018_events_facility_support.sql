-- @formatter:off
-- Per-event facility support matrix: which facilities the event needs staffed, at what
-- level (required / preferred / not required). One row per (event, facility). Edited by
-- the events team (events.plan.update); facility-scoped self-edit lands with the AAR/ADR
-- scope infrastructure in a later pass.

create table if not exists events.facility_support (
    event_id bigint not null references events.event(id) on delete cascade,
    facility text not null,
    -- required | preferred | not_required
    level text not null default 'required',
    notes text not null default '',
    updated_by text references identity.users(id) on delete set null,
    updated_at timestamptz not null default now(),
    primary key (event_id, facility)
);

create trigger trg_events_facility_support_updated_at
before update on events.facility_support
for each row execute function platform.touch_updated_at();
