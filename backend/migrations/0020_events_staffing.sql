-- @formatter:off
-- Per-event ACE staffing requests: how many positions a facility is hoping for vs how
-- many have signed up. One row per (event, facility). Read alongside the event plan;
-- edited with events.staffing_requests.create.

create table if not exists events.staffing_request (
    event_id bigint not null references events.event(id) on delete cascade,
    facility text not null,
    positions_requested int not null default 0 check (positions_requested between 0 and 999),
    positions_filled int not null default 0 check (positions_filled between 0 and 999),
    -- open | met | closed
    status text not null default 'open' check (status in ('open', 'met', 'closed')),
    notes text not null default '',
    updated_by text references identity.users(id) on delete set null,
    updated_at timestamptz not null default now(),
    primary key (event_id, facility)
);

create trigger trg_events_staffing_request_updated_at
before update on events.staffing_request
for each row execute function platform.touch_updated_at();

insert into access.permissions (name, description) values
    ('events.staffing_requests.read', 'View ACE staffing requests'),
    ('events.staffing_requests.create', 'Create / update ACE staffing requests'),
    ('events.staffing_requests.decide', 'Acknowledge or decline ACE staffing requests')
on conflict (name) do nothing;
