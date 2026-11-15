#!/bin/bash
# ==============================================================================
# Tests for .github/scripts/check-migration-versions.sh.
#
#   bash bin/lib/check-migration-versions.test.sh
#
# Runs the real script, never a stub, against migration directories built under a
# temp dir. Every run but the default-directory case starts from a cwd with no
# backend/migrations, so a mutant that ignores $1 finds nothing instead of this
# checkout's real migrations. Each case asserts the exit code and the message:
# several mutants exit 1 for the wrong reason. Pure shell; runs under bash 3.2 and
# Linux. Self-reports pass/fail, exits non-zero on any failure.
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/.github/scripts/check-migration-versions.sh"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/check-migration-versions-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

[ -f "$SCRIPT" ] || { echo "check-migration-versions: $SCRIPT not found"; exit 1; }

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

# migrations DIR FILE... — create DIR holding each named (empty) file
migrations() {
    local dir="$1"; shift
    mkdir -p "$dir"
    for f in "$@"; do : > "$dir/$f"; done
}

# check CWD [ARG] — run the real script from CWD; sets RC, OUT (stdout), ERR (stderr).
# CHECK_LC, when set, is the run's LC_ALL; otherwise the caller's locale applies.
CHECK_LC=""
check() {
    local cwd="$1"; shift
    if [[ -n $CHECK_LC ]]; then
        (cd "$cwd" && LC_ALL="$CHECK_LC" bash "$SCRIPT" "$@") >"$WORK/out" 2>"$WORK/err" </dev/null
    else
        (cd "$cwd" && bash "$SCRIPT" "$@") >"$WORK/out" 2>"$WORK/err" </dev/null
    fi
    RC=$?
    OUT="$(cat "$WORK/out")"
    ERR="$(cat "$WORK/err")"
}

# expect_rc NAME WANT_RC — compare RC, print the run's output on a mismatch
expect_rc() {
    if [[ $RC -eq $2 ]]; then ok "$1: exit $2"; else bad "$1: want exit $2, got $RC (stdout: $OUT; stderr: $ERR)"; fi
}
# has NAME STREAM TEXT / lacks NAME STREAM TEXT — fixed-string match on OUT or ERR
has() {
    if grep -qF -- "$3" <<<"$2"; then ok "$1: says '$3'"; else bad "$1: missing '$3' in: $2"; fi
}
lacks() {
    if grep -qF -- "$3" <<<"$2"; then bad "$1: unexpected '$3' in: $2"; else ok "$1: does not say '$3'"; fi
}

mkdir -p "$WORK/cwd"

echo "== 1. Numerically equal versions collide =="
migrations "$WORK/dup" 0090_a.sql 90_b.sql
check "$WORK/cwd" "$WORK/dup"
expect_rc "0090_a + 90_b" 1
has "0090_a + 90_b" "$ERR" "Duplicate migration version"
has "0090_a + 90_b" "$ERR" "0090_a.sql"
has "0090_a + 90_b" "$ERR" "90_b.sql"
lacks "0090_a + 90_b" "$OUT" "unique"

echo "== 2. Distinct versions pass =="
# Two underscores per name: a prefix cut at the last underscore fails the regex.
migrations "$WORK/distinct" 0090_add_a.sql 0091_add_b.sql
check "$WORK/cwd" "$WORK/distinct"
expect_rc "0090_add_a + 0091_add_b" 0
has "0090_add_a + 0091_add_b" "$OUT" "migration versions are unique (2 files)"
lacks "0090_add_a + 0091_add_b" "$ERR" "Duplicate"

echo "== 3. A non-versioned file is refused =="
migrations "$WORK/unversioned" 0090_a.sql abc_x.sql
check "$WORK/cwd" "$WORK/unversioned"
expect_rc "abc_x" 1
has "abc_x" "$ERR" "not a versioned migration (sqlx needs NNNN_name.sql): abc_x.sql"

echo "== 4. Digits followed by a letter are refused =="
migrations "$WORK/trailing" 0090_a.sql 0091a_x.sql
check "$WORK/cwd" "$WORK/trailing"
expect_rc "0091a_x" 1
has "0091a_x" "$ERR" "not a versioned migration (sqlx needs NNNN_name.sql): 0091a_x.sql"

echo "== 5. A letter followed by digits is refused =="
migrations "$WORK/leading" 0090_a.sql v0091_x.sql
check "$WORK/cwd" "$WORK/leading"
expect_rc "v0091_x" 1
has "v0091_x" "$ERR" "not a versioned migration (sqlx needs NNNN_name.sql): v0091_x.sql"

echo "== 6. An empty directory fails =="
# The message, not only the exit: without nullglob the literal *.sql exits 1 at the prefix check.
migrations "$WORK/empty"
check "$WORK/cwd" "$WORK/empty"
expect_rc "empty" 1
has "empty" "$ERR" "no migrations found in $WORK/empty"

echo "== 7. A duplicate the glob order does not make adjacent =="
# Glob order is 90, 91, 90: only the numeric sort brings the pair together.
migrations "$WORK/apart" 0090_a.sql 0091_b.sql 90_c.sql
check "$WORK/cwd" "$WORK/apart"
expect_rc "0090_a + 0091_b + 90_c" 1
has "0090_a + 0091_b + 90_c" "$ERR" "0090_a.sql"
has "0090_a + 0091_b + 90_c" "$ERR" "90_c.sql"
lacks "0090_a + 0091_b + 90_c" "$ERR" "0091_b.sql"

echo "== 8. No argument reads backend/migrations under the cwd =="
# A changed default also exits 1 ("no migrations found"), so assert the files it names.
migrations "$WORK/default-cwd/backend/migrations" 0042_a.sql 42_b.sql
check "$WORK/default-cwd"
expect_rc "no argument" 1
has "no argument" "$ERR" "0042_a.sql"
has "no argument" "$ERR" "42_b.sql"

echo "== 9. Files other than *.sql are not read =="
migrations "$WORK/mixed" 0090_a.sql 0091_b.sql README.md
check "$WORK/cwd" "$WORK/mixed"
expect_rc "README.md beside two migrations" 0
has "README.md beside two migrations" "$OUT" "migration versions are unique (2 files)"

echo "== 10. The sort is numeric, whatever the locale collates =="
# The script sorts "<version> <name>" lines. glibc's en_US.UTF-8 collation ignores the
# space, so a plain text sort orders these 90, 900, 90 and the pair is not adjacent.
migrations "$WORK/collate" 0090_a.sql 90_b.sql 900_c.sql
EN_US="$(locale -a 2>/dev/null | grep -E '^en_US\.(UTF-8|utf8)$' | head -n 1)"
for lc in C "$EN_US"; do
    if [[ -z $lc ]]; then echo "  SKIP: no en_US.UTF-8 locale on this host"; continue; fi
    CHECK_LC="$lc"
    check "$WORK/cwd" "$WORK/collate"
    expect_rc "LC_ALL=$lc 0090_a + 90_b + 900_c" 1
    has "LC_ALL=$lc 0090_a + 90_b + 900_c" "$ERR" "0090_a.sql"
    has "LC_ALL=$lc 0090_a + 90_b + 900_c" "$ERR" "90_b.sql"
    lacks "LC_ALL=$lc 0090_a + 90_b + 900_c" "$ERR" "900_c.sql"
done
CHECK_LC=""

echo "== 11. A name with no digits before the underscore is refused =="
migrations "$WORK/noprefix" 0090_a.sql _x.sql
check "$WORK/cwd" "$WORK/noprefix"
expect_rc "_x" 1
has "_x" "$ERR" "not a versioned migration (sqlx needs NNNN_name.sql): _x.sql"

echo ""
echo "check-migration-versions: $PASS passed, $FAIL failed"
[[ $FAIL -eq 0 ]]
