-- VATUSA member sync: enrich identity.users with the details fetched from the VATUSA API on
-- sign-in, mirror the member's roles/visits, and persist the per-facility webhook secrets used
-- to verify inbound roster-change deliveries. Keyed on VATSIM CID throughout (the natural key).

alter table identity.users
    add column if not exists home_facility text,
    add column if not exists rating_numeric int,
    add column if not exists flag_home_controller boolean,
    add column if not exists facility_join timestamptz,
    add column if not exists vatusa_synced_at timestamptz;

-- The member's VATUSA roles, e.g. INS@ZDC, DATM@ZDC (fully replaced on each sync).
create table if not exists identity.vatusa_roles (
    id bigint generated always as identity primary key,
    cid bigint not null,
    facility text not null,
    role text not null,
    granted_at timestamptz,
    created_at timestamptz not null default now(),
    unique (cid, facility, role)
);
create index if not exists idx_vatusa_roles_cid on identity.vatusa_roles (cid);

-- Facilities the member visits (fully replaced on each sync).
create table if not exists identity.vatusa_visits (
    id bigint generated always as identity primary key,
    cid bigint not null,
    facility text not null,
    created_at timestamptz not null default now(),
    unique (cid, facility)
);
create index if not exists idx_vatusa_visits_cid on identity.vatusa_visits (cid);

-- One registered outbound webhook per facility. The `secret` is returned by VATUSA only at
-- creation time and is used to verify each delivery's HMAC signature, so it must be persisted.
create table if not exists identity.vatusa_webhooks (
    facility text primary key,
    webhook_id bigint not null,
    secret text not null,
    url text not null,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);
