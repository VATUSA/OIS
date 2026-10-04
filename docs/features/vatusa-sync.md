# VATUSA sync and role mapping

> **Status: built (#548, #605, 2026-10).** A daily v3 pull of the whole division, the role → group
> mapping, and its editor. Mappings ship **unseeded** — VATUSA grants nobody anything until an admin
> adds a mapping.

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

## Freshness

Every controller is at most **a day** stale, whatever the division's size — the old per-member reconcile
reached ≤ 800 members a day, so its staleness grew with the user count. A webhook delivery brings the
pull forward. Whether VATUSA sends one for a *role* change specifically is still unverified against the
live contract (v3 documents no delivery schema); it no longer matters for correctness, only for how soon
inside that day a change lands. Sign-in is always fresh for the person signing in.
