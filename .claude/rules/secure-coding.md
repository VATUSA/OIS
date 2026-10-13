# Secure Coding Rules (always loaded)

The permission model (explicit `segments.action` strings, national or ARTCC-scoped grants, deny beats
allow, `SERVER_ADMIN`), the auth model and the two origin lists are facts in `AGENTS.md`
§ Permissions, § Conventions & gotchas and § Environment variables. This file is the review standard
on top of them.

---

## Access Control
- Every handler that reads or writes anything beyond the caller's own data declares
  `RequirePermission<P>` in its signature. The data-dependent check (ARTCC scope, ownership, record state) sits on top of it, in
  the handler.
- `RequirePermission<P>` is scope-blind by design: no ARTCC exists at the extractor. Holding the
  permission anywhere passes it (401 otherwise); the handler's
  `principal.permission_scope(&state, …).allows(Some(artcc))` decides the facility (403).
  A handler that checks only the extractor lets a ZDC-scoped holder write at ZNY (#626, #636).
- Take the caller as `Actor(principal)`, which admits a user, an API key and a service account and
  caps a key at its owner's live scope. A handler that must refuse machines is listed `UserOnly` with
  its reason in `handlers/actor_ratchet_tests.rs`.
- NEVER trust a path or body id alone. Resolve a child through its parent in the same query
  (`where artcc = $1 and id = $2`), so a foreign id is simply not found.
- Attribution and ownership come from `principal.attribution()`, never from a request field. Request
  structs carry only the fields a caller may set.
- An object the caller may not see answers 404, the same as a missing id, so a 403 does not confirm
  that a hidden record exists (`require_visible_event_fca`, #746).

## Axum handler authorization

Every handler is an endpoint, reachable directly with any credential the router accepts, whatever
page or client normally calls it.

- **The check goes at the top of the handler**, not inside a helper it delegates to, where a reviewer
  has to trace the call to confirm it fires.
- **An id in the path or body is attacker-chosen, and the permission alone will not save it.** The
  actor legitimately holds the permission, so the gate passes and a write to another facility's row
  still lands. Scope first, authorize as well.
- **An edit checks the scope of the stored row AND of the new values.** #636 let a scoped holder
  relabel another facility's FCA as their own, then release on it; the fix checks the stored and the
  new `artcc` on edit.
- **A guard has two halves.** That the actor holds the capability at all, and that they hold it at
  this record's scope. When a check ANDs a role or membership with a scope, give each half a fixture
  where the other half passes (#577; `test-writing.md` → "Security guards").
- **Widening what a query can see also widens every write that resolves through it.** When you relax
  a lookup, list every handler that resolves a record through it and re-apply the constraint at each
  write.
- **Check what wraps a guard before trusting its status code.** A 401 from the extractor and a 403
  from the scope check are different contracts; assert the status the route documents, plus the
  untouched row.
- **Fix the siblings in the same pass.** A set of routes is written together and shares the omission:
  #746's order, release and swap skipped the guard the edit routes had.
- **A handler's test goes through the real router.** Calling the handler function skips the
  extractors that are the gate. Use `scope_test_support::send` / `send_json`
  (`test-writing.md` → "A handler call cannot prove an authorization gate").

## A check at upgrade or sign-in is a mount gate, and a live credential outlives the role

OIS resolves permissions per request: `ensure_permission` reads the caller's effective permissions
from the database on every call, so a revoked grant stops the next REST request. Anything checked
only once, at a moment, is not covered by that.

- **The websocket is authorized at the upgrade only.** `realtime::ws` refuses an anonymous caller,
  then streams for as long as the socket lives, past a revoked session or a demotion. That is safe
  only because a nudge carries nothing but a topic name. Never send data over the socket; a payload
  would reach someone who lost access after connecting.
- **A role reconciled only at sign-in outlives its removal** for every live session and API key,
  which never sign in again. #805: a server admin removed from `OIS_SERVER_ADMIN_CID` kept the role
  until their next interactive sign-in. The fix demotes at startup and every 5 minutes
  (`server_admin_reconcile`), as well as at sign-in.
- **Work done later on a principal's behalf rebuilds the principal at run time.** The event-package
  lifecycle job acts as the stored actor through `Principal::from_actor`, which applies the bearer
  lookup's liveness rules, and re-checks scope then (#607). A revoked key or a disabled service
  account resolves to `None` and nothing acts as it. Never store a permission snapshot to act on
  later.

## A rule that preserves access must check the access still exists

"Keep what the member already holds" is half of a safe rule. A preservation clause that compares only
stored fields can re-grant what a deliberate revocation or deny left behind.

- Deny state lives in rows with `granted = false`. #559 found the access editor's scoped save deleted
  every row in a scope, denies included, and re-inserted only grants, so saving a user's ZDC scope
  unchanged widened their access. Replace grants only.
- A restriction checked only when a grant is written is bypassed by any older or widened path to the
  same permission. #584: the service-account denylist (`api_keys.*`, `service_accounts.*`) is
  enforced at resolution too, so a forbidden permission reached through a role is never held.
- Test existing rows in each non-active state (revoked, expired, denied, demoted) through every path
  that writes, the job as well as the handler. A test that seeds only the default row proves the rule
  for active rows and says nothing about the rows the clause is dangerous for.

## Route-group membership is not authorization

Nesting a route in `router.rs` under `/api/v1/admin/…` is not a gate. A handler with no auth
extractor is public, and `handlers/auth_annotation_tests.rs` requires every such handler to be in
its `PUBLIC` list with a reason, and every handler's OpenAPI annotation to declare exactly the
credentials its extractors accept (#586, #587). A public route that reads an optional identity
(`Option<Actor>`) is still public: decide what an anonymous caller sees and test it.

## Injection Prevention
- sqlx with `.bind(...)` for every value. A `format!` into SQL only for a trusted code constant, with
  a comment saying so (`repos::audit::resolve_principal_actor_id` interpolates an internal column
  name and binds the values). `QueryBuilder::push_bind` for dynamic lists.
- No `std::process::Command` built from input. Pass arguments as a list, never through a shell.
- Validate input at the boundary: a request struct, then explicit checks that return 400. Allowlist
  enums and statuses against the values the newest migration permits (`database-postgres.md`).
- A URL from a caller is parsed and its scheme allowlisted (`http`/`https`), as
  `handlers/facility_documents.rs` does, before it is stored or sent anywhere.
- **Discord message injection.** Text OIS posts to Discord is parsed for mentions and markdown.
  Outbound posts set `allowed_mentions` explicitly (`discord/src/jobs/{tmi,advisory}.rs` set none, so
  an `@everyone` in a remark pings nobody), and user text inside a code block cannot close it
  (`flatten` replaces backticks and newlines, #436). A new
  job kind that posts caller-supplied text fixes both, and tests the built message.

## Data-protection obligations (flag, don't implement)

Personal data (VATSIM CIDs, names, emails, ratings, Discord ids, IP addresses, session and
diagnostics data) is held under VATSIM's data protection and handling policy and the privacy page
(`web/src/pages/privacy.tsx`). Point-of-edit reminders: `.claude/rules/data-protection.md`
(path-scoped). Flag misalignment; do not build compliance machinery off the back of a review.

- **Fixtures and test data** → never production rows; seed with SQL in the test (`seed_user`).
- **Issues and PRs** carry ids and counts, never personal data (a CID, a name, an email, an IP).
- **Personal data leaving for a third party** (Discord, VATUSA) → the privacy page must still
  describe it.
- **A new personal-data column or table** → `data-protection.md` → "The four tells".

Fix a misalignment this branch introduces on the branch. A pre-existing one is a follow-up only if it
passes the tests in `github-issues.md` → "Scope, and when a follow-up issue is allowed".

## Output Encoding
- React escapes text. `dangerouslySetInnerHTML` needs a justification in review and an escape at
  every interpolation; the one site, `MapTooltip` in `web/src/components/map/MapCanvas.tsx`, renders
  HTML built by `web/src/components/map/lib/tooltip.ts`, which escapes every value through `esc()`.
- Errors reach clients only as the stable envelope (`ApiError` codes), never a sqlx message.

## Authentication & API Security
- VATSIM Connect is the only human sign-in. `return_to` is checked against the configured origin
  allowlist (`validate_return_to`); the desktop loopback `http://127.0.0.1:8765` belongs in
  `OAUTH_RETURN_TO_ORIGINS`, never `CORS_ALLOWED_ORIGINS` (#346).
- The session cookie is `HttpOnly`, `SameSite=Lax`, and `Secure` when `COOKIE_SECURE` is set; credentialed CORS is
  limited to `CORS_ALLOWED_ORIGINS`. There is no CSRF token, so those two settings are the CSRF
  defense: never loosen either, and never accept a state-changing `GET`.
- API keys and service-account secrets are stored as SHA-256 (`secret_hash`) and looked up by hash.
  Session tokens (`identity.sessions.session_token`) are stored as issued, so a database read is a
  session read: never log, return or copy them.
- An API key is capped at its owner's live access (`fetch_api_key_access`). Never resolve a key's
  authority from the key's grants alone.
- Every `/api/` request draws on the per-caller rate-limit bucket (`rate_limit::enforce`, #588). The
  buckets are per replica. A route outside `/api/` is not limited; say why in the PR.
- **A webhook or shared-secret check rejects an empty or missing configured secret.** The VATUSA
  webhook answers 503 with no `OIS_SECRET_KEY` and 404 with no stored registration, and compares the
  HMAC with `verify_slice` (constant time). `/metrics` is the one documented exception: open when
  `METRICS_TOKEN` is unset, for in-network scraping (`metrics::scrape_authorized`).
- Compare secrets and MACs in constant time, never with `==`.
- Credentials live in the environment only. `.env.example` carries dev-safe placeholders, never a real
  value; `.env` and `web/.env.local` are never committed. Stored third-party secrets are encrypted
  with `backend/src/secrets.rs` under `OIS_SECRET_KEY`; without the key the feature is off, not
  plaintext.
- A credential shown once stays shown once: nothing persists it client-side, pinned by a source-scan
  guard (#531; `test-writing.md` → "Absence needs a guard, not a grep").
- Rotate a compromised credential immediately.

### Desktop
- Grant the narrowest capability set. Pop-out windows get `desktop/src-tauri/capabilities/popout.json`,
  narrower than `default.json`; a new permission goes in the narrowest file that needs it.
- The CSP in `desktop/src-tauri/tauri.conf.json` is security config. Justify every origin added to
  `connect-src`, `img-src` or `script-src`.
- The desktop session token is stored only where `desktop/src-tauri/src/auth.rs` stores it, never in
  web storage, a URL or a log.

## Logging
- DO log: auth failures, authorization refusals worth investigating, admin and access changes (the
  audit log records mutations; access changes carry a human reason), webhook processing, skipped side
  effects.
- NEVER log: session tokens, API keys, service-account secrets, `OIS_SECRET_KEY`, webhook secrets,
  OAuth codes, `DISCORD_BOT_TOKEN`, `VATUSA_API_KEY`. That includes `tracing` fields. Log the
  principal id instead.
- **Log an error's `Display`, not its `Debug`, where the statement or URL holds personal data or a
  secret.** A Postgres error's `Debug` (`?e`) includes the `detail` field, which carries the
  conflicting key's values (an email on a unique violation). reqwest's `Display` appends the request
  URL, query and all: VATUSA v2 takes the key as `?apikey=`, so `fetch_member` strips it with
  `reqwest::Error::without_url` (#757).
- **Free text bound for a third party is dropped, not scrubbed.** Send the structured part (type,
  code, an id you control) and replace prose with a fixed placeholder.
- Structured context always: `tracing::warn!(user_id = %id, "…")`.

## Dependencies
- `cargo deny check` (`deny.toml`) and `pnpm audit --audit-level=high` run in CI and in the full local
  gate (`just test`); a high advisory blocks.
- A new dependency needs a reason in the PR; the `dependency-researcher` agent evaluates it.
- A high advisory against an unchanged lockfile is repo-wide: file it once instead of fixing it on an
  unrelated branch.
- Patch SLA: Critical 48 hours, High 1 week, Medium next release.

## PR Security Checklist
Before every PR, verify:
1. Every new non-public handler takes `RequirePermission<P>`, and every write checks the record's
   ARTCC scope (stored and new) on top
2. Every child record is resolved through its parent; a hidden record answers 404
3. No `format!` into SQL except a commented trusted constant
4. A new permission is in all three places (`AGENTS.md` § Permissions)
5. A new public handler is in `auth_annotation_tests::PUBLIC` with a reason; a new user-only handler
   is in `actor_ratchet_tests` with a reason
6. No secret, token or personal data in logs, responses, fixtures, issues or `.env.example`
7. Secrets compared in constant time; API keys and service-account secrets stored hashed
8. Work done later for a principal rebuilds that principal and re-checks scope at run time
9. Nothing sent over the websocket but a topic name
10. Text posted to Discord sets `allowed_mentions` and cannot break out of its formatting
11. Desktop: capabilities narrowest, CSP changes justified
12. Personal data stays under its controls (`data-protection.md`)
