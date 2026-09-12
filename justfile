# OIS monorepo task runner. Cross-language wrapper over cargo (Rust) + pnpm/turbo (JS).
set shell := ["bash", "-cu"]

# List available recipes
default:
    @just --list

# --- local infra ---
# Start Postgres only (for hot-reload dev: `just backend` / `just web` run natively)
up:
    docker compose up -d postgres

# Stop the stack
down:
    docker compose down

# Wipe infra + volumes (fresh DB)
reset:
    docker compose down -v && docker compose up -d postgres

# Full stack in containers, built locally (mirrors prod; same compose, dev .env values)
stack:
    docker compose up --build -d

# Pull the CI-built images and run them (prod-style: needs prod values in .env)
deploy:
    docker compose pull && docker compose up -d
    just smoke

# Post-deploy health check: retries briefly (the container needs a moment to start), fails loudly
# (non-zero exit, real reason printed) if the API never comes up or its DB probe reports down —
# /health always returns HTTP 200 even when the database is unreachable, so this checks the body.
smoke:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -f .env ]; then set -a; . ./.env; set +a; fi
    url="http://${BIND_HOST:-127.0.0.1}:${API_PORT:-3000}/health"
    for i in $(seq 1 10); do
      if body="$(curl -fsS "$url" 2>/dev/null)"; then
        if [ "$(echo "$body" | jq -r .status)" = "ok" ] && [ "$(echo "$body" | jq -r .database)" = "true" ]; then
          echo "smoke: healthy — $body"; exit 0
        fi
        echo "smoke: unhealthy response — $body" >&2; exit 1
      fi
      sleep 2
    done
    echo "smoke: $url never became reachable" >&2; exit 1

# --- rust (backend, discord, crates) ---
check:
    cargo check --workspace --all-targets

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

test-rust:
    cargo test --workspace --all-targets -- --test-threads=1

# Run the backend API (migrations run on startup)
backend:
    cargo run -p ois-backend

# Run the discord bot
bot:
    cargo run -p ois-discord

# --- js (web, packages) ---
web:
    pnpm --filter web dev

test-js:
    pnpm test

# --- docs (VitePress user docs) ---
# Run the docs site dev server (live reload)
docs:
    pnpm --filter docs-site dev

# Build the static docs site
docs-build:
    pnpm --filter docs-site build

# --- everything ---
# Full local stack: infra + backend + web + bot
dev: up
    @echo "start backend/web/bot in separate terminals: just backend | just web | just bot"

# CI-equivalent local validation
ci: fmt-check check test-rust
    pnpm lint && pnpm typecheck
