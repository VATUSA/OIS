-- Desktop diagnostics reports (#629): a bundle a user chooses to send from the desktop app — platform
-- facts, the app's own context, a note, and its redacted log files (gzip). Read and deleted by staff
-- holding the permissions below; pruned after 30 days (`jobs::spawn_diagnostics_report_prune`); removed
-- with the user who sent it. See docs/features/diagnostics.md for what is stored and why.
create schema if not exists diagnostics;

create table diagnostics.reports (
    id text primary key default gen_random_uuid()::text,
    user_id text not null references identity.users(id) on delete cascade,
    created_at timestamptz not null default now(),
    app_version text not null default '',
    os text not null default '',
    os_version text not null default '',
    arch text not null default '',
    webview_version text not null default '',
    window_label text not null default '',
    route text not null default '',
    note text not null default '',
    -- Everything the desktop sent, as sent (redacted on the device), for fields with no column.
    meta jsonb not null,
    logs bytea not null,
    logs_bytes integer not null
);

-- Retention prunes by age; the admin list pages newest first.
create index diagnostics_reports_created_at on diagnostics.reports (created_at desc);
-- The per-user hourly cap counts a user's recent reports.
create index diagnostics_reports_user_created_at on diagnostics.reports (user_id, created_at desc);

insert into access.permissions (name, description) values
    ('diagnostics.reports.read', 'Read desktop diagnostics reports, including their logs'),
    ('diagnostics.reports.delete', 'Delete a desktop diagnostics report (e.g. on an erasure request)')
on conflict (name) do nothing;

-- VATUSA staff hold the whole catalogue (0094); these are no exception. Facility roles get neither.
insert into access.role_permissions (role_name, permission_name) values
    ('VATUSA_STAFF', 'diagnostics.reports.read'),
    ('VATUSA_STAFF', 'diagnostics.reports.delete')
on conflict do nothing;
