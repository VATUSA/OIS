-- Which facility/ies a configured guild serves, so channel_id/role_id (backend/src/repos/
-- integration.rs) can prefer the guild mapped to the relevant facility when two guilds define the
-- same logical channel/role name, instead of always picking whichever guild was configured first
-- (VATUSA/OIS#194).

create table if not exists integration.discord_config_facilities (
    config_id text not null references integration.discord_configs(id) on delete cascade,
    artcc_id  text not null,
    primary key (config_id, artcc_id)
);
