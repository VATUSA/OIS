-- @formatter:off
-- VATUSA events cache. Synced from the public VATUSA v3 events API; the anchor that
-- per-event planning (DCC, facility support, AAR/ADR, ACE, TMI packages) hangs off of.

create table if not exists events.event (
    id bigint primary key,                 -- VATUSA event id
    title text not null default '',
    body text not null default '',         -- HTML/BBCode blurb from VATUSA
    banner_image_url text not null default '',
    facility text not null default '',     -- host ARTCC id (e.g. ZTL)
    start_time timestamptz not null,
    end_time timestamptz not null,
    review_status text not null default '',
    synced_at timestamptz not null default now()
);

create index if not exists idx_events_event_start on events.event(start_time);

insert into access.permissions (name, description) values
    ('events.plan.read', 'View event planning'),
    ('events.plan.update', 'Edit event planning'),
    ('events.rate.update', 'Set event airport arrival/departure rates')
on conflict (name) do nothing;
