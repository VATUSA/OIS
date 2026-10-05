# Deploying OIS

Deploy is deliberately manual — a person decides when a build goes live, on which host. This is
the `docker compose`-based path used for both the test server (`next` images) and prod (`main`
images); see [#87](https://github.com/VATUSA/OIS/issues/87) for how those two image lines are
produced and [#90](https://github.com/VATUSA/OIS/issues/90) for the scope of what's automated here
versus not.

## What's automated, what isn't

- **Automated:** every merge to `next` or `main` builds and pushes images
  (`.github/workflows/build-images.yml`); a `just deploy` runs a `/health` smoke check and fails
  loudly if the app doesn't come up (`justfile`'s `smoke` recipe); pushing a `v*` tag cuts a GitHub
  Release with auto-generated notes (`.github/workflows/release.yml`).
- **Manual, by design:** actually pulling a new image onto a host and restarting it. Nothing
  triggers a deploy automatically — that stays a deliberate action. (Triggered/remote deploy and
  Discord failure notifications are tracked as follow-up work, not built yet — they need a target
  host + credentials and a webhook URL that aren't available today.)

## One-time host setup

On the deploy host (test server or prod), besides `docker`/`docker compose`/`just`: the `smoke`
recipe also needs `curl` and `jq` (both usually already present on any Linux server image; install
them if not).

1. Copy `.env.example` to `.env` and fill in real values — see the file's own comments for what
   each one does. The same `docker-compose.yml` runs every environment; only `.env` differs.
2. Point a reverse proxy (Caddy/nginx/Cloudflare) at the three bound ports
   (`API_PORT`/`WEB_PORT`/`DOCS_PORT`) — `docker-compose.yml`'s header comment has the exact
   subdomain mapping. Set `TRUSTED_PROXY_HOPS` to the number of proxies in that chain (default 2,
   Cloudflare → Traefik; **1** behind a single Caddy/nginx), or rate limits and audit IPs key on the
   proxy's address instead of the client's.

## Deploying to the test server (`next` images)

```bash
# .env: IMAGE_TAG=next
just deploy
```

`IMAGE_TAG=next` pulls whatever a manual `workflow_dispatch` of `build-images.yml` on the `next`
branch last tagged `:next` (merging into `next` itself does **not** build images — see #87). To cut
a fresh test build, dispatch that workflow manually on `next` first, then deploy.

## Deploying to prod (`main` images)

```bash
# .env: IMAGE_TAG=latest  (or a pinned vX.Y.Z — see "Cutting a release" below)
just deploy
```

`main` merges build automatically. `IMAGE_TAG=latest` always tracks the newest `main` build;
pinning to a specific `vX.Y.Z` (below) is safer for prod if you want deploys and releases to be the
same event.

## Running more than one backend replica

Supported. Realtime nudges (the websocket's "something changed" signals) reach clients on every replica
through Postgres `LISTEN/NOTIFY` on the `ois_realtime` channel, so nothing beyond the shared database is
needed; each replica holds one extra Postgres connection for its listener. Delivery is best-effort — a
nudge lost while a listener reconnects is picked up by the clients' 60-second fallback poll.

## What `just deploy` does

```bash
docker compose pull && docker compose up -d
just smoke
```

The smoke check retries `/health` for up to ~60s by default (`SMOKE_ATTEMPTS` env var to tune it) —
covering both the API not being reachable yet and being reachable but still reporting its database
unready, since migrations run before the API's TCP listener binds and can take a while on a fresh
DB. It fails the command (non-zero exit) only once that budget is spent without a healthy response
— `/health` itself always returns HTTP 200, even when the DB is down, so a plain "did it return
200" check would miss that. A failing `just deploy` means the new containers
are already running but unhealthy; docker doesn't automatically revert.

## After a deploy: confirm the VATUSA webhook

Check that the division webhook is **registered**, not just that nothing warned (#688). A run that never
reached VATUSA warns about nothing either. The backend log should show one of:

- `VATUSA division webhook registered` (it created one), or
- `VATUSA division webhook already registered` (the stored one is still valid).

Anything else is a failure: a `VATUSA webhook:` warning, which now carries VATUSA's own response body (a
`400` names what VATUSA objected to, e.g. a bad API key), or a `not registered:` line for a missing
`OIS_PUBLIC_URL`/`OIS_SECRET_KEY`, or **no line at all**. Until it registers, roster changes reach OIS
only through the daily pull.

## The Discord bot (optional)

The bot is a separate compose service, opt-in via a profile:

```bash
docker compose --profile discord up -d
```

It builds from `deploy/discord.Dockerfile` the same way backend/web/docs do, has no database
access, and reaches the backend over the compose network (`OIS_API_BASE=http://backend:3000`).
`.env`'s discord section documents `DISCORD_BOT_TOKEN`, `OIS_API_TOKEN` (a service-account token,
not a user key), and `OIS_POLL_SECS`. Leaving the profile off (the default) runs OIS without it.

**The bot's service account key must be `discord`** (#656). The job queue hands a service account only
its own consumer's jobs, and the consumer is the account's key, so a bot whose account has another key
leases nothing. Migration `0115` renames it on deploy when exactly one active account holds a current
`BOT` grant and none is keyed `discord` yet. It leaves anything else alone (no bot account, two
candidates), and then you set it yourself in the admin UI, or with one statement:
`update access.service_accounts set key = 'discord' where id = '<the bot account id>';`

## Observability (optional)

Prometheus + Grafana ship as a **second compose file**, so a deployment that doesn't want them
never runs them. Merged `-f` files share one project and one network, which is what lets Prometheus
reach the API by service name:

```bash
docker compose -f docker-compose.yml -f docker-compose.observability.yml up -d
```

That brings up:

- **Prometheus** on `${PROMETHEUS_PORT:-9090}`, scraping `backend:3000/metrics` every 15s and
  keeping `${PROMETHEUS_RETENTION:-15d}` of history in the `ois_prometheus` volume. Config lives in
  `deploy/observability/prometheus.yml`.
- **Grafana** on `${GRAFANA_PORT:-3001}`, provisioned at boot with the Prometheus datasource and
  the committed **OIS overview** dashboard (live ops counts, API rate/latency, feed freshness, job
  health, DB pool) — no manual setup. The dashboard is
  `deploy/observability/grafana/dashboards/ois-overview.json`; edit that file to change it, since
  the provider is `allowUiUpdates: false` and the file wins on restart.

Both bind host-local via `BIND_HOST` like every other service, so front them with the same reverse
proxy (`metrics.<domain>`, `grafana.<domain>`). **Set `GRAFANA_ADMIN_PASSWORD` before the first
`up`** — it defaults to `admin`.

> **`GRAFANA_ADMIN_PASSWORD` only takes effect on Grafana's first boot.** Grafana stores its users
> in the `ois_grafana` volume and reads that variable only when it creates the default admin, so
> changing it later and recreating the container does **nothing** — the old password keeps working.
> If you already booted the stack once with the default, reset it explicitly:
>
> ```bash
> docker compose -f docker-compose.yml -f docker-compose.observability.yml \
>   exec grafana grafana-cli admin reset-admin-password '<new password>'
> ```
>
> (or drop the volume and start clean, which also discards any saved Grafana state).

One local-dev gotcha: `docker compose` only auto-merges `docker-compose.override.yml` when you pass
*no* `-f` flags. The command above passes two, so it pulls the published backend image rather than
building yours. To run the stack against a locally-built backend, name the override explicitly:

```bash
docker compose -f docker-compose.yml -f docker-compose.override.yml \
               -f docker-compose.observability.yml up -d --build
```

### Portainer (paste-the-YAML deploys)

The merge-file layout above works when Docker can see this repo's files (a local checkout, or a
Portainer stack deployed **from Git**). A Portainer **web-editor** stack can't use it: its `./…`
bind-mount sources resolve on the host to Portainer's per-stack dir, which doesn't hold these
config files, so Prometheus and Grafana would come up unconfigured.

For that, use **`docker-compose.observability.portainer.yml`** — a self-contained variant that
carries every config **inline** as Docker `configs` (no bind mounts, no files on the host) and joins
the base stack's network as **external** instead of being a merge fragment. Deploy it as its own
Portainer stack:

1. Deploy the base OIS stack first — it creates the shared network (`<base-stack-name>_default`).
2. **New stack → Web editor →** paste `docker-compose.observability.portainer.yml`.
3. Set env: `OIS_NETWORK` to the base stack's network name (default `ois_default`),
   `GRAFANA_ADMIN_PASSWORD`, and the ports if you don't want the defaults.

Requires a Compose with inline `configs` content (v2.23.1+); Portainer 2.19+ ships one.

That file is **generated** from the `deploy/observability/*` sources (the editable source of truth,
still used by the merge-file stack above) — never hand-edit it. After changing any source config or
the dashboard, regenerate it so the two can't drift:

```bash
deploy/observability/gen-portainer-compose.sh
```

### Securing `/metrics`

`GET /metrics` is deliberately **not** in the OpenAPI spec or the typed client: it is text
exposition for Prometheus, not JSON the SPA consumes, so a change there never needs a client regen.

The observability stack publishes **no new port** for it — Prometheus scrapes it in-network at
`backend:3000/metrics`. But it is a route on the API's own listener, so **anyone who can reach the
API can scrape it**, including through a public `api.<domain>` proxy. It exposes operational
numbers (traffic counts, active TMIs, job health, build version), not user data, but on a publicly
proxied API you should either block `/metrics` at the proxy or set a token:

1. Set `METRICS_TOKEN` in `.env` and recreate the backend. The endpoint then answers `401` without
   `Authorization: Bearer <token>`.
2. Prometheus does not expand environment variables in its own config, so the token has to reach it
   as a **file**: write the same value to `deploy/observability/metrics_token` (gitignored — it is
   a credential), uncomment that volume line in `docker-compose.observability.yml`, and uncomment
   the `authorization` block in `deploy/observability/prometheus.yml`.

Do both or neither: setting `METRICS_TOKEN` without step 2 leaves Prometheus scraping with no
credential, and the target goes `DOWN` with `server returned HTTP status 401 Unauthorized`.

## Rolling back

Set `.env`'s `IMAGE_TAG` back to the previous known-good value (a prior `vX.Y.Z`, or the previous
image digest if you tagged one) and run `just deploy` again.

## Cutting a release

```bash
# bump VERSION, commit it on next, promote to main, then:
git tag v1.2.0
git push origin v1.2.0
```

The tag push triggers two independent workflows: `build-images.yml` builds and pushes prod images
tagged with that version, and `release.yml` creates a GitHub Release with notes auto-generated from
merged PR titles since the last tag.
