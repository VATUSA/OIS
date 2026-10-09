# VATUSA sync and role mapping

> **Status: built (#548, #605, 2026-10).** A daily v3 pull of the whole division, the role → group
> mapping, and its editor. Two **default mappings** ship (#699, below); everything else is mapped by an
> admin.

## What is synced

| Path | API | When | What |
|---|---|---|---|
| `vatusa_division_pull` job | v3 `GET /v3/division/controllers` | daily; on demand from Background Tasks; when a webhook delivery says the roster changed | **every controller** and every role grant |
| Sign-in | v2 `GET /v2/user/{cid}` | every login, **awaited** before the session is issued (5 s budget) | that one person |

**The pull** (`feed::vatusa::apply_division`, #605) seeds or refreshes every controller in
`identity.users`, and **diffs** their roles and visits rather than rewriting them — 500 controllers per
transaction, a handful of bulk statements each. 10,000 controllers with 1,500 role grants took **2.0 s**
locally the first time and **0.2 s** on a rerun. Members whose roles changed are re-reconciled through
the role mapping, below. Controllers no longer in the division lose their stored roles, and so the
access mapped from them.

⚠️ **A truncated response is refused, not applied.** Absence from the pull strips roles, so an empty
response, or one under half the members already synced, fails the run on Background Tasks and writes
nothing.

**Sign-in's v2 fetch is the one remaining v2 call, deliberately.** v3 carries no `discord_id`, which
the bot resolves DMs through, and sign-in makes a brand-new member correct before the next daily pull.
The pull therefore **never touches Discord mappings** — a missing field is not a cleared link. A source
test fails if a second v2 call appears.

**Seeded users** — controllers who have never signed in — have no audit actor and `last_login_at` null.
The admin user browser and search show only people who have signed in, **except on an exact CID
match**, so an admin can still grant access before someone's first sign-in. Sign-in owns names and
email (from VATSIM Connect); the pull never overwrites them.

v3 marks a division-wide role with facility `*`; OIS stores it as **`ZHQ`**, the marker v2 used, so the
national mapping below works whichever API a role came from. Codes are trimmed and uppercased.

Everything no-ops when `VATUSA_API_KEY` is unset.

## The division webhook

v3 scopes a webhook to the calling key, so the division has **one** (it replaced 22 per-facility
registrations, deleted on VATUSA's side by the first registration). VATUSA returns its secret **once**,
and verifying a delivery's HMAC needs the secret itself, so it can't be hashed like every other
credential: it is stored **encrypted** with XChaCha20-Poly1305 under `OIS_SECRET_KEY`
(`backend/src/secrets.rs`), in `identity.vatusa_webhook`.

The receiver, `POST /api/v1/webhooks/vatusa`, verifies `X-Mithril-Signature` in constant time and, on a
`roster_change`, **triggers the division pull** — one request for everyone, with a burst of deliveries
coalesced into a single run. Registration runs at startup and is re-checked after every pull: a webhook
missing from VATUSA's list, written under another key, or unreadable is deleted and registered again.
There is no re-encryption path, by design — the webhook is disposable.

Without `OIS_PUBLIC_URL` or `OIS_SECRET_KEY` there is simply no webhook; the daily pull keeps everyone
current regardless.

Every check logs registration positively (`… registered` / `… already registered`), and a failed VATUSA
call logs VATUSA's response body. `docs/deploy.md` has the post-deploy check (#688).

### Replay protection

The HMAC proves who signed a body, not when, so a captured delivery would verify forever (#627). The
preferred fix binds the signature to a moment: reject a signed timestamp outside a window. v3 offers
nothing to bind to, though. Its spec documents no delivery schema and no signed timestamp header, and
VATUSA has not been asked to add one. So the receiver **dedupes** instead: it remembers the SHA-256 of
every *verified* body (`ReplayGuard` in `backend/src/handlers/webhooks.rs`) and acknowledges a repeat
with `200` without acting on it. A body is forgotten once 10 minutes pass without it being seen again.

The set is in memory and per process. A restart forgets it, and each replica guards alone. An identical
body that VATUSA itself re-sends inside the window is skipped too. All three are acceptable while a
delivery only brings the idempotent, coalesced pull forward. If the receiver ever starts acting on a
payload's contents, revisit this with VATUSA (a signed timestamp) or move the set to Postgres.

## Role → group mapping

`access.vatusa_role_mappings` maps `(vatusa_role, facility?)` → an OIS group. A null facility means
"held at any facility". Every path above ends in `upsert_member`, which reconciles the member's
`source = 'vatusa'` group grants in the same transaction, so the three paths cannot disagree:

- the grant is scoped to the facility the VATUSA role is held at;
- **`ZHQ` division roles become national grants** (`artcc_id` null) — a division role is national, and
  `ZHQ` is deliberately not added to `org.facilities`, where it would leak into every facility picker;
- any other facility OIS doesn't know is **skipped**, so the `access.user_roles` FK is never hit;
- only `vatusa` rows are added or removed — a hand-made grant of the same group survives a demotion
  (see provenance in [`../architecture/permissions.md`](../architecture/permissions.md));
- a sync that changes anything is audited **exactly as an admin edit is**: one `UPDATE` on
  `USER_ACCESS`, keyed on the member, with the full access snapshot either side — so one query finds
  a controller's whole history, by hand or by sync. The actor is `VATUSA sync`; the reason names each
  change and the VATUSA role behind it (`VATUSA sync: granted EC at ZDC (holds DATM@ZDC)`)

A sign-in and a webhook for the same person cannot race: `upsert_member` updates the member's
`identity.users` row first, and that row lock holds until commit, so their syncs run one at a time.

### The vocabulary, and the defaults (#699)

The division pull sends **long-form** role names: `EVENT_COORDINATOR`, `FACILITY_ACADEMY_EDITOR`,
`INSTRUCTOR`, `WEB_MAINTAINER`, `DIVISION_TECH_TEAM` (`repos::vatusa::DOCUMENTED_VATUSA_ROLES`; a
division-wide grant arrives with facility `*`, stored as `ZHQ`). VATUSA's per-facility endpoint shows the
same grants under short codes (`EC`, `INS`, `WM`, `FACCBT`, …) that the sync never receives, so a mapping
must name the long form. A role name may hold `A–Z`, `0–9` and `_`, up to 64 characters. The list is
documentation, not a filter: the editor offers whatever synced members actually hold.

Migration 0124 ships two mappings (owner's decision on #699):

| VATUSA role | held at | grants |
| --- | --- | --- |
| `EVENT_COORDINATOR` | any facility | `EC`, at that ARTCC |
| `DIVISION_TECH_TEAM` | `ZHQ` (the division) | `VATUSA_STAFF`, national |

**There is no assistant role.** VATUSA's "AEC" is a holder of `EVENT_COORDINATOR` who isn't the facility's
point of contact (`info.ec`), which a mapping can't see. So every holder gets `EC`, and OIS's `AEC`
group stays hand-assigned; the two groups seed the same domains. The `VATUSA_STAFF` default is a
reviewed migration, not an editor action, which is why it ships despite that group being server-admin
only in the editor. Defaults grant at each member's next reconcile, not at migrate time; an admin can
remove either one, which revokes its grants.

The editor shows how many synced members each mapping matches, and warns when one matches nobody; with
no mappings at all it says VATUSA grants nothing.

### Editing mappings

Each non-system group's card on **Admin → Groups** has a *VATUSA roles* section
(`/api/v1/admin/vatusa-role-mappings`). Adding or removing a mapping re-reconciles every synced member
holding that VATUSA role **immediately**, from their stored roles — no VATUSA call, no waiting for the
next sync. The role picker offers only roles seen in synced members.

A mapping grants its group to everyone holding the role, so changing one takes the same gate as setting
the group's whole contents: the editor must hold every permission the group grants, nationally and
unrestricted (`VATUSA_STAFF` is server-admin only). **System groups — `SERVER_ADMIN`, `USER`, `BOT`,
`SERVICE_APP` — can never be mapped**, even by a server admin: `SERVER_ADMIN` has no `role_permissions`
for the gate to check, so without that refusal VATUSA could become a source of server admins.

## The roster grant: `CONTROLLER` (#730)

Every signed-in user is in `USER`, which is unscoped and opens nothing operational. A rostered
controller also gets **`CONTROLLER`**, the baseline operational group (migration 0126). It's granted
**once per facility**: at their home ARTCC (`identity.users.home_facility`) and at each visiting ARTCC
(`identity.vatusa_visits`).

- **Same machinery as the role mappings.** `desired_vatusa_grants` adds the roster rows beside the
  mapped ones, so sign-in, the pull, the webhook, a mapping edit and Resync all produce it. Rows carry
  `source = 'vatusa'` and come and go with the roster. A hand-made `CONTROLLER` grant survives, and a
  detached member (#549) is left alone. The audit names the reason: `granted CONTROLLER at ZDC (holds
  roster home ZDC)`.
- **Never national.** A facility missing from `org.facilities` grants nothing, and so does a `ZHQ`
  home, which is not an ARTCC. A member with no home and no visits holds only `USER`.
- **Lifecycle.**
  - The pull reconciles **every** member on each run, not only those whose roles changed. So a transfer
    or a dropped visit moves the grant on the next pull, and the first pull after a new grant source
    ships backfills everyone.
  - An unchanged member writes and audits nothing.
  - A member who leaves the division loses their roles, visits **and** home facility, and with them the
    grant. "Left" means missing from the pull while holding any of the three: a plain controller holds
    no VATUSA role.
- **What it grants**, each scoped to the facility:
  - `flow.fca.read`, `flow.fca.update`, `flow.fca.delete`;
  - `flow.route.update`;
  - `tmu.cfr.assign`;
  - `flow.runway.read`, `flow.runway.update`;
  - `tmu.program.read`;
  - `tmu.tmi.read`, `tmu.adv.read`, `tmu.ntml.read`, `tmu.gdp.read`, `tmu.groundstop.read`, `tmu.delays.read`;
  - `flow.sectors.read` (#725, migration 0129), so a controller opens Operations → Sector Monitor.

  Every write among them is ARTCC-scoped in its handler. Runway writes became so with this change:
  they're checked against the airport's ARTCC, so a ZDC grant writes only ZDC's airports. A read gate
  is met by a grant at any scope, so `flow.sectors.read` at ZDC reads every ARTCC's sector demand (the
  page's view-only neighbour tables) and also opens the admin sector viewer (Admin → Flow → Sectors);
  it edits no limit or consolidation, which have their own scoped permissions. There's no Planning
  (`events.plan.*`), Historical (`stats.*`) or `*.publish` permission, and no admin permission beyond
  that one read. `the_controller_group_is_exactly_the_operational_baseline` pins the set exactly.

## Reset all access to VATUSA (#795)

Resync (#549) puts one member back on role sync and only touches `vatusa` rows. **Reset all access to
VATUSA** (Admin → Access) does it for everyone and also removes hand-made grants, so each user ends up
holding exactly their `system` grants plus what VATUSA justifies.

- **Server admin only.** Every route checks the caller holds `SERVER_ADMIN` and answers `403` to anyone
  else, a national `access.users.update` holder included. There is no catalog permission for it.
- **Dry run first.** `GET /api/v1/admin/access/vatusa-reset` lists every user who would change and the
  rows they would gain and lose (group or permission, scope, source, allow or deny). It runs each
  user's reset in a transaction it rolls back, against the VATUSA data the last pull stored, so it
  writes nothing.
- **The reset.** `POST /api/v1/admin/access/vatusa-reset` with a reason. It answers at once with
  `202 {run_id}` and runs in the background (#806). A blank reason is `400`, and an unset
  `VATUSA_API_KEY` is `503 vatusa_not_configured`; neither starts a run. The run pulls the division
  first; if the pull fails, nothing is reset and the run fails with `vatusa_pull_failed`. Then, one
  transaction per user in CID order: clear the detach, delete every `manual` row in
  `access.user_roles` and `access.user_permissions` (denies included), reconcile the `vatusa` group
  grants, and write one `USER_ACCESS` audit entry naming every row removed and added. A user with no
  change gets no entry. Because the pull is fresh, the result can differ from the dry run if VATUSA
  changed since the last pull; the result lists what the reset actually did.
- **Its result.** `GET /api/v1/admin/access/vatusa-reset/runs/{id}` returns the run: `running`, then
  `succeeded` with the result or `failed` with the failure. Runs live in `access.vatusa_reset_runs`
  (0131), so any replica answers. The dialog polls it each second and shows the result when it
  finishes; closing the dialog doesn't stop the run.
- **`USER` and `SERVER_ADMIN` are never removed**, whatever their `source`. Migration 0098 backfilled
  both as `manual` for everyone who held them then.
- **A failure part-way** fails the run with `reset_incomplete`. Users before it are reset and
  audited, the failing user is rolled back whole, and users after it are untouched. `users_reset`
  says how many were done; running it again finishes the rest. A run whose backend stopped (a crash
  or a redeploy) reads as `reset_interrupted`, with `users_reset` counted from its audit entries.
- **One at a time, never beside the pull.** A run holds a Postgres advisory lock for its whole life,
  so a second `POST` gets `409` with the running run's id (the dialog says so and waits for that run).
  If the lock stays held for 2 s with no run recorded, the `POST` gets `503 reset_lock_busy`. A run
  also holds the division lock from before its pull until its last member. The division pull job
  takes the same lock, so a scheduled, admin-triggered or webhook-driven pull waits for a reset, and
  a reset waits for a pull. The locks are in Postgres, so they hold across replicas; as a side effect,
  two replicas' daily pulls no longer overlap either. A lock lives on a connection of its own, so it
  is released when the run ends, panics or its process dies.
- **In Background Tasks.** Each run appears as `vatusa_access_reset` in the job registry, and so in
  `/metrics`: running while it runs, then its outcome and `N of M members reset`. The registry is per
  process, so only the replica that ran it lists it. The entry appears with the first run after a
  start.
- Service accounts keep their grants (`access.service_account_roles` is not touched). API keys follow
  their owner's live access.

### How long a reset takes, and the ingress timeout (#806)

The reset used to run inside the request, so an ingress read timeout shorter than the run cut the
admin off with a `504`. Dropping the request's future also stopped the run between members: with the
run moved back inline, `a_dropped_client_does_not_stop_the_run` fails exactly so. Since #806 the
request returns as soon as the run starts, so the reset no longer depends on that timeout.

- **The ingress timeout for `/api/v1/admin/*` in production is not known from this repository.** The
  ingress lives in the cluster's deployment config; `deploy/nginx.conf` serves only the web bundle. It
  can be changed later without affecting the reset.
- **Measured locally, not in production** (2026-10-09; a debug build on a shared, loaded dev host;
  15,003 users; the division pull stubbed out). Time from start to stored result:

  | Members whose access changes | Time |
  | --- | --- |
  | 2 (the roster already in line) | 0.9 s |
  | 150 (1%) | 1.7 s |
  | 15,000 (every member) | 144 s |

  The per-member transactions dominate: about 10 ms each. The real division pull comes on top: its
  fetch may take up to 120 s (`division_client`), then its chunked write. How many members drift in
  production is not known, and the time scales with it.

## Freshness

Every controller is at most **a day** stale, whatever the division's size — the old per-member reconcile
reached ≤ 800 members a day, so its staleness grew with the user count. A webhook delivery brings the
pull forward. Whether VATUSA sends one for a *role* change specifically is still unverified against the
live contract (v3 documents no delivery schema); it no longer matters for correctness, only for how soon
inside that day a change lands. Sign-in is always fresh for the person signing in.
