# Permissions

OIS uses osmium's explicit, path-based permission model, extended with a per-ARTCC scope for national operation. Nothing
is implied by role name — every capability is a granted permission.

## The permission string

`segments.action`, e.g. `events.items.create`, `tmu.tmi.publish`, `ace.requests.claim`.

- **Segments** — one or more lowercase snake identifiers naming the resource path.
- **Action** — a terminal verb from a fixed set:
  `read, create, update, delete, publish, assign, decide, request, approve, deny, claim`.

In the JSON tree a segment is *either* a parent (more segments) *or* a leaf (an action array) — never both. A 3-segment
permission nested under a same-prefix 2-segment one would collide, so use a distinct sibling segment (osmium's
`feedback.items_self`
pattern, migration 0048). The pure logic lives in
[`crates/ois-core/src/permissions.rs`](../../crates/ois-core/src/permissions.rs).

## Storage (`access` schema)

Ported from osmium, plus the `artcc_id` scope column:

- `roles`, `permissions` — the catalogs.
- `role_permissions` — role → permission.
- `user_roles (user_id, role_name, artcc_id?, source)` — role grant, optionally scoped to an ARTCC.
- `user_permissions (user_id, permission_name, granted, artcc_id?, source)` — direct grant,
  `granted=false` is an explicit **deny** that beats any allow.
- `service_accounts` + `service_account_credentials` + `service_account_roles` — machine clients (the bot). Already
  carry a `scope_type`/`scope_key` dimension.
- `actors`, `audit_logs` — who did what; the "Recorded as a dossier entry" line in the access editor writes here with
  the required `reason`.

### Provenance (`source`)

Both grant tables carry `source` — `manual`, `vatusa`, or `system` — naming **who created the row and
therefore whose row it is to remove**. Without it a VATUSA sync had only two options, both wrong:
write authoritatively and silently undo every hand-made grant on each run, or write additively and
never clean up after a demotion.

| `source` | Written by |
|---|---|
| `manual` | an admin in the access editor; a group membership set by hand |
| `vatusa` | the VATUSA sync, which may reconcile it away |
| `system` | OIS itself — the `SERVER_ADMIN` env reconciliation, and the `USER` baseline group |

Two rules make it work:

- **Every writer declares it.** The column is `not null` with **no default** (the default in
  `0098` exists only to backfill existing rows as `manual`, and the migration then drops it). A writer
  that forgets fails the insert instead of silently claiming to be a human grant.
- **Each owner deletes only its own rows.** `replace_user_permissions_scoped` — the admin editor's
  save — deletes the scope's `manual` rows and rewrites them; `set_user_role_scoped` takes the source
  on both the grant and the revoke side. So a sync reconciles without touching anything set by hand.

The accepted consequence: **an admin cannot un-grant a synced role from the editor.** If VATUSA says
someone is an EC, the editor does not get to silently disagree until the next sync puts it back —
removing it means detaching the user from sync. The alternative is the editor and the sync fighting,
last writer winning, with no way to tell which rows were whose.

One consequence to be aware of, and the reason provenance needs to reach the UI: the editor shows
*effective* state, so it cannot yet tell a synced grant from a hand-made one. An admin who opens the
editor and saves an unchanged form writes a `manual` row beside the existing `vatusa` one — pinning
that grant, so a later demotion no longer removes the access. This is strictly better than the old
behaviour (which deleted the synced row outright), but it means **surfacing `source` in the access
editor is a prerequisite for trusting sync-driven revocation**, not a cosmetic follow-up.

Uniqueness is keyed on `(user, name, scope, source)`, so a manual and a synced grant of the same thing
coexist as separate rows and a demotion removes only one of them. The two grant readers
(`fetch_user_role_grants`, `fetch_user_direct_grants`) therefore `select distinct`, so the editor still
lists such an entry once.

**No reader consults `source`.** The effective-permissions view and `fetch_effective_permissions`
ignore it entirely, so a grant's authority never depends on who created it. Provenance answers only
"whose row is this to remove?".

Time bounds (`starts_at`/`ends_at`) were considered and **deferred**: nothing yet needs time-bounded
membership, and adding the column would make every reader filter on it for a value no writer sets.
`service_account_roles` has them because machine credentials expire; a human's role does not, yet.

### Effective permissions

**One resolver**, `repos::access::fetch_effective_permissions`. It reads
`access.v_effective_user_permissions` — which emits one `(permission, artcc_id, granted)` row per
grant or deny, carrying the scope — and composes them into a `PermissionScope` per permission.
`permission_scope()` and `fetch_user_permission_names()` are both thin views onto it and hold no SQL
of their own.

There used to be two implementations, and they disagreed (#543): the view honoured denies but dropped
`artcc_id` on every arm, so **a deny scoped to one ARTCC revoked the permission nationally**, while
`permission_scope()` honoured scope and never read `granted = false` at all. Each was blind to
precisely what the other saw.

**The deny rule.** A deny removes the permission **at its own scope**; a national deny
(`artcc_id is null`) removes it **everywhere**, even where a scoped allow exists. So a national allow
with a deny at ZDC is "everywhere except ZDC" — which is why `PermissionScope::National` carries an
`except` set rather than being a bare marker. This is what the deny bullet above has always promised:
an explicit deny beats any allow.

**A resource with no resolvable owning ARTCC** (`allows(None)`) is permitted only at *unrestricted*
national scope. A holder carrying any scoped deny fails closed there, because there is no ARTCC to
test the deny against.

### The two-stage gate — a contract, not an accident

`RequirePermission<P>` is **deliberately scope-blind** (#543). It runs in an axum extractor, where the
ARTCC does not exist yet: it arrives as a path segment, a body field, or a live `owning_artcc` lookup.
So the gate answers one question — *do you hold this permission anywhere?* — and rejects with **401**.
The ARTCC is then the handler's to enforce, with `scope.allows(Some(artcc))`, rejecting with **403**.

That 401/403 split is load-bearing: it is how a test can tell which gate answered.

The exposure this leaves is worth stating plainly. Only the handlers that opt in enforce scope —
`flow.facility_map.update`, `events.config.update`, `events.rate.update`, `events.support.update`,
`facilities.docs.update`, `flow.surface_data.update`, `flow.route.update` / `.delete`,
`tmu.adv.update` / `.publish`, the flight-exclusion permission, and `events.capture.*`. **For every
other permission a facility-scoped grant still behaves as national**, because nothing downstream of
the coarse gate narrows it. Closing that is per-domain work, not a resolver change.

## Enforcement

`RequirePermission<P>` is an Axum extractor keyed by a marker type (one per permission, generated by the `permission!`
macro). Declaring it in a handler's argument list is the *only* way to satisfy it, so a missing check is a visible gap
in the signature rather than a silent omission in the body. Data-dependent checks (owner-or-approver, self-vs- other,
ARTCC scope) are still done explicitly in the handler in addition to the extractor.

The extractor and the handler scope check both resolve the request's **principal** — a session user, or a
user-owned **API key**. A key's effective permissions are re-computed on every request as its granted subset
**intersected with its owner's current access** (permissions and ARTCC scope), so a key can never exceed the
person who owns it, and the `api_keys` domain is denylisted on keys entirely. See
[api-keys.md](../features/api-keys.md).

## Catalog

Top-level domains (the collapsible groups in the access editor):

```
access · ace · api_keys · audit · auth · discord · emails · events · facilities ·
feedback · files · flow · integrations · org · pages · publications · stats ·
system · system_rate_limit · tmu · training · users · web
```

Shared domains (access/auth/users/events/training/…) are ported from osmium in Phase 0. The OIS-new domains (`tmu`,
`ace`, `flow`, `discord`, `facilities`) are drafted in
[`crates/ois-core/src/catalog.rs`](../../crates/ois-core/src/catalog.rs) and firmed up in each feature spec.

## Roles

The assignable positional roles are `VATUSA_STAFF`, `EVENTS_TEAM`, `EC`, `AEC`, `ACE`,
`NTMO`, and `DCC_STAFF`. Any of them can be granted nationally or **scoped to an ARTCC** via the
grant's `artcc_id` (e.g. an `EC` scoped to ZDC vs. a national `EC`) — scope is on the
grant, not baked into the role. `SERVER_ADMIN` is a bootstrapped singleton (env CID)
that holds every permission implicitly and is never assignable in the UI. `USER` is the
baseline role; machine actors are `BOT` and `SERVICE_APP`.
