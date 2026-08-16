-- Per-user, per-namespace preferences, stored as an opaque jsonb blob owned by the client.
-- The first consumer is the customizable "My dashboard" layout (namespace = 'dashboard').
create table identity.user_preferences (
    user_id    text not null references identity.users(id) on delete cascade,
    namespace  text not null,
    value      jsonb not null default '{}'::jsonb,
    updated_at timestamptz not null default now(),
    primary key (user_id, namespace)
);
