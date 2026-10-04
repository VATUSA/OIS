-- @formatter:off
-- VATUSA sync moves from per-member v2 fetches to one v3 division pull a day (VATUSA/OIS#605).

-- 1. Who has actually signed in.
--
-- The pull seeds every controller in the division, most of whom will never sign in. The admin user
-- browser listed any user with a CID, so without a marker it would go from dozens of real users to the
-- whole division. Sign-in sets this; a seeded row has it null.
--
-- Backfilled for every existing row: until now sign-in was the only way a user row was created, so
-- each one has signed in at least once. Their latest session is the best date we have.
alter table identity.users
    add column if not exists last_login_at timestamptz;

update identity.users u
set last_login_at = coalesce(
        (select max(s.created_at) from identity.sessions s where s.user_id = u.id),
        u.created_at)
where u.last_login_at is null;

-- 2. One division webhook, its secret encrypted.
--
-- v3 scopes a webhook to the calling key — the division — so the 22 per-facility registrations
-- collapse to one. Its secret cannot be hashed like every other credential in the schema
-- (0045_api_keys.sql), because verifying a delivery's HMAC needs the key itself; so it is encrypted
-- with OIS_SECRET_KEY (backend/src/secrets.rs) and stored as nonce ‖ ciphertext.
--
-- The old table held those secrets in plaintext — the only plaintext credential column in the
-- schema — and is dropped rather than migrated: registration deletes the old webhooks on VATUSA's side
-- and creates the new one, so nothing in it is needed. A singleton row, enforced by the constant key.
create table if not exists identity.vatusa_webhook (
    singleton boolean primary key default true check (singleton),
    vatusa_id bigint,
    url text not null,
    secret_ciphertext bytea not null,
    key_version int not null,
    created_at timestamptz not null default now()
);

drop table if exists identity.vatusa_webhooks;

-- 3. v3 marks a division-wide role with facility '*'; v2 used 'ZHQ'. OIS stores one canonical
-- marker, ZHQ, so the role mapping's national case (0100) and its editor's "ZHQ (division)" choice
-- work for both sources. Any '*' already stored is converted (none is expected — v2 never sent it).
update identity.vatusa_roles r
set facility = 'ZHQ'
where facility = '*'
  and not exists (
      select 1 from identity.vatusa_roles z
      where z.cid = r.cid and z.role = r.role and z.facility = 'ZHQ');
delete from identity.vatusa_roles where facility = '*';

-- 4. The unique keys on both tables already lead on cid, so these were redundant — and the daily
-- division diff writes to both tables, where every extra index is extra work.
drop index if exists identity.idx_vatusa_roles_cid;
drop index if exists identity.idx_vatusa_visits_cid;
