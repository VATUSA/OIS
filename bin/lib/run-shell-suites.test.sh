#!/bin/bash
# ==============================================================================
# Tests for bin/run-shell-suites.sh: the one runner CI's `shell-tests` job and the
# `shell suites` step of bin/run-tests.sh both call.
#
#   bash bin/lib/run-shell-suites.test.sh
#
# Each case copies the runner into a scratch git repo whose bin/lib holds stub
# suites written by the case, so every suite's output and exit code is whatever the
# case says. Never runs the runner against this checkout: the runner would run this
# suite, which would run the runner again. The last cases scan ci.yml, since the
# workflow runs only on GitHub. Self-reports pass/fail, exits non-zero on any failure.
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/bin/run-shell-suites.sh"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/run-shell-suites-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR
export GIT_AUTHOR_NAME=T GIT_AUTHOR_EMAIL=t@e.com GIT_COMMITTER_NAME=T GIT_COMMITTER_EMAIL=t@e.com
export GIT_CONFIG_NOSYSTEM=1 HOME="$WORK"

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

# new_repo NAME: a committed scratch repo holding a copy of the runner and an empty
# bin/lib. Sets REPO.
new_repo() {
    REPO="$WORK/repo.$1"
    export RAN="$WORK/ran.$1"
    mkdir -p "$REPO/bin/lib"
    git -C "$REPO" init -q -b feat/1/x
    cp "$SCRIPT" "$REPO/bin/run-shell-suites.sh"
    git -C "$REPO" add bin
    git -C "$REPO" -c core.hooksPath=/dev/null commit -qm init
}

# suite PATH BODY: write a stub suite at $REPO/PATH (relative), mode 0755.
suite() {
    mkdir -p "$(dirname "$REPO/$1")"
    printf '#!/bin/bash\n%s\n' "$2" > "$REPO/$1"
    chmod +x "$REPO/$1"
}

# invoke: run the repo's copy of the runner from its top level with a 60 s ceiling;
# sets RC and OUTPUT.
invoke() {
    OUTPUT="$(cd "$REPO" && timeout 60 bash "$REPO/bin/run-shell-suites.sh" 2>&1)"
    RC=$?
}

# has TEXT: OUTPUT contains the line TEXT exactly.
has() { grep -qxF -- "$1" <<<"$OUTPUT"; }
# ran: the suites that ran in this case, in order, joined by spaces.
ran() { [[ -f "$RAN" ]] && paste -sd' ' "$RAN"; }
# A suite body that records its name and exits CODE.
rec() { printf 'echo %q >> "$RAN"; exit %s' "$1" "${2:-0}"; }

echo "Runs every suite the glob finds:"
new_repo all-pass
suite bin/lib/c.test.sh "$(rec c)"; suite bin/lib/a.test.sh "$(rec a)"; suite bin/lib/b.test.sh "$(rec b)"
invoke
if [[ $RC -eq 0 && "$(ran)" == "a b c" ]] && has "PASS bin/lib/a.test.sh" && has "PASS bin/lib/b.test.sh" \
    && has "PASS bin/lib/c.test.sh" && has "shell suites: 4 passed, 0 failed"; then
    ok "every suite passes: exit 0, each named PASS, run in glob order"
else bad "all pass: rc=$RC ran='$(ran)': $OUTPUT"; fi

new_repo new-name
suite bin/lib/zz-brand-new-919.test.sh "$(rec zz)"
invoke
[[ $RC -eq 0 && "$(ran)" == zz ]] && has "PASS bin/lib/zz-brand-new-919.test.sh" \
    && ok "a suite under a name nothing lists is run" || bad "new name: rc=$RC ran='$(ran)': $OUTPUT"

new_repo not-suites
suite bin/lib/a.test.sh "$(rec a)"
for f in bin/lib/helper.sh bin/lib/x.test.sh.bak bin/lib/x.test.py bin/x.test.sh bin/lib/sub/x.test.sh; do
    suite "$f" "$(rec "$f" 1)"
done
invoke
if [[ $RC -eq 0 && "$(ran)" == a ]] && has "shell suites: 1 passed, 0 failed" \
    && ! grep -qE 'helper|\.bak|\.py|bin/x\.test|sub/' <<<"$OUTPUT"; then
    ok "only bin/lib/*.test.sh runs: not helper.sh, .bak, .py, bin/x.test.sh or bin/lib/sub/"
else bad "non-suites: rc=$RC ran='$(ran)': $OUTPUT"; fi

new_repo space
suite "bin/lib/a b.test.sh" "$(rec "a b")"
invoke
[[ $RC -eq 0 && "$(ran)" == "a b" ]] && has "PASS bin/lib/a b.test.sh" \
    && ok "a suite whose name has a space is run and named" || bad "space: rc=$RC ran='$(ran)': $OUTPUT"

new_repo no-exec
suite bin/lib/a.test.sh "$(rec a)"
chmod 0644 "$REPO/bin/lib/a.test.sh"
invoke
[[ $RC -eq 0 && "$(ran)" == a ]] && ok "a suite without the executable bit is run" || bad "no exec bit: rc=$RC: $OUTPUT"

echo "A failing suite fails the run, and is named:"
new_repo one-fails
suite bin/lib/a.test.sh "$(rec a)"; suite bin/lib/b.test.sh "$(rec b 1)"; suite bin/lib/c.test.sh "$(rec c)"
invoke
if [[ $RC -eq 1 && "$(ran)" == "a b c" ]] && has "FAIL bin/lib/b.test.sh (exit 1)" && has "PASS bin/lib/c.test.sh" \
    && has "failing: bin/lib/b.test.sh"; then
    ok "one suite exits 1: the run exits 1, names it, and still runs the suites after it"
else bad "one fails: rc=$RC ran='$(ran)': $OUTPUT"; fi

new_repo exit-3
suite bin/lib/a.test.sh 'exit 3'
invoke
[[ $RC -eq 1 ]] && has "FAIL bin/lib/a.test.sh (exit 3)" && ok "a suite exiting 3 is FAIL with its exit code; the run exits 1" \
    || bad "exit 3: rc=$RC: $OUTPUT"

new_repo killed
suite bin/lib/a.test.sh 'kill -9 $$'
invoke
[[ $RC -eq 1 ]] && has "FAIL bin/lib/a.test.sh (exit 137)" && has "shell suites: 0 passed, 1 failed" \
    && ok "a suite killed by a signal is FAIL and the summary still prints" || bad "killed: rc=$RC: $OUTPUT"

new_repo counts
for n in a c e; do suite "bin/lib/$n.test.sh" "$(rec $n)"; done
for n in b d; do suite "bin/lib/$n.test.sh" "echo out-$n; echo err-$n >&2; exit 1"; done
suite bin/lib/a.test.sh "echo pass-token-a; $(rec a)"
invoke
if [[ $RC -eq 1 ]] && has "shell suites: 3 passed, 2 failed" && has "failing: bin/lib/b.test.sh, bin/lib/d.test.sh"; then
    ok "two suites fail: the summary counts 3 passed, 2 failed and names both"
else bad "counts: rc=$RC: $OUTPUT"; fi
has "    out-b" && has "    err-b" && has "    out-d" && has "    err-d" \
    && ok "...each failing suite's stdout and stderr are printed" || bad "failing logs missing: $OUTPUT"
[[ "$OUTPUT" != *pass-token-a* ]] && ok "...a passing suite's output is not" || bad "passing log printed: $OUTPUT"

echo "Fails closed when nothing runs:"
new_repo empty
invoke
[[ $RC -eq 1 && "$OUTPUT" == *"no suite matches bin/lib/*.test.sh"* ]] \
    && ok "no suite in bin/lib: exit 1, saying so" || bad "empty: rc=$RC: $OUTPUT"
new_repo no-lib
rm -r "$REPO/bin/lib"
invoke
[[ $RC -eq 1 && "$OUTPUT" == *"no suite matches bin/lib/*.test.sh"* ]] \
    && ok "no bin/lib directory: exit 1, saying so" || bad "no bin/lib: rc=$RC: $OUTPUT"
# The runner is run from another scratch repo's bin/, never this checkout's: one that resolved its
# root from its own location would then run this checkout's suites, this one among them, and recurse.
new_repo home
suite bin/lib/home.test.sh "$(rec home)"
HOME_RUNNER="$REPO/bin/run-shell-suites.sh"
mkdir -p "$WORK/no-repo"
export RAN="$WORK/ran.no-repo"
OUTPUT="$(cd "$WORK/no-repo" && timeout 60 bash "$HOME_RUNNER" 2>&1)"; RC=$?
[[ $RC -eq 1 && "$OUTPUT" == *"not inside a git repository"* && -z "$(ran)" ]] \
    && ok "outside a git repository: exit 1, saying so, running no suite" || bad "no repo: rc=$RC ran='$(ran)': $OUTPUT"

echo "Runs the way CI and the gate need it to:"
new_repo stdin
suite bin/lib/a.test.sh 'cat > "$RAN"; exit 0'
# stdin held open by a writer that outlives the timeout: without </dev/null the suite's cat blocks.
OUTPUT="$(cd "$REPO" && timeout 20 bash "$REPO/bin/run-shell-suites.sh" 2>&1 < <(exec 2>/dev/null; sleep 25; echo leaked))"; RC=$?
[[ $RC -eq 0 && ! -s "$RAN" ]] && ok "a suite reading stdin gets end-of-file and does not hang the run" \
    || bad "stdin: rc=$RC (124 is the hang) stdin='$(cat "$RAN" 2>/dev/null)': $OUTPUT"

new_repo serial
for n in a b; do suite "bin/lib/$n.test.sh" "echo START $n >> \"\$RAN\"; sleep 1; echo END $n >> \"\$RAN\""; done
invoke
[[ $RC -eq 0 && "$(ran)" == "START a END a START b END b" ]] && ok "suites run one at a time" \
    || bad "serial: '$(ran)'"

new_repo subdir
mkdir -p "$REPO/sub"
suite bin/lib/a.test.sh 'pwd > "$RAN"'
OUTPUT="$(cd "$REPO/sub" && bash ../bin/run-shell-suites.sh 2>&1)"; RC=$?
[[ $RC -eq 0 && "$(cat "$RAN")" == "$(cd "$REPO" && pwd -P)" ]] \
    && ok "run from a subdirectory by a relative path: suites run from the top level" \
    || bad "subdir: rc=$RC pwd='$(cat "$RAN" 2>/dev/null)': $OUTPUT"

new_repo cwd-repo
suite bin/lib/a.test.sh "$(rec a)"
OUTPUT="$(cd "$REPO" && timeout 60 bash "$HOME_RUNNER" 2>&1)"; RC=$?
[[ $RC -eq 0 && "$(ran)" == a ]] && has "shell suites: 1 passed, 0 failed" \
    && ok "a runner from another repo's bin/ runs the current repo's suites only" \
    || bad "cwd repo: rc=$RC ran='$(ran)': $OUTPUT"

echo "SKIP comes only from inside a suite:"
new_repo skip-0
suite bin/lib/a.test.sh "$(rec a)"; suite bin/lib/b.test.sh 'echo "SKIP no zip here"; exit 0'
invoke
[[ $RC -eq 0 ]] && has "PASS bin/lib/b.test.sh" && has "shell suites: 2 passed, 0 failed" \
    && ok "a suite printing SKIP and exiting 0 passes" || bad "skip 0: rc=$RC: $OUTPUT"
new_repo skip-1
suite bin/lib/a.test.sh 'echo "SKIP no zip here"; exit 1'
invoke
[[ $RC -eq 1 ]] && has "FAIL bin/lib/a.test.sh (exit 1)" \
    && ok "a suite printing SKIP and exiting 1 fails: only the exit code counts" || bad "skip 1: rc=$RC: $OUTPUT"

echo "CI's shell-tests job:"
# The job's block, from its key to the next job key at the same indent. A source scan: the
# workflow only runs on GitHub, and the red run on the PR (#909) is the behavioral proof.
JOB="$(awk '/^  shell-tests:$/ {on=1; print; next} on && /^  [A-Za-z0-9_-]+:$/ {exit} on' "$ROOT/.github/workflows/ci.yml")"
if [[ -z "$JOB" ]]; then
    bad "ci.yml has no shell-tests job"
else
    grep -qE '^    runs-on: ubuntu-latest$' <<<"$JOB" && grep -qE '^      - run: bash bin/run-shell-suites\.sh$' <<<"$JOB" \
        && ok "ci.yml's shell-tests job runs bin/run-shell-suites.sh on ubuntu-latest" || bad "shell-tests job: $JOB"
    ! grep -qE '^ +(continue-on-error|if):' <<<"$JOB" \
        && ok "...with no continue-on-error or if: that would let a red run pass or skip the job" \
        || bad "shell-tests job can pass red or be skipped: $JOB"
    names=0 hits=""
    for f in "$ROOT"/bin/lib/*.test.sh; do
        names=$((names + 1))
        n="$(basename "$f")"
        grep -qF -- "$n" <<<"$JOB" && hits="$hits job:$n"
        grep -vE '^[[:space:]]*#' "$ROOT/bin/run-shell-suites.sh" | grep -qF -- "$n" && hits="$hits runner:$n"
    done
    if [[ $names -lt 30 ]]; then bad "the suite glob matched only $names names"
    elif [[ -n "$hits" ]]; then bad "a suite is named, not globbed:$hits"
    else ok "neither the job nor the runner names any of the $names suites"; fi
fi

echo ""
echo "run-shell-suites: $PASS passed, $FAIL failed"
(( FAIL == 0 ))
