-- @formatter:off
-- Gate/SID → departure-runway rules, per airport configuration (VATUSA/OIS#512, sub-issue D of #434).
--
-- #511's prediction ladder is: manual override → facility rule → the active config's
-- departure_runways → none. 0089 (sub-issue A) built the store and the config default; this is the
-- facility-rule rung, which until now had nothing behind it, so every flight fell through to the
-- default.
--
-- Mirrors flow.runway_config.star_rules (0024), which already solves this shape for arrivals, rather
-- than inventing a second configuration idiom.
--
-- TWO maps, not one: a gate named "A1" and a SID named "A1" are different things, and a single map
-- keyed on either would make that ambiguous for good. SID keys are stored revision-stripped by
-- feed::runway::star_base (CAMRN4 → CAMRN) — the same normalisation the arrival side uses, because the
-- route parser already treats STAR and SID names identically.
--
-- Defaulting to '{}' is what lets the epic ship incrementally: all 185 airports start with no rules
-- and behave exactly as they do today. One facility configuring rules must not change the other 184.

alter table flow.airport_config
    add column if not exists sid_rules jsonb not null default '{}'::jsonb;
alter table flow.airport_config
    add column if not exists gate_rules jsonb not null default '{}'::jsonb;

comment on column flow.airport_config.sid_rules is
    'Departure SID base → runway, e.g. {"CAMRN": "31L"}; empty means no rules (#512).';
comment on column flow.airport_config.gate_rules is
    'Departure gate/stand name → runway, e.g. {"A1": "04L"}; empty means no rules (#512).';
