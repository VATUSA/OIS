#!/bin/sh
# Runs at container start (nginx image's /docker-entrypoint.d). Emits the runtime config the SPA
# reads: the API base (window.__OIS_API_URL__ from OIS_API_URL; empty → same-origin) and the docs
# site URL (window.__OIS_DOCS_URL__ from DOCS_URL; empty → no docs link in the nav).
set -e
{
  printf 'window.__OIS_API_URL__ = "%s";\n' "${OIS_API_URL:-}"
  printf 'window.__OIS_DOCS_URL__ = "%s";\n' "${DOCS_URL:-}"
} > /usr/share/nginx/html/config.js
