---
paths:
  - "backend/migrations/**"
  - "backend/src/repos/{users,auth,access,audit,api_keys,vatusa,stats}.rs"
  - "backend/src/handlers/{auth,users,diagnostics,webhooks}.rs"
  - "backend/src/feed/vatusa.rs"
  - "backend/src/feed/stats/**"
  - "backend/src/audit.rs"
  - "backend/src/jobs.rs"
  - "desktop/src-tauri/src/{diagnostics,redact,logging}.rs"
  - "web/src/pages/privacy.tsx"
  - "docs/features/diagnostics.md"
---
# Data Protection (not loaded at startup; read in full before storing, exporting, sending or deleting member data)

Point-of-edit reminders for personal data. OIS holds VATSIM members' data under VATSIM's data
protection and handling policy; the user-facing description is `web/src/pages/privacy.tsx`, which says
it is descriptive and not VATUSA's official policy. Always-loaded summary: `secure-coding.md` →
"Data-protection obligations".

**Flag, don't implement.** Fix a misalignment this branch introduces on the branch. A pre-existing one
is a follow-up only if it passes the tests in `github-issues.md`. Do not build erasure, export or
retention machinery off the back of a review; that is the owner's call.

---

## What already exists, so a change has something to keep in step with

- **Identity:** `identity.users` (CID, email, full and display name, rating, home facility, VATUSA
  sync fields), `identity.sessions`, `identity.vatusa_roles` and `identity.vatusa_visits` (keyed on
  CID), and the Discord id in `integration.external_sync_mappings`.
- **Activity:** `access.audit_logs` (actor, IP, before/after JSON snapshots that include CIDs),
  `access.api_keys.last_used_ip`, the `*_by` / `*_by_actor` attribution columns.
- **Network data:** `stats.member` (CID, name), `stats.flight` and `stats.flight_leg` (CID, callsign),
  from the public VATSIM feed.
- **Diagnostics:** `diagnostics.reports` (sender, note, redacted desktop logs), described field by
  field in `docs/features/diagnostics.md`.
- **Retention:** `jobs.rs` prunes audit rows after `AUDIT_RETAIN_DAYS` (180), diagnostics reports
  after `DIAGNOSTICS_RETAIN_DAYS` (30) and delay legs after `DELAY_LEG_RETAIN_DAYS` (30). The division
  pull clears a departed member's VATUSA roles, visits and home facility (`repos::vatusa::clear_departed`);
  the user row stays.
- **Erasure and export:** none. No application path deletes an `identity.users` row or exports a
  member's data. Rows keyed on `user_id` with `on delete cascade` (sessions, diagnostics reports,
  actors) would follow a deleted user; rows keyed on CID (`identity.vatusa_*`, `stats.*`) and the
  JSON inside audit snapshots would not.

## The four tells

- **A new personal-data column or table** → in the same change: a retention (a prune pass like
  `spawn_audit_log_prune`, or the reason it has none), a key that a user's removal reaches (a foreign
  key to `identity.users` with `on delete cascade`, not a bare CID), and a line on the privacy page if
  it is a new kind of data. A column nothing ever removes is held forever on day one.
- **A stored blob or free text holding personal data** (a diagnostics bundle, a note, an audit
  snapshot) → redact before storing where the producer allows it (`desktop/src-tauri/src/redact.rs`
  runs on the device), prune it, and give staff a delete path (`diagnostics.reports.delete`).
- **Fixtures and test data** → never rows from production or a production snapshot. Seed with SQL in
  the test. Quote counts from a snapshot, never a CID, name, email or IP, in an issue, PR, commit or
  test.
- **Personal data leaving for a third party** (a name or Discord id in a Discord post or DM, a CID
  sent to the VATUSA API) → the privacy page must still describe what is shared, and a new recipient
  is the owner's decision, not the branch's.
