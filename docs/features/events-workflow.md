# Events workflow

## Problem

Replace the cobalt/webapps event posting flow and close the audited gaps: approval before public posting, minimum lead
time, multi-host/featured facilities, CC-an-ARTCC staffing requests, event slots, and auto Discord threads.

## Scope

- **First cut**: approval workflow, lead-time validation, multi-host + featured facilities, structured API metadata,
  Discord auto-thread + staff ping.
- **Later**: event slots/booking, auto T1 staffing per DP003 (FNOs), myVATSIM cross-post (pending VATSIM API).

## Data model *(draft)*

- `events.events` — id, title, description, **multiple** hosting ARTCCs (join table, not a single `facility`),
  start/end, `review_status` (pending→approved/rejected), submitted_by, reviewed_by.
- `events.event_hosts` — event_id, artcc_id, role (host/featured).
- `events.positions` — position roster; later extended with slots/booking.
- `events.staffing_requests` — event_id, requested_artcc_id, status, notification state.

Port osmium's `review_status` + `ReviewEvent` model as the approval base.

## Permissions

- `events.items.{create,update,delete}` — post/edit/remove (held by `EC`, **`AEC`**, ATM/DATM via facility scope).
- `events.approval.decide` — approve/reject submissions (national events staff).
- `events.featured.update` — feature other facilities.
- `events.staffing_requests.{create,read}` — CC an ARTCC / see incoming requests.
- `events.discord.publish` — trigger the Discord thread + ping.
- `events.positions.{assign,publish,delete}`, `events.positions.self.request` — staffing.

## API

`GET /event/{id}`, `/event/page`, `/event/upcoming` return structured metadata including **all** hosting ARTCCs +
featured fields + start/end (fixes the myVATSIM hosting-ARTCC gap). Approval, staffing-request, and featured endpoints
gated by the permissions above.

## Discord

On publish (with `events.discord.publish`), enqueue an outbound job to create a DCC thread/forum post and ping required
staff roles. (Consideration: threads → forums.)

## Open questions

- Lead-time rule: hard block at 7 days, or warn + allow override by national staff?
- Slots: per-position booking model vs. free-form signup.
- DP003 T1 auto-trigger: exact FNO criteria + who gets notified.
