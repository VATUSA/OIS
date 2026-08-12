# OIS monorepo task runner. Cross-language wrapper over cargo (Rust) + pnpm/turbo (JS).
set shell := ["bash", "-cu"]

# List available recipes
default:
    @just --list

# --- local infra ---
# Start Postgres (and other deps) for local development
up:
    docker compose up -d postgres

# Stop local infra
down:
    docker compose down

# Wipe local infra + volumes (fresh DB)
reset:
    docker compose down -v && docker compose up -d postgres

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

# --- everything ---
# Full local stack: infra + backend + web + bot
dev: up
    @echo "start backend/web/bot in separate terminals: just backend | just web | just bot"

# CI-equivalent local validation
ci: fmt-check check test-rust
    pnpm lint && pnpm typecheck
