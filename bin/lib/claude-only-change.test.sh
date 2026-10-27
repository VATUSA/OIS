#!/bin/bash
# ==============================================================================
# Tests for bin/lib/claude-only-change.sh.
#
#   bash bin/lib/claude-only-change.test.sh
#
# Builds throwaway git repos under a temp dir, with origin/next faked by
# update-ref so no network or remote is needed. Pure shell; runs unchanged on
# macOS bash 3.2 and Linux. Self-reports pass/fail, exits non-zero on any failure.
#
# Covers the helper only. A gate that sources it carries its own exemption test.
# ==============================================================================
set -uo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LIB="$DIR/claude-only-change.sh"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/claude-only-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

# A hook run by git exports these; inherited, they would retarget the real repo.
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR GIT_OBJECT_DIRECTORY GIT_PREFIX
export GIT_AUTHOR_NAME=T GIT_AUTHOR_EMAIL=t@e.com GIT_COMMITTER_NAME=T GIT_COMMITTER_EMAIL=t@e.com
export GIT_CONFIG_NOSYSTEM=1 HOME="$WORK"

# shellcheck source=claude-only-change.sh
. "$LIB"

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

g() { git -c core.hooksPath=/dev/null "$@"; }

# new_repo NAME [BASE_REF] — next with one code commit, BASE_REF (default
# origin/next) pointed at it, then a feature branch checked out.
new_repo() {
    local repo="$WORK/$1" base="${2:-refs/remotes/origin/next}"
    mkdir -p "$repo/backend/src" "$repo/.claude"
    (
        cd "$repo" || exit 1
        g init -q -b next
        echo x > backend/src/base.rs
        echo x > .claude/base.md
        echo x > CLAUDE.md
        g add . && g commit -qm init
        [ "$base" = none ] || g update-ref "$base" HEAD
        g checkout -qb chore/abc1234/test
    ) || return 1
    echo "$repo"
}

commit_files() {
    local repo="$1" f
    shift
    for f in "$@"; do
        mkdir -p "$repo/$(dirname "$f")"
        echo "$RANDOM" >> "$repo/$f"
    done
    (cd "$repo" && g add -A && g commit -qm change)
}

exempt_in() { (cd "$1" && claude_only_change); }

expect_exempt() {
    if exempt_in "$1"; then ok "$2"; else bad "$2 (expected exempt)"; fi
}
expect_gated() {
    if exempt_in "$1"; then bad "$2 (expected NOT exempt)"; else ok "$2"; fi
}

echo "== 1. helper: path classification =="
R="$(new_repo only-claude)"; commit_files "$R" .claude/rules/x.md .claude/agents/y.md
expect_exempt "$R" "only .claude/ changes -> exempt"

R="$(new_repo hooks)"; commit_files "$R" .claude/rules/x.md .claude/hooks/y.sh
expect_gated "$R" ".claude/hooks/ change -> not exempt"

R="$(new_repo settings)"; commit_files "$R" .claude/settings.json
expect_gated "$R" ".claude/settings.json -> not exempt"

R="$(new_repo settings-local)"; commit_files "$R" .claude/settings.local.json
expect_gated "$R" ".claude/settings.local.json -> not exempt"

R="$(new_repo only-claudemd)"; commit_files "$R" CLAUDE.md
expect_exempt "$R" "only CLAUDE.md -> exempt"

R="$(new_repo mixed)"; commit_files "$R" .claude/rules/x.md backend/src/x.rs
expect_gated "$R" ".claude/ plus backend/src/x.rs -> not exempt"

R="$(new_repo app-only)"; commit_files "$R" backend/src/x.rs
expect_gated "$R" "backend/src/ only -> not exempt"

R="$(new_repo guidance-docs)"; commit_files "$R" AGENTS.md
expect_exempt "$R" "AGENTS.md -> exempt"

# The web tests read DESIGN.md (colours.guard.test.ts, user-group.dom.test.tsx), so it gates.
R="$(new_repo design-only)"; commit_files "$R" DESIGN.md
expect_gated "$R" "only DESIGN.md -> not exempt (tests read it)"
R="$(new_repo design-and-guidance)"; commit_files "$R" CLAUDE.md AGENTS.md DESIGN.md
expect_gated "$R" "DESIGN.md beside CLAUDE.md and AGENTS.md -> not exempt"

for f in docs/AGENTS.md docs/DESIGN.md docs/github-issues.md docs-site/index.md README.md web/src/x.test.ts bin/run-tests.sh .githooks/pre-push justfile; do
    R="$(new_repo "gated-$(printf '%s' "$f" | tr '/.' '__')")"; commit_files "$R" CLAUDE.md "$f"
    expect_gated "$R" "$f -> not exempt"
done

R="$(new_repo claudex)"; commit_files "$R" .claudex/foo
expect_gated "$R" ".claudex/foo -> not exempt"

R="$(new_repo nested-claudemd)"; commit_files "$R" docs/CLAUDE.md
expect_gated "$R" "docs/CLAUDE.md -> not exempt"

R="$(new_repo nested-dotclaude)"; commit_files "$R" backend/.claude/x.md
expect_gated "$R" "backend/.claude/x.md -> not exempt"

R="$(new_repo rename-in)"; (cd "$R" && g mv backend/src/base.rs .claude/base.rs && g commit -qm mv)
expect_gated "$R" "moving backend code into .claude/ -> not exempt"
# Into an exempt directory, a rename lists only .claude/rules/base.rs; --no-renames also lists backend/src/base.rs.
R="$(new_repo rename-into-rules)"; (cd "$R" && mkdir -p .claude/rules && g mv backend/src/base.rs .claude/rules/base.rs && g commit -qm mv)
expect_gated "$R" "moving backend code into .claude/rules/ -> not exempt"
R="$(new_repo rename-into-agents-md)"; (cd "$R" && g mv backend/src/base.rs AGENTS.md && g commit -qm mv)
expect_gated "$R" "moving backend code onto AGENTS.md -> not exempt"

R="$(new_repo guidance-dirs)"; commit_files "$R" .claude/plans/p.md .claude/skills/s/SKILL.md .claude/commands/start.md .claude/agents/tdd-planner.md
expect_exempt "$R" "plans, skills and non-review commands and agents -> exempt"

for f in .claude/review-lenses.tsv .claude/commands/code-review.md .claude/commands/review-before-shipping.md \
    .claude/commands/ship.md .claude/agents/code-review-agent.md .claude/agents/security-audit-agent.md \
    .claude/agents/ticket-reviewer.md .claude/agents/test-reviewer.md .claude/workflows/council.mjs \
    .claude/worktrees/wt/backend/src/x.rs .claude/markers/tests/abc .claude/x.md .claude/hooks/pre-pr-gate.sh \
    .claude/scripts/board-status.sh; do
    R="$(new_repo "defines-$(printf '%s' "$f" | tr '/.' '__')")"; commit_files "$R" .claude/rules/x.md "$f"
    expect_gated "$R" "$f -> not exempt"
done

echo "== 1b. helper: the ref under test =="
R="$(new_repo ref-arg)"; commit_files "$R" .claude/rules/x.md
(cd "$R" && g checkout -qb feat/1/app next && echo y > backend/src/y.rs && g add . && g commit -qm app && g checkout -q chore/abc1234/test)
if (cd "$R" && claude_only_change feat/1/app); then bad "an app branch named as REF -> not exempt"; else ok "an app branch named as REF -> not exempt"; fi
if (cd "$R" && claude_only_change); then ok "HEAD by default -> exempt"; else bad "HEAD by default -> exempt"; fi

echo "== 2. helper: fail closed =="
R="$(new_repo no-base none)"; commit_files "$R" .claude/rules/x.md
expect_gated "$R" "no origin/next -> not exempt"

R="$(new_repo empty-diff)"
expect_gated "$R" "empty diff -> not exempt"

R="$(new_repo other-base refs/remotes/origin/main)"; commit_files "$R" .claude/rules/x.md
expect_gated "$R" "only origin/main, no origin/next -> not exempt"

R="$(new_repo stale-base)"; commit_files "$R" .claude/rules/x.md
(cd "$R" && g checkout -q next && echo y > backend/src/upstream.rs && g add . && g commit -qm up \
    && g update-ref refs/remotes/origin/next HEAD && g checkout -q chore/abc1234/test)
expect_exempt "$R" "upstream backend commits on origin/next do not count against the branch (three-dot)"

NOT_REPO="$WORK/not-a-repo"; mkdir -p "$NOT_REPO"
if (cd "$NOT_REPO" && GIT_CEILING_DIRECTORIES="$WORK" claude_only_change); then
    bad "outside a git repo -> not exempt"
else
    ok "outside a git repo -> not exempt"
fi

echo ""
echo "claude-only-change: $PASS passed, $FAIL failed"
[[ $FAIL -eq 0 ]]
