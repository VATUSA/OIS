# Integrating with OIS

This page takes you from "we'd like to integrate" to a working release, without reading OIS's source.
It builds on [Using the API](./api.md), which covers the API description and Swagger.

::: tip Examples here are tested
Every API call on this page is checked against the real API in CI, so a renamed endpoint can't leave
a stale example behind.
:::

## Quickstart

**1. Get a token.** For a first look, create a personal API key: see
[API keys § Getting a key](./api-keys.md#getting-a-key). It's shown once; keep it in an environment
variable rather than in code:

```bash
export OIS_TOKEN=ois_pat_xxxxxxxx…
```

**2. Make an authenticated call.** List the flow constrained areas (FCAs):

```bash
curl -H "Authorization: Bearer $OIS_TOKEN" https://<your-ois-host>/api/v1/flow/fcas
```

**3. Read metered traffic.** Pick an FCA's `id` from that list and read the flights metered across it,
in crossing order:

```bash
curl -H "Authorization: Bearer $OIS_TOKEN" https://<your-ois-host>/api/v1/flow/fcas/{id}/traffic
```

Each flight carries its crossing estimate (`cross_time`), its delay (`delay_min`), its sequence
(`seq`), whether it holds a release (`released`), its EDCT (`edct`), and — when released — the
release's `release_version`, which you'll need to change it (see
[the release workflow](#the-release-workflow)). Every time is RFC 3339 UTC.

To see the same picture by airport, as the IDST does:

```bash
curl -H "Authorization: Bearer $OIS_TOKEN" "https://<your-ois-host>/api/v1/flow/idst?airports=KJFK"
```

That's the whole read path. Everything below is about writing safely.

## Choosing a credential

OIS accepts two kinds of machine credential. Both are sent the same way,
`Authorization: Bearer <token>`.

| | API key | Service account |
| --- | --- | --- |
| Token | `ois_pat_…` | `ois_sa_…` |
| Belongs to | one person | an organisation's integration |
| Created by | its owner, under **API keys** in the sidebar | an admin, in **Admin → Service accounts** |
| Can do | at most what its owner can, and only the permissions picked for it | what its roles grant |
| Expiry | optional, set at creation | 90 days by default, at most 365 |

**Use a service account for anything an organisation runs** — a TMU tool, a bridge, a bot. It
doesn't depend on one person staying on staff or keeping their access, and its writes are attributed
to the integration rather than to whoever happened to create the key. An API key is for a person's
own scripts.

**Rotating.** Both can be rotated in place: a new token is issued and the old one stops working. An
API key's owner rotates it with:

```bash
curl -X POST -H "Authorization: Bearer $OIS_TOKEN" https://<your-ois-host>/api/v1/api-keys/{id}/rotate
```

and an admin rotates a service account (optionally setting a new `expires_in_days`) with:

```bash
curl -X POST -H "Authorization: Bearer $OIS_TOKEN" https://<your-ois-host>/api/v1/admin/service-accounts/{id}/rotate
```

**Revoking.** Disable or delete the credential (from the same pages). Its future requests fail with
`401` at once. **Releases it already issued stand** — a pilot may already hold them — and they stay
recorded as that integration's, even after the credential is deleted. A replacement credential is a
different integration, so it can't take them over; a person can still clear them.

## The release workflow

OIS is the system of record for releases. Your tool writes releases through the same endpoints a
controller uses, and its writes are **proposals OIS accepts or refuses** — there's never a second
truth to reconcile. The rules below cover FCA releases and CFRs alike.

### Who may change a release

A release is *held* by whoever wrote it last.

| the release is held by | a person writes | your tool writes |
| --- | --- | --- |
| nobody | allowed | allowed |
| a person | allowed | **409 `held_by_person`** |
| your tool | allowed (a person overrides a tool) | allowed |
| another tool | allowed | **409 `held_by_other_machine`** |

People always win: a controller can override anything your tool did, and your tool can never
overwrite a controller's decision.

### Versions: no double issue, no silent overwrite

Every release has a `version`. It's returned in the `ETag` header of a successful write, and as
`release_version` on each flight in the traffic list. **A tool must send a precondition on every
write** (a swap excepted); without one the write is refused with `428 precondition_required`.

**Issue a release** only if the flight doesn't hold one — `If-None-Match: *`. `ready` is the
pilot's ready time, `HHMM` in **UTC** (it resolves to the nearest occurrence within ±12 hours); omit
it for "ready now", which takes the metered slot:

```bash
curl -X POST -H "Authorization: Bearer $OIS_TOKEN" -H "If-None-Match: *" -H "Content-Type: application/json" -d '{"ready":"1415"}' https://<your-ois-host>/api/v1/flow/fcas/{id}/release/{callsign}
```

**Change it** by naming the version you last saw — `If-Match: "1"`:

```bash
curl -X POST -H "Authorization: Bearer $OIS_TOKEN" -H 'If-Match: "1"' -H "Content-Type: application/json" -d '{"ready":"1430"}' https://<your-ois-host>/api/v1/flow/fcas/{id}/release/{callsign}
```

**Clear it** the same way:

```bash
curl -X DELETE -H "Authorization: Bearer $OIS_TOKEN" -H 'If-Match: "2"' https://<your-ois-host>/api/v1/flow/fcas/{id}/release/{callsign}
```

**Swap** two held departures' release times without re-metering (no precondition; your tool needs
authority over both):

```bash
curl -X POST -H "Authorization: Bearer $OIS_TOKEN" -H "Content-Type: application/json" -d '{"a":"AAL123","b":"DAL456"}' https://<your-ois-host>/api/v1/flow/fcas/{id}/swap
```

**CFRs** follow the same rules, issued with `POST /api/v1/tmu/cfr` (an RFC 3339 `ready_time`). A
CFR's version is the `ETag` of the write that issued it, and `cfr_version` on each departure in a
field's departures list, so you can always re-read it:

```bash
curl -H "Authorization: Bearer $OIS_TOKEN" https://<your-ois-host>/api/v1/tmu/departures/{dep}
```

Clear one with:

```bash
curl -X DELETE -H "Authorization: Bearer $OIS_TOKEN" -H 'If-Match: "1"' https://<your-ois-host>/api/v1/tmu/cfr/{callsign}
```

### Retrying safely

Because every write names what it expects, **a retried request can never issue a time twice**:

1. You issue with `If-None-Match: *`. It succeeds (`200`, `ETag: "1"`), but the response is lost.
2. You retry the same request. It's refused with `412 precondition_failed`, `ETag: "1"`.
3. That tells you the first attempt landed at version 1 — nothing was re-issued. Carry on from `"1"`.

Likewise, changing with a stale version (`If-Match: "1"` after someone moved it to 2) is `412` with
the current `ETag: "2"`: re-read the flight, decide again, and retry with `"2"`.

### What each refusal means

| Response | Meaning | What to do |
| --- | --- | --- |
| `403 forbidden` | the FCA or airport is in an ARTCC outside your credential's scope | write only where your credential is scoped |
| `409 held_by_person` | a controller holds it | leave it; people win |
| `409 held_by_other_machine` | another tool holds it | leave it; coordinate out of band |
| `412 precondition_failed` | it isn't at the version you named; the current one is in `ETag` | re-read, decide, retry with the new version |
| `428 precondition_required` | you sent no precondition | send `If-None-Match: *` or `If-Match` |
| `400 bad_request` | a weak tag (`W/"3"`), a list, or an unparseable value | send exactly one strong version |

## Errors, pagination and rate limits

**Every error has the same body**, a single machine-readable code:

```json
{ "error": "precondition_failed" }
```

| Status | `error` |
| --- | --- |
| 400 | `bad_request` |
| 401 | `unauthorized` — no token, a bad one, **or a token without the permission the endpoint needs** |
| 403 | `forbidden` — authenticated, but not allowed this particular thing (e.g. another facility's data) |
| 404 | `not_found` |
| 409 | `conflict`, or a specific reason such as `held_by_person` |
| 412 / 428 | `precondition_failed` / `precondition_required` |
| 413 | `payload_too_large` |
| 429 | `too_many_requests` |
| 503 | `service_unavailable` — e.g. no live VATSIM feed yet; retry shortly |

Note that a missing permission is `401`, not `403`. Which permission each endpoint needs is listed in
[API permissions](./api-permissions.md).

**Paged lists** take `page` (from 1) and `page_size` (default 25 or 50, at most 100) and answer
`{ "items": [...], "total": …, "page": …, "page_size": … }`.

**Rate limits** are per credential (by default, 300 requests a minute for a key or service account). Every
response says where you stand in `RateLimit-*` headers; a `429` carries `Retry-After`. Details:
[API keys § Rate limits](./api-keys.md#rate-limits).

## Realtime and the job queue

**Live updates.** Rather than polling, hold a websocket open and be told the moment something
changes, then fetch the new data as usual — see
[API keys § Live updates](./api-keys.md#live-updates-websocket).

**The job queue** is for a service account that carries out work OIS hands off (the Discord bot is
the first). It leases pending jobs, does them, and acknowledges each:

```bash
curl -X POST -H "Authorization: Bearer $OIS_TOKEN" "https://<your-ois-host>/api/v1/integration/jobs/lease?limit=10"
curl -X POST -H "Authorization: Bearer $OIS_TOKEN" -H "Content-Type: application/json" -d '{"success":true}' https://<your-ois-host>/api/v1/integration/jobs/{id}/ack
```

Only a service account can lease, and it only ever sees **its own** jobs: the consumer is the
account's key, so naming another one is refused. An acknowledgement for a lease you no longer hold is
`404`.

## Supported surface and stability

**Nothing is stable before version 1.0.** Until then an endpoint may change without notice; changes
are recorded in the [API changelog](./api-changelog.md).

From 1.0, the operations OIS supports for integrators are frozen, and retiring one takes at least 30
days' notice: its responses carry a `Deprecation` header (the date it was deprecated) and a `Sunset`
header (the date it stops working). Watch for them.

::: info Coming with #586
Which reads are deliberately public — answering without a token — is being settled in
[#586](https://github.com/VATUSA/OIS/pull/630). This section will list them once it lands.
:::
