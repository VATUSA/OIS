-- @formatter:off
-- Manually-added runway ends, for the rare airport the bundled OurAirports dataset lacks.
-- { id, hdg, len } objects, merged with the dataset ends when building the board.

alter table flow.runway_config
    add column if not exists custom_ends jsonb not null default '[]'::jsonb;
