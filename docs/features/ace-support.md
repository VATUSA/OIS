# ACE support requests

## Problem

Merge ACE Team requests into the OIS site (today the ACE page is hardcoded mock data), let users request ACE support
from the website, auto-post new requests to Discord with a **claim** button, and notify ECs when a request is
claimed/booked.

## Scope

- **First cut**: request submission, request queue on the site, Discord embed with claim button, claim → notify.
- **Later**: scheduling/booking of claimed requests, ACE team roster management wired to the backend (replace the mock
  display page).

## Data model *(draft)*

- `ace.requests` — id, requested_by, artcc_id, details, `status`
  (open→claimed→completed/cancelled), claimed_by, claimed_at.
- `ace.team_members` — id, user_id, role — replaces the hardcoded ACE display page.

## Permissions

- `ace.requests.create` — a controller opens a request.
- `ace.requests.read` — view the queue.
- `ace.requests.claim` — claim an open request (uses the `claim` action).
- `ace.requests.decide` — close/cancel/reassign.
- `ace.team.{read,update}` — manage the ACE roster.

Held by `ACE_NATIONAL` and ACE team members.

## API

`POST /ace/requests`, `GET /ace/requests`, `POST /ace/requests/{id}/claim`, plus team endpoints. Claim is data-dependent
(must be open) on top of the permission gate.

## Discord

- New request → outbound job posts an embed to `#aceteam-requests` with a claim button.
- Claim (button or site) → backend enqueues a job to notify the requesting EC and edits the embed to show the claimer.

## Open questions

- Who may claim — any ACE member, or scoped by ARTCC/rating?
- Booking/scheduling model for a claimed request.
