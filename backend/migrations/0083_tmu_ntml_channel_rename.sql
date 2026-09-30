-- @formatter:off
-- Rename the TMI Discord channel mapping `tmu-advisories` -> `tmu-ntml` (#436).
--
-- The logical channel name is what `handlers::tmu::NTML_CHANNEL` looks up in
-- `integration.discord_channels`, keyed `(config_id, name)`. Renaming the constant without moving
-- the row means `channel_id()` answers `None` for every guild that already had one — and `None` is
-- treated as "don't post", silently: `publish_tmi` still returns 200 and nothing is logged, so TMIs
-- would simply stop reaching Discord with no signal but a quiet channel (#436 review).
--
-- Same shape as `0041_stats_perm_rename.sql`: repoint the existing rows so deployed databases keep
-- working with no admin action, and fresh installs converge to the same end state.
--
-- `(config_id, name)` is unique, so a guild that somehow has both is left alone rather than failing
-- the migration — its `tmu-ntml` mapping is already the one that wins.
--
-- 0083 rather than 0081/0082: those are claimed by the unmerged #433 and #432 branches.

update integration.discord_channels c
   set name = 'tmu-ntml'
 where c.name = 'tmu-advisories'
   and not exists (
       select 1 from integration.discord_channels other
        where other.config_id = c.config_id and other.name = 'tmu-ntml'
   );
