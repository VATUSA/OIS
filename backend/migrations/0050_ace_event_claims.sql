-- Move ACE support requests into per-event planning: event-scope them, add a slot count, and split
-- claiming into a per-person claims table (notes + availability window). This supersedes both the
-- national ACE queue's single-claim model and the old events.staffing_request section.

-- 1) ace.requests: event-scope + slots; drop the single-claim + single-time columns.
alter table ace.requests
    add column if not exists event_id bigint,
    add column if not exists slots int not null default 1;

-- Existing rows are national (no event) — this feature replaces them; clear them so event_id can be
-- made NOT NULL. (No production data yet.)
delete from ace.requests where event_id is null;

alter table ace.requests
    alter column event_id set not null,
    drop column if exists claimed_by,
    drop column if exists claimed_at,
    drop column if exists requested_for;

alter table ace.requests
    add constraint ace_requests_event_fk foreign key (event_id)
        references events.event(id) on delete cascade,
    add constraint ace_requests_slots_ck check (slots between 1 and 99);

-- Relax the status check: 'claimed' is gone (fill is derived from claim count vs slots). Drop the
-- original inline check by whatever name Postgres gave it, then add the new one.
do $$
declare cname text;
begin
    select conname into cname
      from pg_constraint
     where conrelid = 'ace.requests'::regclass and contype = 'c'
       and pg_get_constraintdef(oid) ilike '%status%';
    if cname is not null then
        execute format('alter table ace.requests drop constraint %I', cname);
    end if;
end $$;

alter table ace.requests
    add constraint ace_requests_status_ck check (status in ('open', 'completed', 'cancelled'));

create index if not exists idx_ace_requests_event
    on ace.requests(event_id, status, created_at desc);

-- 2) claims: one row per person per request (notes + availability window within the event).
create table if not exists ace.claims (
    id          text primary key default gen_random_uuid()::text,
    request_id  text not null references ace.requests(id) on delete cascade,
    claimed_by  text not null references identity.users(id) on delete cascade,
    notes       text not null default '',
    start_time  timestamptz,
    end_time    timestamptz,
    claimed_at  timestamptz not null default now(),
    unique (request_id, claimed_by)
);
create index if not exists idx_ace_claims_request on ace.claims(request_id);

-- 3) Drop the old event-staffing system (replaced by event-scoped ACE requests above).
drop table if exists events.staffing_request;
