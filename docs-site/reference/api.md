# Using the API

Everything the OIS web app does goes through a public, versioned HTTP API under `/api/v1`. You can call
it from your own tools.

## The API description

The API is described by an **OpenAPI 3** document. It is the source of truth for every endpoint, request
body and response:

- **Spec:** `https://<your-ois-host>/docs/api/v1/openapi.json`. Point a client generator at it, or
  import it into Postman or Insomnia.

### Swagger UI

- **Interactive reference:** `https://<your-ois-host>/docs/swagger`. Browse every endpoint, see its
  parameters and schemas, and try requests from the browser.

## Authenticating

Send a bearer token on the `Authorization` header:

```bash
curl -H "Authorization: Bearer ois_pat_xxxxxxxx…" \
  https://<your-ois-host>/api/v1/flow/fcas
```

There are two kinds of token:

- **API keys** (`ois_pat_…`) belong to a person and can never do more than that person can. Use one
  for your own scripts. See [API keys](/reference/api-keys).
- **Service accounts** (`ois_sa_…`) belong to an integration rather than a person, so they don't stop
  working when someone's access changes. An administrator creates them.

## What a token may call

Each endpoint requires a permission. [API permissions](/reference/api-permissions) lists every endpoint
with the permission it needs. It is generated from the API itself.

A short example: list the flow constrained areas, then read one FCA's metered traffic.

```bash
curl -H "Authorization: Bearer $OIS_TOKEN" https://<your-ois-host>/api/v1/flow/fcas
curl -H "Authorization: Bearer $OIS_TOKEN" https://<your-ois-host>/api/v1/flow/fcas/{id}/traffic
```

::: tip Examples here are tested
Every API example in these docs is checked against the real API in CI, so a renamed endpoint can't
leave a stale example behind.
:::

For a quickstart, choosing between an API key and a service account, the release workflow end to
end, and errors, paging and rate limits, see [Integrating with OIS](./integrating.md).
