-- @formatter:off
-- VATUSA/OIS#719: remove the Airspace Monitor (#594–#602). The owner decided the shipped Monitor is not
-- the feature needed; it is removed wholesale so the replacement can be designed from the operational
-- model. Nothing here is replaced.
--
-- Reverses 0116 (Monitor Alert Parameters and the flow.monitor.* permissions) and 0118 (sector
-- consolidation). Their data — MAP overrides and consolidations — is dropped for good, by that decision.
--
-- Kept, deliberately: 0111 flow.airspace_sector (the sector dataset, which no public source publishes)
-- and 0120 flow.sectors.read (its admin viewer stays as the dataset's inspector).

drop table if exists flow.sector_consolidation;
drop table if exists flow.sector_map;

-- API-key and service-account grants reference access.permissions with no cascade (0045, 0103), so
-- they go first or the delete below fails. Role and user grants cascade (0003).
delete from access.api_key_permissions
where permission_name in ('flow.monitor.read', 'flow.monitor.update');
delete from access.service_account_permissions
where permission_name in ('flow.monitor.read', 'flow.monitor.update');
delete from access.permissions
where name in ('flow.monitor.read', 'flow.monitor.update');
