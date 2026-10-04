# Desktop diagnostics

## Problem
The desktop app wrote no log of any kind, so every desktop bug (#562, #534, #536) was diagnosed by
conversation, and a white-screen crash was captured nowhere. #629 adds a local log, a user-initiated
"Send diagnostics" report, and a staff-only place to read it.

## Scope
In: a rotating log file on the user's machine; webview console output, uncaught errors, unhandled
rejections and render crashes in that log; a "Send diagnostics" report the user chooses to send;
staff reading and deleting reports; 30-day retention.

Out: automatic or background upload, crash reporting SDKs, server-side log shipping.

## On the user's machine
`tauri-plugin-log` writes `ois*.log` in the OS log directory — `~/Library/Logs/net.vatusa.ois/` (macOS),
`%LOCALAPPDATA%\net.vatusa.ois\logs\` (Windows), `~/.local/share/net.vatusa.ois/logs/` (Linux). Files are
cut at 2 MB and the three most recent rotated files are kept (`desktop/src-tauri/src/logging.rs`).

Both halves of the app write to it: Rust's own records, and the webview's (`web/src/lib/logger.ts`, which
also keeps `console.*` working and records `window.onerror`, `unhandledrejection` and the error
boundaries in `web/src/components/error-boundary.tsx`).

**Redaction.** Every line passes through `desktop/src-tauri/src/redact.rs` before it is written, and a
report is redacted again before it is sent (a file written by an older build was never redacted). It
removes OIS credentials by prefix (`ois_dsk_`, `ois_pat_`, `ois_sa_` — including the websocket
`ois.bearer.` form), `Bearer …` values, `METRICS_TOKEN`, the one-time desktop sign-in `code`/`state`
(query and JSON forms), and as a backstop any run of 32+ hex characters. Nothing leaves the machine
unless the user sends a report.

## What a report contains, and why
| Field | Source | Why |
|---|---|---|
| Sender CID, display name, ARTCC | the **server**, from the desktop session — never from the bundle | so staff can follow up with the person |
| App version, bundle id | desktop (`package_info`, config) | which build |
| OS, OS version, architecture, webview version | desktop | platform-specific bugs |
| Window (`main` / `window-…` / `popout-…`), page | webview | where it happened |
| Capability snapshot, WebGL2 availability, last update check, realtime connection history, user agent | webview | the usual suspects (map fallback, updater, socket) |
| The webview's recent log tail (≤ 200 lines) | webview | what happened just before |
| The rolled log files (≤ 12 MB of text, gzipped) | desktop | what happened |
| A free-text note (≤ 4,000 characters) | the user | what they were doing |

## Data model
`diagnostics.reports` (migration `0109`): `user_id → identity.users on delete cascade`, `created_at`,
the platform and context columns above, `note`, `meta jsonb` (everything sent, as sent), `logs bytea`
(the gzip body), `logs_bytes`.

**Retention:** 30 days, pruned by the `diagnostics_report_prune` job (visible and runnable in the admin
Jobs view). Shorter than the audit log's 180 days: a report is bulky and only useful while its bug is
being worked.

**Deletion:** staff with `diagnostics.reports.delete` can delete a report on request; all of a user's
reports are removed with their account.

## Permissions
| Permission | Grants |
|---|---|
| `diagnostics.reports.read` | the admin **Diagnostics** page, a report's details, and its log download |
| `diagnostics.reports.delete` | deleting a report |

`VATUSA_STAFF` holds both (it carries the whole catalogue, per migration `0094`); no facility role holds either. Server admins hold all permissions.

## API
| Method + path | Who | Notes |
|---|---|---|
| `POST /api/v1/diagnostics/reports` | a signed-in **desktop** session (`ois_dsk_`) | multipart `meta` (JSON) + `logs` (gzip). 5 MB cap (route `DefaultBodyLimit`) → `413 payload_too_large`; more than 5 reports in an hour → `429 too_many_requests`; non-gzip logs → `400`. Called only by the desktop's Rust (`desktop/src-tauri/src/diagnostics.rs`), so not in the OpenAPI document. |
| `GET /api/v1/admin/diagnostics` | `diagnostics.reports.read` | paged list, newest first |
| `GET /api/v1/admin/diagnostics/{id}` | `diagnostics.reports.read` | one report, without logs |
| `GET /api/v1/admin/diagnostics/{id}/logs` | `diagnostics.reports.read` | the gzipped logs |
| `DELETE /api/v1/admin/diagnostics/{id}` | `diagnostics.reports.delete` | `204`, or `404` |

## Discord
None.

## Open questions
None at ship time. Sharing reports outside OIS staff is not supported; VATSIM's data policy allows
keeping request logs but not publishing them, and VATUSA DP001 treats member action logs as private.
