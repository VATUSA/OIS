# API conventions

One versioned REST API under `/api/v1`, served by the backend and consumed by the web
app, the desktop app, and the Discord bot alike. No client gets a private backdoor —
every caller goes through the same authenticated, permission-checked surface.

## Shape

- **Base**: `/api/v1`. Until product 1.0 it is malleable: breaking changes land on `v1` and nothing is
  stable. The freeze, the supported surface and the deprecation rule are policy in `AGENTS.md`
  § Versioning, so they aren't restated here.
- **Resources** are nouns; verbs are HTTP methods. `GET` list/read, `POST` create,
  `PATCH`/`PUT` update, `DELETE` remove. Domain actions that aren't plain CRUD use a
  sub-path (`POST /events/{id}/review`, `POST /ace/requests/{id}/claim`).
- **JSON** in and out.
- **Self** endpoints (`/me`, `/access/self`) act on the caller; `/admin/...` and
  resource paths act on others and carry heavier permissions.

## Auth

Three inbound credential paths, resolved into the request context:

- **Session cookie** (`ois_session`) — human users, issued by VATSIM OAuth login.
- **Bearer token** — two kinds, told apart by prefix, both SHA-256-hashed at rest:
  a user **API key** (`ois_pat_…`, owned by and capped to a person — see
  [api-keys.md](../features/api-keys.md)) or a **service account** (`ois_sa_…`, a
  machine client). An API key's effective authority is re-intersected with its owner's
  live permissions on every request.
- **HMAC signature** — the VATUSA roster-change webhook
  (`POST /api/v1/webhooks/vatusa/{facility}`) carries no session or bearer; it is
  authenticated by verifying an HMAC signature over the request body.

## Authorization

Every protected route declares a `RequirePermission<P>` extractor naming the exact
permission it needs — so a missing check is a visible gap in the handler signature,
not a silent omission. Coarse permission is checked by the extractor; data-dependent
rules (ownership, "request must be open", ARTCC scope) are checked in the handler on
top. See [permissions.md](permissions.md).

## Errors

Uniform envelope, stable machine-readable codes:

```json
{ "error": "unauthorized" }
```

| Status | When |
| --- | --- |
| 400 `bad_request` | malformed input; missing required reason; unknown permission/role |
| 401 `unauthorized` | not authenticated, or lacks the required permission |
| 403 `forbidden` | authenticated but the action is refused (e.g. privilege guard) |
| 404 `not_found` | target doesn't exist |
| 409 `conflict` | concurrent/duplicate change |
| 429 `too_many_requests` | the caller's rate limit is spent — see [Rate limits](#rate-limits) |
| 503 `service_unavailable` | dependency (DB, external API) unavailable |

OAuth has its own precise codes (`oauth_state_mismatch`, etc.) to aid debugging.

## Rate limits

Every `/api/` request is charged to one bucket (`backend/src/rate_limit.rs`), chosen by caller:

| Caller | Keyed on | Default per minute | Env |
| --- | --- | --- | --- |
| API key (`ois_pat_`) / service account (`ois_sa_`) | the credential | 300 | `RATE_LIMIT_CREDENTIAL_PER_MIN` |
| Signed-in user (web cookie or desktop token) | the user | 600 | `RATE_LIMIT_USER_PER_MIN` |
| Unauthenticated | client IP | 120 | `RATE_LIMIT_ANON_PER_MIN` |

A full minute's allowance is available as a burst and refills evenly. Every limited response carries
`RateLimit-Limit`, `RateLimit-Remaining` and `RateLimit-Reset` (seconds until the full allowance is
back); a refused one is `429 too_many_requests` with `Retry-After` (seconds until the next request is
accepted). CORS exposes all four. `/health`, `/metrics` and `/docs` are not limited.

- **Per-credential override (#611).** An admin can set one key's or service account's limit
  (`PUT /api/v1/admin/api-keys/{id}/rate-limit`, `.../service-accounts/{id}/rate-limit`; `null` clears
  it). It lives on the credential row, read by the bearer lookup, so it applies on the next request on
  every replica.
- **Usage (#611).** Each replica counts credential requests and 429s and adds them to
  `access.credential_usage` (hourly, kept 7 days) once a minute; key and service-account lists show the
  sums as `usage`.
- **Per process.** Buckets live in memory, so with N backend replicas a caller can reach N× its limit.
- **Client IP** is read `TRUSTED_PROXY_HOPS` entries from the right of `X-Forwarded-For`, since anything
  further left is client-supplied. The default, 2, matches production (Cloudflare → Traefik); a
  deployment behind one proxy must set 1. The audit log and `api_keys.last_used_ip` use the
  same address.
- Credentials are resolved before the limiter runs, so an unrecognised token is charged to its IP.
  Because a refused request has already been resolved, resolving must stay cheap: a credential's
  `last_used_at` (and an API key's `last_used_ip`) is written **at most once a minute**, so a caller far
  over its limit costs one indexed read per request and no writes. "Last used" is accurate to the
  minute.

## Auditing

Sensitive writes (access changes, and later roster/status/TMI actions) record an
`access.audit_logs` row: actor, action, resource, a **required human reason**, and
before/after snapshots.

## OpenAPI

The backend emits an OpenAPI spec (utoipa) at `/docs/api/v1/openapi.json`. The web +
desktop clients are **generated** from it (`openapi-typescript` → `openapi-fetch`), so
the API contract is the single source of truth for types — no hand-written client
models to drift. The spec is mounted (`router.rs` routes `/docs/api/v1/openapi.json`).

## Realtime

`GET /api/v1/ws` upgrades to a websocket that carries topic nudges only — additive over
REST, never a replacement for it. The upgrade GET is authenticated like any REST route (401 if
unauthenticated): the `ois_session` cookie, or a desktop token, API key or service account as
`Authorization: Bearer` or — for a browser, which can set no other header — the subprotocol list
`ois.v1, ois.bearer.<token>` (`auth/middleware.rs`). A client gets every topic until it sends
`{"subscribe":[…]}`, which replaces its set (acked `{"subscribed":[…]}`; an unknown topic is refused
with `{"error":"unknown_topic"}` and changes nothing). Clients refetch via REST on each nudge; see
[overview.md](overview.md#realtime). Integrator-facing description: `docs-site/reference/api-keys.md`.

## Pagination

List endpoints page with `page` / `page_size` and return the total count, so clients
can render pagers without a second request. Implemented: `GET /api/v1/admin/users`
returns `AdminUserPage { items, total, page, page_size }` (`page_size` default 25, max 100).
