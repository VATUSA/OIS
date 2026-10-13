#!/bin/bash
# ==============================================================================
# Tests for .claude/hooks/test-run-gate.sh: which commands it gates, and the
# SHA-keyed marker it reads.
#
#   bash bin/lib/test-run-gate.test.sh
#
# The matching cases run the gate from a scratch directory outside any git repo,
# where the marker check cannot pass: a command the gate lets through exits 0
# before reaching it, and one it gates exits 2. The marker cases run it inside a
# scratch repo. Self-reports pass/fail, exits non-zero on any failure.
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GATE="$ROOT/.claude/hooks/test-run-gate.sh"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/test-run-gate-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

payload() {
    python3 -c 'import json,sys; print(json.dumps({"tool_name": "Bash", "tool_input": {"command": sys.argv[1]}}))' "$1"
}

# expect WANT(0 passed through | 2 gated) NAME COMMAND [PATH]
expect() {
    local got
    payload "$3" 2>/dev/null | (cd "$WORK" && GIT_CEILING_DIRECTORIES="$WORK" PATH="${4:-$PATH}" "$BASH_BIN" "$GATE" >/dev/null 2>&1)
    got=$?
    if [[ "$got" == "$1" ]]; then ok "$2"; else bad "$2 (want $1, got $got)"; fi
}

BASH_BIN="$(command -v bash)"
NL=$'\n'
PR="gh pr ""create --base next --title x"

echo "Gated:"
expect 2 "bare"                        "$PR"
expect 2 "chained after cd"            "cd /repo && $PR"
expect 2 "inside bash -c"              "bash -c \"$PR\""
expect 2 "heredoc fed to bash"         "bash <<'EOF'${NL}$PR${NL}EOF"
expect 2 "unparseable line"            "echo \"$PR"
expect 2 "the pr new alias"            "gh pr new --base next --title x"
expect 2 "-R before the subcommand"    "gh pr -R VATUSA/OIS create --title x"
expect 2 "-R before the group"          "gh -R VATUSA/OIS pr create --title x"

echo "A quoted string naming the command reads as a run (a grep would too):"
expect 2 "quoted in a grep"            "grep -n \"$PR\" .claude/hooks/*.sh"

echo "Passed through:"
expect 0 "other command"               "ls -la"
expect 0 "commit message"              "git commit -m \"docs: $PR\""
expect 0 "heredoc written to a file"   "cat > body.md <<'EOF'${NL}$PR${NL}EOF"
expect 0 "heredoc fed to python"       "python3 - <<'PY'${NL}print('$PR')${NL}PY"

echo "Fails closed without python3:"
expect 2 "python3 missing"             "ls -la" "/nonexistent:/usr/bin/nonexistent"

echo "Marker:"
REPO="$WORK/repo"
mkdir -p "$REPO"
git -C "$REPO" init -q
git -C "$REPO" config user.email t@t
git -C "$REPO" config user.name t
printf 'x\n' > "$REPO/app.rs"
git -C "$REPO" add app.rs
git -C "$REPO" commit -qm init
SHA="$(git -C "$REPO" rev-parse HEAD)"

# gate_in_repo: run the gate on a PR command from inside $REPO; sets RC, OUT.
gate_in_repo() {
    OUT="$(payload "$PR" | (cd "$REPO" && GIT_CEILING_DIRECTORIES="$WORK" "$BASH_BIN" "$GATE" 2>&1))"
    RC=$?
}

gate_in_repo
if [[ $RC -eq 2 && "$OUT" == *"just test"* ]]; then ok "no marker blocks, naming just test"; else bad "no marker: rc=$RC out=$OUT"; fi

stamp_marker() { python3 -I "$ROOT/bin/lib/marker_manifest.py" stamp "$@"; }
MK="$REPO/.claude/markers"
mkdir -p "$MK/tests"
date +%s > "$MK/tests/$SHA"
gate_in_repo
if [[ $RC -eq 2 && "$OUT" == *"no matching line in .claude/markers/manifest"* ]]; then ok "a hand-written marker is refused"; else bad "hand-written marker: rc=$RC out=$OUT"; fi
printf 'tests %s %s\n' "$SHA" "$(printf 'x' | sha256sum | cut -d' ' -f1)" > "$MK/manifest"
gate_in_repo
if [[ $RC -eq 2 ]]; then ok "a manifest line whose hash is not the marker's is ignored"; else bad "wrong-hash line passed: rc=$RC"; fi
printf 'tests %s %s\n' "$SHA" "$(sha256sum < "$MK/tests/$SHA" | cut -d' ' -f1)" > "$WORK/forged-manifest"
ln -sf "$WORK/forged-manifest" "$MK/manifest"
gate_in_repo
if [[ $RC -eq 2 ]]; then ok "a symlinked manifest is not a manifest"; else bad "symlinked manifest passed: rc=$RC"; fi
rm -f "$MK/manifest" "$MK/tests/$SHA"
printf 'review-shipping %s %s\n' "$SHA" "$(sha256sum < "$WORK/forged-manifest" | cut -d' ' -f1)" > "$MK/manifest"
cp "$WORK/forged-manifest" "$MK/tests/$SHA"
gate_in_repo
if [[ $RC -eq 2 ]]; then ok "a line for another kind does not vouch for a tests marker"; else bad "other-kind line passed: rc=$RC"; fi
rm -f "$MK/manifest" "$MK/tests/$SHA"
stamp_marker "$REPO" tests "$SHA"
ln -s "$MK/tests/$SHA" "$MK/tests/link" && mv "$MK/tests/$SHA" "$WORK/real" && ln -s "$WORK/real" "$MK/tests/$SHA"
gate_in_repo
if [[ $RC -eq 2 ]]; then ok "a symlinked marker is not a marker"; else bad "symlinked marker passed: rc=$RC"; fi
rm -f "$MK/tests/$SHA" "$MK/tests/link" "$MK/manifest"
stamp_marker "$REPO" tests "$SHA"
gate_in_repo
if [[ $RC -eq 0 ]]; then ok "a marker for HEAD with its manifest line passes"; else bad "marker for HEAD: rc=$RC out=$OUT"; fi

git -C "$REPO" commit -q --allow-empty -m moved
gate_in_repo
if [[ $RC -eq 2 ]]; then ok "a marker for an earlier commit blocks"; else bad "stale-SHA marker: rc=$RC out=$OUT"; fi

date +%s > "$REPO/.claude/.tests-passed"
gate_in_repo
if [[ $RC -eq 2 ]]; then ok "the legacy .tests-passed timestamp is not evidence"; else bad "legacy marker passed: rc=$RC out=$OUT"; fi

# The marker must describe the commit the PR opens on: a `cd DIR &&` checkout, or a --head branch.
stamp_marker "$REPO" tests "$(git -C "$REPO" rev-parse HEAD)"

# An unresolved PR head is refused, never checked against whatever HEAD has a marker for.
(cd "$REPO" && . "$ROOT/bin/check-marker.sh" && check_marker_for_head tests Tests "just test" "$(git rev-parse HEAD)" >/dev/null)
if [[ $? -eq 0 ]]; then ok "positive control: check_marker_for_head passes HEAD's marker"; else bad "check_marker_for_head HEAD"; fi
(cd "$REPO" && . "$ROOT/bin/check-marker.sh" && check_marker_for_head tests Tests "just test" "" >/dev/null)
if [[ $? -eq 1 ]]; then ok "an empty PR head does not fall back to HEAD's marker"; else bad "empty PR head fell back to HEAD's marker"; fi
UNTESTED="$WORK/untested"
git clone -q "$REPO" "$UNTESTED" && git -C "$UNTESTED" commit -q --allow-empty -m untested
git -C "$REPO" branch -q feat/2/untested && git -C "$REPO" checkout -q feat/2/untested \
    && git -C "$REPO" commit -q --allow-empty -m untested && git -C "$REPO" checkout -q -
gate_cmd() { # gate_cmd CWD COMMAND; sets RC
    payload "$2" | (cd "$1" && GIT_CEILING_DIRECTORIES="$WORK" "$BASH_BIN" "$GATE" >/dev/null 2>&1)
    RC=$?
}
gate_cmd "$REPO" "$PR"
if [[ $RC -eq 0 ]]; then ok "positive control: the tested checkout passes"; else bad "tested checkout: rc=$RC"; fi
gate_cmd "$REPO" "cd $UNTESTED && $PR"
if [[ $RC -eq 2 ]]; then ok "cd into an untested checkout is checked there"; else bad "cd into untested checkout: rc=$RC"; fi
gate_cmd "$UNTESTED" "cd $REPO && $PR"
if [[ $RC -eq 0 ]]; then ok "cd into the tested checkout is checked there"; else bad "cd into tested checkout: rc=$RC"; fi
gate_cmd "$REPO" "$PR --head feat/2/untested"
if [[ $RC -eq 2 ]]; then ok "--head an untested branch blocks"; else bad "--head untested: rc=$RC"; fi

# expect_from CWD WANT NAME COMMAND
expect_from() {
    gate_cmd "$1" "$4"
    if [[ $RC -eq $2 ]]; then ok "$3"; else bad "$3 (want $2, got $RC)"; fi
}
echo "Other repositories and API PRs, from a tested checkout:"
git -C "$REPO" remote add origin git@github.com:VATUSA/OIS.git
expect_from "$REPO" 0 "positive control: --repo this checkout's origin"  "$PR --repo VATUSA/OIS"
expect_from "$REPO" 2 "--repo another repository"                         "$PR --repo someone/fork"
expect_from "$REPO" 2 "--head on a fork, naming the tested branch"        "$PR --head someone:$(git -C "$REPO" branch --show-current)"
expect_from "$REPO" 0 "--head owner:branch on this checkout's origin"     "$PR --head VATUSA:$(git -C "$REPO" branch --show-current)"
expect_from "$REPO" 2 "gh api POST to pulls"                              "gh api repos/VATUSA/OIS/pulls -f head=x -f base=next"
expect_from "$REPO" 2 "gh api -iX POST to pulls"                          "gh api -iX POST repos/VATUSA/OIS/pulls --input body.json"
expect_from "$REPO" 2 "GraphQL createPullRequest"                         "gh api graphql -f query='mutation { createPullRequest(input: {}) { clientMutationId } }'"

echo "A line that moves directory before the PR, from a tested cwd:"
expect_from "$REPO" 2 "cd DIR; gh pr create"                              "cd $UNTESTED; $PR"
expect_from "$REPO" 2 "cd DIR && git push && gh pr create"                "cd $UNTESTED && git push -u origin x && $PR"
expect_from "$REPO" 2 "pushd DIR && gh pr create"                         "pushd $UNTESTED && $PR"

echo "Guidance-only exemption, judged on the PR head:"
git -C "$REPO" update-ref refs/remotes/origin/next "$(git -C "$REPO" rev-list --max-parents=0 HEAD)"
git -C "$REPO" checkout -q -b chore/3/guidance origin/next && mkdir -p "$REPO/.claude/rules" \
    && printf 'x\n' > "$REPO/.claude/rules/x.md" && git -C "$REPO" add .claude/rules/x.md && git -C "$REPO" commit -qm rules
expect_from "$REPO" 0 "a rules-only branch skips the gate"                "$PR"
expect_from "$REPO" 2 "a rules-only HEAD does not exempt --head another branch" "$PR --head feat/2/untested"
git -C "$REPO" checkout -q -

# A copy of the hook tree without bin/check-marker.sh must refuse, not pass.
COPY="$WORK/copy"
mkdir -p "$COPY/.claude/hooks" "$COPY/bin/lib"
cp -R "$ROOT/.claude/hooks/lib" "$COPY/.claude/hooks/lib"
cp "$GATE" "$COPY/.claude/hooks/test-run-gate.sh"
cp "$ROOT/bin/lib/claude-only-change.sh" "$COPY/bin/lib/claude-only-change.sh"
OUT="$(payload "$PR" | (cd "$REPO" && "$BASH_BIN" "$COPY/.claude/hooks/test-run-gate.sh" 2>&1))"
RC=$?
if [[ $RC -eq 2 && "$OUT" == *"cannot load check_marker_for_head"* ]]; then ok "a missing check-marker.sh is gated"; else bad "missing check-marker.sh: rc=$RC out=$OUT"; fi

echo "A refused line says why:"
out="$(python3 -c 'import json,sys; print(json.dumps({"tool_name": "Bash", "tool_input": {"command": sys.argv[1]}}))' "gh \$'pr' create" \
    | (cd "$WORK" && bash "$GATE" 2>&1))"
rc=$?
if [[ $rc -eq 2 && "$out" == *"ANSI-C"* ]]; then ok "the parser's reason is in the message"; else bad "reason missing (rc=$rc: ${out:0:200})"; fi

echo ""
echo "test-run-gate: $PASS passed, $FAIL failed"
(( FAIL == 0 ))
