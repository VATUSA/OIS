-- @formatter:off
-- Desktop authentication (issue #346, epic #343).
--
-- The desktop app renders the same web SPA inside a Tauri webview, but it has no browser origin and
-- cannot carry the `ois_session` cookie, so it authenticates with a token held in the OS keychain
-- and sent as `Authorization: Bearer ois_dsk_…`.
--
-- That token is deliberately *a session*, not a third credential type: it lives in this same
-- `identity.sessions` table and resolves through the existing lookup to the same `CurrentUser` a
-- cookie would, so every permission check, ARTCC scope rule and audit path behaves identically on
-- both platforms. The only thing that differs is the transport.

-- Which transport minted the session. The reader that matters is the desktop refresh endpoint,
-- which rotates only `desktop` rows — so a stolen browser cookie cannot be exchanged for a
-- long-lived, keychain-resident desktop token.
alter table identity.sessions
    add column if not exists kind text not null default 'web';

alter table identity.sessions
    drop constraint if exists identity_sessions_kind_check;

alter table identity.sessions
    add constraint identity_sessions_kind_check check (kind in ('web', 'desktop'));

-- One-time codes handed to the desktop app at the end of the VATSIM OAuth round trip.
--
-- The OAuth callback cannot hand the token straight to the loopback redirect: it would land in a URL,
-- and from there in browser history and any referer. Instead the callback mints one of these, the app
-- exchanges it for the real token over POST, and it is immediately consumed.
--
-- Hashed at rest like API-key secrets (`repos::access::sha256_hex`) rather than stored plaintext —
-- it is short-lived but it is still a credential.
create table if not exists identity.desktop_auth_codes (
    id text primary key default gen_random_uuid()::text,
    code_hash text not null unique,              -- sha256 of the code; the code itself is never stored
    user_id text not null references identity.users(id) on delete cascade,
    expires_at timestamptz not null,             -- 60s: the app is already waiting on its listener
    consumed_at timestamptz,                     -- set on first exchange; a second attempt is rejected
    created_at timestamptz not null default now()
);

-- Exchange looks a code up by hash and only cares about live, unconsumed rows.
create index if not exists idx_identity_desktop_auth_codes_live
    on identity.desktop_auth_codes(expires_at)
    where consumed_at is null;
