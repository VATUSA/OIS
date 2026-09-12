-- Per-facility (ARTCC) reference documents (SOPs, LOAs, etc.), configurable per facility. Consumed
-- by the Discord ACE-claim DM (a later sub-issue of #143) to hand the claimer their facility's docs.

create table if not exists org.facility_documents (
    id          text primary key default gen_random_uuid()::text,
    facility_id text not null references org.facilities(id) on delete cascade,
    title       text not null,
    url         text not null,
    created_at  timestamptz not null default now(),
    updated_at  timestamptz not null default now()
);

create index if not exists idx_facility_documents_facility
    on org.facility_documents (facility_id, created_at);

create trigger trg_org_facility_documents_updated_at
before update on org.facility_documents
for each row execute function platform.touch_updated_at();

insert into access.permissions (name, description) values
    ('facilities.docs.read', 'Read a facility''s configured reference documents'),
    ('facilities.docs.update', 'Manage a facility''s configured reference documents (facility-scoped)')
on conflict (name) do nothing;
