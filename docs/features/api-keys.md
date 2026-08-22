# API keys (user-owned personal access tokens)

Lets developers integrate with the OIS backend from their own front end or tooling, authenticating
with a per-user token instead of a browser session. The defining property: **a key is a capability
strictly bounded by its owner's live authority** — it can never do anything the owner can't do right
now, and everything it does is attributable and audited.

## Data model (`access` schema, migration 0045)

- `access.api_keys` — one row per key: `owner_user_id` (→ `identity.users`, cascade), `name`,
  `description`, `prefix` (public, shown in listings, e.g. `ois_pat_a1b2c3`), `secret_hash`
  (SHA-256 of the full token, unique), `status` (`active`/`disabled`), `expires_at?`, `last_used_at`,
  `last_used_ip`, `revoked_at`. Single secret per key; rotation overwrites the hash in place.
- `access.api_key_permissions` — the granted `(permission_name, artcc_id?)` subset (grant-only;
  `artcc_id` NULL = national). Mirrors `access.user_permissions` so the same subsetting semantics apply.
- `access.actors` gains an `api_key` actor type + `api_key_id`, so an audited mutation made via a key
  is attributed to the key.

Permissions catalog (`api_keys.key.*`): **`create`** gates the self-service surface (manage your own
keys); **`read`** / **`delete`** are oversight over *any* user's keys. SERVER_ADMIN holds all three
implicitly via `v_effective_user_permissions`.

## Token & bearer resolution

- Format `ois_pat_<32 hex>`; hashed with the shared `sha256_hex` (high-entropy random, so an unsalted
  hash is appropriate). Plaintext is returned **once**, on create/rotate, in a `*TokenBody` — never
  stored or re-derivable.
- `resolve_current_user` dispatches an `Authorization: Bearer …` token by prefix: `ois_pat_` →
  `find_current_api_key_by_bearer_token` (verifies active/unrevoked/unexpired *and owner active*, updates
  `last_used_at`/`last_used_ip`), `ois_sa_` → the service-account resolver. The resolved key is placed in
  request extensions as `Option<CurrentApiKey>`.

## The capping engine (the security core)

A key's effective authority is computed per request as **`key grants ∩ owner's current access`**:

- **Coarse gate** (`RequirePermission<P>` → `ensure_permission`): a third branch calls
  `acl::fetch_api_key_access`, which keeps a granted permission only if (a) the owner effectively holds
  it (the effective view honors explicit **denies**), (b) it isn't denylisted, and (c) the intersection
  of the owner's scope and the key's granted scope for it is non-empty.
- **Fine (ARTCC) gate**: a `Principal` abstraction (`auth::principal`) unifies a session user and an API
  key; `principal_permission_scope` returns, for a key, `owner_scope.intersect(key_scope)` using the
  `PermissionScope` lattice (`National ∩ Facilities(S) = Facilities(S)`, `Facilities(A) ∩ Facilities(B)
  = A∩B`). The scope-enforcing handlers (`facility_map`, `airport_configs`, `events`) were made
  principal-aware so a key's per-ARTCC scope is enforced, always capped by its owner.

Consequence: demoting, rescoping, or deactivating the owner **immediately** narrows every key they own.

**Subset validation** at create/edit (`api_keys_repo::validate_subset`) rejects, up front, any grant the
owner doesn't hold (permission and scope), and any **denylisted** permission. The denylist
(`API_KEY_FORBIDDEN_DOMAINS = ["api_keys"]`) means a key can never manage keys — a leaked key can't mint
more or escalate. The request-time cap enforces the same again as defense in depth.

## Auditing

- The `audit_mutations` middleware attributes non-user principals (this also closed a pre-existing gap
  where **service-account** mutations were logged with no actor). A key's actions attribute to the key
  (`name (prefix)`).
- Each lifecycle change (create/rotate/permission-edit/disable/delete) writes a rich `record_audit`
  entry: `resource_type = API_KEY`, before/after permission snapshots, optional reason, client IP. These
  routes are excluded from the generic middleware to avoid duplicate entries.
- `GET /admin/audit` filters by `actor_id` and `resource_id`; a per-key **dossier** is exposed at
  `GET /api/v1/api-keys/{id}/audit` (owner, or an `api_keys.key.read` holder).

## API surface

Self-service (owner-scoped, gated `api_keys.key.create` unless noted):

| Method / Path | Purpose |
| --- | --- |
| `GET /api/v1/api-keys` | list your keys |
| `GET /api/v1/api-keys/grantable-permissions` | your delegatable permissions + scope (drives the picker) |
| `POST /api/v1/api-keys` | create (subset-validated; token shown once) |
| `GET /api/v1/api-keys/{id}` | your key detail |
| `POST /api/v1/api-keys/{id}/rotate` | new secret (shown once) |
| `PUT /api/v1/api-keys/{id}/permissions` | replace grants (re-validated) |
| `POST /api/v1/api-keys/{id}/disable` · `DELETE /api/v1/api-keys/{id}` | disable / delete |
| `GET /api/v1/api-keys/{id}/audit` | activity dossier (owner or oversight; no `create` gate) |

Admin oversight: `GET /api/v1/admin/api-keys?owner_cid=` (`api_keys.key.read`),
`POST /api/v1/admin/api-keys/{id}/disable` · `DELETE /api/v1/admin/api-keys/{id}` (`api_keys.key.delete`).

Key creation/management is restricted to **session users** — keys are user-owned, and a key can never
hold `api_keys.*`, so a key can't reach these endpoints.

## Frontend

- Self-service page at `/api-keys` (linked from the user menu on `api_keys.key.create`): create with a
  domain-grouped permission picker bounded by `grantable-permissions` + per-permission ARTCC scoping,
  optional expiry, one-time token reveal with copy, and per-key rotate/edit/disable/delete/activity.
- Admin page at `/admin/api-keys` (sidebar, `api_keys.key.read`): all keys by owner, filter by CID,
  revoke; `api_keys.key.read` is in the admin-portal permission set.

## Key files

`backend/migrations/0045_api_keys.sql`; `backend/src/repos/api_keys.rs`; `backend/src/auth/{principal,acl,
middleware,context,permissions}.rs`; `backend/src/handlers/api_keys.rs`; `backend/src/audit.rs` +
`backend/src/repos/audit.rs`; `web/src/lib/api-keys.ts`, `web/src/components/api-keys/permission-picker.tsx`,
`web/src/pages/api-keys.tsx`, `web/src/pages/admin/api-keys.tsx`.
