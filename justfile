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

# Post-deploy health check: retries through both "not reachable yet" and "reachable but DB not
# ready" — migrations (63 today) run before the API's TCP listener binds, so on a fresh DB or a
# heavy migration the port can take a while to open, and a container can start answering /health
# before its own DB pool's first connection lands. Only fails loudly (non-zero exit, real reason
# printed) once the full budget is spent — /health always returns HTTP 200 even when the database
# is unreachable, so this checks the body, not just the status code.
smoke:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -f .env ]; then set -a; . ./.env; set +a; fi
    url="http://${BIND_HOST:-127.0.0.1}:${API_PORT:-3000}/health"
    attempts="${SMOKE_ATTEMPTS:-30}"
    last=""
    for i in $(seq 1 "$attempts"); do
      if body="$(curl -fsS "$url" 2>/dev/null)"; then
        last="$body"
        if [ "$(echo "$body" | jq -r .database)" = "true" ]; then
          echo "smoke: healthy — $body"; exit 0
        fi
        echo "smoke: reachable but database not ready ($i/$attempts) — $body" >&2
      fi
      sleep 2
    done
    echo "smoke: $url never reported healthy after $((attempts * 2))s — last response: ${last:-<unreachable>}" >&2
    exit 1

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

# --- desktop (Tauri shell around the web SPA) ---
# Run the desktop app: starts the web dev server and hot-reloads it into the Tauri webview
desktop:
    pnpm --filter desktop exec tauri dev

# Build the desktop app bundle for the host platform (builds web/dist first)
desktop-build:
    pnpm --filter desktop exec tauri build

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
