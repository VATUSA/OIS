# VATUSA sync and role mapping

> **Status: member sync built; role → group mapping built backend-only (#548, 2026-10).** The mapping
> table and the reconciler ship with **no mappings seeded**, so they grant nobody anything until a
> mapping is added. The admin page for editing mappings is the second half of #548.

## What is synced

`backend/src/feed/vatusa.rs` fetches `GET /v2/user/{cid}` and `repos::vatusa::upsert_member` stores the
member's details, roles (`identity.vatusa_roles`) and visits. It runs on three paths:

| Path | When | Bound |
|---|---|---|
| Sign-in | every login, **awaited** before the session is issued | 5 s budget, then finishes in the background |
| Roster webhook | VATUSA notifies a roster change for a CID we know | single attempt, no retry |
| `vatusa_reconcile` job | every 6 h, the 200 least-recently-synced members | listed on Background Tasks |

Everything no-ops when `VATUSA_API_KEY` is unset. Role and facility codes are trimmed and uppercased on
ingest.

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
- every change is audited as `GRANT`/`REVOKE` on `ACCESS_GROUP_MEMBER` by the `VATUSA sync` actor, with
  a reason naming the VATUSA role(s) that caused it.

A sign-in and a webhook for the same person cannot race: `upsert_member` updates the member's
`identity.users` row first, and that row lock holds until commit, so their syncs run one at a time.

## Known gap: does the webhook fire for role changes?

**Unverified against the live VATUSA contract.** `changed_cids` takes the CID from `row_pk` for a
`controllers` row and otherwise from `old_value.cid` / `new_value.cid`. A change to VATUSA's `roles`
table therefore triggers a sync **only if VATUSA includes `cid` in the role row snapshot** — and nobody
has confirmed it does. It was deliberately not probed against VATUSA's production API.

Until it is confirmed, **the reconcile job is the guaranteed path**: 200 members per 6 h is 800 a day,
so once there are more than ~800 synced users, a role change can take longer than a day to reach OIS
access, and that delay grows with the user count. Sign-in is always fresh — a member who logs in is
reconciled at that moment.

To close the gap: confirm the `roles` webhook payload with VATUSA, or capture one delivery from the
receiver's logs.
