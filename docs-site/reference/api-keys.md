# API keys

**API keys** (personal access tokens) let you — or a tool you build — call the OIS API with your own front end or integration, instead of signing in through the browser each time. A key authenticates as **you**, and it can never do more than you can.

## Getting a key

Creating keys needs the **`api_keys.key.create`** permission — ask an administrator to grant it. Once you have it, an **API keys** entry appears in your user menu.

To create one:

1. Open **API keys** from your user menu and choose **New key**.
2. Give it a **name** (and optional description) so you can recognize it later.
3. Optionally set an **expiry**.
4. Pick the **permissions** the key should have. You can only choose from what *you* currently hold, and you can scope each one **nationally or to specific ARTCCs** (again, only where you have it).
5. **Create** — the token is shown **once**. Copy it now and store it somewhere safe.

::: warning The token is shown once
For your security, OIS only ever displays the full token at creation (and when you rotate it). It's stored hashed — nobody, including an administrator, can recover it. If you lose it, rotate the key to get a new one.
:::

## Using a key

Send the token as a **bearer token** on the `Authorization` header:

```bash
curl -H "Authorization: Bearer ois_pat_xxxxxxxx…" \
  https://<your-ois-host>/api/v1/flow/traffic
```

The full API is described by the OpenAPI document at **`/docs/api/v1/openapi.json`** — point your client generator at it — and you can browse and try it in **Swagger UI** at **`/docs/swagger`**. See [Using the API](/reference/api), and [API permissions](/reference/api-permissions) for what each endpoint requires.

### Rate limits

Each key has its own allowance — **300 requests per minute** by default, available as a burst and
refilling evenly. Every response tells you where you stand:

| Header | Meaning |
| --- | --- |
| `RateLimit-Limit` | requests per minute this key is allowed |
| `RateLimit-Remaining` | requests left right now |
| `RateLimit-Reset` | seconds until the full allowance is back |

Go over it and you get **`429 Too Many Requests`** with a **`Retry-After`** header: wait that many
seconds before trying again — retrying sooner is simply refused again. Polling an endpoint more often
than its data changes (live traffic updates about every 15 seconds) only spends your allowance.

::: tip User keys vs. service accounts
A `ois_pat_…` token is a **user** key, owned by and capped to a person. Machine clients that aren't tied to a person (a bot, shared tooling) use **service accounts** (`ois_sa_…`), which an administrator manages separately.
:::

## A key is capped by your access

This is the most important thing to understand: a key's authority is **your granted permissions on the key, intersected with your live account access** — checked on every request.

- A key can never hold a permission, or reach an ARTCC, that you don't.
- If your own access is later **reduced** — a role removed, a scope narrowed, your account deactivated — every key you own **immediately loses** that access too.
- A key can **never** manage keys (it can't create, read, or revoke keys), so a leaked key can't be used to mint more.

So the safe habit is to grant each key the **least** it needs.

## Managing your keys

From the same page you can, per key:

- **Rotate** — issue a new token and invalidate the old one (use this if a key may have leaked).
- **Edit permissions** — change what it can do (still bounded by your access).
- **Disable** — turn it off without deleting it.
- **Delete** — remove it permanently.
- **Activity** — see what the key has done, from the audit log.

## Live updates (websocket)

Rather than polling, a key or service account can hold a websocket open and be told the moment
something changes, then fetch the new data over the API as usual.

**Connect** to `wss://<your-ois-host>/api/v1/ws` with your token, either as a header (server-side
clients):

```http
Authorization: Bearer ois_pat_xxxxxxxx…
```

or, from a browser — whose `WebSocket` can't set headers — as subprotocols:

```js
new WebSocket("wss://<your-ois-host>/api/v1/ws", ["ois.v1", "ois.bearer.ois_pat_xxxxxxxx…"]);
```

The server selects `ois.v1` and never echoes the token back. A missing, revoked or expired
credential is refused with `401` before the upgrade. Never put the token in the URL — URLs end up
in logs.

**Messages are nudges, not data.** Each one is just the name of what changed:

```json
{ "topic": "flow.release" }
```

When you get one, re-fetch the matching resource — the API, with your key's own access, stays the
only source of the data. A nudge never says *what* changed or for whom.

| Topic | Changed |
| --- | --- |
| `flow.release` | departure releases |
| `flow.cfr` | call-for-release requests |
| `flow.fca` | flow constrained areas |
| `tmu.gdp` | ground delay programs |
| `tmu.tmi` | traffic management initiatives |
| `tmu.groundstop` | ground stops |
| `tmu.program` | rate programs |
| `events.availability` | event availability responses (NTMO / DCC staff) |
| `events.reminder` | ACE claim reminders |
| `access.granted` | someone's access changed (re-check your own) |

**Choose your topics.** Until you say otherwise you receive every topic. Send a subscribe message to
receive only some — it replaces your current set:

```json
{ "subscribe": ["flow.release"] }
```

The server answers `{"subscribed":["flow.release"]}`. A name it doesn't know is refused as a whole —
`{"error":"unknown_topic","topics":["flow.releases"]}` — and your previous subscription stays in
place. Anything else that isn't a subscribe message gets `{"error":"bad_request"}`.

**Stay connected.** The server pings every 30 seconds; standard clients answer automatically. If the
connection drops, reconnect with exponential backoff (for example 1 s doubling to a 60 s cap, with
random jitter) and **send your subscribe message again** — subscriptions don't survive a reconnect.
Nudges sent while you were disconnected are not replayed, so after reconnecting, fetch once to catch
up. Polling remains the fallback: if you can't hold a socket, poll no faster than the data changes.

## Oversight & auditing

Everything a key does is **audited**: every change to a key (create, rotate, permission edit, disable, delete) and every action a key performs is recorded with the key identified. Administrators with the oversight permissions can view **every** key across the platform and revoke any of them.

## See also

- [Roles & permissions](/reference/permissions) — how the permission grants a key can hold work.
- [Signing in](/introduction/signing-in) — browser sign-in for the app itself.
