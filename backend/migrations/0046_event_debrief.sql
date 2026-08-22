-- Post-event debrief: a free-text write-up an event lead keeps alongside the auto-generated capture
-- stats. One row per event. The consolidated debrief view (planned-vs-actual, coordination recap)
-- is composed on the client from existing data; this table stores only the narrative.

create table if not exists events.event_debrief (
    event_id   bigint primary key references events.event(id) on delete cascade,
    notes      text not null default '',
    updated_by text references identity.users(id) on delete set null,
    updated_at timestamptz not null default now()
);

-- Writing a debrief entry is its own action; reading it rides on events.plan.read (viewing the event).
insert into access.permissions (name, description) values
    ('events.debrief.create', 'Write an event''s post-event debrief notes')
on conflict (name) do nothing;
