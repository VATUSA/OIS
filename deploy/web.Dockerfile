# syntax=docker/dockerfile:1
#
# ois-web — the built SPA served by nginx, which also reverse-proxies the API. The bundle is
# built with an empty VITE_OIS_API_URL so the app calls its own origin (relative URLs) and
# nginx forwards /api, /health, /docs to the backend — one image works in every environment,
# and the session cookie stays same-origin. Build context is the repo root:
# `docker build -f deploy/web.Dockerfile .`

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
# Templated at container start via the nginx image's envsubst (only ${BACKEND_URL} is defined,
# so nginx's own $uri/$host are left intact).
COPY deploy/nginx.conf /etc/nginx/templates/default.conf.template
# Writes /config.js from OIS_API_URL at startup so one image serves any environment.
COPY deploy/40-ois-config.sh /docker-entrypoint.d/40-ois-config.sh
RUN chmod +x /docker-entrypoint.d/40-ois-config.sh
COPY --from=builder /app/web/dist /usr/share/nginx/html
# The API base the SPA calls. Set to the API's public origin for a cross-origin deploy
# (e.g. https://api-ois.vzdc.org), or leave empty for same-origin (SPA uses /api on its host).
ENV OIS_API_URL=""
# Where nginx forwards same-origin /api traffic (used only when OIS_API_URL is empty).
ENV BACKEND_URL=http://backend:3000
EXPOSE 80
