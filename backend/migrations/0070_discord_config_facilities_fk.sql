-- Retroactively constrain integration.discord_config_facilities.artcc_id to real facilities —
-- every sibling ARTCC-id column in this schema has this FK (org.facilities, e.g. tmu.tmis.artcc_id
-- in 0008_tmu.sql), but 0069 (which added this table) omitted it. Without it, a typo'd or
-- since-deleted ARTCC can be stored and will silently never match any real event/request facility,
-- reproducing the exact "silently loses to another guild" failure mode #194 exists to fix — just
-- via a data-entry mistake instead of a resolution-order bug. Table is brand new (0069, unreleased
-- before this), so no existing-row cleanup is needed before adding the constraint.

alter table integration.discord_config_facilities
    add constraint discord_config_facilities_artcc_id_fkey
    foreign key (artcc_id) references org.facilities(id) on delete cascade;
