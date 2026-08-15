#!/bin/sh
# Runs at container start (nginx image's /docker-entrypoint.d). Emits the runtime API base the
# SPA reads (window.__OIS_API_URL__) from OIS_API_URL. Empty OIS_API_URL → same-origin.
set -e
printf 'window.__OIS_API_URL__ = "%s";\n' "${OIS_API_URL:-}" > /usr/share/nginx/html/config.js
