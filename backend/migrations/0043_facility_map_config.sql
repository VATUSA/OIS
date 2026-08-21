-- @formatter:off
-- Per-facility "facility map" color-rule configuration. Each ARTCC gets one row: an ordered list of
-- color rules (opaque jsonb owned by the client rule engine) plus a default color for unmatched
-- aircraft. The public facility map reads this; editing is facility-scoped (flow.facility_map.update
-- for the facility's own ARTCC, or nationally). Keyed by facility_id, which IS the owning ARTCC, so
-- scope checks are direct.

create table if not exists flow.facility_map_config (
    facility_id text primary key,                 -- ARTCC id, e.g. "ZDC"
    rules jsonb not null default '[]',            -- ordered [{id,label,color,enabled,conditions:[...]}]
    default_color text not null default '',       -- hex for aircraft matching no rule ('' = theme default)
    updated_by text references identity.users(id) on delete set null,
    updated_at timestamptz not null default now()
);

create trigger trg_facility_map_config_updated_at
before update on flow.facility_map_config
for each row execute function platform.touch_updated_at();

insert into access.permissions (name, description) values
    ('flow.facility_map.update', 'Edit a facility map''s aircraft color rules (facility-scoped)')
on conflict (name) do nothing;
