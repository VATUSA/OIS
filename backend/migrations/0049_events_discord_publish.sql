-- Posting an event's DCC thread to Discord is an explicit, permissioned action (not automatic on
-- VATUSA publish). Seeds the permission; grants are assigned via the access editor like other
-- events.* perms (SERVER_ADMIN holds all).

insert into access.permissions (name, description) values
    ('events.discord.publish', 'Post an event''s coordination thread to Discord')
on conflict (name) do nothing;
