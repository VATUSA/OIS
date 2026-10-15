#!/bin/bash
# ==============================================================================
# Tests for bin/lib/file_lock.py, the exclusive lock the marker ledger
# (.claude/hooks/lib/marker_ledger.py) and the manifest (bin/lib/marker_manifest.py)
# take, and for marker-write-gate.sh passing a call that writes no marker.
#
#   bash bin/lib/file-lock.test.sh
#
# Two processes holding the lock exclude each other on whatever OS runs this; CI
# runs it on windows-latest from Git Bash as well (.github/workflows/ci.yml,
# windows-hooks). The msvcrt branch is also driven here with a fake msvcrt, so its
# call sequence is checked on every OS. Neither marker module imports fcntl.
# Exits non-zero on any failure.
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
LIB="$ROOT/bin/lib"
WORK="$(cd "$(mktemp -d "${TMPDIR:-/tmp}/file-lock-test.XXXXXX")" && pwd -P)"
trap 'rm -rf "$WORK"' EXIT
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR
export GIT_AUTHOR_NAME=T GIT_AUTHOR_EMAIL=t@e.com GIT_COMMITTER_NAME=T GIT_COMMITTER_EMAIL=t@e.com

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

# holder LOCKFILE DIR: take the lock, say so in DIR/held, hold it 1.5 s, write the time to DIR/unlocked and unlock,
# then stay alive 2 s more, so a waiter that gets in before the process exits got in by unlock().
holder() {
    python3 -I -c 'import os, sys, time
sys.path.insert(0, sys.argv[1]); import file_lock
handle = open(sys.argv[2], "a+"); file_lock.lock(handle)
open(os.path.join(sys.argv[3], "held"), "w").close(); time.sleep(1.5)
open(os.path.join(sys.argv[3], "unlocked"), "w").write(repr(time.time())); file_lock.unlock(handle); time.sleep(2)' \
        "$LIB" "$1" "$2"
}
# contender LOCKFILE: print how long lock() waited and the time it returned.
contender() {
    python3 -I -c 'import sys, time
sys.path.insert(0, sys.argv[1]); import file_lock
handle = open(sys.argv[2], "a+"); start = time.time(); file_lock.lock(handle); now = time.time()
print("%r %r" % (now - start, now)); file_lock.unlock(handle)' "$LIB" "$1"
}

echo "Two processes exclude each other through the helper:"
read -r WAITED _ <<<"$(contender "$WORK/free.lock")"
if python3 -c 'import sys; sys.exit(float(sys.argv[1]) >= 0.5)' "$WAITED"
then ok "an uncontended lock is taken at once (${WAITED}s)"; else bad "an uncontended lock waited ${WAITED}s"; fi
mkdir -p "$WORK/x"
holder "$WORK/x.lock" "$WORK/x" &
HOLDER=$!
for _ in $(seq 100); do [[ -e "$WORK/x/held" ]] && break; sleep 0.1; done
read -r WAITED GOT <<<"$(contender "$WORK/x.lock")"
STILL_ALIVE=0; kill -0 "$HOLDER" 2>/dev/null && STILL_ALIVE=1
wait "$HOLDER"
UNLOCKED="$(cat "$WORK/x/unlocked" 2>/dev/null)"
if [[ -e "$WORK/x/held" && -n "$UNLOCKED" ]] && python3 -c 'import sys; w, got, unlocked = map(float, sys.argv[1:4]); sys.exit(not (w >= 1.0 and got >= unlocked))' "$WAITED" "$GOT" "$UNLOCKED"
then ok "a second holder waits until the first unlocks (waited ${WAITED}s)"; else bad "the second holder got in early: waited ${WAITED:-?}s, at ${GOT:-?}, unlocked at ${UNLOCKED:-never}"; fi
if [[ $STILL_ALIVE -eq 1 ]]; then ok "unlock() lets the waiter in while the first holder is still running"
else bad "the waiter got in only after the first holder exited, so unlock() released nothing"; fi

echo "The Windows branch (fcntl unimportable, a fake msvcrt):"
# The script goes through a file: bash 3.2 (macOS) misparses a heredoc inside $(...).
cat > "$WORK/win.py" <<'PY'
import os, sys, threading, types
threading.Timer(10, lambda: (print("lock() never returned"), sys.stdout.flush(), os._exit(3))).start()  # a loop that never ends fails
sys.modules["fcntl"] = None  # `import fcntl` now raises ImportError, as on Windows
calls, failures = [], [2]
fake = types.ModuleType("msvcrt")
fake.LK_UNLCK, fake.LK_LOCK, fake.LK_NBLCK = 0, 1, 2  # the values Windows' msvcrt defines
def locking(fd, mode, nbytes):
    calls.append((os.lseek(fd, 0, os.SEEK_CUR), mode, nbytes))
    if mode == fake.LK_NBLCK and failures[0]:
        failures[0] -= 1
        raise OSError(36, "Resource deadlock avoided")
fake.locking = locking
sys.modules["msvcrt"] = fake
sys.path.insert(0, sys.argv[1])
import file_lock
assert file_lock.fcntl is None, "file_lock took fcntl although it could not be imported"
handle = open(sys.argv[2], "a+")
handle.write("seven b")
handle.flush()  # the handle now sits at offset 7, so a lock taken without seeking would start there
file_lock.lock(handle)
print("lock", calls)
del calls[:]
handle.seek(3)
file_lock.unlock(handle)
print("unlock", calls)
sys.stdout.flush()
os._exit(0)  # past the timer
PY
OUT="$(python3 -I "$WORK/win.py" "$LIB" "$WORK/win.lock")"
LK="$(grep '^lock ' <<<"$OUT")"; UN="$(grep '^unlock ' <<<"$OUT")"
[[ "$LK" == "lock [(0, 2, 1), (0, 2, 1), (0, 2, 1)]" ]] \
    && ok "lock() seeks to 0 and locks one byte with LK_NBLCK, retrying after each OSError until it holds" \
    || bad "Windows lock() calls: ${LK:-$OUT}"
[[ "$UN" == "unlock [(0, 0, 1)]" ]] && ok "unlock() seeks to 0 and unlocks that byte with LK_UNLCK" \
    || bad "Windows unlock() calls: ${UN:-$OUT}"

echo "The marker modules lock through the helper, never fcntl:"
# A source scan, because AC2 is about what the files import: an import inside a function, a try or
# `from fcntl import ...` all count, so it walks the whole syntax tree, not the top-level statements.
SCAN='import ast, sys
for path in sys.argv[1:]:
    tree = ast.parse(open(path).read())
    names = [alias.name for node in ast.walk(tree) if isinstance(node, ast.Import) for alias in node.names]
    names += [node.module for node in ast.walk(tree) if isinstance(node, ast.ImportFrom)]
    print("%s fcntl=%d file_lock=%d" % (path.rsplit("/", 1)[-1], "fcntl" in names, "file_lock" in names))'
OUT="$(python3 -I -c "$SCAN" "$ROOT/.claude/hooks/lib/marker_ledger.py" "$LIB/marker_manifest.py")"
[[ "$OUT" == $'marker_ledger.py fcntl=0 file_lock=1\nmarker_manifest.py fcntl=0 file_lock=1' ]] \
    && ok "neither module imports fcntl, and both import file_lock" || bad "import scan: $OUT"
printf 'def f():\n    import fcntl\n' > "$WORK/a.py"; printf 'try:\n    from fcntl import flock\nexcept ImportError:\n    pass\n' > "$WORK/b.py"
OUT="$(python3 -I -c "$SCAN" "$WORK/a.py" "$WORK/b.py")"
[[ "$OUT" == $'a.py fcntl=1 file_lock=0\nb.py fcntl=1 file_lock=0' ]] \
    && ok "the scan finds an fcntl import inside a function and a from-import inside a try" || bad "scan control: $OUT"

REPO="$WORK/repo"
mkdir -p "$REPO/.claude" "$REPO/bin/lib"
cp -R "$ROOT/.claude/hooks" "$REPO/.claude/hooks"
cp "$LIB/marker_manifest.py" "$LIB/file_lock.py" "$REPO/bin/lib/"
printf '.claude/markers/\n' > "$REPO/.gitignore"
git -C "$REPO" init -q -b next && git -C "$REPO" add . && git -C "$REPO" commit -qm init
SHA="$(git -C "$REPO" rev-parse HEAD)"
LEDGER_LOCK="$REPO/.git/ois-marker-ledger/lock"

# blocked_while_held LOCKFILE COMMAND...: whether COMMAND waits for a lock another process holds through
# the helper and runs once it is released.
blocked_while_held() {
    local lockfile="$1"; shift
    rm -rf "$WORK/h" && mkdir -p "$WORK/h" "$(dirname "$lockfile")"
    holder "$lockfile" "$WORK/h" &
    local pid=$!
    for _ in $(seq 100); do [[ -e "$WORK/h/held" ]] && break; sleep 0.1; done
    "$@" &
    local cmd=$!
    sleep 0.8
    local early=0; kill -0 "$cmd" 2>/dev/null || early=1
    wait "$cmd"; local rc=$?
    wait "$pid"
    [[ $early -eq 0 && $rc -eq 0 ]]
}
open_ledger() { python3 -I -c 'import sys; sys.path.insert(0, sys.argv[1]); import marker_ledger
marker_ledger.Ledger(sys.argv[2]).close()' "$REPO/.claude/hooks/lib" "$REPO"; }
mkdir -p "$REPO/.git/ois-marker-ledger"
if blocked_while_held "$LEDGER_LOCK" open_ledger; then ok "the ledger waits for its lock under python3 -I, then opens and saves"
else bad "the ledger did not wait for its lock, or failed once it was free"; fi
stamp() { (cd "$WORK" && python3 -I "$REPO/bin/lib/marker_manifest.py" stamp "$REPO" tests "$SHA"); }
mkdir -p "$REPO/.claude/markers"
if blocked_while_held "$REPO/.claude/markers/.manifest.lock" stamp \
    && python3 -I "$REPO/bin/lib/marker_manifest.py" vouched "$REPO" tests "$SHA"
then ok "stamp, run under python3 -I from another directory, waits for the manifest lock and writes its line"
else bad "stamp did not wait for the manifest lock, or wrote no vouched line"; fi
[[ ! -e "$REPO/bin/lib/__pycache__" ]] && ok "importing the helper leaves no __pycache__ beside it" || bad "bin/lib/__pycache__ was written"

mkdir -p "$WORK/alone"
cp "$LIB/marker_manifest.py" "$WORK/alone/"
if python3 -I "$WORK/alone/marker_manifest.py" vouched "$REPO" tests "$SHA"; then ok "vouched answers without file_lock.py beside it"
else bad "vouched failed with file_lock.py missing"; fi

echo "marker-write-gate.sh passes a Bash, Write or Edit call that writes no marker, and the ledger records it:"
rm -rf "$REPO/.claude/markers"  # the stamp above was no tool call's, so an audit would rightly undo it
# The payload's cwd comes from Python's own getcwd, so it is a native path on Windows too.
payload() {
    (cd "$REPO" && python3 -I -c 'import json, os, sys
tool, field, value = sys.argv[2], sys.argv[3], sys.argv[4]
print(json.dumps({"tool_name": tool, "tool_use_id": sys.argv[1], "tool_input": {field: value}, "cwd": os.getcwd(),
                  "hook_event_name": "PostToolUse", "tool_response": {}}))' "$@")
}
recorded() {
    (cd "$REPO" && python3 -I -c 'import json, sys
print(" ".join(sorted(json.load(open(sys.argv[1]))["state"]["snapshots"])))' "$(git rev-parse --path-format=absolute --git-common-dir)/ois-marker-ledger/ledger.json")
}
for call in "toolu_bash Bash command ls" "toolu_write Write file_path notes.txt" "toolu_edit Edit file_path README"; do
    read -r ID TOOL FIELD VALUE <<<"$call"
    ERR="$(payload "$ID" "$TOOL" "$FIELD" "$VALUE" | (cd "$REPO" && bash .claude/hooks/marker-write-gate.sh 2>&1))"; RC=$?
    if [[ $RC -eq 0 ]] && [[ " $(recorded) " == *" $ID "* ]]; then ok "$TOOL passes the gate and its snapshot is recorded"
    else bad "$TOOL: gate rc=$RC, snapshots [$(recorded)]: $ERR"; fi
    OUT="$(payload "$ID" "$TOOL" "$FIELD" "$VALUE" | (cd "$REPO" && bash .claude/hooks/marker-audit.sh 2>&1))"; RC=$?
    if [[ $RC -eq 0 && -z "$OUT" ]] && [[ " $(recorded) " != *" $ID "* ]]; then ok "$TOOL's audit closes its snapshot and undoes nothing"
    else bad "$TOOL: audit rc=$RC, snapshots [$(recorded)]: ${OUT:0:240}"; fi
done

echo ""
echo "file-lock: $PASS passed, $FAIL failed"
(( FAIL == 0 ))
