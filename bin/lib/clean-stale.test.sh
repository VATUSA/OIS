#!/bin/bash
# ==============================================================================
# Tests for bin/clean-stale.sh: which build output it deletes, which it keeps,
# when it skips a profile a build is using, and what --scratch adds.
#
#   bash bin/lib/clean-stale.test.sh
#
# Each case builds a scratch repository with a bare "origin" and one linked
# worktree at .claude/worktrees/<NAME> (EnterWorktree's layout), lays cargo-shaped
# output under each checkout's target/ with every file and directory backdated by
# `touch -t`, and runs the script with TMPDIR pointed inside the scratch dir, so
# nothing outside it is touched. Ages are relative to now and straddle the
# threshold by a whole day, clear of any truncation edge. Exits non-zero on any
# failure; a pending case prints TODO and counts as neither.
#
# Coverage matrix (VATUSA/OIS#1020)
#
# Requirement (AC2 unless noted)               | Case                                                    | Status
# ---------------------------------------------|---------------------------------------------------------|-------
# stale build/<pkg>/<hash>/ deleted            | a stale build unit is removed beside a fresh sibling    | done
# fresh build/<pkg>/<hash>/ kept               | (same case, the sibling)                                | done
# stale incremental/<dir>/ deleted, fresh kept | a stale incremental dir is removed beside a fresh one   | done
# "newest file" decides, not the dir mtime     | an old unit holding one fresh file is kept (build)      | done
#                                              | an old unit holding one fresh file is kept (incremental)| done
#                                              | a fresh file two levels down keeps its unit             | done
# emptied build/<pkg>/ removed                 | build/<pkg>/ left empty is removed, non-empty kept      | done
# ois-drift/<dir>/ deleted when stale          | a stale ois-drift dir is removed beside a fresh one     | done
# every worktree's target/, primary included   | stale units in the primary and a linked worktree go     | done
#                                              | the run reaches the primary from inside a worktree      | done
# every <profile>/ under target/               | a second profile (release) is cleaned too               | done
# skip a profile a running build is using      | a held .cargo-build-lock keeps its profile              | done
#                                              | ...while the unlocked profile beside it is cleaned      | done
# lock held while deleting                     | the build lock cannot be taken while a unit is deleted  | done
#                                              | the lock is released after the run                      | done
# keep the output when the check fails         | an unreadable lock file keeps its profile               | done
#                                              | flock missing from PATH keeps every profile             | done
# default N is 7                               | no DAYS: 6 days old kept, 8 days old removed            | done
# explicit N                                   | DAYS 14: 13 days old kept, 15 days old removed          | done
# N is a positive integer (implicit)           | DAYS 0, -1, abc, 7x, empty exit 2, delete nothing       | done
# --scratch gates mutate.* deletion            | without --scratch every mutate.* dir survives           | done
#                                              | with --scratch a stale mutate.* goes, a fresh one stays | done
#                                              | --scratch uses DAYS, not the default                    | done
# never anything else                          | non-target files and non-mutate TMPDIR entries survive  | done
#                                              | output outside build/, incremental/, ois-drift/ survives| done
# a checkout with no target/ (edge)            | a worktree with no target/ is skipped, the rest cleaned | done
# per-target summary (plan)                    | each target/ is named in the output with what it freed  | done
# just clean-stale [days] (AC2 entry point)    | just clean-stale passes DAYS and --scratch through      | done
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/bin/clean-stale.sh"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/clean-stale-test.XXXXXX")"
WORK="$(cd "$WORK" && pwd -P)"
HOLDER=""
cleanup() { [[ -n "$HOLDER" ]] && kill "$HOLDER" 2>/dev/null; chmod -R u+rwx "$WORK" 2>/dev/null; rm -rf "$WORK"; }
trap cleanup EXIT

unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR CARGO_TARGET_DIR
export GIT_AUTHOR_NAME=T GIT_AUTHOR_EMAIL=t@e.com GIT_COMMITTER_NAME=T GIT_COMMITTER_EMAIL=t@e.com
export GIT_CONFIG_NOSYSTEM=1 HOME="$WORK"

PASS=0
FAIL=0
TODO=0
ok()   { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad()  { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }
todo() { echo "  TODO $1"; TODO=$((TODO + 1)); }

NOW="$(date +%s)"
# stamp DAYS: the `touch -t` timestamp DAYS whole days before NOW (GNU or BSD date).
stamp() { local at=$((NOW - $1 * 86400)); date -d "@$at" +%Y%m%d%H%M 2>/dev/null || date -r "$at" +%Y%m%d%H%M; }
# unit PATH DAYS: a cargo-shaped output dir holding one file, every entry DAYS old.
unit() { mkdir -p "$1" && printf 'x\n' > "$1/out" && find "$1" -depth -exec touch -t "$(stamp "$2")" {} +; }

# setup CASE: $MAIN (on next, pushed to a bare origin) with a worktree $WT at
# .claude/worktrees/$NAME. Neither has a target/ until a case lays one.
setup() {
    local base="$WORK/$1"
    rm -rf "$base"; mkdir -p "$base/tmp"
    git init -q --bare "$base/origin.git"
    MAIN="$base/main"
    git init -q -b next "$MAIN"
    printf 'target/\n.claude/\n' > "$MAIN/.gitignore"
    git -C "$MAIN" add .gitignore && git -C "$MAIN" -c core.hooksPath=/dev/null commit -qm init
    git -C "$MAIN" remote add origin "$base/origin.git"
    git -C "$MAIN" push -q -u origin next
    NAME="chore+1+$1"
    WT="$MAIN/.claude/worktrees/$NAME"
    git -C "$MAIN" worktree add -q -b "chore/1/$1" "$WT" next
    SCRATCH="$base/tmp"
}

# clean [ARGS...]: from $MAIN unless CWD is set, TMPDIR=$SCRATCH; sets OUT (colors stripped) and RC.
clean() {
    OUT="$(cd "${CWD:-$MAIN}" && TMPDIR="$SCRATCH" bash "$SCRIPT" "$@" 2>&1 | sed $'s/\033\\[[0-9;]*m//g'; exit "${PIPESTATUS[0]}")"
    RC=$?
}
has()     { case "$OUT" in *"$2"*) ok "$1" ;; *) bad "$1 (no '$2' in: $OUT)" ;; esac; }
lacks()   { case "$OUT" in *"$2"*) bad "$1 ('$2' in: $OUT)" ;; *) ok "$1" ;; esac; }
kept()    { if [[ -e "$2" ]]; then ok "$1"; else bad "$1 ($2 gone): $OUT"; fi; }
removed() { if [[ ! -e "$2" ]]; then ok "$1"; else bad "$1 ($2 left): $OUT"; fi; }

echo "== stale units go, fresh siblings stay =="
setup units
B="$MAIN/target/debug/build" I="$MAIN/target/debug/incremental"
unit "$B/pkga/aaa" 8; unit "$B/pkga/bbb" 1
unit "$I/crate-1abc" 8; unit "$I/crate-2def" 1
# An old unit holding one fresh file; the dir itself is re-backdated so only the file is fresh.
unit "$B/pkgb/ccc" 8; printf 'y\n' > "$B/pkgb/ccc/new"; touch -t "$(stamp 1)" "$B/pkgb/ccc/new"; touch -t "$(stamp 8)" "$B/pkgb/ccc"
unit "$I/crate-3ghi" 8; printf 'y\n' > "$I/crate-3ghi/new"; touch -t "$(stamp 1)" "$I/crate-3ghi/new"; touch -t "$(stamp 8)" "$I/crate-3ghi"
# rustc's incremental layout is two levels deep: crate-<hash>/s-<session>/<file>.
unit "$I/crate-4jkl/s-session" 8; printf 'y\n' > "$I/crate-4jkl/s-session/q.bin"
touch -t "$(stamp 1)" "$I/crate-4jkl/s-session/q.bin"; touch -t "$(stamp 8)" "$I/crate-4jkl/s-session" "$I/crate-4jkl"
unit "$B/pkgc/ddd" 8; unit "$B/pkgd/eee" 8; unit "$B/pkgd/fff" 1
unit "$MAIN/target/ois-drift/run-old" 8; unit "$MAIN/target/ois-drift/run-new" 1
unit "$MAIN/target/release/build/pkga/ggg" 8; unit "$MAIN/target/release/build/pkga/hhh" 1
clean
[[ "$RC" -eq 0 ]] && ok "the run exits 0" || bad "the run exits 0 (got $RC): $OUT"
removed "a stale build unit is removed" "$B/pkga/aaa"
kept "...beside a fresh sibling, kept" "$B/pkga/bbb/out"
removed "a stale incremental dir is removed" "$I/crate-1abc"
kept "...beside a fresh one, kept" "$I/crate-2def/out"
kept "an old build unit holding one fresh file is kept" "$B/pkgb/ccc/out"
kept "an old incremental dir holding one fresh file is kept" "$I/crate-3ghi/out"
kept "a fresh file two levels down keeps its unit" "$I/crate-4jkl/s-session/q.bin"
removed "build/<pkg>/ left empty is removed" "$B/pkgc"
kept "build/<pkg>/ still holding a unit is kept" "$B/pkgd/fff/out"
removed "...and its stale unit is removed" "$B/pkgd/eee"
removed "a stale ois-drift dir is removed" "$MAIN/target/ois-drift/run-old"
kept "...beside a fresh one, kept" "$MAIN/target/ois-drift/run-new/out"
removed "a second profile (release) is cleaned too" "$MAIN/target/release/build/pkga/ggg"
kept "...keeping its fresh unit" "$MAIN/target/release/build/pkga/hhh/out"

if [[ "$(id -u)" -ne 0 ]]; then
    setup unreadable-unit
    # find cannot read the unit's subdir, so its age is unknown: kept, not deleted.
    unit "$MAIN/target/debug/build/pkga/aaa/locked" 8; chmod 000 "$MAIN/target/debug/build/pkga/aaa/locked"
    touch -t "$(stamp 8)" "$MAIN/target/debug/build/pkga/aaa"
    unit "$MAIN/target/debug/build/pkga/bbb" 8
    clean
    [[ "$RC" -eq 0 ]] && ok "a unit find cannot read does not fail the run" || bad "a unit find cannot read does not fail the run (got $RC): $OUT"
    kept "a unit find cannot fully read is kept" "$MAIN/target/debug/build/pkga/aaa"
    removed "...while a readable stale sibling goes" "$MAIN/target/debug/build/pkga/bbb"
    chmod 755 "$MAIN/target/debug/build/pkga/aaa/locked"

    setup rmfail
    # A read-only build/<pkg>/ makes rm fail on the stale unit inside it.
    unit "$MAIN/target/debug/build/pkga/aaa" 8; unit "$MAIN/target/debug/build/pkgb/bbb" 8
    chmod a-w "$MAIN/target/debug/build/pkga"
    clean
    chmod u+w "$MAIN/target/debug/build/pkga"
    [[ "$RC" -eq 1 ]] && ok "a directory rm cannot delete exits 1" || bad "a directory rm cannot delete exits 1 (got $RC): $OUT"
    has "...and names it" "could not delete $MAIN/target/debug/build/pkga/aaa"
    removed "...after finishing the rest" "$MAIN/target/debug/build/pkgb"
else
    echo "  SKIP: running as root, which chmod does not stop"
fi

echo "== every checkout's target/ =="
setup checkouts
unit "$MAIN/target/debug/build/pkga/aaa" 8; unit "$MAIN/target/debug/build/pkga/bbb" 1
unit "$WT/target/debug/build/pkga/aaa" 8; unit "$WT/target/debug/build/pkga/bbb" 1
clean
removed "a stale unit in the primary checkout is removed" "$MAIN/target/debug/build/pkga/aaa"
removed "...and one in a linked worktree" "$WT/target/debug/build/pkga/aaa"
kept "the primary's fresh sibling is kept" "$MAIN/target/debug/build/pkga/bbb/out"
kept "the worktree's fresh sibling is kept" "$WT/target/debug/build/pkga/bbb/out"
has "the primary's target/ is named in the summary" "$MAIN/target"
has "the worktree's target/ is named in the summary" "$WT/target"
has "the summary says how many dirs went" "removed 1 dirs"
unit "$MAIN/target/debug/build/pkga/aaa" 8
CWD="$WT" clean
removed "a run from inside a worktree cleans the primary checkout's target/ too" "$MAIN/target/debug/build/pkga/aaa"
setup notarget
unit "$MAIN/target/debug/build/pkga/aaa" 8
clean
[[ "$RC" -eq 0 ]] && ok "a worktree with no target/ does not fail the run" || bad "no-target run exits 0 (got $RC): $OUT"
removed "...and the rest are cleaned" "$MAIN/target/debug/build/pkga/aaa"
removed "...and no target/ is created in the worktree" "$WT/target"

echo "== a profile a build is using =="
setup locked
unit "$MAIN/target/debug/build/pkga/aaa" 8; unit "$MAIN/target/release/build/pkga/aaa" 8
touch "$MAIN/target/debug/.cargo-build-lock" "$MAIN/target/release/.cargo-build-lock"
flock -x "$MAIN/target/debug/.cargo-build-lock" sleep 60 & HOLDER=$!
for _ in $(seq 1 100); do flock -n -x "$MAIN/target/debug/.cargo-build-lock" true || break; read -t 0.1 <> <(:) || true; done
if flock -n -x "$MAIN/target/debug/.cargo-build-lock" true; then bad "precondition: the holder took the lock"; else ok "precondition: the holder took the lock"; fi
clean
kept "a held .cargo-build-lock keeps its profile" "$MAIN/target/debug/build/pkga/aaa/out"
has "...and says so" "kept $MAIN/target/debug: a running build holds .cargo-build-lock"
removed "...while the unlocked profile beside it is cleaned" "$MAIN/target/release/build/pkga/aaa"
kill "$HOLDER" 2>/dev/null; wait "$HOLDER" 2>/dev/null; HOLDER=""

setup lockdelete
unit "$MAIN/target/debug/build/pkga/aaa" 8; unit "$MAIN/target/debug/build/pkga/bbb" 8
L="$MAIN/target/debug/.cargo-build-lock"; touch "$L"
mkdir -p "$WORK/rmstub"
cat > "$WORK/rmstub/rm" <<EOF
#!/bin/bash
if flock -n -x "$L" true; then echo free >> "$WORK/rm.log"; else echo held >> "$WORK/rm.log"; fi
exec "$(command -v rm)" "\$@"
EOF
chmod +x "$WORK/rmstub/rm"
flock -n -x "$L" true && ok "precondition: the lock is free before the run" || bad "precondition: the lock is free before the run"
PATH="$WORK/rmstub:$PATH" clean
removed "the stale unit is removed under the stubbed rm" "$MAIN/target/debug/build/pkga/aaa"
if [[ -s "$WORK/rm.log" ]] && ! grep -qv '^held$' "$WORK/rm.log"; then ok "the build lock cannot be taken while a unit is being deleted"; else bad "the build lock cannot be taken while a unit is being deleted (rm.log: $(cat "$WORK/rm.log" 2>/dev/null))"; fi
flock -n -x "$L" true && ok "the build lock is released after the run" || bad "the build lock is released after the run"

setup unreadable
unit "$MAIN/target/debug/build/pkga/aaa" 8; unit "$MAIN/target/release/build/pkga/aaa" 8
touch "$MAIN/target/debug/.cargo-build-lock"; chmod 000 "$MAIN/target/debug/.cargo-build-lock"
if [[ "$(id -u)" -eq 0 ]]; then
    echo "  SKIP: running as root, which chmod 000 does not stop"
else
    clean
    kept "an unreadable lock file keeps its profile" "$MAIN/target/debug/build/pkga/aaa/out"
    has "...and says the lock could not be taken" "or the lock could not be taken"
    removed "...while a readable profile is cleaned" "$MAIN/target/release/build/pkga/aaa"
fi
chmod 644 "$MAIN/target/debug/.cargo-build-lock"

setup noflock
unit "$MAIN/target/debug/build/pkga/aaa" 8; unit "$MAIN/target/release/build/pkga/aaa" 8
unit "$MAIN/target/ois-drift/run-old" 8
mkdir -p "$WORK/noflock"
IFS=: read -r -a dirs <<< "$PATH"
for d in "${dirs[@]}"; do
    for t in "$d"/*; do
        n="${t##*/}"; [[ "$n" == flock || -e "$WORK/noflock/$n" || ! -x "$t" ]] && continue
        ln -s "$t" "$WORK/noflock/$n"
    done
done
PATH="$WORK/noflock" clean
[[ "$RC" -eq 0 ]] && ok "flock missing still exits 0" || bad "flock missing still exits 0 (got $RC): $OUT"
kept "flock missing from PATH keeps the debug profile" "$MAIN/target/debug/build/pkga/aaa/out"
kept "...and the release profile" "$MAIN/target/release/build/pkga/aaa/out"
has "...and names flock as missing" "flock is not installed"
removed "...while ois-drift, which takes no lock, is cleaned (positive control)" "$MAIN/target/ois-drift/run-old"

echo "== the age threshold =="
setup threshold
B="$MAIN/target/debug/build/pkga"
unit "$B/six" 6; unit "$B/eight" 8; unit "$B/thirteen" 13; unit "$B/fifteen" 15
mkdir -p "$SCRATCH/mutate.x"; unit "$SCRATCH/mutate.x" 30
unit "$MAIN/target/debug/build/pkgz/thirty" 30
for d in 0 -1 abc 7x 007 ''; do
    clean "$d"
    [[ "$RC" -eq 2 ]] && ok "DAYS '$d' exits 2" || bad "DAYS '$d' exits 2 (got $RC): $OUT"
    has "...with the usage line" "usage:"
    clean --scratch "$d"
    [[ "$RC" -eq 2 ]] && ok "--scratch with DAYS '$d' exits 2" || bad "--scratch with DAYS '$d' exits 2 (got $RC)"
done
clean 7 8
[[ "$RC" -eq 2 ]] && ok "two DAYS values exit 2" || bad "two DAYS values exit 2 (got $RC)"
kept "an invalid DAYS deletes nothing" "$MAIN/target/debug/build/pkgz/thirty/out"
kept "...and leaves mutate.* alone" "$SCRATCH/mutate.x/out"
clean 14
kept "DAYS 14 keeps a 13-day-old unit" "$B/thirteen/out"
kept "DAYS 14 keeps an 8-day-old unit" "$B/eight/out"
removed "DAYS 14 removes a 15-day-old unit" "$B/fifteen"
clean
kept "with no DAYS a 6-day-old unit is kept" "$B/six/out"
removed "with no DAYS an 8-day-old unit is removed" "$B/eight"

echo "== --scratch =="
setup scratch
unit "$SCRATCH/mutate.old" 8; unit "$SCRATCH/mutate.new" 1; unit "$SCRATCH/mutate.older" 15
clean
kept "without --scratch a stale mutate.* dir survives" "$SCRATCH/mutate.older/out"
lacks "...and the summary does not mention mutate.*" "mutate.*"
clean --scratch 14
kept "--scratch 14 keeps an 8-day-old mutate.* dir" "$SCRATCH/mutate.old/out"
removed "--scratch 14 removes a 15-day-old one" "$SCRATCH/mutate.older"
clean --scratch
removed "with --scratch a stale mutate.* dir is removed" "$SCRATCH/mutate.old"
kept "...and a fresh one kept" "$SCRATCH/mutate.new/out"

echo "== nothing else is touched =="
setup untouched
unit "$MAIN/target/debug/build/pkga/aaa" 8; unit "$MAIN/target/ois-drift/run-new" 1
unit "$MAIN/old-dir" 8; printf 'n\n' > "$MAIN/notes.txt"; touch -t "$(stamp 8)" "$MAIN/notes.txt"
unit "$SCRATCH/other.abc" 8; unit "$SCRATCH/setup-worktree-test.x" 8
printf 'm\n' > "$SCRATCH/mutate.txt"; touch -t "$(stamp 8)" "$SCRATCH/mutate.txt"
unit "$MAIN/target/debug/deps/old" 8; unit "$MAIN/target/debug/examples/old" 8; unit "$MAIN/target/doc/old" 8
unit "$MAIN/target/tmp/old" 8
clean --scratch
removed "the stale build unit is removed (positive control)" "$MAIN/target/debug/build/pkga/aaa"
for f in "$MAIN/old-dir/out" "$MAIN/notes.txt" "$SCRATCH/other.abc/out" "$SCRATCH/setup-worktree-test.x/out" \
         "$SCRATCH/mutate.txt" "$MAIN/target/debug/deps/old/out" "$MAIN/target/debug/examples/old/out" \
         "$MAIN/target/doc/old/out" "$MAIN/target/tmp/old/out" "$MAIN/target/debug/.cargo-build-lock"; do
    kept "untouched: ${f#"$WORK"/}" "$f"
done
for d in doc tmp ois-drift; do
    removed "no lock file is created in target/$d, which is not a profile" "$MAIN/target/$d/.cargo-build-lock"
done

echo "== just clean-stale =="
if command -v just >/dev/null 2>&1; then
    setup recipe
    B="$MAIN/target/debug/build/pkga"
    unit "$B/thirteen" 13; unit "$B/fifteen" 15; unit "$SCRATCH/mutate.older" 15
    # The recipe runs bin/clean-stale.sh relative to the justfile, so the scratch checkout
    # carries copies of the real justfile and the scripts it reaches.
    mkdir -p "$MAIN/bin/lib"
    cp "$ROOT/justfile" "$MAIN/justfile"; cp "$SCRIPT" "$MAIN/bin/clean-stale.sh"; cp "$ROOT/bin/lib/portable.sh" "$MAIN/bin/lib/portable.sh"
    OUT="$(cd "$MAIN" && TMPDIR="$SCRATCH" just clean-stale --scratch 14 2>&1)"; RC=$?
    [[ "$RC" -eq 0 ]] && ok "just clean-stale exits 0" || bad "just clean-stale exits 0 (got $RC): $OUT"
    kept "just clean-stale 14 keeps a 13-day-old unit" "$B/thirteen/out"
    removed "just clean-stale 14 removes a 15-day-old unit" "$B/fifteen"
    removed "just clean-stale --scratch reaches the script" "$SCRATCH/mutate.older"
else
    echo "  SKIP: just is not installed"
fi

echo
echo "clean-stale: $PASS passed, $FAIL failed, $TODO todo"
[ "$FAIL" -eq 0 ]
