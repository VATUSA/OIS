# The supported API surface

> **Proposed for 1.0. Not stable today, and the owner has to confirm it.** Before product 1.0 nothing in
> `/api/v1` is stable (`AGENTS.md` § Versioning). This page is the list the freeze would commit to,
> written down now so 1.0 freezes a decision rather than whatever happens to exist.

At 1.0 we stand behind the operations below and nothing else. Every other path in `/api/v1` is
**internal**: the web app, the desktop app and the Discord bot use it through the generated client,
and it may change in any release. It's still in the OpenAPI document and still permission-checked,
but integrators shouldn't build on it.

From 1.0, a supported operation is retired only after at least 30 days of `Deprecation` and `Sunset`
headers plus an entry in the API changelog (`docs-site/reference/api-changelog.md`).

## Proposed operations

Paths are relative to `/api/v1`. They're picked for the integrations #582 is opening OIS to: an external
metering tool reading flow and issuing departure releases.

| Area | Operation | What it is |
|---|---|---|
| Identity | `GET /me` | Who the credential acts as |
| Flow | `GET /flow/fcas` | Flow constrained areas |
| | `GET /flow/fcas/{id}` | One FCA |
| | `GET /flow/fcas/{id}/traffic` | An FCA's metered traffic |
| | `GET /flow/idst` | Departure release list (IDST) |
| Releases | `POST /flow/fcas/{id}/release/{callsign}` | Issue or replace a release |
| | `DELETE /flow/fcas/{id}/release/{callsign}` | Clear a release |
| | `POST /flow/fcas/{id}/swap` | Trade two flights' release times |
| CFRs | `POST /tmu/cfr` | Issue a call-for-release time |
| | `DELETE /tmu/cfr/{callsign}` | Release a CFR |
| | `GET /tmu/flow/{icao}` | An airport's departure flow, with CFR state |
| Realtime | `GET /ws` | Topic nudges, instead of polling |

## Open questions for the owner

- **Is this the right subset?** In particular, should the public-on-purpose flow reads (#586), or
  writes beyond releases and CFRs, be in it?
- **Does "additive-only" cover the whole supported surface, or only its schemas?** That is, may a
  supported operation gain a required request field after 1.0 if the old shape keeps working for a
  deprecation window?
