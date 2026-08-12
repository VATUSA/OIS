# Proposal: OIS — a unified VATUSA operations platform

**Prepared for:** VATUSA senior / division staff
**Prepared by:** _[project lead — fill in]_
**Date:** 2026-08-12
**Status:** Draft for approval

---

## Executive summary

VATUSA's operational tooling is spread across disconnected systems, and a backlog of
requested improvements — staffing coordination, traffic management (NTML/ADV/TMI), ACE
support, Discord automation, and fine-grained permissions — has gone unbuilt because
there is no common foundation to build them on.

**OIS** is a single platform that provides that foundation: one backend (the authority
for identity, permissions, and OIS's operational domains), one website, one Discord bot,
and — later — a desktop app for controllers. It replaces the third-party flow tool
(`vatflow.io`) with software VATUSA owns, brings the operational, traffic, and ACE
capabilities in-house, and **works alongside the current VATUSA website**, which
continues to own event creation, review, and posting.

The foundation is **already built and working** — VATSIM login, sessions, and a
fine-grained, auditable permissions system. This proposal asks division leadership to
approve OIS as the path forward and to support building out the operational features on
top of it.

## The problem today

- **Fragmentation.** Core functions live in separate systems (a backend, a website,
  and the external `vatflow.io`) that don't share identity, permissions, or data. Every
  new capability has to be bolted onto whichever system is least inconvenient.
- **A stalled backlog.** A review of requested operations features found that most have
  no home to be built in today. (Full list in [Appendix A](#appendix-a-requested-features--where-they-land).)
- **Coarse permissions.** Access is largely implied by role name, which makes
  delegating narrow capabilities — or scoping them to one ARTCC — awkward and hard to
  audit.
- **Dependence on third-party tools.** Traffic-management flow relies on `vatflow.io`,
  outside VATUSA's control, with no API to integrate against.

## What we propose

Build OIS as one project (a "monorepo") with four parts that share a single API:

| Part | What it is |
| --- | --- |
| **Backend** | The authority for identity, permissions, and every operational domain. A single high-performance service (Rust) over a Postgres database, exposing one versioned API. |
| **Website** | The staff- and controller-facing site, consuming that API. |
| **Discord bot** | Automates event threads, traffic advisories, and ACE requests, and links VATSIM identities to Discord. |
| **Desktop app** | A native app for controllers with tools that suit a desktop (live traffic/flow monitor, always-on displays) — a later phase. |

Every client — website, bot, desktop — goes through the **same authenticated,
permission-checked API**. There are no side doors, and everything is auditable.

OIS **complements** the current VATUSA website rather than replacing it wholesale: event
creation, review, and posting stay there; OIS owns the operational window around events
(staffing, sign-up, coordination, debrief) plus the traffic, ACE, and access tooling,
and integrates with the current site where they meet.

## What OIS delivers

Grouped by area, each mapping to specific requests from the feature review:

- **Events (operational)** — the coordination window around an event: "CC an ARTCC"
  staffing requests, controller position sign-up/slots, auto-created Discord
  coordination threads with staff pings, and post-event debrief. (Event creation,
  review, and posting stay in the current VATUSA website.)
- **Traffic management (TMU)** — NTML and advisories brought onto the VATUSA site with
  a plain-language view, TMI generation and publishing, and an average-delay page — all
  exposed through the API.
- **ACE support** — controllers request ACE support on the site; requests auto-post to
  Discord with a one-click **claim** button; ECs are notified when a request is claimed.
- **Flow (traffic management)** — VATUSA's own traffic-management functions (the
  capabilities `vatflow.io` provides today), reimplemented natively and fed by the
  VATSIM data feed — replacing the third-party tool.
- **Access control** — fine-grained, per-ARTCC-scopable permissions with a full audit
  trail (**built today**).

Detailed, per-feature designs live in [docs/features/](features/).

## Why build it this way

- **One source of truth.** Identity, permissions, and operational data live in one
  place, so features compose instead of being re-implemented per system.
- **Fine-grained, auditable access.** Every capability is an explicit permission,
  optionally scoped to an ARTCC, and every change is logged with a reason — the control
  and accountability leadership has asked for.
- **VATUSA-owned.** Replacing the third-party flow tool and the current stack with
  software VATUSA controls removes external dependencies and unblocks integration.
- **An API for everything.** Because every capability is exposed through one documented
  API, future tools (including community projects) can build on VATUSA data cleanly and
  safely.
- **Proven design.** The architecture is adapted from an existing, working ARTCC
  platform, so this is applying a known-good pattern at national scale — not inventing
  from scratch.

## Where it stands today

This is not a blank-slate pitch. The foundation is built and demonstrable:

- **VATSIM login**, sessions, and the current-user profile.
- The **fine-grained permissions system** end-to-end: roles, per-user allow/deny
  grants, the per-ARTCC scope, a server-admin bootstrap, and the effective-permission
  resolution.
- The **access editor API** — read the catalog, read a user's access, and change it
  with a required reason that is written to an audit log — verified working.

See [access-control.md](features/access-control.md) and the build status in
[PLAN.md](PLAN.md).

## Phased plan

Delivered in phases so value ships incrementally and leadership can review at each
step. Sequencing is fixed; calendar dates depend on contributor availability.

| Phase | Focus | Status |
| --- | --- | --- |
| 0 | Foundation: identity, permissions, access editor | **In progress** (auth + access done) |
| 1 | Detailed feature designs (this doc set) | In progress |
| 2 | Backend features: events → TMU → ACE → Discord queue → flow | Next |
| 3 | Website | After Phase 2 |
| 4 | Discord bot | With/after Phase 3 |
| 5 | Desktop app | Later |

## Benefits

- **Controllers** get event sign-ups, ACE support, and clearer traffic information in
  one place.
- **ECs / event staff** get cross-ARTCC staffing coordination, controller position
  sign-up, and automatic Discord coordination instead of manual steps.
- **TMU staff** get first-party NTML/ADV/TMI tools and delay reporting.
- **Division leadership** gets precise, auditable control over who can do what, and a
  platform VATUSA owns outright.

## Risks and how we manage them

| Risk | Mitigation |
| --- | --- |
| **Scope** — the platform is broad | Phased delivery; each phase is independently useful and reviewable. |
| **Migration** from current systems | The backend mirrors, rather than replaces, VATSIM/VATUSA as the identity/roster authority; cutover is planned per-domain, not big-bang. |
| **Contributor capacity / bus factor** | Ordinary, well-documented technology and a documented design (this doc set) so the work is transferable, not locked to one person. |
| **External API dependencies** (VATSIM, VATUSA, myVATSIM) | Integrations are isolated behind clear boundaries; features that depend on an unavailable external API (e.g. myVATSIM cross-posting) are marked and deferred. |
| **Hosting & operations** | Standard containerized deployment; requirements are modest. Hosting ownership to be confirmed with the division. |

## What we're asking for

1. **Approval** to proceed with OIS as VATUSA's operations platform — replacing the
   third-party flow tool and delivering the operational, traffic, ACE, and permissions
   capabilities — working alongside the current VATUSA website, which retains event
   creation, review, and posting.
2. **A designated point of contact** in division leadership for approvals and
   priorities.
3. **Access to VATUSA API credentials** (and any myVATSIM integration path) so roster
   sync and cross-posting can be built against real data.
4. **Agreement on hosting** — where the production service runs and who operates it.
5. **Confirmation of feature priorities** for Phase 2 (proposed order: events → TMU →
   ACE → Discord → flow).

---

## Appendix A: Requested features → where they land

| Request | Home | Status |
| --- | --- | --- |
| Approval before postings go public | Current VATUSA site | Not OIS |
| Minimum lead time (no posting within 7 days) | Current VATUSA site | Not OIS |
| Auto cross-post VATUSA → myVATSIM | Current VATUSA site | Pending VATSIM API |
| AECs (not just ECs) can post events | Current VATUSA site (posting) | OIS `AEC` role scopes coordination |
| Feature other facilities on a posting | Current VATUSA site | Not OIS |
| Structured event metadata + API | Current VATUSA site | Not OIS |
| CC an ARTCC + staffing-request notification | OIS · events | Design ready |
| Auto T1 staffing for FNOs (per DP003) | OIS · events | Design ready (criteria TBD) |
| Event position sign-up / slots | OIS · events | Design ready |
| Auto DCC Discord thread + staff ping | OIS · events + discord | Design ready |
| Post-event debrief | OIS · events | Design ready |
| NTML/ADV → plain-language TMU view | OIS · tmu | Design ready |
| NTML/ADV on the VATUSA site + API | OIS · tmu | Design ready |
| Average-delay page | OIS · tmu | Design ready (via flow / VATSIM feed) |
| Traffic-management tooling (vatflow / SimTraffic) | OIS · flow | Native reimplementation (feature audit pending) |
| Merge ACE requests into the site | OIS · ace | Design ready |
| Auto-post ACE requests to Discord (claim button) | OIS · ace + discord | Design ready |
| Notify ECs on ACE claim/book | OIS · ace + discord | Design ready |
| Fine-grained, auditable, ARTCC-scopable permissions | OIS · access | **Built** |

_Design detail for each item is in [docs/features/](features/)._
