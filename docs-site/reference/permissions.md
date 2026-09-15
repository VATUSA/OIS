# Roles & permissions

What you can do in OIS is governed by **permissions**, granted through **roles** and scoped per-ARTCC.

## How it works

- Permissions are fine-grained and path-based (`segments.action` — e.g. `flow.fca.update`).
- They're bundled into **roles** (for example a TMU or events role), which are assigned to users — optionally **scoped to a specific ARTCC** so a role only applies within that facility's airspace.
- A national-level grant applies everywhere; an ARTCC-scoped grant applies only there. For example, `flow.facility_map.update` (edit a facility map's aircraft coloring) is typically granted **scoped to one ARTCC**, so a controller can recolor only their own facility's map.

## VATUSA roles vs. OIS access

Your [VATUSA roles](/introduction/profile) (ATM, INS, …) describe your real-world facility position. Your **OIS access** is what you can do in OIS. They're related but managed separately — having a VATUSA staff position doesn't automatically grant OIS permissions unless an administrator maps it.

## API keys

Holders of **`api_keys.key.create`** can mint their own [API keys](/reference/api-keys) — personal access tokens for the OIS API. A key is always **capped by its owner's live permissions**: it can never hold a permission or reach an ARTCC the owner doesn't, and it loses access the moment the owner does. Oversight of *all* keys is governed by `api_keys.key.read` (view) and `api_keys.key.delete` (revoke), which server admins hold implicitly.

## Managing access

Administrators manage grants in the **access control** editor. It lists **all users in a browsable, paginated table** (search by name or CID when you know who you want), and for each user shows their roles and permissions at the national level and per ARTCC. Every change is recorded with a **reason** to the user's audit log.

## Server admin

A small set of CIDs configured on the server hold **SERVER_ADMIN**, which grants everything. This is set in deployment configuration, not through the UI.

## Assignable roles

An administrator grants one or more of these positional roles to a user, nationally or scoped to an
ARTCC:

| Role | Typically held by |
| --- | --- |
| `VATUSA_STAFF` | Division staff |
| `EVENTS_TEAM` | Events team members |
| `EC` | Events coordinator |
| `AEC` | Assistant events coordinator |
| `ACE` | ACE (traffic-management support) team |
| `NTMO` | National Traffic Management Officer |
| `DCC_STAFF` | DCC (Discord coordination) staff |

Roles bundle a set of the permissions below; ask an administrator which role covers what you need
rather than requesting individual permissions.

## Full permission catalog

Every permission OIS enforces, grouped by domain. Most are only ever granted as part of a role
above — this table is here so you know exactly what a role covers, or what to ask for if you need
something narrower.

**Traffic management (`tmu`)**

| Permission | Grants |
| --- | --- |
| `tmu.tmi.read` / `.create` / `.update` / `.publish` / `.delete` | View, draft, edit, publish, and remove NTML restrictions |
| `tmu.program.read` / `.update` / `.delete` | View and manage an airport's AAR/spacing program |
| `tmu.groundstop.read` / `.create` / `.publish` / `.delete` | View, draft, publish, and remove ground stops |
| `tmu.gdp.read` / `.create` / `.publish` / `.delete` | View, draft, publish, and remove Ground Delay Programs |
| `tmu.cfr.assign` | Issue or cancel a CFR release |

**Flow (`flow`)**

| Permission | Grants |
| --- | --- |
| `flow.fca.read` / `.update` / `.delete` | View, edit, and delete Flow Constrained Areas |
| `flow.route.update` / `.delete` | Edit or delete a route (visibility comes from `flow.fca.read`) |
| `flow.runway.read` / `.update` | View and manage the Runway Balancer |
| `flow.facility_map.update` | Edit a facility's map color rules (public to view) |
| `flow.surface_data.update` | Edit an airport's gates/ramp areas/taxiways |
| `flow.aircraft_profiles.read` / `.update` | View and edit aircraft performance profiles |

**Event planning (`events`)**

| Permission | Grants |
| --- | --- |
| `events.plan.read` / `.update` | View and edit an event's planning modules (TMI packages, FCAs) |
| `events.rate.update` | Set an event's airport AAR/ADR (facility-scoped) |
| `events.config.update` | Manage an airport's runway configs (facility-scoped) |
| `events.support.update` | Set your facility's event support level |
| `events.availability.update` | Record staff availability for an event |
| `events.debrief.create` | Write a post-event debrief |
| `events.discord.publish` | Open the event's Discord coordination thread |

**ACE support (`ace`)**

| Permission | Grants |
| --- | --- |
| `ace.requests.read` / `.create` | View and submit ACE support requests |
| `ace.requests.claim` | Claim an open request |
| `ace.requests.decide` | Approve or decline a request |

**Statistics (`stats`)**

| Permission | Grants |
| --- | --- |
| `stats.data.read` | View [Historical](/historical/overview) — network stats, replay, delays, taxi insights |
| `stats.capture.update` | Save or manage a capture window |

**Facility documents (`facilities`)**

| Permission | Grants |
| --- | --- |
| `facilities.docs.read` / `.update` | View and edit [facility documents](/planning/facility-documents) |

**Discord (`discord`)**

| Permission | Grants |
| --- | --- |
| `discord.config.read` / `.update` | View and edit which Discord guild/roles map to a facility |

**API keys & access (`api_keys`, `access`, `users`, `audit`)**

| Permission | Grants |
| --- | --- |
| `api_keys.key.create` | Mint your own [API keys](/reference/api-keys) |
| `api_keys.key.read` / `.delete` | View and revoke *any* user's keys (oversight) |
| `access.users.read` / `.update` | View and edit any user's roles/permissions in the access editor |
| `users.directory.read` | Browse the user directory |
| `audit.logs.read` | View the audit log |

**Your own account (`auth`)**

Every signed-in user holds `auth.profile.read`, `auth.profile.update`, and `auth.sessions.delete` —
viewing and editing your own profile and ending your own sessions needs no special grant.

A handful of server-operator-only permissions (`system.jobs.*`, `service_accounts.*`,
`integration.jobs.update`) aren't listed here — they're not capabilities any facility staff role
grants, only something a platform administrator holds directly.
