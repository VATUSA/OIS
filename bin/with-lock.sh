#!/bin/bash
# ==============================================================================
# with-lock.sh — serialize heavy, machine-global work across parallel worktrees.
#
#   bin/with-lock.sh postgres -- <command...>
#
# The box runs several git worktrees at once, and they share one Postgres. Every
# `#[sqlx::test]` creates and drops its own database on that server, and two full
# nextest runs at once contend for it and flake inside sqlx-core, never in an
# assertion: `database "_sqlx_test_…" does not exist`, `PoolTimedOut`, `already
# exists`, on a different unrelated test each run (.claude/rules/test-quality.md).
# bin/run-tests.sh runs the full Rust suite under this lock.
#
# A waiter queues and prints who it is waiting on.
#
# clippy, a targeted nextest run (bin/run-tests.sh rust ...) and the JS suite run
# UNLOCKED — the queue is deliberately kept to work that is genuinely
# machine-global.
#
# Escape hatches:
#   OIS_NO_LOCK=1       run the command with no lock at all
#   CI (any non-empty)  auto-skip — CI runners are isolated, nothing to contend
#   OIS_LOCK_TIMEOUT    seconds to wait before giving up (0 = wait forever).
#                       Defaults are per-resource; see the case below.
#   OIS_LOCK_DIR        override the lock directory. Exists so the test suite can
#                       take real locks without contending with live runs.
#
# Portable across macOS (BSD userland, /bin/bash 3.2) and Linux.
# Tests: bin/lib/flock-run.test.sh
# ==============================================================================
set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/portable.sh
. "$SELF_DIR/lib/portable.sh"

usage() {
    echo "usage: bin/with-lock.sh postgres -- <command...>" >&2
    exit 2
}

RESOURCE="${1:-}"
[[ -n "$RESOURCE" ]] || usage
shift
[[ "${1:-}" == "--" ]] || usage
shift
[[ $# -gt 0 ]] || usage

# Doubles as the allowlist, so a new class cannot be accepted without declaring
# how long a waiter should queue for it.
case "$RESOURCE" in
    postgres) DEFAULT_TIMEOUT=2700 ;;
    *) echo "with-lock: unknown resource '$RESOURCE' (expected postgres)" >&2; exit 2 ;;
esac

# Isolated environments have nothing to serialize — run straight through.
if [[ -n "${OIS_NO_LOCK:-}" || -n "${CI:-}" ]]; then
    exec "$@"
fi

# No perl means no portable flock. Warn, but never block the work over it.
if ! command -v perl >/dev/null 2>&1; then
    echo "with-lock: perl not found — running WITHOUT a lock (contention possible)" >&2
    exec "$@"
fi

# Anchor the lock on the MAIN checkout, so every worktree of this clone shares it
# (main_repo_root resolves via --git-common-dir and is identical from any
# worktree). Keyed by a hash of that path so a second, unrelated clone gets its
# own locks rather than blocking on ours.
REPO_ROOT="$(main_repo_root)" || { echo "with-lock: not inside a git repository" >&2; exit 2; }
REPO_ID="$(printf '%s' "$REPO_ROOT" | cksum | cut -d' ' -f1)"

# /tmp is a local filesystem on macOS and Linux, never a network mount, where
# flock is unreliable.
LOCK_DIR="${OIS_LOCK_DIR:-/tmp/ois-locks-${REPO_ID}}"
mkdir -p "$LOCK_DIR"
LOCK_FILE="$LOCK_DIR/$RESOURCE.lock"

TIMEOUT="${OIS_LOCK_TIMEOUT:-$DEFAULT_TIMEOUT}"

exec perl "$SELF_DIR/lib/flock-run.pl" "$LOCK_FILE" "$TIMEOUT" "$RESOURCE" -- "$@"
