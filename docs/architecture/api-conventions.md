# API conventions

One versioned REST API under `/api/v1`, served by the backend and consumed by the web
app, the desktop app, and the Discord bot alike. No client gets a private backdoor —
every caller goes through the same authenticated, permission-checked surface.

## Shape

- **Base**: `/api/v1`. Breaking changes get a new version prefix.
- **Resources** are nouns; verbs are HTTP methods. `GET` list/read, `POST` create,
  `PATCH`/`PUT` update, `DELETE` remove. Domain actions that aren't plain CRUD use a
  sub-path (`POST /events/{id}/review`, `POST /ace/requests/{id}/claim`).
- **JSON** in and out.
- **Self** endpoints (`/me`, `/access/self`) act on the caller; `/admin/...` and
  resource paths act on others and carry heavier permissions.

## Auth

Two credential types, resolved by one middleware into the request context:

- **Session cookie** (`ois_session`) — human users, issued by VATSIM OAuth login.
- **Bearer token** — service accounts (the bot, tooling); hashed at rest.

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
| 503 `service_unavailable` | dependency (DB, external API) unavailable |

OAuth has its own precise codes (`oauth_state_mismatch`, etc.) to aid debugging.

## Auditing

Sensitive writes (access changes, and later roster/status/TMI actions) record an
`access.audit_logs` row: actor, action, resource, a **required human reason**, and
before/after snapshots.

## OpenAPI

The backend emits an OpenAPI spec (utoipa) at `/docs/api/v1/openapi.json`. The web +
desktop clients are **generated** from it (`openapi-typescript` → `openapi-fetch`), so
the API contract is the single source of truth for types — no hand-written client
models to drift. *(Spec mount is a Phase 0 remainder.)*

## Pagination

List endpoints page with `page` / `page_size` and return the total count, so clients
can render pagers without a second request. *(Standardized as list endpoints land.)*
