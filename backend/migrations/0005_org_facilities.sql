-- @formatter:off
-- VATUSA facilities (ARTCCs). This is what the per-ARTCC permission scope (`artcc_id`)
-- references. Seeded with the known VATUSA ARTCC list; a later VATUSA sync reconciles
-- names/membership. Codes are the primary key (e.g. 'ZDC').

create table if not exists org.facilities (
    id text primary key,
    name text not null,
    region text,
    active boolean not null default true,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);

create trigger trg_org_facilities_updated_at
before update on org.facilities
for each row execute function platform.touch_updated_at();

insert into org.facilities (id, name) values
    ('ZAB', 'Albuquerque ARTCC'),
    ('ZAN', 'Anchorage ARTCC'),
    ('ZAU', 'Chicago ARTCC'),
    ('ZBW', 'Boston ARTCC'),
    ('ZDC', 'Washington ARTCC'),
    ('ZDV', 'Denver ARTCC'),
    ('ZFW', 'Fort Worth ARTCC'),
    ('ZHU', 'Houston ARTCC'),
    ('ZID', 'Indianapolis ARTCC'),
    ('ZJX', 'Jacksonville ARTCC'),
    ('ZKC', 'Kansas City ARTCC'),
    ('ZLA', 'Los Angeles ARTCC'),
    ('ZLC', 'Salt Lake City ARTCC'),
    ('ZMA', 'Miami ARTCC'),
    ('ZME', 'Memphis ARTCC'),
    ('ZMP', 'Minneapolis ARTCC'),
    ('ZNY', 'New York ARTCC'),
    ('ZOA', 'Oakland ARTCC'),
    ('ZOB', 'Cleveland ARTCC'),
    ('ZSE', 'Seattle ARTCC'),
    ('ZTL', 'Atlanta ARTCC'),
    ('HCF', 'Honolulu Control Facility')
on conflict (id) do nothing;

-- Make the scope columns real foreign keys now that facilities exist. NULL stays
-- allowed (national scope); a non-null artcc_id must be a known facility.
alter table access.user_roles
    add constraint fk_access_user_roles_artcc
    foreign key (artcc_id) references org.facilities(id) on delete cascade;

alter table access.user_permissions
    add constraint fk_access_user_permissions_artcc
    foreign key (artcc_id) references org.facilities(id) on delete cascade;

alter table access.service_account_roles
    add constraint fk_access_service_account_roles_artcc
    foreign key (artcc_id) references org.facilities(id) on delete cascade;

-- Facility administration permissions (assignable in the access editor; directory
-- listing itself is public reference data).
insert into access.permissions (name, description) values
    ('facilities.directory.read', 'Read the facilities directory'),
    ('facilities.directory.update', 'Update facility records')
on conflict (name) do nothing;
