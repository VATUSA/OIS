-- @formatter:off
-- Configurable per-aircraft performance profiles for the trajectory / ETA model
-- (backend/src/feed/trajectory.rs). Each row is a climb / cruise / descent speed schedule (KIAS,
-- with an optional Mach for the high-altitude segments), climb & descent rates (fpm), and a
-- service ceiling. Matched to a flight by exact ICAO type, then wake class (L/M/H/J), then the
-- single global default -- so an unmatched aircraft behaves like the legacy hard-coded model.
-- National reference data (aircraft performance is universal); managed by staff holding
-- flow.aircraft_profiles.update.

create table if not exists flow.aircraft_profile (
    kind text not null check (kind in ('type', 'wake', 'default')),
    -- ICAO type (e.g. C172), a wake token (L/M/H/J), or '' for the singleton default.
    key  text not null default '',
    name text not null default '',                -- display label, e.g. "Cessna 172"
    -- climb speed schedule (KIAS) + rates (fpm)
    climb_ias_lo     double precision not null default 250,
    climb_ias_hi     double precision not null default 290,
    climb_mach       double precision,            -- null = no Mach segment
    climb_fpm_lo     double precision not null default 2000,
    climb_fpm_hi     double precision not null default 1500,
    -- cruise
    cruise_tas       double precision,            -- ceiling on the (capped) filed TAS; null = filed
    cruise_mach      double precision,
    service_ceiling_ft double precision not null default 45000,
    -- descent speed schedule (KIAS) + rate (fpm)
    desc_mach        double precision,
    desc_ias_hi      double precision not null default 290,
    desc_ias_lo      double precision not null default 250,
    desc_fpm         double precision not null default 1800,
    updated_by text references identity.users(id) on delete set null,
    updated_at timestamptz not null default now(),
    primary key (kind, key)
);

-- One row per (kind, key); the default is the single row with kind='default', key=''.
create unique index if not exists idx_aircraft_profile_default
    on flow.aircraft_profile(kind) where kind = 'default';

create trigger trg_aircraft_profile_updated_at
before update on flow.aircraft_profile
for each row execute function platform.touch_updated_at();

insert into access.permissions (name, description) values
    ('flow.aircraft_profiles.read',   'View aircraft performance profiles'),
    ('flow.aircraft_profiles.update', 'Manage aircraft performance profiles (national)')
on conflict (name) do nothing;

-- Seed: the global default (the legacy 250/290, 2000/1500, FL450, plain jet descent), the four
-- wake-class defaults, and a starter set of common types. All editable in the UI afterwards.
insert into flow.aircraft_profile
    (kind, key, name,
     climb_ias_lo, climb_ias_hi, climb_mach, climb_fpm_lo, climb_fpm_hi,
     cruise_tas, cruise_mach, service_ceiling_ft,
     desc_mach, desc_ias_hi, desc_ias_lo, desc_fpm)
values
    ('default', '',   'Default',
        250, 290, null, 2000, 1500,  null, null, 45000,  null, 290, 250, 1800),
    -- wake-class fallbacks
    ('wake', 'L', 'Light (default)',
        90,  110, null, 700,  700,   130,  null, 18000,  null, 110, 90,  700),
    ('wake', 'M', 'Medium (default)',
        250, 290, 0.74, 2200, 1600,  440,  0.74, 41000,  0.74, 290, 250, 1800),
    ('wake', 'H', 'Heavy (default)',
        250, 310, 0.84, 2500, 1800,  490,  0.84, 43000,  0.84, 300, 250, 2000),
    ('wake', 'J', 'Super (default)',
        250, 310, 0.85, 2000, 1500,  500,  0.85, 43000,  0.85, 300, 250, 1800),
    -- light pistons / turboprops
    ('type', 'C172', 'Cessna 172',
        75,  90,  null, 500,  500,   110,  null, 14000,  null, 110, 90,  500),
    ('type', 'PA28', 'Piper PA-28',
        80,  90,  null, 600,  600,   115,  null, 14000,  null, 110, 90,  600),
    ('type', 'C208', 'Cessna 208 Caravan',
        100, 120, null, 900,  700,   170,  null, 25000,  null, 140, 110, 800),
    -- regional jets
    ('type', 'CRJ7', 'Bombardier CRJ700',
        250, 290, 0.78, 2000, 1500,  440,  0.78, 41000,  0.78, 290, 250, 1800),
    ('type', 'E75L', 'Embraer E175',
        250, 290, 0.78, 2200, 1600,  450,  0.78, 41000,  0.78, 290, 250, 1800),
    -- narrowbodies
    ('type', 'B738', 'Boeing 737-800',
        250, 290, 0.78, 2500, 1800,  450,  0.78, 41000,  0.78, 290, 250, 1800),
    ('type', 'A320', 'Airbus A320',
        250, 290, 0.78, 2200, 1800,  450,  0.78, 39000,  0.78, 290, 250, 1800),
    -- widebodies
    ('type', 'B763', 'Boeing 767-300',
        250, 300, 0.80, 2200, 1600,  460,  0.80, 41000,  0.80, 300, 250, 1900),
    ('type', 'B77W', 'Boeing 777-300ER',
        250, 310, 0.84, 2500, 1800,  490,  0.84, 43000,  0.84, 300, 250, 2000),
    ('type', 'B789', 'Boeing 787-9',
        250, 310, 0.85, 2600, 1900,  490,  0.85, 43000,  0.85, 300, 250, 2000),
    ('type', 'A359', 'Airbus A350-900',
        250, 310, 0.85, 2600, 1900,  490,  0.85, 43000,  0.85, 300, 250, 2000)
on conflict (kind, key) do nothing;
