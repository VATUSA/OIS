# OIS monorepo task runner. Cross-language wrapper over cargo (Rust) + pnpm/turbo (JS).
set shell := ["bash", "-cu"]

# Each checkout builds into its own target/ (bin/setup-worktree.sh seeds a worktree's from the
# main checkout's by a copy-on-write clone); one target dir shared across worktrees ran stale test
# binaries. sccache wraps rustc when it is installed (an empty RUSTC_WRAPPER means none). A value
# already in the environment wins. bin/lib/portable.sh sets the same for the bin/ scripts and git
# hooks.
export RUSTC_WRAPPER := env("RUSTC_WRAPPER", `command -v sccache >/dev/null 2>&1 && echo sccache || true`)

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
# ready" — migrations run before the API's TCP listener binds, so on a fresh DB or a
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

# Regenerate the API permission map (docs-site/reference/api-permissions.md) from the handlers
docs-permissions:
    OIS_REGENERATE_DOCS=1 cargo test -p ois-backend --lib docs_tests::the_published_permission_map_is_current

# --- everything ---
# Full local stack: infra + backend + web + bot
dev: up
    @echo "start backend/web/bot in separate terminals: just backend | just web | just bot"

# Fail on two migrations sharing a version — sqlx would apply both and half-migrate the DB (#569)
check-migrations:
    .github/scripts/check-migration-versions.sh

# Not what CI runs: it skips clippy, doc tests, pnpm test, the audits and the client-drift check,
# which `just test` and `just ci-full` add.
# Fast local validation (a subset of CI; see test)
ci: check-migrations fmt-check check test-rust
    pnpm lint && pnpm typecheck

# The full local gate, once per commit: every job .github/workflows/ci.yml runs that can run here
# (bin/run-tests.sh lists them), each step's result, then a summary. pnpm audit and cargo deny are
# ADVISORY there: printed, never blocking the stamp (CI's js and deny jobs block the merge). A green
# run on a clean committed HEAD stamps the tests marker that the PR gate and bin/pre-push accept for
# that commit.
# With arguments it is a targeted run and never stamps:
#   just test rust -p ois-backend -E 'test(scope_)'     (cargo nextest run ...)
#   just test web web src/lib/time.test.ts              (pnpm --filter web exec vitest run ...)
# Everything CI runs, in one go; stamps the tests marker when green and clean
[positional-arguments]
test *args:
    bash bin/run-tests.sh "$@"

# The same steps as `just test` (bin/run-tests.sh, so the two never drift), without the marker.
# A step it cannot run here is MISSING, never passed. Not covered: the macOS/Windows/Linux
# `desktop` matrix (clippy compiles the Tauri shell for this host only) and the `windows-hooks`
# job (bin/lib/file-lock.test.sh on Windows).
# Everything .github/workflows/ci.yml runs that can run on this machine
ci-full:
    bash bin/run-tests.sh --no-stamp

# .githooks/: pre-commit (bin/pre-commit: rustfmt, debug calls, gitleaks, migration versions),
# commit-msg (no AI attribution), pre-push (bin/pre-push: branch name, clippy and the suites unless
# `just test` stamped the commit; cargo deny and pnpm audit as warnings, which CI's jobs enforce).
# Run once per clone; every worktree shares the setting (bin/verify-worktree.sh fails without it).
# Enable the repo's git hooks (core.hooksPath=.githooks)
setup:
    git config core.hooksPath .githooks
    @echo "git hooks enabled: core.hooksPath=$(git config core.hooksPath)"
