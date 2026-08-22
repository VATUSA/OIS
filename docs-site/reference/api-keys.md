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

The full API is described by the OpenAPI document at **`/docs/api/v1/openapi.json`** — point your client generator at it.

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

## Oversight & auditing

Everything a key does is **audited**: every change to a key (create, rotate, permission edit, disable, delete) and every action a key performs is recorded with the key identified. Administrators with the oversight permissions can view **every** key across the platform and revoke any of them.

## See also

- [Roles & permissions](/reference/permissions) — how the permission grants a key can hold work.
- [Signing in](/introduction/signing-in) — browser sign-in for the app itself.
