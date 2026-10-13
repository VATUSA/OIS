#!/bin/bash
set -uo pipefail

# ==============================================================================
# Provision an OIS worktree
# ==============================================================================
# The counterpart to bin/teardown-worktree.sh. Without it, every worktree gets
# provisioned by hand and the omissions are silent: no .env makes every
# #[sqlx::test] fail with `DATABASE_URL must be set`, no node_modules makes the JS
# half die with `turbo: command not found`, and both read as branch regressions
# rather than as a half-built environment (.claude/rules/ticket-lifecycle.md → "Worktrees").
#
# Idempotent. Re-run it on an existing worktree to repair drift — that is why
# there is no separate resume script.
#
# It creates nothing in the shared dev database. It names the throwaway database
# this worktree may create to boot its own backend (bin/lib/portable.sh
# worktree_db_name), so bin/teardown-worktree.sh can drop it.
#
# Usage:
#   bash bin/setup-worktree.sh feat+47+save-event-replays            # .claude/worktrees/<NAME>
#   bash bin/setup-worktree.sh ../ois-wt/feat/47/save-event-replays  # any worktree, by path
#   bash bin/setup-worktree.sh                                       # the checkout you are in
# ==============================================================================

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

fatal() { echo -e "${RED}FATAL:${NC} $1" >&2; exit 1; }
step()  { echo -e "\n${CYAN}==>${NC} $1"; }
ok()    { echo -e "  ${GREEN}ok${NC} $1"; }
warn()  { echo -e "  ${YELLOW}warn${NC} $1"; }

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/portable.sh
. "$SCRIPT_DIR/lib/portable.sh"

TARGET="${1:-.}"
# A bare name is EnterWorktree's: .claude/worktrees/<NAME> in the main checkout, `+` standing
# for the branch's `/`. `[[ =~ ]]`, not `echo | grep`, which matches per line.
if [[ "$TARGET" =~ ^[A-Za-z0-9_+-]+$ ]] && MAIN_ROOT="$(main_repo_root)" && [[ -d "$MAIN_ROOT/.claude/worktrees/$TARGET" ]]; then
    TARGET="$MAIN_ROOT/.claude/worktrees/$TARGET"
fi
[[ -d "$TARGET" ]] || fatal "No worktree at ${TARGET}
Create it first — prefer the EnterWorktree tool over raw git; see .claude/commands/start.md."
WORKTREE_DIR="$(git -C "$TARGET" rev-parse --show-toplevel 2>/dev/null)" || fatal "${TARGET} is not inside a git checkout."
cd "$WORKTREE_DIR" || fatal "Cannot enter ${WORKTREE_DIR}"
PROJECT_ROOT="$(main_repo_root)" || fatal "Not inside a git repository."

# Linked, not the main checkout: the main checkout keeps its own .env and is never
# provisioned from itself.
if [[ "$(git rev-parse --absolute-git-dir)" == "$(git rev-parse --path-format=absolute --git-common-dir)" ]]; then
    fatal "${WORKTREE_DIR} is the main checkout, not a linked worktree."
fi
DB_NAME="$(worktree_db_name "$WORKTREE_DIR")" || fatal "Cannot derive the throwaway database name for ${WORKTREE_DIR}."

# -- Mirror everything to a log, so a failed run is never silent --
# .claude/ is gitignored apart from the shared workflow files.
mkdir -p "$WORKTREE_DIR/.claude"
LOG="$WORKTREE_DIR/.claude/worktree-setup.log"
exec > >(tee -a "$LOG") 2>&1

echo -e "${YELLOW}Provisioning worktree:${NC} ${CYAN}${WORKTREE_DIR}${NC}"
echo "  branch: $(git symbolic-ref --quiet --short HEAD 2>/dev/null || echo '(detached)')"
echo "  main:   ${PROJECT_ROOT}"

# -- .env --
step ".env"
if [[ -f .env ]]; then
    ok ".env already present (leaving it alone)"
else
    [[ -f "$PROJECT_ROOT/.env" ]] || fatal "No .env in the main checkout to copy from (copy .env.example there and fill it in)."
    cp "$PROJECT_ROOT/.env" .env || fatal "Could not copy ${PROJECT_ROOT}/.env"
    ok "copied from the main checkout"
fi
grep -qE '^DATABASE_URL=.+' .env || fatal ".env sets no DATABASE_URL; every #[sqlx::test] needs it (see .env.example)."
ok ".env sets DATABASE_URL"

# -- node_modules --
#
# Installed here, never linked: a symlinked node_modules runs this branch's tests
# against the packages another checkout's lockfile names. --prefer-offline reuses
# the pnpm store, so a worktree install takes seconds when nothing changed.
step "node_modules"
command -v pnpm >/dev/null 2>&1 || fatal "pnpm not found (corepack enable, or brew install pnpm)."
if [[ -L node_modules ]]; then
    warn "node_modules was a symlink — replacing it with this worktree's own install"
    rm node_modules
fi
pnpm install --frozen-lockfile --prefer-offline || fatal "pnpm install --frozen-lockfile failed — output above"
ok "pnpm install --frozen-lockfile done"

# -- Cargo target dir and sccache --
#
# Each worktree builds into its own target/. A single target dir shared across
# worktrees is unsafe: cargo hashes path packages workspace-relative and judges their
# freshness by mtime, so one worktree ran another's test binary and passed on code it
# did not have. To skip the cold build, a missing target/ is seeded with a
# copy-on-write clone of the primary checkout's (APFS clonefile on macOS, reflink on
# Linux): seconds, and no extra disk until either side writes. Never into an existing
# target/, never into the primary itself. sccache, when installed, caches the rest.
step "cargo target dir"
if [[ "$WORKTREE_DIR" == "$PROJECT_ROOT" ]]; then
    ok "the primary checkout keeps its own target/ (not seeded)"
elif [[ -e "$WORKTREE_DIR/target" ]]; then
    ok "target/ already present (not seeded)"
elif [[ ! -d "$PROJECT_ROOT/target" ]]; then
    warn "the primary checkout has no target/ to seed from: the first build here is cold"
else
    case "$(uname -s)" in
        Darwin) CLONE=(cp -c -R) ;;
        *) CLONE=(cp -R --reflink=always) ;;
    esac
    SEED_START="$(date +%s)"
    if "${CLONE[@]}" "$PROJECT_ROOT/target" "$WORKTREE_DIR/target"; then
        # Load-bearing, and it must run right after the clone: every tracked file is
        # touched so each workspace source is newer than any seeded artifact. Without
        # it cargo treats a crate whose source differs from the primary's as fresh and
        # runs the primary's stale build of it. Dependencies, keyed on their version,
        # stay reused. -c because ls-files lists index entries that may be deleted in
        # the worktree, which a plain touch recreates empty; -h touches a symlink itself.
        if git -C "$WORKTREE_DIR" ls-files -z | xargs -0 touch -c -h; then
            ok "seeded target/ from ${PROJECT_ROOT}/target in $(( $(date +%s) - SEED_START ))s (${CLONE[*]}); sources touched so workspace crates rebuild"
        else
            rm -rf "$WORKTREE_DIR/target"
            warn "could not touch the tracked files after seeding, so the seed was removed: the first build here is cold"
        fi
    else
        rm -rf "$WORKTREE_DIR/target"
        warn "could not clone ${PROJECT_ROOT}/target (${CLONE[*]} failed: no copy-on-write on this filesystem?); removed the partial copy, the first build here is cold"
    fi
fi
export_build_env
if [[ -n "${RUSTC_WRAPPER:-}" ]]; then
    ok "sccache found: RUSTC_WRAPPER=${RUSTC_WRAPPER}"
    echo "  For a shell of your own, export:"
    echo "    export RUSTC_WRAPPER=${RUSTC_WRAPPER}"
else
    warn "sccache not installed — builds work, uncached across worktrees (optional: brew install sccache)"
fi

# -- Throwaway database --
#
# The shared dev database carries other branches' migrations, so this branch's
# backend boots against its own database (.claude/rules/database-postgres.md).
# Named here, created only when needed; bin/teardown-worktree.sh drops it.
step "throwaway database"
ok "name: ${DB_NAME} (not created; nothing is written to the shared dev database)"
DEV_URL="$(grep -E '^DATABASE_URL=' .env | tail -1 | cut -d= -f2- | sed -E 's/[[:space:]]+#.*$//; s/^"(.*)"$/\1/')"
DEV_URL="${DEV_URL%%\?*}"
DEV_USER="${DEV_URL#*://}"; DEV_USER="${DEV_USER%%@*}"; DEV_USER="${DEV_USER%%:*}"
echo "  To boot this branch's backend against it (.claude/rules/database-postgres.md):"
echo "    docker exec ois-postgres-1 psql -U ${DEV_USER} -d postgres -c 'create database ${DB_NAME};'"
echo "    DATABASE_URL=${DEV_URL%/*}/${DB_NAME} BIND_ADDR=127.0.0.1:3407 cargo run -p ois-backend"

# -- Health check --
step "health check"
if bash "$SCRIPT_DIR/verify-worktree.sh"; then
    echo -e "\n${GREEN}Worktree ${CYAN}${WORKTREE_DIR}${GREEN} is ready.${NC}"
    exit 0
fi

echo -e "\n${RED}Provisioning finished but the health check failed.${NC}"
echo -e "${YELLOW}Read the log:${NC} ${LOG}"
exit 1
