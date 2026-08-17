-- @formatter:off
-- Facility support becomes an auto-derived involvement list (host ARTCC + configured airports'
-- ARTCCs + ACE staffing), and editing a facility's own support level is now facility-scoped via a
-- dedicated permission (mirroring events.rate.update) so a facility's staff can confirm/adjust
-- their own row without the events-team-wide events.plan.update grant.

insert into access.permissions (name, description) values
    ('events.support.update', 'Set a facility''s event support level (facility-scoped)')
on conflict (name) do nothing;
