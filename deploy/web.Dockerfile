# syntax=docker/dockerfile:1
#
# ois-web — the built SPA served by nginx. The API is cross-origin (its own subdomain), so the
# bundle carries no baked API URL; instead deploy/40-ois-config.sh writes /config.js from
# OIS_API_URL / DOCS_URL at container start, so one image works in every environment. Build
# context is the repo root: `docker build -f deploy/web.Dockerfile .`

FROM node:22-bookworm-slim AS builder
RUN corepack enable
WORKDIR /app
COPY . .
RUN pnpm install --frozen-lockfile
# Relative API base (see header). @ois/ui and @ois/api-client are consumed as source, so only
# the web package needs building.
ENV VITE_OIS_API_URL=""
RUN pnpm --filter web build

FROM nginx:1-alpine AS runtime
# Static SPA-serving config (no API proxy — the API is cross-origin).
COPY deploy/nginx.conf /etc/nginx/conf.d/default.conf
# Writes /config.js from OIS_API_URL / DOCS_URL at startup so one image serves any environment.
COPY deploy/40-ois-config.sh /docker-entrypoint.d/40-ois-config.sh
RUN chmod +x /docker-entrypoint.d/40-ois-config.sh
COPY --from=builder /app/web/dist /usr/share/nginx/html
# The API origin the SPA calls (e.g. https://api.ois.vzdc.org). Empty = same-origin.
ENV OIS_API_URL=""
# Public URL of the docs site (e.g. https://docs.ois.vzdc.org). Empty hides the nav docs link.
ENV DOCS_URL=""
EXPOSE 80
