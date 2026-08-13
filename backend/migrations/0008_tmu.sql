-- @formatter:off
-- TMU domain — first table: Traffic Management Initiatives (TMIs). Draft→published
-- lifecycle. NTML entries + advisories follow the same pattern in later migrations.

create table if not exists tmu.tmis (
    id text primary key default gen_random_uuid()::text,
    artcc_id text references org.facilities(id) on delete set null,
    kind text not null,
    element text not null,
    restriction text not null,
    reason text,
    effective_start timestamptz not null default now(),
    effective_end timestamptz,
    status text not null default 'draft'
        check (status in ('draft', 'published', 'expired', 'cancelled')),
    created_by text references identity.users(id) on delete set null,
    published_by text references identity.users(id) on delete set null,
    published_at timestamptz,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);

create index if not exists idx_tmu_tmis_status on tmu.tmis(status);
create index if not exists idx_tmu_tmis_artcc on tmu.tmis(artcc_id);

create trigger trg_tmu_tmis_updated_at
before update on tmu.tmis
for each row execute function platform.touch_updated_at();

-- Seed the TMU permission catalog (held by NTMO + ARTCC-scoped TMU staff; SERVER_ADMIN
-- holds them implicitly). Assignable in the access editor.
insert into access.permissions (name, description) values
    ('tmu.ntml.read', 'Read NTML entries'),
    ('tmu.ntml.create', 'Create NTML entries'),
    ('tmu.ntml.update', 'Update NTML entries'),
    ('tmu.ntml.delete', 'Delete NTML entries'),
    ('tmu.adv.read', 'Read advisories'),
    ('tmu.adv.create', 'Create advisories'),
    ('tmu.adv.update', 'Update advisories'),
    ('tmu.adv.publish', 'Publish advisories'),
    ('tmu.tmi.read', 'Read TMIs'),
    ('tmu.tmi.create', 'Create TMIs'),
    ('tmu.tmi.update', 'Update TMIs'),
    ('tmu.tmi.publish', 'Publish TMIs'),
    ('tmu.tmi.delete', 'Delete TMIs'),
    ('tmu.delays.read', 'Read the average-delay data')
on conflict (name) do nothing;
