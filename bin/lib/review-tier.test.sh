#!/bin/bash
# ==============================================================================
# Tests for bin/lib/review-tier.sh: the tier a commit's changed paths put it in, and the lenses
# that tier requires.
#
#   bash bin/lib/review-tier.test.sh
#
# Exits non-zero on any failure.
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
LIB="$ROOT/bin/lib/review-tier.sh"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/review-tier-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
# The host's git config (commit signing, hooks) stays out of the scratch repository.
export HOME="$WORK" GIT_CONFIG_NOSYSTEM=1

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

# shellcheck source=review-tier.sh
. "$LIB"

echo "Each path's tier:"
# path_is WANT PATH
path_is() { local got; got="$(tier_name "$(tier_of_path "$2")")"; [[ "$got" == "$1" ]] && ok "$1: $2" || bad "$2 (want $1, got $got)"; }
for p in docs/x.md docs-site/reference/api.md CLAUDE.md AGENTS.md DESIGN.md .claude/rules/code-quality.md \
        .claude/commands/ticket-loop.md .claude/agents/ticket-worker.md web/src/lib/time.test.ts \
        web/src/pages/x.test.tsx backend/tests/feed.rs tests/smoke.sh bin/lib/pre-push.test.sh; do
    path_is Light "$p"
done
for p in .claude/hooks/marker-write-gate.sh .claude/hooks/lib/git_policy.py .claude/settings.json \
        .claude/review-lenses.tsv .githooks/pre-push bin/pre-push bin/lib/review-attestation.sh justfile \
        .claude/commands/code-review.md .claude/commands/review-before-shipping.md .claude/commands/ship.md \
        .claude/agents/ticket-reviewer.md .claude/agents/code-review-agent.md .claude/agents/security-audit-agent.md \
        .claude/rules/secure-coding.md .claude/rules/data-protection.md bin/lib/nested/x.test.sh; do
    path_is Standard "$p"
done
for p in backend/src/feed/predict.rs backend/src/jobs.rs backend/src/auth/principal.rs backend/src/router.rs \
        backend/src/openapi.rs backend/migrations/0134_x.sql web/src/pages/x.tsx packages/api-client/src/x.ts \
        crates/ois-core/src/catalog.rs desktop/src-tauri/tauri.conf.json Cargo.lock pnpm-lock.yaml \
        .github/workflows/ci.yml backend/src/scope_test_support.rs unknown.txt \
        backend/src/handlers/tmu.rs backend/src/handlers/flow_event_fca_visibility_tests.rs \
        backend/src/repos/flow.rs backend/src/models/flow.rs discord/src/jobs/tmi.rs discord/Cargo.toml; do
    path_is Full "$p"
done

echo "Which paths are test files (lens tests in any tier):"
for p in web/src/lib/time.test.ts web/src/pages/x.test.tsx tests/smoke.sh backend/tests/feed.rs \
        backend/src/handlers/flow_event_fca_visibility_tests.rs bin/lib/pre-push.test.sh; do
    is_test_path "$p" && ok "test file: $p" || bad "$p should be a test file"
done
for p in docs/x.md backend/src/handlers/tmu.rs bin/lib/review-tier.sh bin/lib/nested/x.test.sh web/src/lib/time.ts; do
    is_test_path "$p" && bad "$p is not a test file" || ok "not a test file: $p"
done

echo "Each tier's lenses:"
[[ "$(tier_lenses Light)" == "code ac" ]] && ok "Light: code ac" || bad "Light: $(tier_lenses Light)"
[[ "$(tier_lenses Standard)" == "code ac security" ]] && ok "Standard: code ac security" || bad "Standard: $(tier_lenses Standard)"
[[ "$(tier_lenses Full)" == "code ac tests security" ]] && ok "Full: code ac tests security" || bad "Full: $(tier_lenses Full)"
[[ "$(tier_lenses bogus)" == "code ac tests security" ]] && ok "an unknown tier name is Full" || bad "unknown: $(tier_lenses bogus)"

echo "A commit's tier, from the diff against its base:"
REPO="$WORK/repo"
git init -q -b next "$REPO"
cd "$REPO" || exit 1
git config user.email t@e.com
git config user.name t
mkdir -p docs backend/src/repos backend/src/feed
printf 'x\n' > docs/a.md
git add . && git commit -qm base
BASE="$(git rev-parse HEAD)"
# commit_paths PATH...: a commit on top of BASE touching exactly these paths; prints its SHA.
commit_paths() {
    git checkout -q --detach "$BASE"
    local p
    for p in "$@"; do mkdir -p "$(dirname "$p")"; printf '%s\n' "$RANDOM" >> "$p"; done
    git add -- "$@" && git commit -qm change && git rev-parse HEAD
}
# tier_is WANT NAME SHA
tier_is() { local got; got="$(REVIEW_TIER_BASE="$BASE" review_tier "$3" "$REPO")"; [[ "$got" == "$1" ]] && ok "$1: $2" || bad "$2 (want $1, got $got)"; }
tier_is Light "docs only" "$(commit_paths docs/a.md .claude/rules/x.md)"
tier_is Standard "a gate script plus docs" "$(commit_paths bin/pre-push docs/a.md)"
tier_is Standard "a security rule" "$(commit_paths .claude/rules/secure-coding.md)"
tier_is Full "a bot job plus docs" "$(commit_paths discord/src/jobs/tmi.rs docs/a.md)"
tier_is Full "a Light-only diff plus one Full path" "$(commit_paths docs/a.md web/src/x.test.ts backend/src/feed/predict.rs)"
tier_is Full "a Standard diff plus one Full path" "$(commit_paths bin/pre-push backend/src/repos/flow.rs)"
tier_is Full "an unknown path" "$(commit_paths notes.txt)"
tier_is Full "an empty diff (the base itself)" "$BASE"
got="$(REVIEW_TIER_BASE=no-such-ref review_tier "$(commit_paths docs/a.md)" "$REPO")"
[[ "$got" == "Full" ]] && ok "Full: a base that does not resolve" || bad "unresolved base gave $got"
got="$(review_tier "$(commit_paths docs/a.md)" "$REPO")"
[[ "$got" == "Full" ]] && ok "Full: no origin/next in the repository" || bad "no origin/next gave $got"
echo "A commit's lenses: the tier's, plus tests when it changes a test file:"
# lenses_are WANT NAME SHA
lenses_are() { local got; got="$(REVIEW_TIER_BASE="$BASE" review_lenses "$3" "$REPO")"; [[ "$got" == "$1" ]] && ok "$1: $2" || bad "$2 (want $1, got $got)"; }
lenses_are "code ac" "Light, no test file" "$(commit_paths docs/a.md)"
lenses_are "code ac tests" "Light with a vitest file" "$(commit_paths docs/a.md web/src/x.test.ts)"
lenses_are "code ac tests" "Light with a top-level tests/ file" "$(commit_paths tests/smoke.sh)"
lenses_are "code ac tests" "Light with a bin/lib test script" "$(commit_paths bin/lib/x.test.sh)"
lenses_are "code ac security" "Standard, no test file" "$(commit_paths bin/pre-push)"
lenses_are "code ac tests security" "Standard with a test file" "$(commit_paths bin/pre-push bin/lib/pre-push.test.sh)"
lenses_are "code ac tests security" "Full, no test file" "$(commit_paths backend/src/repos/flow.rs)"
lenses_are "code ac tests security" "Full with a *_tests.rs module" "$(commit_paths backend/src/handlers/x_tests.rs)"
lenses_are "code ac tests security" "an empty diff" "$BASE"
LIGHT="$(commit_paths docs/a.md)"
got="$(cd "$REPO" && REVIEW_TIER_BASE="$BASE" bash "$LIB" "$LIGHT")"
[[ "$got" == "Light code ac" ]] && ok "the command line prints the tier and its lenses" || bad "command line: $got"
got="$(cd "$REPO" && REVIEW_TIER_BASE="$BASE" bash "$LIB" "$(commit_paths docs/a.md web/src/x.test.ts)")"
[[ "$got" == "Light code ac tests" ]] && ok "the command line adds tests for a test file" || bad "command line: $got"
git -C "$REPO" update-ref refs/remotes/origin/next "$BASE"
got="$(unset REVIEW_TIER_BASE; review_tier "$LIGHT" "$REPO")"
[[ "$got" == "Light" ]] && ok "Light: the default base origin/next resolves" || bad "default base origin/next gave $got"

echo ""
echo "review-tier: $PASS passed, $FAIL failed"
(( FAIL == 0 ))
