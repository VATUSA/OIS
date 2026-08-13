-- @formatter:off
-- Per-event DCC (Data Coordination Center) support: whether the event needs national
-- DCC coverage. One row per event; edited by the events team (events.plan.update).

create table if not exists events.dcc_request (
    event_id bigint primary key references events.event(id) on delete cascade,
    -- not_needed | requested | confirmed
    status text not null default 'not_needed',
    notes text not null default '',
    updated_by text references identity.users(id) on delete set null,
    updated_at timestamptz not null default now()
);

create trigger trg_events_dcc_request_updated_at
before update on events.dcc_request
for each row execute function platform.touch_updated_at();
