-- Retroactively constrain integration.discord_config_facilities.artcc_id to real facilities.
-- Some ARTCC-id columns in this schema are deliberately free text (ace.requests.artcc_id,
-- access.api_key_permissions.artcc_id) where an unresolvable value is harmless — but this one is
-- part of a composite primary key in a pure join table (config_id, artcc_id) whose only purpose is
-- matching against org.facilities.id at read time (channel_id/role_id), so an unconstrained typo'd
-- or since-deleted ARTCC would silently never match anything, reproducing the exact "silently loses
-- to another guild" failure mode #194 exists to fix — just via a data-entry mistake instead of a
-- resolution-order bug. Table is brand new (0069, unreleased before this), so no existing-row
-- cleanup is needed before adding the constraint.

alter table integration.discord_config_facilities
    add constraint discord_config_facilities_artcc_id_fkey
    foreign key (artcc_id) references org.facilities(id) on delete cascade;
