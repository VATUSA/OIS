-- DCC event-thread availability: NTMOs (and other authorized staff) indicate whether they can cover
-- an event by pressing the 🟢/🟡/🔴 buttons on the DCC planning thread. Responses land here — one row
-- per person, latest press wins — and the event planner surfaces who indicated what.

create table if not exists events.availability (
    event_id   bigint not null references events.event(id) on delete cascade,
    user_id    text   not null references identity.users(id) on delete cascade,
    status     text   not null check (status in ('available', 'partial', 'unavailable')),
    updated_at timestamptz not null default now(),
    primary key (event_id, user_id)
);

create index if not exists idx_event_availability_event on events.availability(event_id);

create trigger trg_event_availability_updated_at
before update on events.availability
for each row execute function platform.touch_updated_at();

-- Permission to respond (press a button). Viewing the panel reuses events.plan.read. NTMOs and DCC
-- staff may respond by default; extend to other roles via Access Control.
insert into access.permissions (name, description) values
    ('events.availability.update', 'Indicate availability on a DCC event thread')
on conflict (name) do nothing;

insert into access.role_permissions (role_name, permission_name) values
    ('NTMO',      'events.availability.update'),
    ('DCC_STAFF', 'events.availability.update')
on conflict do nothing;
