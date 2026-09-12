# syntax=docker/dockerfile:1
#
# ois-docs — the VitePress user documentation, built to static HTML and served by nginx.
# Build context is the repo root: `docker build -f deploy/docs.Dockerfile .`

FROM node:22-bookworm-slim AS builder
RUN corepack enable
WORKDIR /app
COPY . .
RUN pnpm install --frozen-lockfile
# Full version (1.0.1-<sha>) surfaced in the docs nav; CI sets it, else the config reads VERSION.
ARG OIS_VERSION=""
ENV OIS_VERSION=$OIS_VERSION
RUN pnpm --filter docs-site build

FROM nginx:1-alpine AS runtime
# Patch OS packages to the latest in the Alpine 3.x branch repos so fixed CVEs in the base image
# (e.g. util-linux/libuuid) don't ship — the pinned nginx tag lags the repos. Rebuilds re-apply it.
RUN apk upgrade --no-cache
COPY deploy/docs-nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=builder /app/docs-site/.vitepress/dist /usr/share/nginx/html
EXPOSE 80
