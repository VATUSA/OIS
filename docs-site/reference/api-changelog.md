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

- **New: `GET /api/v1/admin/access/vatusa-reset` and `POST /api/v1/admin/access/vatusa-reset`**
  (#795). The dry run and the reset of every user's access to what VATUSA justifies. Server admin
  only. The `POST` takes `AccessResetRequest` (`reason`) and returns `AccessResetBody`; a failed VATUSA
  pull (`502`), an unconfigured VATUSA (`503`) or a run that stopped part-way (`500`) returns
  `AccessResetFailure` with `error`, `message` and `users_reset`. The dry run reads the VATUSA data the last pull stored and the reset pulls fresh, so the two
  can differ if VATUSA changed in between. The reset keeps `system` grants, so the baseline `USER` and
  `SERVER_ADMIN` stay; a row of either written by hand as `manual` is removed like any other (#805).
  Additive; no existing caller changes.
