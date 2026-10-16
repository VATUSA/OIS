#!/bin/bash

# ==============================================================================
# Verify a worktree is actually usable
# ==============================================================================
# Standalone health check. Run it any time; run it before believing a test
# result you got inside a worktree.
#
# The failure this exists to catch is not "the worktree is broken" — it is "the
# worktree is subtly wrong and every gate you run inside it lies to you". A fresh
# worktree has no .env, so every #[sqlx::test] fails with `DATABASE_URL must be
# set`, and no node_modules, so the JS half dies with `turbo: command not found`;
# a stale node_modules runs the tests against packages the lockfile no longer
# names (#477, #539). Each reads as a branch regression and is not one.
#
# Environment: Postgres from `docker compose up -d postgres` (just up), shared by
# every checkout; Cargo builds into each checkout's own target/ (bin/setup-worktree.sh
# seeds it from the main checkout's).
#
# Usage: bash bin/verify-worktree.sh          (from inside the worktree)
# Exit:  0 all critical checks passed, 1 otherwise.
# Tests: bin/lib/verify-worktree.test.sh
# ==============================================================================

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

PASS=0
FAIL=0
WARN=0

check_pass() { echo -e "  ${GREEN}PASS${NC} $1"; PASS=$((PASS + 1)); }
check_fail() { echo -e "  ${RED}FAIL${NC} $1"; FAIL=$((FAIL + 1)); }
check_warn() { echo -e "  ${YELLOW}WARN${NC} $1"; WARN=$((WARN + 1)); }

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/portable.sh
. "$SCRIPT_DIR/lib/portable.sh"

WORKTREE_DIR="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
NAME="$(basename "$WORKTREE_DIR")"

echo ""
echo -e "${YELLOW}Worktree health check: ${CYAN}${NAME}${NC}"
echo "-----------------------------------------------------------"

# -- 1. Is this even a linked worktree? --
#
# In a linked worktree --absolute-git-dir points at .git/worktrees/<name> while
# --git-common-dir points at the main .git. In the main checkout they are equal.
GIT_DIR_PATH=$(git rev-parse --absolute-git-dir 2>/dev/null || echo "")
GIT_COMMON_PATH=$(cd "$(git rev-parse --git-common-dir 2>/dev/null || echo .)" 2>/dev/null && pwd)
IS_WORKTREE=0
if [[ -n "$GIT_DIR_PATH" ]] && [[ -n "$GIT_COMMON_PATH" ]] && [[ "$GIT_DIR_PATH" != "$GIT_COMMON_PATH" ]]; then
    IS_WORKTREE=1
    check_pass "Linked worktree"
else
    check_warn "This is the MAIN checkout, not a worktree — some checks are skipped"
fi

# -- 2. .env and DATABASE_URL --
DB_URL=""
if [[ -f "$WORKTREE_DIR/.env" ]]; then
    check_pass ".env exists"
    DB_URL=$(grep -E '^DATABASE_URL=' "$WORKTREE_DIR/.env" | tail -1 | cut -d= -f2- | sed -E 's/[[:space:]]+#.*$//; s/^"(.*)"$/\1/; s/^'"'"'(.*)'"'"'$/\1/')
    if [[ -z "$DB_URL" ]]; then
        check_fail "DATABASE_URL not set in .env — every #[sqlx::test] needs it (see .env.example)"
    else
        DB_HOST="$(url_part "$DB_URL" host)"
        case "$DB_HOST" in
            127.0.0.1|localhost|::1|'[::1]') check_pass "DATABASE_URL points at the local Postgres (${DB_HOST}:$(url_part "$DB_URL" port)/$(url_part "$DB_URL" db))" ;;
            *) check_warn "DATABASE_URL points at ${DB_HOST}, not this machine — tests create and drop databases on that server" ;;
        esac
    fi
else
    check_fail ".env missing — run: bash bin/setup-worktree.sh (it copies the main checkout's)"
fi

# -- 3. Postgres reachable --
DB_UP=0
if [[ -n "$DB_URL" ]]; then
    if db_reachable "$(url_part "$DB_URL" host)" "$(url_part "$DB_URL" port)"; then
        DB_UP=1
        check_pass "Postgres reachable at $(url_part "$DB_URL" host):$(url_part "$DB_URL" port)"
    else
        check_fail "Postgres not reachable at $(url_part "$DB_URL" host):$(url_part "$DB_URL" port) — run: just up"
    fi
fi

# -- 4. Migrations: the shared dev database against this branch --
#
# The dev database is shared by every checkout. A migration it has applied that this
# branch lacks makes the backend refuse to boot here ("migration N was previously
# applied but is missing"); a migration this branch adds would be applied to it, for
# every other checkout, the moment this backend boots. Either way, boot against a
# throwaway database instead (.claude/rules/database-postgres.md).
if [[ "$DB_UP" -eq 1 ]]; then
    THROWAWAY="$(worktree_db_name "$WORKTREE_DIR" 2>/dev/null || echo ois_tmp)"
    APPLIED="$(DATABASE_URL="$DB_URL" db_query 'select version from _sqlx_migrations order by version')"
    QUERY_RC=$?
    BRANCH_VERSIONS="$(cd "$WORKTREE_DIR" && for f in backend/migrations/*.sql; do
        v="${f##*/}"; v="${v%%_*}"
        [[ -f "$f" && "$v" =~ ^[0-9]+$ ]] && echo $((10#$v))
    done | sort -n)"
    if [[ "$QUERY_RC" -eq 2 ]]; then
        check_warn "neither psql nor a Postgres container on that port — cannot compare the dev database's migrations"
    elif [[ "$QUERY_RC" -ne 0 ]]; then
        check_warn "could not read _sqlx_migrations from the dev database (never migrated?)"
    else
        AHEAD="$(comm -23 <(printf '%s\n' "$APPLIED" | grep -E '^[0-9]+$' | sort) <(printf '%s\n' "$BRANCH_VERSIONS" | sort) | sort -n | tr '\n' ' ')"
        BEHIND="$(comm -13 <(printf '%s\n' "$APPLIED" | grep -E '^[0-9]+$' | sort) <(printf '%s\n' "$BRANCH_VERSIONS" | sort) | sort -n | tr '\n' ' ')"
        if [[ -n "${AHEAD// /}" ]]; then
            check_warn "the shared dev database has migration(s) ${AHEAD% } this branch lacks — the backend will not boot against it (\"previously applied but is missing\"). Boot against a throwaway database: ${THROWAWAY} (.claude/rules/database-postgres.md)"
        elif [[ -n "${BEHIND// /}" ]]; then
            check_warn "this branch adds migration(s) ${BEHIND% } the shared dev database lacks — booting the backend against it would apply them for every checkout. Boot against a throwaway database: ${THROWAWAY}"
        else
            check_pass "the shared dev database's migrations match this branch's"
        fi
    fi
fi

# -- 5. node_modules matches the lockfile --
#
# pnpm keeps the lockfile it last installed from at node_modules/.pnpm/lock.yaml. When
# it differs from pnpm-lock.yaml the packages under test are not the ones this branch
# declares, and a frontend repro proves nothing.
if [[ ! -d "$WORKTREE_DIR/node_modules" ]]; then
    check_fail "node_modules missing — run: pnpm install --frozen-lockfile (bash bin/setup-worktree.sh does it)"
elif [[ -L "$WORKTREE_DIR/node_modules" ]]; then
    check_fail "node_modules is a symlink to another checkout — rm it and run: pnpm install --frozen-lockfile"
elif [[ ! -f "$WORKTREE_DIR/node_modules/.pnpm/lock.yaml" ]]; then
    check_fail "node_modules was not installed by pnpm (no .pnpm/lock.yaml) — run: pnpm install --frozen-lockfile"
elif ! cmp -s "$WORKTREE_DIR/pnpm-lock.yaml" "$WORKTREE_DIR/node_modules/.pnpm/lock.yaml"; then
    check_fail "node_modules was installed from a different pnpm-lock.yaml — run: pnpm install --frozen-lockfile"
else
    check_pass "node_modules matches pnpm-lock.yaml"
fi

# -- 6. This worktree's own Cargo target dir, and sccache --
#
# A CARGO_TARGET_DIR (or CARGO_BUILD_TARGET_DIR) outside this checkout shares build output with another one, and
# cargo then judges a path package fresh by mtime and runs that checkout's test binary
# (bin/lib/portable.sh export_build_env).
# CARGO_BUILD_TARGET_DIR is the same setting (build.target-dir) by another name; CARGO_TARGET_DIR wins over it.
OWN_TARGET=1
for VAR in CARGO_TARGET_DIR CARGO_BUILD_TARGET_DIR; do
    VALUE="${!VAR:-}"
    if [[ -n "$VALUE" && "${VALUE%/}" != "$WORKTREE_DIR/target" ]]; then
        check_fail "${VAR}=${VALUE} is exported: cargo builds there, not in this checkout's target/, and a target dir shared with another checkout runs its stale test binaries. unset ${VAR}"
        OWN_TARGET=0
    fi
done
if [[ "$OWN_TARGET" -eq 0 ]]; then
    :
elif [[ -d "$WORKTREE_DIR/target" ]]; then
    check_pass "cargo builds into this checkout's own target/"
else
    check_pass "cargo builds into this checkout's own target/ (not built yet; bash bin/setup-worktree.sh seeds it from the primary checkout's)"
fi
if command -v sccache >/dev/null 2>&1; then
    if [[ "${RUSTC_WRAPPER:-}" == sccache ]]; then
        check_pass "sccache installed and RUSTC_WRAPPER=sccache"
    else
        check_warn "sccache installed but this shell's RUSTC_WRAPPER is '${RUSTC_WRAPPER:-}': just and bin/ scripts set it, a bare cargo builds uncached"
    fi
else
    check_warn "sccache not installed — builds work, uncached across worktrees (optional: brew install sccache)"
fi

# -- 7. Throwaway database --
if [[ "$IS_WORKTREE" -eq 1 ]]; then
    THROWAWAY="$(worktree_db_name "$WORKTREE_DIR" 2>/dev/null)"
    if [[ -n "$THROWAWAY" ]]; then
        check_pass "throwaway database name for booting the backend: ${THROWAWAY} (bin/teardown-worktree.sh drops it)"
    fi
fi

# -- 8. Branch name matches the enforced convention --
BRANCH=$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo "unknown")
if [[ "$BRANCH" == "main" || "$BRANCH" == "next" ]]; then
    if [[ "$IS_WORKTREE" -eq 1 ]]; then
        check_warn "On ${BRANCH} — a worktree should be on a feature branch"
    else
        check_pass "Main checkout on ${BRANCH}"
    fi
elif [[ "$BRANCH" =~ ^(feat|fix|chore)/[a-z0-9]+/[a-z0-9]+(-[a-z0-9]+){1,3}$ ]]; then
    if [[ ${#BRANCH} -gt 50 ]]; then
        check_fail "Branch '${BRANCH}' is ${#BRANCH} chars (max 50)"
    else
        check_pass "Branch name: ${BRANCH}"
    fi
elif [[ "$BRANCH" == worktree-* ]]; then
    check_warn "Branch '${BRANCH}' is the EnterWorktree name — rename it before pushing, as /start does"
else
    check_fail "Branch '${BRANCH}' does not match {type}/{issue-or-hash}/{description} (a 2-4 word description)"
    echo -e "         ${YELLOW}bin/pre-push will reject this. Rename:${NC} git branch -m ${BRANCH} <valid>"
fi

# -- 9. Branch ancestry — must descend from next, not another feature branch --
if [[ "$BRANCH" != "main" && "$BRANCH" != "next" ]]; then
    MERGE_BASE=$(git merge-base HEAD origin/next 2>/dev/null || echo "")
    if [[ -z "$MERGE_BASE" ]]; then
        check_warn "Cannot determine a merge base with origin/next"
    else
        BEHIND_TIP=$(git rev-list --count "${MERGE_BASE}..origin/next" 2>/dev/null || echo "?")
        check_pass "Forks from origin/next (${BEHIND_TIP} commits behind its tip)"

        # Compare against origin/next, never the local ref: a stale local next
        # makes every commit landed upstream look like contamination.
        # Merging origin/next (or origin/main, for a release back-merge) into the
        # branch is routine. A merge only brings in another branch's work when one
        # of its non-first parents carries commits on neither.
        FOREIGN=""
        while read -r merge; do
            [[ -n "$merge" ]] || continue
            for parent in $(git rev-list --parents -n 1 "$merge" | cut -d' ' -f3-); do
                # shellcheck disable=SC2046  # origin/main, or nothing when there is none
                if [[ -n "$(git rev-list -n 1 "$parent" --not origin/next $(git rev-parse --verify --quiet origin/main) 2>/dev/null)" ]]; then
                    FOREIGN+="$(git log -1 --oneline "$merge")"$'\n'
                    break
                fi
            done
        done < <(git rev-list HEAD --not origin/next --merges 2>/dev/null)
        FOREIGN="$(printf '%s' "$FOREIGN" | head -5)"
        if [[ -n "$FOREIGN" ]]; then
            check_fail "Branch carries merge commits from branches other than next and main:"
            echo "$FOREIGN" | while read -r line; do echo -e "         ${RED}${line}${NC}"; done
            echo -e "         ${YELLOW}Possible cross-ticket contamination.${NC}"
        else
            check_pass "No merge commits from other branches"
        fi
    fi
fi

# -- 10. The repo's git hooks are installed --
#
# .githooks/ (pre-commit, commit-msg, pre-push) runs only once core.hooksPath points at
# it, and nothing else notices when it does not: commits then skip the attribution and
# migration checks and pushes skip clippy and the suites.
HOOKS_PATH="$(git config core.hooksPath 2>/dev/null)"
if [[ "$HOOKS_PATH" == ".githooks" ]]; then
    check_pass "git hooks installed (core.hooksPath=.githooks)"
else
    check_fail "git hooks not installed (core.hooksPath is '${HOOKS_PATH}', not .githooks): run just setup once per clone"
fi

# -- Summary --
echo "-----------------------------------------------------------"
echo -e "  ${GREEN}${PASS} passed${NC}  ${RED}${FAIL} failed${NC}  ${YELLOW}${WARN} warnings${NC}"
echo ""

if [[ "$FAIL" -gt 0 ]]; then
    echo -e "  ${RED}HEALTH CHECK FAILED${NC} — fix the above before trusting any gate run here."
    if [[ -f "$WORKTREE_DIR/.claude/worktree-setup.log" ]]; then
        echo -e "  ${YELLOW}Setup output was captured:${NC} .claude/worktree-setup.log"
    fi
    exit 1
fi

echo -e "  ${GREEN}HEALTH CHECK PASSED${NC}"
exit 0
