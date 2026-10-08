# API changelog

Changes to the OIS API (`/api/v1`) that a tool calling it might notice: new endpoints, changed request or
response shapes, new error responses, and anything removed.

::: warning Nothing is stable before 1.0
Until OIS 1.0, any part of the API can change in any release, and entries here are informational only.
From 1.0, OIS stands behind a defined set of endpoints. Those change only in ways that keep existing
callers working. Anything retired is announced here and carries `Deprecation` and `Sunset` headers on
its responses for at least 30 days before it goes.
:::

Each entry names the endpoint, what changed, and whether an existing caller has to do anything.

## Unreleased (pre-1.0)

- **New:** `PUT /api/v1/flow/sector-consolidations/{artcc}` changes several of one ARTCC's sector
  consolidations in one request. The body is `{ "into": { "<sector_id>": "<target_sector_id>" | null } }`:
  each sector is worked at the target given, or given its own row back for `null`. It needs
  `flow.sector_consolidations.update` for that ARTCC, like the single-sector `PUT` and `DELETE`, and is
  all or nothing: a self-reference or two keys naming one sector (400), an unknown or other ARTCC's
  sector (404) or a loop (409) writes none of it. Answers with the ARTCC's consolidations. Existing
  callers need do nothing. (#794)
