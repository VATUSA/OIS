# Integrations

OIS talks to four external systems. Each integration has a defined trust boundary and
a single place in the codebase that owns it.

## VATSIM (identity)

VATSIM Connect (OAuth2) is the **only** way users authenticate — there are no local
passwords. Login redirects to VATSIM, the callback exchanges the code for a token,
fetches the user's profile (CID, name, email, rating), and issues an OIS session.

- Owned by `backend/src/auth/vatsim.rs` + `handlers/auth.rs`.
- Dev vs. prod hosts (`auth-dev.vatsim.net` / `auth.vatsim.net`) resolve from
  `VATSIM_DEV_MODE`; Basic vs. POST client auth is configurable.
- CSRF-protected via a signed state cookie; `return_to` redirects are validated
  against the CORS allowlist (no open redirect).
- The VATSIM **CID is the stable identity key** across every other system.

## VATUSA (roster & facilities)

VATUSA is the authority for who is on which ARTCC's roster and at what rating. OIS
mirrors that data rather than owning it.

- A background sync job reconciles the roster/facilities from the VATUSA API
  (`org` domain), backed by migration `0033_vatusa_sync.sql`. *(Implemented.)*
- The inbound roster-change webhook `POST /api/v1/webhooks/vatusa/{facility}` is live;
  it carries no session or bearer and is authenticated by **HMAC signature verification**
  over the request body.
- Needs `VATUSA_API_BASE` + `VATUSA_API_KEY`.
- ARTCC records from VATUSA are what the per-ARTCC permission scope (`artcc_id`)
  points at.

## Discord (bot)

The Discord bot is a **separate Rust binary that owns no data**. Backend and bot are
decoupled through a durable job queue plus an authenticated callback:

1. A backend action (event published, TMI issued, ACE request opened) inserts a row
   into `integration.outbound_jobs` (`job_type`, JSON `payload`, `status`,
   `attempt_count`, `next_attempt_at`).
2. The bot polls the backend for pending jobs, performs the Discord side effect
   (create thread + ping, post embed), and acks success/failure.
3. When a user interacts with a bot message (e.g. claims an ACE request), the bot
   calls the backend API **as a service account** (bearer token); the backend applies
   the change and may enqueue follow-up jobs.

This keeps all business logic in the backend and makes the bot restartable and
stateless. Guild/channel/role configuration lives in `integration` tables, edited via
`discord.config.{read,update}`. Full detail in
[../features/discord-integration.md](../features/discord-integration.md).

## Email (transactional)

Transactional email (notifications, verifications) is sent from the backend via a
provider (e.g. SES), with per-user, per-category preferences. *(Planned — attaches
when the first notifying feature lands.)*

## Service accounts

Machine clients (the bot, external tooling) authenticate with a **hashed bearer
token** (`access.service_accounts` + `service_account_credentials`). A service account
holds roles/permissions like a user and is subject to the same `RequirePermission<P>`
checks, so the bot's callbacks are authorized by the same access model as everything
else. Management endpoints are implemented:
`GET`/`POST /api/v1/admin/service-accounts` and
`.../{id}/rotate` | `.../{id}/disable` | `.../{id}/roles`.

**What a machine can write.** A handler a machine may drive takes the `Actor` extractor
(`backend/src/auth/principal.rs`), which admits a session user, a user's API key, or a service account.
Its write records `principal.attribution()`: the legacy `*_by` user column for a person, and a sibling
`*_by_actor` column (a foreign key to `access.actors`) for **every** caller — so a machine's row names
the machine, never a person and never nobody (an API key's owner stays reachable through its actor).
Reads coalesce the two names. Scope is unchanged: a key is capped by its owner's live scope, and a
service account is scoped by its roles' ARTCCs.

Every handler is machine-capable except the ones listed `UserOnly("why")` in
`backend/src/handlers/actor_ratchet_tests.rs` — things about a person by nature: their own access, API
keys and dashboards; managing access groups or another person's access (a machine there could grant
access); and ACE requests, whose requester is a person (#607). The ratchet fails if a new handler takes
`CurrentUser` without being listed.

**Acting later on someone's behalf.** The event-package lifecycle job publishes a package when its event
nears, and archives it when the event ends, as whoever armed or activated it — the package's
`updated_by_actor`, rebuilt with `Principal::from_actor`. It is held to that principal's scope at fire
time (a key at its owner's scope intersected with its own grant), and a credential that has since been
revoked or disabled publishes nothing.