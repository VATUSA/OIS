#!/bin/bash
# ==============================================================================
# Tests for bin/pre-push: the ref a push is judged by, the worktree-* branch
# refusal, the clippy and suite skip a vouched tests marker earns, the clippy and
# suite skip a guidance-only push earns, the failure lines it surfaces, the error
# count from cargo deny (a warning, as pnpm audit's), and where the installed
# .githooks/pre-push finds the hook.
#
#   bash bin/lib/pre-push.test.sh
#
# cargo and pnpm are stubs, scripted per case through STUB_* variables and recording
# that they ran, so nothing real runs. Exits non-zero on any failure.
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
HOOK="$ROOT/bin/pre-push"
WRAPPER="$ROOT/.githooks/pre-push"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/pre-push-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR CARGO_TARGET_DIR DATABASE_URL
export GIT_AUTHOR_NAME=T GIT_AUTHOR_EMAIL=t@e.com GIT_COMMITTER_NAME=T GIT_COMMITTER_EMAIL=t@e.com
export GIT_CONFIG_NOSYSTEM=1 HOME="$WORK"
export OIS_LOCK_DIR="$WORK/locks"
unset OIS_NO_LOCK CI

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

export STUB_LOG="$WORK/stub.log"
mkdir -p "$WORK/bin"
cat > "$WORK/bin/cargo" <<'STUB'
#!/bin/bash
case "$1 ${2:-}" in
    "nextest --version") [[ -z "${STUB_NO_NEXTEST:-}" ]]; exit ;;
    "deny --version") [[ -z "${STUB_NO_DENY:-}" ]]; exit ;;
esac
echo "cargo $* DATABASE_URL=${DATABASE_URL:-} CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-}" >> "$STUB_LOG"
case "$1" in
    clippy) printf '%b' "${STUB_CLIPPY_OUT:-}"; exit "${STUB_CLIPPY_RC:-0}" ;;
    nextest) printf '%b' "${STUB_NEXTEST_OUT:-}"; exit "${STUB_NEXTEST_RC:-0}" ;;
    deny) printf '%b' "${STUB_DENY_OUT:-}"; exit "${STUB_DENY_RC:-0}" ;;
esac
exit 99
STUB
cat > "$WORK/bin/pnpm" <<'STUB'
#!/bin/bash
echo "pnpm $*" >> "$STUB_LOG"
case "$1 ${2:-} ${3:-} ${4:-}" in
    "test   ") exit 98 ;;
    "exec turbo run test") printf '%b' "${STUB_VITEST_OUT:-}"; exit "${STUB_VITEST_RC:-0}" ;;
esac
case "$1" in
    audit) exit "${STUB_AUDIT_RC:-0}" ;;
esac
exit 99
STUB
chmod +x "$WORK/bin/cargo" "$WORK/bin/pnpm"

# new_repo DIR: two commits on next, a .env and node_modules (both ignored).
new_repo() {
    git init -q -b next "$1"
    printf '.env\nnode_modules/\n.claude/markers/\n' > "$1/.gitignore"
    git -C "$1" add .gitignore
    git -C "$1" -c core.hooksPath=/dev/null commit -q -m init
    git -C "$1" -c core.hooksPath=/dev/null commit -q --allow-empty -m second
    printf 'DATABASE_URL=postgres://ois:ois@127.0.0.1:5432/ois\n' > "$1/.env"
    mkdir -p "$1/node_modules"
}
REPO="$WORK/repo"
new_repo "$REPO"
HEAD_SHA="$(git -C "$REPO" rev-parse HEAD)"
OTHER_SHA="$(git -C "$REPO" rev-parse HEAD~1)"
Z=0000000000000000000000000000000000000000
GOOD=fix/289/logbook-map-starburst

# push_lines STDIN [CWD] [HOOK] -> sets OUT (color codes stripped) and RC; git feeds the hook the updates on stdin
push_lines() {
    : > "$STUB_LOG"
    OUT="$(cd "${2:-$REPO}" && PATH="$WORK/bin:$PATH" bash "${3:-$HOOK}" 2>&1 <<<"$1" | sed $'s/\033\\[[0-9;]*m//g'; exit "${PIPESTATUS[0]}")"
    RC=$?
}
# push_output BRANCH -> the hook's output for a push of that checked-out branch to its own name
push_output() {
    git -C "$REPO" checkout -q -B "$1"
    push_lines "refs/heads/$1 $HEAD_SHA refs/heads/$1 $Z"
    printf '%s\n' "$OUT"
}
# expect_line NAME PATTERN OUTPUT
expect_line() {
    if grep -qE "$2" <<<"$3"; then ok "$1"; else bad "$1 (no /$2/ in: $3)"; fi
}
refute_line() {
    if grep -qE "$2" <<<"$3"; then bad "$1 (/$2/ in: $3)"; else ok "$1"; fi
}
same() { if [[ "$2" == "$3" ]]; then ok "$1"; else bad "$1 (got '$2', want '$3')"; fi; }
ran() { grep -q "^$1" "$STUB_LOG"; }

echo "Branch naming:"
out="$(push_output worktree-fix+289+logbook-map)"
expect_line "a worktree-* branch is told to rename"   'Branch naming\.\.\. fail \(rename from worktree-\* before pushing' "$out"
expect_line "...and the push fails"                   'Pre-push checks failed' "$out"
out="$(push_output "$GOOD")"
expect_line "positive control: a well named branch"  'Branch naming\.\.\. pass' "$out"
expect_line "...and every check passes"              'All pre-push checks passed' "$out"
out="$(push_output bad_name)"
expect_line "another bad name gets the format"        'Branch naming\.\.\. fail \(must be feat\|fix\|chore' "$out"
out="$(push_output chore/1617e87/abcdefghijklmnopqrstuvw-abcdefghijklm)"
expect_line "a 51-character name fails on length"     'Branch naming\.\.\. fail \(51 chars, max 50\)' "$out"
out="$(push_output chore/1617e87/abcdefghijklmnopqrstuv-abcdefghijklm)"
expect_line "...and a 50-character one passes"        'Branch naming\.\.\. pass' "$out"
out="$(push_output feat/47/replays)"
expect_line "a one-word description fails"            'Branch naming\.\.\. fail \(must be feat\|fix\|chore.*2-4 word description' "$out"
out="$(push_output feat/47/save-the-event-replays-now)"
expect_line "...and so does a five-word one"          'Branch naming\.\.\. fail \(must be' "$out"
for name in chore/1617e87/aviodeck-workflow-sync chore/a1b2c3d/session-learnings chore/812/review-a1b2c3d; do
    out="$(push_output "$name")"
    expect_line "$name passes"                        'Branch naming\.\.\. pass' "$out"
done

echo "The branch judged is the one written on the remote:"
git -C "$REPO" checkout -q -B worktree-fix+289+logbook-map
push_lines "refs/heads/worktree-fix+289+logbook-map $HEAD_SHA refs/heads/$GOOD $Z"
expect_line "HEAD pushed from a worktree-* branch to a valid name passes naming" 'Branch naming\.\.\. pass' "$OUT"
same "...and the push passes" "$RC" "0"
git -C "$REPO" checkout -q -B "$GOOD"
push_lines "refs/heads/$GOOD $HEAD_SHA refs/heads/bad_name $Z"
expect_line "a valid local name pushed to a bad remote name fails" 'Branch naming\.\.\. fail \(must be feat\|fix\|chore' "$OUT"
same "...and the push fails" "$RC" "1"
for protected in main next; do
    push_lines "refs/heads/$GOOD $HEAD_SHA refs/heads/$protected $Z"
    same "a push to $protected alone exits 0" "$RC" "0"
    same "...and runs no check" "$(cat "$STUB_LOG")$OUT" ""
    push_lines "refs/heads/$GOOD $HEAD_SHA refs/heads/$protected $Z
refs/heads/$GOOD $HEAD_SHA refs/heads/bad_name $Z"
    expect_line "a $protected ref first still checks the branch pushed with it" 'Branch naming\.\.\. fail \(must be' "$OUT"
done
git -C "$REPO" checkout -q next && git -C "$REPO" reset -q --hard "$HEAD_SHA"
push_lines "refs/heads/next $HEAD_SHA refs/heads/$GOOD $Z"
expect_line "local next pushed to a feature ref is checked" "Running pre-push checks on $GOOD" "$OUT"
git -C "$REPO" checkout -q -B "$GOOD"
push_lines "refs/tags/v1 $HEAD_SHA refs/tags/v1 $Z"
expect_line "a tag-only push is judged as HEAD's branch" "Running pre-push checks on $GOOD" "$OUT"
push_lines ""
expect_line "no update on stdin is judged as HEAD's branch" "Running pre-push checks on $GOOD" "$OUT"

echo "Only HEAD, and only one branch, can be vouched for:"
push_lines "refs/heads/x $OTHER_SHA refs/heads/$GOOD $Z"
expect_line "a push of another commit is refused" "the checks judge HEAD, but this push sends ${OTHER_SHA:0:8}" "$OUT"
same "...and exits non-zero" "$RC" "1"
same "...and runs no check" "$(cat "$STUB_LOG")" ""
push_lines "refs/heads/x $HEAD_SHA refs/heads/$GOOD $Z
refs/heads/y $HEAD_SHA refs/heads/fix/289/other-work $Z"
expect_line "a second branch in the same push is refused" 'updates more than one branch' "$OUT"
same "...and exits non-zero" "$RC" "1"
same "...and runs no check" "$(cat "$STUB_LOG")" ""

echo "What runs without a marker:"
LINE="refs/heads/$GOOD $HEAD_SHA refs/heads/$GOOD $Z"
push_lines "$LINE"
ran 'cargo clippy --workspace --all-targets -- -D warnings ' && ok "clippy over the workspace, warnings as errors" || bad "clippy ($(cat "$STUB_LOG"))"
ran 'cargo nextest run --workspace --all-targets --profile ci DATABASE_URL=postgres://ois:ois@127.0.0.1:5432/ois ' && ok "nextest with the .env's DATABASE_URL" || bad "nextest ($(cat "$STUB_LOG"))"
grep -qx 'pnpm exec turbo run test --force' "$STUB_LOG" && ok "the JS suite, through turbo with --force (no cached replay)" || bad "pnpm test ($(cat "$STUB_LOG"))"
! grep -qx 'pnpm test' "$STUB_LOG" && ok "...and not the cacheable pnpm test script" || bad "ran pnpm test ($(cat "$STUB_LOG"))"
grep -q "CARGO_TARGET_DIR=$" "$STUB_LOG" && ! grep -q "CARGO_TARGET_DIR=." "$STUB_LOG" && ok "no CARGO_TARGET_DIR: cargo builds into the checkout's own target/" || bad "target dir ($(cat "$STUB_LOG"))"
[[ -n "$(ls -A "$OIS_LOCK_DIR" 2>/dev/null)" ]] && ok "nextest took the postgres lock" || bad "no lock file under $OIS_LOCK_DIR"

echo "A vouched tests marker for the pushed commit skips clippy and the suites:"
mkdir -p "$REPO/.claude/markers/tests"
touch "$REPO/.claude/markers/tests/$HEAD_SHA"
push_lines "$LINE"
ran 'cargo nextest' && ran 'cargo clippy' && ok "a marker with no manifest line: both run" || bad "a marker with no manifest line: both run ($OUT)"
rm -f "$REPO/.claude/markers/tests/$HEAD_SHA"
python3 -I "$ROOT/bin/lib/marker_manifest.py" stamp "$REPO" tests "$OTHER_SHA"
push_lines "$LINE"
ran 'cargo nextest' && ran 'cargo clippy' && ok "a marker for another commit: both run" || bad "a marker for another commit: both run ($OUT)"
python3 -I "$ROOT/bin/lib/marker_manifest.py" stamp "$REPO" tests "$HEAD_SHA"
push_lines "$LINE"
if ran 'cargo clippy' || ran 'cargo nextest' || ran 'pnpm exec turbo run test'; then bad "a vouched marker for the pushed commit: clippy and the suites are skipped ($(cat "$STUB_LOG"))"; else ok "a vouched marker for the pushed commit: clippy and the suites are skipped"; fi
expect_line "...and the skip says why"   "clippy \(workspace, -D warnings\)\.\.\. skip \(just test passed on ${HEAD_SHA:0:8}" "$OUT"
expect_line "...for each suite"          "Rust tests \(nextest\)\.\.\. skip \(just test passed on ${HEAD_SHA:0:8}" "$OUT"
expect_line "...both of them"            "JS tests \(pnpm test\)\.\.\. skip \(just test passed on ${HEAD_SHA:0:8}" "$OUT"
same "...and the push passes" "$RC" "0"
ran 'cargo deny' && ran 'pnpm audit' && ok "...and the audits still run" || bad "...and the audits still run ($(cat "$STUB_LOG"))"
rm -rf "$REPO/.claude"

echo "Failures are shown:"
export STUB_CLIPPY_RC=101 STUB_CLIPPY_OUT='    Checking ois-backend\nerror: unused variable: `x`\n'
push_lines "$LINE"
expect_line "a clippy error fails the check"           'clippy \(workspace, -D warnings\)\.\.\. fail' "$OUT"
expect_line "...shows clippy's error line"             'error: unused variable' "$OUT"
same "...and the push fails" "$RC" "1"
unset STUB_CLIPPY_RC STUB_CLIPPY_OUT
export STUB_NEXTEST_RC=100 STUB_NEXTEST_OUT='        FAIL [   0.012s] ois-backend handlers::tmu::tests::purge\n     Summary [   0.029s] 3 tests run: 2 passed, 1 failed, 1 skipped\nerror: test run failed\n'
push_lines "$LINE"
expect_line "a failing Rust test is named"             'FAIL \[   0.012s\] ois-backend handlers::tmu::tests::purge' "$OUT"
expect_line "...with nextest's summary"                'Summary \[   0.029s\] 3 tests run: 2 passed, 1 failed' "$OUT"
expect_line "...and Rust tests reported as failed"     'Rust tests \(nextest\)\.\.\. fail' "$OUT"
unset STUB_NEXTEST_RC STUB_NEXTEST_OUT
export STUB_VITEST_RC=1 STUB_VITEST_OUT='web:test:  FAIL  src/a.test.ts > b\nweb:test:  Test Files  1 failed (1)\n'
push_lines "$LINE"
expect_line "a failing vitest file is named"           'FAIL  src/a.test.ts > b' "$OUT"
expect_line "...and JS tests reported as failed"       'JS tests \(pnpm test\)\.\.\. fail' "$OUT"
unset STUB_VITEST_RC STUB_VITEST_OUT
mv "$REPO/.env" "$REPO/.env.away"
push_lines "$LINE"
expect_line "no DATABASE_URL fails the Rust tests"     'Rust tests \(nextest\)\.\.\. fail \(no DATABASE_URL' "$OUT"
ran 'cargo nextest' && bad "...nextest ran without DATABASE_URL" || ok "...and nextest does not run"
mv "$REPO/.env.away" "$REPO/.env"
export STUB_NO_NEXTEST=1
push_lines "$LINE"
expect_line "no cargo-nextest fails the Rust tests"    'Rust tests \(nextest\)\.\.\. fail \(cargo-nextest is not installed' "$OUT"
unset STUB_NO_NEXTEST

echo "cargo deny counts its errors and warns; pnpm audit warns; neither blocks:"
export STUB_DENY_RC=1 STUB_DENY_OUT='error[vulnerability]: a\nerror[vulnerability]: b\nwarning[duplicate]: c\nerror[license]: d\n'
out="$(push_output "$GOOD")"
expect_line "three errors, warnings not counted"       'cargo deny\.\.\. warnings \(3 error\(s\); non-blocking' "$out"
expect_line "...each shown"                            'error\[license\]: d' "$out"
expect_line "...and the push passes"                   'All pre-push checks passed' "$out"
unset STUB_DENY_RC STUB_DENY_OUT
export STUB_NO_DENY=1
out="$(push_output "$GOOD")"
expect_line "no cargo-deny is a named skip"            'cargo deny\.\.\. skip \(cargo-deny is not installed' "$out"
expect_line "...that does not fail the push"           'All pre-push checks passed' "$out"
unset STUB_NO_DENY
export STUB_AUDIT_RC=1
out="$(push_output "$GOOD")"
expect_line "a pnpm advisory is a warning"             'pnpm audit\.\.\. warnings \(non-blocking' "$out"
expect_line "...that does not fail the push"           'All pre-push checks passed' "$out"
unset STUB_AUDIT_RC

echo "A guidance-only push skips clippy and the suites, and nothing else:"
git -C "$REPO" update-ref refs/remotes/origin/next "$HEAD_SHA"
# branch_with BRANCH PATH...: BRANCH off the pushed base with one commit adding each PATH; sets TIP
branch_with() {
    local branch="$1" path
    shift
    git -C "$REPO" checkout -q -B "$branch" "$HEAD_SHA"
    for path in "$@"; do mkdir -p "$REPO/$(dirname "$path")" && printf 'x\n' > "$REPO/$path" && git -C "$REPO" add -f "$path"; done
    git -C "$REPO" -c core.hooksPath=/dev/null commit -qm "add $*"
    TIP="$(git -C "$REPO" rev-parse HEAD)"
}
branch_with chore/abc123/rules-only CLAUDE.md AGENTS.md .claude/rules/x.md .claude/agents/worker.md .claude/commands/start.md
push_lines "refs/heads/chore/abc123/rules-only $TIP refs/heads/chore/abc123/rules-only $Z"
expect_line "clippy is skipped, and says why"   'clippy \(workspace, -D warnings\)\.\.\. skip \(only CLAUDE\.md, AGENTS\.md and \.claude guidance changed' "$OUT"
expect_line "the Rust suite is skipped"         'Rust tests \(nextest\)\.\.\. skip \(only CLAUDE\.md' "$OUT"
expect_line "the JS suite is skipped"           'JS tests \(pnpm test\)\.\.\. skip \(only CLAUDE\.md' "$OUT"
if ran 'cargo clippy' || ran 'cargo nextest' || ran 'pnpm exec turbo run test'; then bad "...and none ran ($(cat "$STUB_LOG"))"; else ok "...and none ran"; fi
expect_line "the branch name is still checked"  'Branch naming\.\.\. pass' "$OUT"
expect_line "the audits still run"              'cargo deny\.\.\. pass' "$OUT"
expect_line "...both of them"                   'pnpm audit\.\.\. pass' "$OUT"
same "...and the push passes" "$RC" "0"
export STUB_DENY_RC=1 STUB_DENY_OUT='error[vulnerability]: a\n'
push_lines "refs/heads/chore/abc123/rules-only $TIP refs/heads/bad_name $Z"
expect_line "a bad remote name still fails a guidance-only push" 'Branch naming\.\.\. fail \(must be' "$OUT"
expect_line "...and an advisory still warns"    'cargo deny\.\.\. warnings \(1 error\(s\)' "$OUT"
unset STUB_DENY_RC STUB_DENY_OUT
for touched in DESIGN.md bin/tool.sh .claude/hooks/gate.sh .claude/commands/code-review.md .claude/agents/code-review-agent.md .claude/settings.json backend/src/lib.rs web/src/a.ts; do
    branch_with chore/abc123/mixed-change CLAUDE.md "$touched"
    push_lines "refs/heads/chore/abc123/mixed-change $TIP refs/heads/chore/abc123/mixed-change $Z"
    if ran 'cargo clippy' && ran 'cargo nextest' && ran 'pnpm exec turbo run test'; then ok "guidance plus $touched runs clippy and the suites"; else bad "guidance plus $touched skipped a check: $OUT"; fi
    refute_line "...with no skip line" 'guidance changed' "$OUT"
done
git -C "$REPO" update-ref -d refs/remotes/origin/next
branch_with chore/abc123/rules-only CLAUDE.md
push_lines "refs/heads/chore/abc123/rules-only $TIP refs/heads/chore/abc123/rules-only $Z"
if ran 'cargo clippy' && ran 'cargo nextest'; then ok "with no origin/next to compare against, they run"; else bad "no origin/next skipped a check: $OUT"; fi
git -C "$REPO" checkout -q -B "$GOOD" "$HEAD_SHA" && git -C "$REPO" clean -qfd

echo "The installed wrapper (.githooks/pre-push) finds the hook:"
mkdir -p "$REPO/.githooks"
cp "$WRAPPER" "$REPO/.githooks/pre-push"
push_lines "$LINE" "$REPO" "$REPO/.githooks/pre-push"
expect_line "no bin/pre-push anywhere is refused" 'pre-push: bin/pre-push not found' "$OUT"
same "...and exits non-zero" "$RC" "1"
same "...and runs no check" "$(cat "$STUB_LOG")" ""
mkdir -p "$REPO/bin/lib"
cp "$ROOT/bin/pre-push" "$ROOT/bin/check-marker.sh" "$ROOT/bin/with-lock.sh" "$REPO/bin/"
cp "$ROOT/bin/lib/git-hooks.sh" "$ROOT/bin/lib/marker_manifest.py" "$ROOT/bin/lib/portable.sh" "$ROOT/bin/lib/flock-run.pl" "$ROOT/bin/lib/claude-only-change.sh" "$REPO/bin/lib/"
python3 -I "$ROOT/bin/lib/marker_manifest.py" stamp "$REPO" tests "$HEAD_SHA"
push_lines "$LINE" "$REPO" "$REPO/.githooks/pre-push"
expect_line "the checkout's own bin/pre-push is used" 'All pre-push checks passed' "$OUT"
expect_line "...and its marker skips the suites"      'Rust tests \(nextest\)\.\.\. skip \(just test passed' "$OUT"
# A worktree with neither bin/ nor .githooks/ (both untracked above) falls back to the main checkout's.
git -C "$REPO" worktree add -q -b fix/289/older-hook "$WORK/wt" "$HEAD_SHA"
cp "$REPO/.env" "$WORK/wt/.env" && mkdir -p "$WORK/wt/node_modules"
push_lines "refs/heads/fix/289/older-hook $HEAD_SHA refs/heads/fix/289/older-hook $Z" "$WORK/wt" "$REPO/.githooks/pre-push"
expect_line "a worktree without bin/ uses the main checkout's" 'All pre-push checks passed' "$OUT"
ran 'cargo nextest' && ok "...and the main checkout's marker does not vouch for the worktree" || bad "...and the main checkout's marker does not vouch for the worktree ($OUT)"
mv "$REPO/bin" "$WORK/wt/bin"
python3 -I "$ROOT/bin/lib/marker_manifest.py" stamp "$WORK/wt" tests "$HEAD_SHA"
push_lines "refs/heads/fix/289/older-hook $HEAD_SHA refs/heads/fix/289/older-hook $Z" "$WORK/wt" "$REPO/.githooks/pre-push"
expect_line "a worktree with bin/ does not need the main checkout's" 'All pre-push checks passed' "$OUT"
expect_line "...and its own marker skips the suites"  'Rust tests \(nextest\)\.\.\. skip \(just test passed' "$OUT"

echo ""
echo "pre-push: $PASS passed, $FAIL failed"
(( FAIL == 0 ))
