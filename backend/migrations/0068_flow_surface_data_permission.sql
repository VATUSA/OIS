-- Dedicated permission for the airport surface-data editor (#164's sub-issue B), replacing the
-- borrowed events.config.update placeholder that #177's CRUD handlers shipped with.

insert into access.permissions (name, description) values
    ('flow.surface_data.update', 'Manage an airport''s gates/ramp areas/taxiways (facility-scoped)')
on conflict (name) do nothing;
