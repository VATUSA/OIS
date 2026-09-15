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
   subdomain mapping.

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

## The Discord bot (optional)

The bot is a separate compose service, opt-in via a profile:

```bash
docker compose --profile discord up -d
```

It builds from `deploy/discord.Dockerfile` the same way backend/web/docs do, has no database
access, and reaches the backend over the compose network (`OIS_API_BASE=http://backend:3000`).
`.env`'s discord section documents `DISCORD_BOT_TOKEN`, `OIS_API_TOKEN` (a service-account token,
not a user key), and `OIS_POLL_SECS`. Leaving the profile off (the default) runs OIS without it.

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
