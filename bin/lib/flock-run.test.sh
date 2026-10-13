#!/bin/bash
# ==============================================================================
# Tests for bin/lib/flock-run.pl and bin/with-lock.sh.
#
#   bash bin/lib/flock-run.test.sh
#
# Uses throwaway lockfiles under a temp dir, so it never touches the real
# /tmp/ois-locks-* namespace a live suite would use. Pure shell + perl; runs
# unchanged on macOS and Linux. No external test framework: it self-reports
# pass/fail and exits non-zero when anything failed.
#
# flock-run.pl is a general tool keyed on the lockfile PATH. Test 3 exercises that
# path-independence, which with-lock.sh's per-clone REPO_ID isolation relies on.
#
# Section 7 drives with-lock.sh end-to-end and so takes real locks. It sets
# OIS_LOCK_DIR to a temp dir, which is the ONLY reason it does not contend
# with a live suite on the same box. Never drop that variable.
#
# NOT covered: the 2700s default timeout itself — asserting a default would mean
# waiting it out. Section 7 proves OIS_LOCK_TIMEOUT is honoured. Section 8
# drives flock-run.pl directly against a lockfile in the temp dir.
# ==============================================================================
set -uo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUN="perl $DIR/flock-run.pl"
WITH_LOCK="$DIR/../with-lock.sh"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/flock-test.XXXXXX")"
cleanup() {
    [[ -f "$WORK/child.pid" ]] && kill "$(cat "$WORK/child.pid")" 2>/dev/null
    rm -rf "$WORK"
}
trap cleanup EXIT

# Helper command bodies as files — avoids fragile inline quoting through perl.
cat > "$WORK/bracket.sh" <<'EOF'
#!/bin/bash
# $1 = marker file. Records a non-overlapping [START,END] window.
printf 'START %s\n' "$(date +%s)" >> "$1"
sleep 2
printf 'END %s\n' "$(date +%s)" >> "$1"
EOF
cat > "$WORK/holder.sh" <<'EOF'
#!/bin/bash
# $1 = pid file, $2 = seconds to hold. Records its own pid so the test can reap it.
echo $$ > "$1"
sleep "$2"
EOF
chmod +x "$WORK/bracket.sh" "$WORK/holder.sh"

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }
now() { date +%s; }
reap_holder() { [[ -f "$WORK/child.pid" ]] && kill "$(cat "$WORK/child.pid")" 2>/dev/null; rm -f "$WORK/child.pid"; }

echo "== 1. mutual exclusion: two holders of one lock do not overlap =="
LOCK="$WORK/a.lock"; MARK="$WORK/a.marks"; : > "$MARK"
$RUN "$LOCK" 0 postgres -- bash "$WORK/bracket.sh" "$MARK" >/dev/null 2>&1 &
P1=$!
sleep 1
$RUN "$LOCK" 0 postgres -- bash "$WORK/bracket.sh" "$MARK" >/dev/null 2>&1 &
P2=$!
wait "$P1"; wait "$P2"
SEQ="$(awk '{print $1}' "$MARK" | tr '\n' ' ')"
if [[ "$SEQ" == "START END START END " ]]; then
    ok "runs serialized (order: $SEQ)"
else
    bad "runs overlapped (order: $SEQ) — expected 'START END START END'"
fi

echo "== 2. auto-release: a SIGKILLed holder frees the lock immediately =="
LOCK="$WORK/b.lock"
$RUN "$LOCK" 0 postgres -- bash "$WORK/holder.sh" "$WORK/child.pid" 30 >/dev/null 2>&1 &
HOLDER=$!
sleep 1
kill -9 "$HOLDER" 2>/dev/null
reap_holder
T0="$(now)"
$RUN "$LOCK" 5 postgres -- true >/dev/null 2>&1
RC=$?
ELAPSED=$(( $(now) - T0 ))
if [[ $RC -eq 0 && $ELAPSED -lt 3 ]]; then
    ok "reacquired in ${ELAPSED}s after kill (rc=$RC)"
else
    bad "reacquire rc=$RC after ${ELAPSED}s — kernel should have freed it on death"
fi

echo "== 3. path-independent: a held lock does not block a DIFFERENT lockfile =="
# with-lock.sh keys each clone's lock on a REPO_ID hash, so two clones get
# different lockfiles and must not block each other. Same mechanism, two paths.
$RUN "$WORK/repo-a.lock" 0 postgres -- bash "$WORK/holder.sh" "$WORK/child.pid" 10 >/dev/null 2>&1 &
HOLDER=$!
sleep 1
T0="$(now)"
$RUN "$WORK/repo-b.lock" 5 postgres -- true >/dev/null 2>&1
RC=$?
ELAPSED=$(( $(now) - T0 ))
kill -9 "$HOLDER" 2>/dev/null; reap_holder
if [[ $RC -eq 0 && $ELAPSED -lt 3 ]]; then
    ok "repo-b acquired while repo-a held (${ELAPSED}s, rc=$RC)"
else
    bad "repo-b waited ${ELAPSED}s (rc=$RC) — different lockfiles must not block"
fi

echo "== 4. timeout: a waiter gives up and fails when the holder outlives it =="
LOCK="$WORK/d.lock"
$RUN "$LOCK" 0 postgres -- bash "$WORK/holder.sh" "$WORK/child.pid" 10 >/dev/null 2>&1 &
HOLDER=$!
sleep 1
T0="$(now)"
$RUN "$LOCK" 2 postgres -- true >/dev/null 2>&1
RC=$?
ELAPSED=$(( $(now) - T0 ))
kill -9 "$HOLDER" 2>/dev/null; reap_holder
if [[ $RC -eq 75 && $ELAPSED -ge 2 && $ELAPSED -lt 4 ]]; then
    ok "timed out after ${ELAPSED}s with exit 75 on every platform (rc=$RC)"
else
    bad "timeout behavior wrong: rc=$RC after ${ELAPSED}s (want exit 75, ~2s)"
fi

echo "== 5. exit code is the wrapped command's, not the wrapper's =="
$RUN "$WORK/e.lock" 0 postgres -- bash -c 'exit 7' >/dev/null 2>&1
RC=$?
[[ $RC -eq 7 ]] && ok "propagated exit 7" || bad "expected exit 7, got $RC"

echo "== 6. with-lock.sh: escape hatches and arg validation =="
OIS_NO_LOCK=1 bash "$WITH_LOCK" postgres -- true >/dev/null 2>&1 && ok "OIS_NO_LOCK runs through" || bad "OIS_NO_LOCK should exit 0"
CI=1 bash "$WITH_LOCK" postgres -- true >/dev/null 2>&1 && ok "CI auto-skip runs through" || bad "CI should exit 0"
bash "$WITH_LOCK" bogus -- true >/dev/null 2>&1; [[ $? -eq 2 ]] && ok "rejects unknown resource" || bad "unknown resource should exit 2"
bash "$WITH_LOCK" postgres true >/dev/null 2>&1; [[ $? -eq 2 ]] && ok "rejects missing -- separator" || bad "missing -- should exit 2"

echo "== 7. with-lock.sh: two postgres runs DO queue =="
LOCKS="$WORK/locks-7"
OIS_LOCK_DIR="$LOCKS" bash "$WITH_LOCK" postgres -- bash "$WORK/holder.sh" "$WORK/child.pid" 10 >/dev/null 2>&1 &
HOLDER=$!
sleep 1
T0="$(now)"
OIS_LOCK_DIR="$LOCKS" OIS_LOCK_TIMEOUT=2 bash "$WITH_LOCK" postgres -- true >/dev/null 2>&1
RC=$?
ELAPSED=$(( $(now) - T0 ))
kill -9 "$HOLDER" 2>/dev/null; reap_holder
if [[ $RC -ne 0 && $ELAPSED -ge 2 && $ELAPSED -lt 5 ]]; then
    ok "second postgres run queued then timed out after ${ELAPSED}s (rc=$RC)"
else
    bad "expected a queued timeout (~2s, non-zero); got rc=$RC after ${ELAPSED}s"
fi

echo "== 8. a lockfile planted as a symlink is refused, and its target untouched =="
printf 'precious\n' > "$WORK/target"
ln -s "$WORK/target" "$WORK/planted.lock"
rm -f "$WORK/ran"
$RUN "$WORK/planted.lock" 1 postgres -- touch "$WORK/ran" >/dev/null 2>&1
RC=$?
if [[ $RC -ne 0 && ! -f "$WORK/ran" && "$(cat "$WORK/target")" == "precious" ]]; then
    ok "symlinked lockfile refused, target kept (rc=$RC)"
else
    bad "symlinked lockfile: rc=$RC ran=$([[ -f "$WORK/ran" ]] && echo yes || echo no) target=$(cat "$WORK/target")"
fi

echo ""
echo "== $PASS passed, $FAIL failed =="
[[ $FAIL -eq 0 ]]
