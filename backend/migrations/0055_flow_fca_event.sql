-- @formatter:off
-- Event-specific FCAs. An FCA planned inside the event manager carries an `event_id` and a lifecycle
-- `event_status`: 'planned' (visible only in the event's builder), 'published' (live on every map, like
-- a normal FCA), 'archived' (hidden again, kept as event history). A NULL `event_id` is an ordinary
-- shared FCA — unchanged behaviour. `auto_publish` (per-FCA) drives the 30-min-before-start scheduler.

alter table flow.fca
    add column if not exists event_id bigint references events.event(id) on delete cascade,
    add column if not exists event_status text check (event_status in ('planned', 'published', 'archived')),
    add column if not exists auto_publish boolean not null default false,
    add column if not exists published_at timestamptz,
    add column if not exists archived_at timestamptz;

-- Non-event FCAs (the overwhelming majority) shouldn't be indexed; scope to the event ones.
create index if not exists idx_flow_fca_event on flow.fca (event_id) where event_id is not null;
