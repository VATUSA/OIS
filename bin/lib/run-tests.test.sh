#!/bin/bash
# ==============================================================================
# Tests for bin/run-tests.sh: which runs write the tests marker the PR gate reads,
# what each step's summary must say, and the targeted interfaces.
#
#   bash bin/lib/run-tests.test.sh
#
# Each case runs the script inside a scratch git repo with stub `cargo` and `pnpm`
# first on PATH, so every step's output and exit code are whatever the case says.
# Removing any guard turns at least one refusal case red. Self-reports pass/fail,
# exits non-zero on any failure.
#
# The nextest and doc-test lines are byte-for-byte from real `cargo nextest run
# --profile ci` (0.9.144) and `cargo test --doc` runs on a scratch crate; the vitest
# lines are byte-for-byte from real `vitest run` runs (FORCE_COLOR=1, as turbo
# runs it), with turbo's `web:test: ` prefix added. None is typed.
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/bin/run-tests.sh"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/run-tests-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR CARGO_TARGET_DIR
export GIT_AUTHOR_NAME=T GIT_AUTHOR_EMAIL=t@e.com GIT_COMMITTER_NAME=T GIT_COMMITTER_EMAIL=t@e.com
export GIT_CONFIG_NOSYSTEM=1 HOME="$WORK"
# Real locks, in a directory no live run uses.
export OIS_LOCK_DIR="$WORK/locks"
unset OIS_NO_LOCK CI

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

NX_GREEN='     Summary [   0.009s] 3 tests run: 3 passed, 1 skipped'
NX_RED='    test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 3 filtered out; finished in 0.00s\n     Summary [   0.029s] 3 tests run: 2 passed, 1 failed, 1 skipped\nerror: test run failed'
NX_FLAKY='    test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 3 filtered out; finished in 0.00s\n     Summary [   0.017s] 3 tests run: 3 passed (1 flaky), 1 skipped'
NX_NONE='     Summary [   0.000s] 0 tests run: 0 passed, 4 skipped\nerror: no tests to run'
DOC_OK='test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s'
DOC_FAILED='test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.19s'
VT_GREEN='web:test: \033[2m Test Files \033[22m \033[1m\033[32m1 passed\033[39m\033[22m\033[90m (1)\033[39m\nweb:test: \033[2m      Tests \033[22m \033[1m\033[32m2 passed\033[39m\033[22m\033[2m | \033[22m\033[33m1 skipped\033[39m\033[90m (3)\033[39m'
VT_RED='web:test: \033[41m\033[1m FAIL \033[22m\033[49m a.test.ts\033[2m > \033[22mmaybe\nweb:test: \033[2m Test Files \033[22m \033[1m\033[31m1 failed\033[39m\033[22m\033[90m (1)\033[39m\nweb:test: \033[2m      Tests \033[22m \033[1m\033[31m1 failed\033[39m\033[22m\033[2m | \033[22m\033[1m\033[32m1 passed\033[39m\033[22m\033[2m | \033[22m\033[33m1 skipped\033[39m\033[90m (3)\033[39m'
VT_NONE='web:test: \033[31mNo test files found, exiting with code 1'
REAL_GIT="$(command -v git)"

# Stubs. Each records its arguments in $STUB_LOG and answers per STUB_* variables:
#   cargo nextest run  prints $STUB_NX (default NX_GREEN), runs $STUB_HOOK, exits $STUB_NX_RC
#   cargo test --doc   prints $STUB_DOC (default DOC_OK), exits $STUB_DOC_RC
#   pnpm exec turbo run test  prints $STUB_VT (default VT_GREEN), then the file $STUB_VT_TAIL if set, exits $STUB_VT_RC
#   cargo <fmt|clippy|deny check>, pnpm audit, pnpm exec turbo run <lint|typecheck>  exit $STUB_<STEP>_RC
#   pnpm lint|typecheck|test (the scripts, which turbo may answer from its cache)  exit 98
#   cargo nextest|deny --version  fail when $STUB_NO_NEXTEST / $STUB_NO_DENY is set
#   pnpm ... codegen   rewrites the schema when $STUB_DRIFT is set
mkdir -p "$WORK/bin"
cat > "$WORK/bin/cargo" <<'STUB'
#!/bin/bash
echo "cargo $*" >> "$STUB_LOG"
case "$1 ${2:-}" in
    "nextest --version") [[ -z "${STUB_NO_NEXTEST:-}" ]]; exit ;;
    "deny --version") [[ -z "${STUB_NO_DENY:-}" ]]; exit ;;
    "nextest run")
        printf '%s\n' "$*" > "$WORK_ARGS"
        printf '%b\n' "${STUB_NX-$NX_GREEN}"
        [[ -n "${STUB_HOOK:-}" ]] && eval "$STUB_HOOK"
        exit "${STUB_NX_RC:-0}" ;;
    "fmt "*) exit "${STUB_FMT_RC:-0}" ;;
    "clippy "*) exit "${STUB_CLIPPY_RC:-0}" ;;
    "deny check") exit "${STUB_DENY_RC:-0}" ;;
    "test "*)
        case " $* " in
            *" --doc "*) printf '%b\n' "${STUB_DOC-$DOC_OK}"; exit "${STUB_DOC_RC:-0}" ;;
            *openapi::dump_openapi_json*) printf '{}\n' > "$OIS_OPENAPI_OUT"; exit 0 ;;
        esac ;;
esac
echo "cargo stub: unexpected $*" >&2
exit 99
STUB
cat > "$WORK/bin/pnpm" <<'STUB'
#!/bin/bash
echo "pnpm $*" >> "$STUB_LOG"
case "$1" in
    install) exit 0 ;;
    lint|typecheck|test) exit 98 ;;
    exec)
        case "$2 $3 $4" in
            "turbo run lint") exit "${STUB_LINT_RC:-0}" ;;
            "turbo run typecheck") exit "${STUB_TYPECHECK_RC:-0}" ;;
            "turbo run test") printf '%b\n' "${STUB_VT-$VT_GREEN}"; [[ -n "${STUB_VT_TAIL:-}" ]] && cat "$STUB_VT_TAIL"; exit "${STUB_VT_RC:-0}" ;;
        esac ;;
    audit) exit "${STUB_AUDIT_RC:-0}" ;;
    --filter)
        case "$*" in
            *codegen*)
                [[ -f "$OIS_OPENAPI_URL" ]] || exit 1
                [[ -n "${STUB_DRIFT:-}" ]] && echo "// drifted" >> packages/api-client/src/generated/schema.d.ts
                exit 0 ;;
            *"exec vitest run"*) printf '%s\n' "$*" > "$WORK_ARGS"; exit 0 ;;
        esac ;;
esac
echo "pnpm stub: unexpected $*" >&2
exit 99
STUB
# With GIT_FAIL_STATUS set, `git status` fails as a corrupt index would.
cat > "$WORK/bin/git" <<STUB
#!/bin/bash
if [[ -n "\${GIT_FAIL_STATUS:-}" && "\$1" == "status" ]]; then exit 128; fi
exec "$REAL_GIT" "\$@"
STUB
chmod +x "$WORK/bin/cargo" "$WORK/bin/pnpm" "$WORK/bin/git"
export NX_GREEN DOC_OK VT_GREEN

# new_repo NAME [unborn]: a repo laid out as the gate expects, committed unless unborn,
# with a .env and node_modules (both ignored).
new_repo() {
    REPO="$WORK/repo.$1"
    mkdir -p "$REPO/.github/scripts" "$REPO/packages/api-client/src/generated" "$REPO/node_modules"
    git -C "$REPO" init -q -b feat/1/x
    printf '#!/bin/bash\nexit "${STUB_MIGRATIONS_RC:-0}"\n' > "$REPO/.github/scripts/check-migration-versions.sh"
    chmod +x "$REPO/.github/scripts/check-migration-versions.sh"
    printf 'export interface paths {}\n' > "$REPO/packages/api-client/src/generated/schema.d.ts"
    printf 'DATABASE_URL=postgres://ois:ois@127.0.0.1:5432/ois\n' > "$REPO/.env"
    printf '.claude/markers/\n.env\nnode_modules/\ntarget/\n' > "$REPO/.gitignore"
    SHA=""
    [[ "${2:-}" == unborn ]] && return
    git -C "$REPO" add -A
    git -C "$REPO" -c core.hooksPath=/dev/null commit -qm init
    SHA="$(git -C "$REPO" rev-parse HEAD)"
}

# invoke NAME [ARGS...]: run the script in the current $REPO; sets RC and OUTPUT, then
# clears every per-case STUB_* variable.
invoke() {
    export STUB_LOG="$WORK/log.$1" WORK_ARGS="$WORK/args.$1"
    : > "$STUB_LOG"
    shift
    OUTPUT="$(cd "$REPO" && PATH="$WORK/bin:$PATH" bash "$SCRIPT" "$@" 2>&1)"
    RC=$?
    unset STUB_HOOK GIT_FAIL_STATUS STUB_NX STUB_NX_RC STUB_DOC STUB_DOC_RC STUB_VT STUB_VT_RC STUB_VT_TAIL \
        STUB_FMT_RC STUB_CLIPPY_RC STUB_DENY_RC STUB_LINT_RC STUB_TYPECHECK_RC STUB_AUDIT_RC \
        STUB_MIGRATIONS_RC STUB_NO_NEXTEST STUB_NO_DENY STUB_DRIFT
}

# run NAME [ARGS...]: invoke in a fresh committed repo.
run() {
    new_repo "$1"
    invoke "$@"
}

stamped() { [[ -n "$SHA" ]] && python3 -I "$ROOT/bin/lib/marker_manifest.py" vouched "$REPO" tests "$SHA"; }

expect_stamp() {
    if stamped && [[ $RC -eq 0 ]]; then ok "$1"; else bad "$1 (rc=$RC, marker missing): $OUTPUT"; fi
}

# No marker file, and no claim of one: a guard that falls through to a failed
# write must not pass as a guard that refused. REASON, when given, must be the one named.
expect_none() {
    if [[ -n "$(ls -A "$REPO/.claude/markers/tests" 2>/dev/null)" ]]; then
        bad "$1 (marker written)"
    elif [[ "$OUTPUT" != *"no tests marker written${2:+ ($2}"* ]]; then
        bad "$1 (did not refuse${2:+ with '$2'}): $OUTPUT"
    else
        ok "$1"
    fi
}
# summary_has NAME TEXT: TEXT is among the summary lines
summary_has() {
    if sed -n '/^==> run-tests summary$/,$p' <<<"$OUTPUT" | grep -qF -- "$2"; then ok "$1"; else bad "$1 (no '$2' in the summary): $OUTPUT"; fi
}

echo "Stamps:"
run clean
expect_stamp "every step green on a clean commit (captured nextest, doc-test and vitest summaries)"
summary_has "...the summary lists each step's result" "PASS     rust tests (cargo nextest --profile ci)"
summary_has "...the vitest step included" "PASS     pnpm test"
if [[ "$(cat "$WORK_ARGS")" == "nextest run --workspace --all-targets --profile ci" ]]; then ok "runs cargo nextest run --workspace --all-targets --profile ci"; else bad "nextest arguments: $(cat "$WORK_ARGS")"; fi
want="cargo fmt --all -- --check|cargo clippy --workspace --all-targets -- -D warnings|cargo nextest --version|cargo nextest run|cargo test --workspace --doc|pnpm install --frozen-lockfile --prefer-offline|pnpm exec turbo run lint --force|pnpm exec turbo run typecheck --force|pnpm exec turbo run test --force|pnpm audit --audit-level=high|cargo deny --version|cargo deny check|cargo test -p ois-backend --lib openapi::dump_openapi_json|pnpm --filter @ois/api-client codegen"
got="$(sed -E 's/^(cargo nextest run|cargo test -p ois-backend --lib openapi::dump_openapi_json|pnpm --filter @ois\/api-client codegen).*/\1/' "$STUB_LOG" | paste -sd'|' -)"
[[ "$got" == "$want" ]] && ok "...every CI step, in CI's order, and no cargo check" || bad "steps run: $got"
for task in lint typecheck test; do
    grep -qx "pnpm exec turbo run $task --force" "$STUB_LOG" && ok "...turbo runs $task with --force (no cached replay)" || bad "$task without --force: $(cat "$STUB_LOG")"
done
summary_has "...the audits are labeled ADVISORY" "ADVISORY pnpm audit --audit-level=high (pass)"
summary_has "...both of them" "ADVISORY cargo deny check (pass)"
[[ -n "$(ls -A "$OIS_LOCK_DIR" 2>/dev/null)" ]] && ok "...the Rust suite took the postgres lock" || bad "no lock file under $OIS_LOCK_DIR"
grep -q "CARGO_TARGET_DIR=<cargo default> " <<<"$OUTPUT" && ok "...and exports no CARGO_TARGET_DIR (cargo builds into the checkout's own target/)" || bad "target dir line: $(grep CARGO_TARGET_DIR <<<"$OUTPUT")"

export STUB_NX="$NX_FLAKY"
run flaky
expect_stamp "a flaky test that passed on retry, as CI's profile allows"

STUB_VT="$(printf '%b' "$VT_GREEN" | sed $'s/\033\\[[0-9;]*m//g' | sed 's/^web:test: //')"
export STUB_VT
run plain-vitest
expect_stamp "the vitest summary without color codes or a turbo prefix"

echo "The audits are advisory: they print their result and stamp anyway (an advisory on next or no network must not
dead-end every branch; CI's js and deny jobs still gate the merge):"
for step in AUDIT DENY; do
    export "STUB_${step}_RC=1"
    run "advisory-$step"
    expect_stamp "a failing ${step} step"
done
summary_has "...the failure is shown, labeled ADVISORY" "ADVISORY cargo deny check (failed: exited 1; does not block the marker"
export STUB_NO_DENY=1
run no-deny
expect_stamp "cargo-deny not installed"
summary_has "...reported as an advisory step not run" "ADVISORY cargo deny check (not run: cargo-deny is not installed"

echo "Does not stamp:"
export STUB_NX_RC=100
run nextest-exit
expect_none "a non-zero nextest exit over a clean summary" "failed: rust tests"
[[ $RC -eq 1 ]] && ok "...and the gate exits 1" || bad "exit code $RC, want 1"

export STUB_NX="$NX_RED"
run nextest-red
expect_none "exit 0 over the captured nextest failure summary" "failed: rust tests"
summary_has "...and the summary says why" "Summary reports failures: 3 tests run: 2 passed, 1 failed, 1 skipped"

export STUB_NX="$NX_NONE"
run nextest-none
expect_none "exit 0 over a nextest run that ran no test" "failed: rust tests"
summary_has "...named as no tests run" "Summary reports no tests run"

export STUB_NX="running 3 tests"
run nextest-no-summary
expect_none "exit 0 with no nextest Summary line" "failed: rust tests"

export STUB_NX='     Summary [   0.009s] 3 tests run: 2 passed, 1 skipped'
run nextest-short
expect_none "fewer passed than run with no failure word" "failed: rust tests"

export STUB_DOC="$DOC_FAILED"
run doc-red
expect_none "exit 0 over a FAILED doc-test result" "failed: doc tests"

export STUB_DOC="Doc-tests ois_core"
run doc-none
expect_none "doc tests with no test result line" "failed: doc tests"

export STUB_VT="$VT_RED"
run vitest-red
expect_none "exit 0 over the captured vitest failure summary" "failed: pnpm test"

export STUB_VT="$VT_NONE"
run vitest-none
expect_none "exit 0 over 'No test files found'" "failed: pnpm test"

export STUB_VT='web:test: \033[2m Test Files \033[22m \033[1m\033[32m1 passed\033[39m\033[22m\033[90m (1)\033[39m\nweb:test: \033[2m      Tests \033[22m \033[33m3 skipped\033[39m\033[90m (3)\033[39m'
run vitest-zero
expect_none "a vitest run that passed no test" "failed: pnpm test"

export STUB_VT="$VT_GREEN\nweb:test:  Errors  1 error"
run vitest-errors
expect_none "a clean vitest summary contradicted by an Errors line" "failed: pnpm test"

# More than a pipe buffer after the match: a `printf | grep -q` check SIGPIPEs and misses it. The 300KB goes
# through a file: Linux refuses to exec with any one environment string over 128KB (MAX_ARG_STRLEN).
{ head -c 300000 /dev/zero | tr '\0' 'x'; printf '\n%b\n' "$VT_GREEN"; } > "$WORK/vt-tail"
export STUB_VT="web:test:  FAIL  src/a.test.ts > b" STUB_VT_TAIL="$WORK/vt-tail"
run vitest-long
expect_none "a FAIL line followed by 300KB of output" "failed: pnpm test"

for step in FMT CLIPPY LINT TYPECHECK MIGRATIONS; do
    export "STUB_${step}_RC=1"
    run "exit-$step"
    expect_none "a failing ${step} step" "failed:"
done

export STUB_DRIFT=1
run drift
expect_none "a stale generated client" "failed: client drift"
[[ -z "$(git -C "$REPO" status --porcelain)" ]] && ok "...and the schema is restored afterwards" || bad "schema left modified: $(git -C "$REPO" status --porcelain)"

export STUB_NO_NEXTEST=1
run no-nextest
expect_none "cargo-nextest not installed" "could not run: rust tests"
summary_has "...reported as MISSING with the install line" "MISSING  rust tests (cargo nextest --profile ci) (cargo-nextest is not installed"
if grep -q '^cargo nextest run' "$STUB_LOG"; then bad "...and nothing pretends to run it"; else ok "...and nothing pretends to run it"; fi


new_repo unborn unborn
invoke unborn
expect_none "a repo with no HEAD" "HEAD could not be read"

new_repo dirty
printf 'x\n' > "$REPO/untracked.rs"
export STUB_HOOK='rm untracked.rs'
invoke dirty
expect_none "a tree dirty at start and reverted during the run" "working tree was not clean at start"

new_repo hidden-untracked
git -C "$REPO" config status.showUntrackedFiles no
printf 'x\n' > "$REPO/untracked.rs"
invoke hidden-untracked
expect_none "untracked files hidden by status.showUntrackedFiles=no" "working tree was not clean at start"

new_repo status-fails
export GIT_FAIL_STATUS=1
invoke status-fails
expect_none "git status failing" "working tree was not clean at start"

export STUB_HOOK='git -c core.hooksPath=/dev/null commit -q --allow-empty -m moved'
run moved
expect_none "HEAD moved during the run" "HEAD moved during the run"

export STUB_HOOK='printf "y\n" > edited.rs'
run dirtied
expect_none "a tree dirtied during the run" "working tree changed during the run"

run no-stamp --no-stamp
expect_none "--no-stamp runs the gate and never stamps" "--no-stamp"
[[ $RC -eq 0 ]] && ok "...and exits 0 when every step passed" || bad "--no-stamp exit $RC"

echo "Refuses to start:"
new_repo no-env
rm "$REPO/.env"
invoke no-env
expect_none "no .env" ".env missing"
[[ ! -s "$STUB_LOG" ]] && ok "...before running any step" || bad "steps ran without .env: $(cat "$STUB_LOG")"
new_repo no-url
printf 'API_PORT=3000\n' > "$REPO/.env"
invoke no-url
expect_none "a .env without DATABASE_URL" "DATABASE_URL unset"
new_repo no-modules
rmdir "$REPO/node_modules"
invoke no-modules
expect_none "no node_modules" "node_modules missing"

echo "Subdirectory:"
new_repo subdir
mkdir -p "$REPO/bin/lib" "$REPO/sub"
cp "$SCRIPT" "$ROOT/bin/with-lock.sh" "$REPO/bin/" && cp "$ROOT/bin/lib/marker_manifest.py" "$ROOT/bin/lib/file_lock.py" "$ROOT/bin/lib/portable.sh" "$ROOT/bin/lib/flock-run.pl" "$REPO/bin/lib/" && printf 'x\n' > "$REPO/sub/f"
git -C "$REPO" add bin sub && git -C "$REPO" -c core.hooksPath=/dev/null commit -qm script
SHA="$(git -C "$REPO" rev-parse HEAD)"
export STUB_LOG="$WORK/log.subdir" WORK_ARGS="$WORK/args.subdir"
OUTPUT="$(cd "$REPO/sub" && PATH="$WORK/bin:$PATH" bash ../bin/run-tests.sh 2>&1)"
RC=$?
expect_stamp "run by a relative path from a subdirectory"

echo "Targeted runs never stamp:"
run rust rust -p ois-backend -E 'test(scope_)'
if [[ -z "$(ls -A "$REPO/.claude/markers/tests" 2>/dev/null)" ]]; then ok "a targeted Rust run"; else bad "a targeted Rust run (marker written)"; fi
if [[ "$(cat "$WORK_ARGS")" == "nextest run -p ois-backend -E test(scope_)" ]]; then ok "...passes its arguments to cargo nextest run"; else bad "rust arguments: $(cat "$WORK_ARGS")"; fi
[[ "$(wc -l < "$STUB_LOG" | tr -d ' ')" == 1 ]] && ok "...and runs nothing else" || bad "rust ran more: $(cat "$STUB_LOG")"
export STUB_NX_RC=100
run rust-red rust -p ois-backend
[[ $RC -eq 100 ]] && ok "...and returns nextest's exit code" || bad "rust exit $RC, want 100"

run web web web src/lib/x.test.ts
if [[ -z "$(ls -A "$REPO/.claude/markers/tests" 2>/dev/null)" ]]; then ok "a targeted vitest run"; else bad "a targeted vitest run (marker written)"; fi
if [[ "$(cat "$WORK_ARGS")" == "--filter web exec vitest run src/lib/x.test.ts" ]]; then ok "...runs vitest in the named package"; else bad "web arguments: $(cat "$WORK_ARGS")"; fi

run web-no-pkg web
[[ $RC -eq 2 ]] && ok "web with no package is a usage error" || bad "web with no package exit $RC"
run unknown --filter=Foo
[[ $RC -eq 2 && "$OUTPUT" == *usage:* ]] && ok "an unknown argument is a usage error" || bad "unknown argument exit $RC: $OUTPUT"
[[ ! -s "$STUB_LOG" ]] && ok "...and runs nothing" || bad "unknown argument ran: $(cat "$STUB_LOG")"

echo ""
echo "run-tests: $PASS passed, $FAIL failed"
(( FAIL == 0 ))
