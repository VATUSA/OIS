#!/bin/bash
# ==============================================================================
# The full local gate, once per commit. A green run on a clean committed HEAD that
# did not move writes the tests marker (.claude/markers/tests/<HEAD sha>) and its
# manifest line (bin/lib/marker_manifest.py) that .claude/hooks/test-run-gate.sh
# checks before `gh pr create`, and that lets bin/pre-push skip clippy and the
# suite for that commit. The gate's own process writes them, so a backgrounded or
# redirected run counts.
#
#   bash bin/run-tests.sh                           # full gate; may stamp (`just test`)
#   bash bin/run-tests.sh --no-stamp                # full gate; never stamps (`just ci-full`)
#   bash bin/run-tests.sh rust [NEXTEST-ARGS...]    # cargo nextest run NEXTEST-ARGS; never stamps
#   bash bin/run-tests.sh web PKG [VITEST-ARGS...]  # pnpm --filter PKG exec vitest run VITEST-ARGS; never stamps
#
#   e.g. bash bin/run-tests.sh rust -p ois-backend -E 'test(scope_)'
#        bash bin/run-tests.sh web web src/lib/time.test.ts
#
# The full gate is every job .github/workflows/ci.yml runs that can run here, in CI's
# order: migration versions, cargo fmt --check, clippy -D warnings, nextest (--profile
# ci), doc tests, pnpm install --frozen-lockfile, lint, typecheck, vitest, pnpm audit,
# cargo deny and the client-drift check. Every step runs and prints its result, then a
# summary. A step whose tool is missing is MISSING, never passed, and blocks the
# marker: CI runs every one of them. Not covered: CI's macOS/Windows/Linux desktop
# matrix (clippy compiles the Tauri shell for this host only).
#
# pnpm audit and cargo deny are ADVISORY: they run and print their result, but neither a
# failure nor a missing cargo-deny blocks the marker. Both judge the dependency tree
# against a live advisory database, so a new advisory on next itself, or no network,
# would refuse the marker on every branch; CI's js and deny jobs stay the merge gate.
#
# lint, typecheck and vitest run through turbo with --force: turbo's cache key ignores
# root files the tasks read (DESIGN.md, desktop/src-tauri/tauri.conf.json,
# eslint.config.mjs), and its cache is shared across worktrees, so a cached replay once
# stamped a commit whose vitest run was red.
#
# A step passes on its exit code AND, for the suites, on the summary line the runner
# printed (.claude/rules/test-quality.md): nextest's `Summary [...] N tests run: N
# passed`, with at least one test run and nothing failed; each `test result:` line
# of the doc tests (OIS has none today, so zero passed is a pass); vitest's `Test
# Files` and `Tests` lines, with nothing failed and at least one test passed.
#
# .env is loaded (`set -a; . ./.env; set +a`) because every #[sqlx::test] needs
# DATABASE_URL; the full gate refuses to start without it or without node_modules.
# The Rust suite runs under `bin/with-lock.sh postgres`, so two worktrees' full
# suites never share the Postgres server at once (the sqlx-harness flake). Cargo
# builds into this checkout's own target/, with sccache when installed (bin/lib/portable.sh).
#
# Tests: bin/lib/run-tests.test.sh
# ==============================================================================
set -uo pipefail

# Resolved before the cd below, which would strand a relative path to this script.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(git rev-parse --show-toplevel 2>/dev/null)"
if [[ -z "$ROOT" ]]; then
    echo "run-tests: not inside a git repository." >&2
    exit 1
fi
cd "$ROOT" || exit 1
# shellcheck source=lib/portable.sh
. "$SCRIPT_DIR/lib/portable.sh"
export_build_env

usage() {
    echo "usage: bash bin/run-tests.sh [--no-stamp] | rust [NEXTEST-ARGS...] | web PKG [VITEST-ARGS...]" >&2
    exit 2
}

load_env() {
    # shellcheck disable=SC1091
    [[ -f .env ]] && { set -a; . ./.env; set +a; }
    return 0
}

STAMP=1
case "${1:-}" in
    "") ;;
    --no-stamp) [[ $# -eq 1 ]] || usage; STAMP=0 ;;
    rust)
        shift
        load_env
        exec cargo nextest run "$@"
        ;;
    web)
        [[ $# -ge 2 ]] || usage
        PKG="$2"
        shift 2
        [[ -d node_modules ]] || { echo "run-tests: no node_modules in $ROOT; run: pnpm install --frozen-lockfile" >&2; exit 1; }
        exec pnpm --filter "$PKG" exec vitest run "$@"
        ;;
    *) usage ;;
esac

# Prints the porcelain status, or UNKNOWN when git cannot say: an unreadable
# tree must never pass for a clean one.
tree_state() {
    local out
    out="$(git status --porcelain --untracked-files=normal 2>/dev/null)" || { echo UNKNOWN; return; }
    printf '%s' "$out"
}

refuse() {
    echo "run-tests: $1" >&2
    echo "run-tests: no tests marker written ($2)."
    exit 1
}

if [[ ! -f .env ]]; then
    refuse "no .env in $ROOT. Every #[sqlx::test] needs DATABASE_URL: copy the primary checkout's (bash bin/setup-worktree.sh)." ".env missing"
fi
load_env
if [[ -z "${DATABASE_URL:-}" ]]; then
    refuse ".env sets no DATABASE_URL, which every #[sqlx::test] needs (see .env.example)." "DATABASE_URL unset"
fi
if [[ ! -d node_modules ]]; then
    refuse "no node_modules in $ROOT. Run: pnpm install --frozen-lockfile (bash bin/setup-worktree.sh does it)." "node_modules missing"
fi

START_SHA="$(git rev-parse --verify --quiet HEAD 2>/dev/null)"
START_TREE="$(tree_state)"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/run-tests.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
ESC="$(printf '\033')"

echo "run-tests: CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-<cargo default>} RUSTC_WRAPPER=${RUSTC_WRAPPER:-<none>}"

RESULTS=()
FAILED=""
MISSING=""
STEP_N=0

# plain LOG: the log with ANSI color codes stripped (turbo and vitest color even into a pipe).
plain() { sed "s/${ESC}\[[0-9;]*[A-Za-z]//g" "$1"; }

# Here-strings, not `printf | grep -q`: under pipefail an early-exiting grep can SIGPIPE the writer and read as no match.

# verify_nextest LOG: prints why nextest's summary does not prove a pass; nothing when it does.
verify_nextest() {
    local summary re run passed
    summary="$(plain "$1" | grep -E '^[[:space:]]*Summary \[' | tail -n 1)"
    if [[ -z "$summary" ]]; then echo "no nextest Summary line"; return; fi
    re='([0-9]+) tests? run: ([0-9]+) passed'
    if [[ ! "$summary" =~ $re ]]; then echo "unreadable Summary line: ${summary#"${summary%%[![:space:]]*}"}"; return; fi
    run="${BASH_REMATCH[1]}" passed="${BASH_REMATCH[2]}"
    if [[ "$run" -eq 0 ]]; then echo "Summary reports no tests run"; return; fi
    if grep -qiE 'fail|timed out|abort|cancel|not run' <<<"$summary"; then echo "Summary reports failures: ${summary#*] }"; return; fi
    if [[ "$passed" -ne "$run" ]]; then echo "Summary: $passed of $run tests passed"; return; fi
}

# verify_doctests LOG: every `test result:` line is ok. Zero doc tests pass: OIS has none.
verify_doctests() {
    local results
    results="$(plain "$1" | grep -E '^[[:space:]]*test result:')"
    if [[ -z "$results" ]]; then echo "no 'test result:' line"; return; fi
    if grep -qvE '^[[:space:]]*test result: ok\.' <<<"$results"; then echo "a doc-test run did not report ok"; return; fi
}

# verify_vitest LOG: vitest's `Test Files` and `Tests` lines (turbo prefixes them with
# `<pkg>:test: `) report nothing failed and at least one test passed.
verify_vitest() {
    local text files tests total=0 n
    text="$(plain "$1")"
    files="$(grep -E '(^|: )[[:space:]]*Test Files[[:space:]]' <<<"$text")"
    tests="$(grep -E '(^|: )[[:space:]]*Tests[[:space:]]+[0-9]' <<<"$text")"
    if [[ -z "$files" || -z "$tests" ]]; then echo "no vitest Test Files/Tests summary line"; return; fi
    if grep -qiE 'fail|error' <<<"$files"$'\n'"$tests"; then echo "a vitest summary reports failures"; return; fi
    if grep -qE '(^|: )[[:space:]]*Errors[[:space:]]+[0-9]' <<<"$text"; then echo "vitest reports unhandled errors"; return; fi
    if grep -qE '(^|: )[[:space:]]*FAIL[[:space:]]' <<<"$text"; then echo "output reports a FAIL line"; return; fi
    if grep -qi 'no test files found' <<<"$text"; then echo "a package found no test files"; return; fi
    while IFS= read -r n; do total=$((total + n)); done < <(grep -oE '[0-9]+ passed' <<<"$tests" | cut -d' ' -f1)
    if [[ "$total" -eq 0 ]]; then echo "the vitest summaries report no passed test"; return; fi
}

# run_step NAME VERIFY -- COMMAND...: runs COMMAND, tees its output, records PASS or FAIL.
# VERIFY is a verify_* function reading the log, or - for the exit code alone.
run_step() {
    local name="$1" verify="$2" log rc why=""
    shift 3
    STEP_N=$((STEP_N + 1))
    log="$WORK/step.$STEP_N.log"
    echo
    echo "==> $name"
    "$@" 2>&1 | tee "$log"
    rc=${PIPESTATUS[0]}
    if [[ $rc -ne 0 ]]; then
        why="exited $rc"
    elif [[ "$verify" != - ]]; then
        why="$("$verify" "$log")"
    fi
    if [[ -z "$why" ]]; then
        RESULTS+=("PASS     $name")
    else
        RESULTS+=("FAIL     $name ($why)")
        FAILED="$FAILED, $name"
    fi
}

# run_advisory NAME -- COMMAND...: runs COMMAND, tees its output, records ADVISORY with its
# result; it never fails the gate or blocks the marker (see the header).
run_advisory() {
    local name="$1" log rc
    shift 2
    STEP_N=$((STEP_N + 1))
    log="$WORK/step.$STEP_N.log"
    echo
    echo "==> $name (advisory)"
    "$@" 2>&1 | tee "$log"
    rc=${PIPESTATUS[0]}
    if [[ $rc -eq 0 ]]; then
        RESULTS+=("ADVISORY $name (pass)")
    else
        RESULTS+=("ADVISORY $name (failed: exited $rc; does not block the marker, CI's job still blocks the merge)")
    fi
}

# missing NAME REASON: a step that could not run here.
missing() {
    echo
    echo "==> $1: MISSING ($2)"
    RESULTS+=("MISSING  $1 ($2)")
    MISSING="$MISSING, $1"
}

has_cargo_sub() { cargo "$1" --version >/dev/null 2>&1; }

# client drift: regenerate the client from the compiled OpenAPI document and compare it to
# the checked-in one, then restore the worktree copy. The document goes to a path keyed on
# this checkout, never CI's shared /tmp path, so concurrent worktrees each check their own
# contract (#760).
drift() {
    local schema=packages/api-client/src/generated/schema.d.ts saved doc status
    doc="${CARGO_TARGET_DIR:-$ROOT/target}/ois-drift/$(printf '%s' "$ROOT" | cksum | cut -d' ' -f1)/ois-openapi.json"
    saved="$WORK/schema.d.ts.saved"
    cp "$schema" "$saved" || return 1
    mkdir -p "${doc%/*}" && rm -f "$doc" &&
        OIS_OPENAPI_OUT="$doc" cargo test -p ois-backend --lib openapi::dump_openapi_json -- --exact --ignored --quiet &&
        OIS_OPENAPI_URL="$doc" pnpm --filter @ois/api-client codegen >/dev/null &&
        cmp -s "$schema" "$saved"
    status=$?
    if [[ $status -ne 0 ]] && ! cmp -s "$schema" "$saved"; then
        diff -u "$saved" "$schema" | head -40
        echo "client drift: the generated client is stale. Regenerate it (AGENTS.md § The API contract) and commit it."
    fi
    cp "$saved" "$schema"
    return $status
}

# migrations job
run_step "migration versions" - -- .github/scripts/check-migration-versions.sh
# rust job
run_step "cargo fmt --check" - -- cargo fmt --all -- --check
run_step "cargo clippy -D warnings" - -- cargo clippy --workspace --all-targets -- -D warnings
if has_cargo_sub nextest; then
    run_step "rust tests (cargo nextest --profile ci)" verify_nextest -- \
        bash "$SCRIPT_DIR/with-lock.sh" postgres -- cargo nextest run --workspace --all-targets --profile ci
else
    missing "rust tests (cargo nextest --profile ci)" "cargo-nextest is not installed: cargo install --locked cargo-nextest"
fi
run_step "doc tests" verify_doctests -- cargo test --workspace --doc
# js job
run_step "pnpm install --frozen-lockfile" - -- pnpm install --frozen-lockfile --prefer-offline
run_step "pnpm lint" - -- pnpm exec turbo run lint --force
run_step "pnpm typecheck" - -- pnpm exec turbo run typecheck --force
run_step "pnpm test" verify_vitest -- pnpm exec turbo run test --force
run_advisory "pnpm audit --audit-level=high" -- pnpm audit --audit-level=high
# deny job
if has_cargo_sub deny; then
    run_advisory "cargo deny check" -- cargo deny check
else
    RESULTS+=("ADVISORY cargo deny check (not run: cargo-deny is not installed: cargo install --locked cargo-deny)")
fi
# client-drift job
run_step "client drift" - -- drift

echo
echo "==> run-tests summary"
printf '  %s\n' "${RESULTS[@]}"

REASON=""
if [[ -n "$FAILED" ]]; then
    REASON="failed: ${FAILED#, }"
elif [[ -n "$MISSING" ]]; then
    REASON="could not run: ${MISSING#, }"
elif [[ "$STAMP" -eq 0 ]]; then
    REASON="--no-stamp"
elif [[ -z "$START_SHA" ]]; then
    REASON="HEAD could not be read"
elif [[ -n "$START_TREE" ]]; then
    REASON="working tree was not clean at start"
elif [[ "$(git rev-parse --verify --quiet HEAD 2>/dev/null)" != "$START_SHA" ]]; then
    REASON="HEAD moved during the run"
elif [[ -n "$(tree_state)" ]]; then
    REASON="working tree changed during the run"
fi

if [[ -n "$REASON" ]]; then
    echo "run-tests: no tests marker written ($REASON)."
elif python3 -I "$SCRIPT_DIR/lib/marker_manifest.py" stamp "$ROOT" tests "$START_SHA"; then
    echo "run-tests: tests marker written for ${START_SHA:0:8}."
else
    echo "run-tests: could not write the tests marker." >&2
fi

[[ -z "$FAILED" && -z "$MISSING" ]]
