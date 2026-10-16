#!/bin/bash
# ==============================================================================
# Tests for the marker ledger: .claude/hooks/marker-write-gate.sh (the PreToolUse
# snapshot), .claude/hooks/marker-audit.sh (the PostToolUse audit),
# .claude/hooks/lib/marker_ledger.py and bin/lib/marker_manifest.py.
#
#   bash bin/lib/marker-audit.test.sh
#
# Plays tool calls as Claude Code does (the gate, the command, the audit, one
# tool_use_id) in a scratch repository with a worktree. A marker and manifest line
# that a call cannot account for are undone, by every route; the lines just
# test, bin/run-tests.sh and bin/review-marker.sh write in the foreground, or
# backgrounded with a grant, are kept and pass check_marker_for_head, on a first
# run and a re-run. A line only another call's open snapshot could explain is held
# out of the manifest until that call's own audit, so a snapshot whose call never
# reaches an audit authorizes nothing. Exits non-zero on any failure.
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Resolved (pwd -P): the audit names checkouts by their real path, and macOS's TMPDIR sits behind /var -> /private/var.
WORK="$(cd "$(mktemp -d "${TMPDIR:-/tmp}/marker-audit-test.XXXXXX")" && pwd -P)"
trap 'rm -rf "$WORK"' EXIT
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR
export GIT_AUTHOR_NAME=T GIT_AUTHOR_EMAIL=t@e.com GIT_COMMITTER_NAME=T GIT_COMMITTER_EMAIL=t@e.com
export GIT_CONFIG_NOSYSTEM=1 HOME="$WORK"

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

REPO="$WORK/repo"
mkdir -p "$REPO/.claude" "$REPO/bin/lib" "$REPO/rules"
cp -R "$ROOT/.claude/hooks" "$REPO/.claude/hooks"
cp "$ROOT/bin/check-marker.sh" "$ROOT/bin/review-marker.sh" "$REPO/bin/"
# The ledger, not the gate, is under test here (bin/lib/run-tests.test.sh drives the real gate with stub cargo and
# pnpm): this bin/run-tests.sh stamps HEAD's tests marker the way the real one does after a green run on a clean tree.
cat > "$REPO/bin/run-tests.sh" <<'STUB'
#!/bin/bash
ROOT="$(git rev-parse --show-toplevel)" && cd "$ROOT" || exit 1
[[ $# -gt 0 ]] && exit 0
[[ -z "$(git status --porcelain --untracked-files=normal)" ]] || { echo "run-tests: no tests marker written (working tree was not clean at start)."; exit 0; }
python3 -I bin/lib/marker_manifest.py stamp "$ROOT" tests "$(git rev-parse HEAD)"
STUB
cp "$ROOT/bin/lib/marker_manifest.py" "$ROOT/bin/lib/file_lock.py" "$ROOT/bin/lib/review-attestation.sh" "$REPO/bin/lib/"
printf 'code\trules/a.md\n' > "$REPO/.claude/review-lenses.tsv"
printf '## Alpha\n' > "$REPO/rules/a.md"
printf '.claude/markers/\n__pycache__/\n' > "$REPO/.gitignore"
git -C "$REPO" init -q -b next && git -C "$REPO" add . && git -C "$REPO" commit -qm init
SHA="$(git -C "$REPO" rev-parse HEAD)"
M="$REPO/.claude/markers"
LEDGER="$REPO/.git/ois-marker-ledger"
WT="$WORK/wt"
git -C "$REPO" worktree add -q -b feat/1/x "$WT"

mkdir -p "$WORK/bin"
printf '#!/bin/bash\n[[ "$*" == test ]] && exec bash bin/run-tests.sh\nexit 1\n' > "$WORK/bin/just"
chmod +x "$WORK/bin/just"
# macOS tar and zip carry extended attributes as ._ files, which would leave the scratch tree unclean.
export COPYFILE_DISABLE=1
export PATH="$WORK/bin:$PATH"

N=0
payload() { # payload ID COMMAND BACKGROUND
    python3 -c 'import json,sys; print(json.dumps({"tool_name": "Bash", "tool_use_id": sys.argv[1], "cwd": sys.argv[4],
        "tool_input": {"command": sys.argv[2], "run_in_background": sys.argv[3] == "1"}}))' "$1" "$2" "$3" "$REPO"
}
# call COMMAND [ACTUAL] [BACKGROUND]: the gate sees COMMAND, the shell runs ACTUAL (default COMMAND), then
# the audit. Sets PRE (the gate's exit) and OUT/RC (the audit's). A backgrounded call runs ACTUAL after
# the audit, as a run_in_background command finishes after its call returns.
call() {
    local command="$1" actual="${2:-$1}" background="${3:-0}" id
    N=$((N + 1)); id="toolu_$N"
    payload "$id" "$command" "$background" | (cd "$REPO" && bash .claude/hooks/marker-write-gate.sh >/dev/null 2>&1)
    PRE=$?
    [[ $PRE -eq 0 && "$background" == 0 ]] && (cd "$REPO" && bash -c "$actual" >/dev/null 2>&1)
    OUT="$(payload "$id" "$command" "$background" | (cd "$REPO" && bash .claude/hooks/marker-audit.sh 2>&1))"
    RC=$?
    [[ $PRE -eq 0 && "$background" == 1 ]] && (cd "$REPO" && bash -c "$actual" >/dev/null 2>&1)
    return 0
}
# pre ID COMMAND [BACKGROUND], post ID COMMAND [BACKGROUND]: one call's gate and its audit (sets OUT), run
# apart so calls can interleave.
pre() { payload "$1" "$2" "${3:-0}" | (cd "$REPO" && bash .claude/hooks/marker-write-gate.sh >/dev/null 2>&1); }
post() { OUT="$(payload "$1" "$2" "${3:-0}" | (cd "$REPO" && bash .claude/hooks/marker-audit.sh 2>&1))"; }
# postfail ID COMMAND [BACKGROUND]: the audit as PostToolUseFailure delivers it (an error, no tool_response).
postfail() {
    OUT="$(payload "$1" "$2" "${3:-0}" | python3 -c 'import json, sys
data = json.load(sys.stdin)
data.update({"hook_event_name": "PostToolUseFailure", "error": "Exit code 1", "is_interrupt": False})
print(json.dumps(data))' | (cd "$REPO" && bash .claude/hooks/marker-audit.sh 2>&1))"
}
# stamp_by_hand KIND: marker_manifest.py stamp run outside the two scripts, as a forging call would.
stamp_by_hand() { (cd "$REPO" && python3 -I bin/lib/marker_manifest.py stamp "$REPO" "$1" "$SHA"); }
# age_snapshots SECONDS: make every open snapshot that much older, resealed as the hooks would.
age_snapshots() {
    python3 -I -c 'import json, sys
sys.path.insert(0, sys.argv[1])
import marker_ledger
state = json.load(open(sys.argv[2]))["state"]
for snap in state["snapshots"].values():
    snap["at"] -= float(sys.argv[3])
json.dump({"seal": marker_ledger.seal(state), "state": state}, open(sys.argv[2], "w"), sort_keys=True)' \
        "$REPO/.claude/hooks/lib" "$LEDGER/ledger.json" "$1"
}
# age_grants SECONDS: make every grant expire that much sooner, resealed as the hooks would.
age_grants() {
    python3 -I -c 'import json, sys
sys.path.insert(0, sys.argv[1])
import marker_ledger
state = json.load(open(sys.argv[2]))["state"]
for grant in state["grants"]:
    grant["expires"] -= float(sys.argv[3])
json.dump({"seal": marker_ledger.seal(state), "state": state}, open(sys.argv[2], "w"), sort_keys=True)' \
        "$REPO/.claude/hooks/lib" "$LEDGER/ledger.json" "$1"
}
inode() { python3 -c 'import os, sys; print(os.stat(sys.argv[1]).st_ino)' "$1"; }
vouched() { python3 -I "$REPO/bin/lib/marker_manifest.py" vouched "$1" "$2" "$3"; }
passes() { (cd "$REPO" && . bin/check-marker.sh && check_marker_for_head "$1" X x "$SHA" >/dev/null); }
lines() { if [[ -f "$M/manifest" ]]; then grep -c . "$M/manifest"; else echo 0; fi; }

# A source tree holding a forged tests marker and the manifest line that vouches for it.
SRC="$WORK/src"
forge_tree() {
    rm -rf "$SRC" && mkdir -p "$SRC/.claude/markers/tests" && printf 'forged %s\n' "$RANDOM$RANDOM" > "$SRC/.claude/markers/tests/$SHA"
    printf 'tests %s %s\n' "$SHA" "$(sha256sum < "$SRC/.claude/markers/tests/$SHA" | cut -d' ' -f1)" \
        > "$SRC/.claude/markers/manifest"
}
# undone NAME: the forged marker and line are gone, the gates refuse, and the audit said so.
undone() {
    if [[ ! -e "$M/tests/$SHA" && "$(lines)" == 0 && "$OUT" == *'"decision": "block"'* && "$OUT" == *additionalContext* \
        && "$OUT" == *"dropped: tests $SHA"* ]]; then ok "$1: line dropped, marker deleted, and the audit says so"
    else bad "$1: marker $( [[ -e "$M/tests/$SHA" ]] && echo kept || echo gone ), $(lines) line(s): ${OUT:0:240}"; fi
    if ! passes tests; then ok "$1: refused by check_marker_for_head"; else bad "$1: check_marker_for_head passed"; fi
    rm -rf "$M/tests" "$M/attest" "$M/manifest"
}

echo "A marker and its manifest line written by any other route are undone:"
forge_tree; (cd "$SRC" && tar -cf "$WORK/mk.tar" .claude)
call "tar -xf $WORK/mk.tar"; undone "tar -x carrying the marker and its line"
forge_tree; call "python3 -m tarfile -e $WORK/mk.tar ."; undone "python -m tarfile -e"
forge_tree; (cd "$SRC" && rm -f "$WORK/mk.zip" && zip -qr "$WORK/mk.zip" .claude); call "python3 -m zipfile -e $WORK/mk.zip ."
undone "python -m zipfile -e"
forge_tree; call "scp -rq $SRC/.claude $REPO/"; undone "scp -r"
forge_tree; printf '.claude/markers/tests/%s\n.claude/markers/manifest\n' "$SHA" > "$WORK/list"
call "rsync -r --files-from=$WORK/list $SRC $REPO"; undone "rsync --files-from"
forge_tree; call "cp -r $SRC/. \"\$PWD\""; undone "cp -r into \$PWD"
forge_tree
LINE="tests $SHA $(sha256sum < "$SRC/.claude/markers/tests/$SHA" | cut -d' ' -f1)"
printf 'diff --git a/.claude/markers/tests/%s b/.claude/markers/tests/%s\nnew file mode 100644\n--- /dev/null\n+++ b/.claude/markers/tests/%s\n@@ -0,0 +1 @@\n+%s\n' \
    "$SHA" "$SHA" "$SHA" "$(cat "$SRC/.claude/markers/tests/$SHA")" > "$WORK/p.diff"
printf 'diff --git a/.claude/markers/manifest b/.claude/markers/manifest\nnew file mode 100644\n--- /dev/null\n+++ b/.claude/markers/manifest\n@@ -0,0 +1 @@\n+%s\n' "$LINE" >> "$WORK/p.diff"
call "git apply $WORK/p.diff"; undone "git apply"
call "patch -s -p1 -i $WORK/p.diff"; undone "patch"
printf '#!/bin/bash\ncp -r %s/. .\n' "$SRC" > "$WORK/forge.sh"
forge_tree; call "bash $WORK/forge.sh"; undone "a script run by path"
mkdir -p "$M/attest"
forge_tree; rm -rf "$WORK/flat" && cp -r "$SRC/.claude/markers" "$WORK/flat"
call "ln -s .. .claude/markers/attest/up && cp -r $WORK/flat/. .claude/markers/attest/up/"
undone "a copy through a link in attest/"
forge_tree; call "(sleep 1; cp -r $SRC/. .) & wait"; undone "a background sleep, then the copy"
forge_tree; call "true" "cp -r $SRC/. ."; undone "a write the call does not show"

echo "...and so is a forged line between calls, by the next audit:"
forge_tree; (cd "$REPO" && cp -r "$SRC/." .); call "ls"; undone "a line that appeared between calls"

echo "A stamping call keeps exactly what it accounts for:"
call "just test"
if vouched "$REPO" tests "$SHA" && [[ -z "$OUT" ]] && passes tests; then ok "just test: kept silently and passes"; else bad "just test: ${OUT:0:240}"; fi
call "bash bin/run-tests.sh"
if vouched "$REPO" tests "$SHA" && [[ -z "$OUT" ]]; then ok "bash bin/run-tests.sh: kept silently"; else bad "run-tests: ${OUT:0:240}"; fi
call "just test 2>&1 | tail -5"
if [[ -z "$OUT" ]]; then ok "just test piped to tail: still a stamping call"; else bad "piped just test: ${OUT:0:240}"; fi
mkdir -p "$M/attest/$SHA" && printf '| rules/a.md | Alpha | Checked | x.rs:1 |\n' > "$M/attest/$SHA/code.md"
call "bash bin/review-marker.sh shipping $SHA"
if vouched "$REPO" review-shipping "$SHA" && vouched "$REPO" code-review "$SHA" && [[ -z "$OUT" ]] && passes review-shipping
then ok "review-marker.sh shipping: both kept and pass"; else bad "review-marker shipping: ${OUT:0:240}"; fi
rm -rf "$M/code-review" "$M/review-shipping" && call "ls"
call "bash bin/review-marker.sh code-review $SHA" \
    "bash bin/review-marker.sh code-review $SHA && printf 'review-shipping %s %064d\n' $SHA 0 >> .claude/markers/manifest"
if vouched "$REPO" code-review "$SHA" && ! grep -q "^review-shipping $SHA 0000" "$M/manifest" && [[ "$OUT" == *"dropped: review-shipping"* ]]
then ok "a line of a kind the call does not stamp is dropped, its own kept"; else bad "kind attribution: ${OUT:0:240}"; fi
rm -rf "$M" && call "ls" # a new stamp in the second of an accepted one would be that same line
call "just test; touch x"
if [[ "$OUT" == *"dropped: tests $SHA"* ]]; then ok "a stamper with another command beside it is not a stamping call"; else bad "stamper + other: ${OUT:0:240}"; fi
rm -rf "$M/tests" "$REPO/x" && call "ls"
call "cd . && just test"
if [[ "$OUT" == *"dropped: tests $SHA"* ]]; then ok "a stamper after a relative cd is not a stamping call"; else bad "relative cd + stamper: ${OUT:0:240}"; fi
call "cd $REPO/bin && just test" "just test"
if [[ "$OUT" == *"dropped: tests $SHA"* ]]; then ok "a stamper after a cd below the checkout root is not a stamping call"; else bad "cd below root + stamper: ${OUT:0:240}"; fi
call "cd $REPO && timeout 600 just test" "just test"
if [[ "$OUT" == *"dropped: tests $SHA"* ]]; then ok "a wrapped stamper after a cd is not a stamping call"; else bad "cd + wrapper: ${OUT:0:240}"; fi
call "cd $REPO && just test"
if vouched "$REPO" tests "$SHA" && [[ -z "$OUT" ]] && passes tests; then ok "cd ABSDIR && just test: kept silently and passes"; else bad "cd ABSDIR + stamper: ${OUT:0:240}"; fi
call "cd $WT && just test"
if vouched "$WT" tests "$SHA" && [[ -z "$OUT" ]]; then ok "cd to a worktree's root stamps for that worktree"; else bad "cd WORKTREE + stamper: ${OUT:0:240}"; fi
rm -rf "$M" "$WT/.claude/markers" && call "ls"
call "cd $WT && just test" "just test"
if [[ "$OUT" == *"$REPO dropped: tests $SHA"* ]] && ! vouched "$REPO" tests "$SHA"; then ok "...and for no other checkout"; else bad "cd WORKTREE authority: ${OUT:0:240}"; fi
rm -rf "$M"

echo "A backgrounded stamper's grant covers the line it writes after the call returns:"
call "just test" "just test" 1
if [[ -z "$OUT" ]]; then ok "positive control: the launch is silent"; else bad "background launch: ${OUT:0:240}"; fi
[[ -e "$M/tests/$SHA" ]] && ok "positive control: the backgrounded run then wrote its marker" || bad "the backgrounded run wrote nothing"
call "ls"
if vouched "$REPO" tests "$SHA" && [[ -z "$OUT" ]] && passes tests; then ok "the next audit keeps it under the grant, and it passes"; else bad "granted line: ${OUT:0:240}"; fi
forge_tree; cp "$M/manifest" "$WORK/kept"
cp "$SRC/.claude/markers/tests/$SHA" "$M/tests/$SHA" && cat "$SRC/.claude/markers/manifest" >> "$M/manifest"
call "ls"
if [[ "$OUT" == *"dropped: tests $SHA"* ]] && cmp -s "$M/manifest" "$WORK/kept"; then ok "the grant is one-shot: a second line for it is dropped"
else bad "one-shot grant: ${OUT:0:240}"; fi
rm -rf "$M"
call "bash bin/review-marker.sh code-review $SHA" "true" 1
forge_tree; call "ls" "cp -r $SRC/. ."
if [[ "$OUT" == *"dropped: tests $SHA"* ]]; then ok "a grant for one kind does not cover another"; else bad "grant kind: ${OUT:0:240}"; fi
rm -rf "$M"

call "ls"
call "just test" "true" 1
(cd "$WT" && python3 -I "$REPO/bin/lib/marker_manifest.py" stamp "$WT" tests "$SHA"); call "ls"
if [[ "$OUT" == *"$WT dropped: tests $SHA"* && ! -e "$WT/.claude/markers/tests/$SHA" ]]; then ok "a grant for one checkout does not cover a line in another"; else bad "grant root: ${OUT:0:240}"; fi
stamp_by_hand tests; call "ls"
if vouched "$REPO" tests "$SHA" && [[ -z "$OUT" ]]; then ok "positive control: that grant was open, and still covers its own checkout"; else bad "grant root control: ${OUT:0:240}"; fi
rm -rf "$M" && call "ls"
call "just test" "true" 1
age_grants 14400
stamp_by_hand tests; call "ls"
if [[ "$OUT" == *"dropped: tests $SHA"* ]] && ! vouched "$REPO" tests "$SHA"; then ok "a grant past its expiry covers nothing"; else bad "expired grant: ${OUT:0:240}"; fi
rm -rf "$M" && call "ls"

echo "A re-run on a SHA that is already stamped ends with a vouched marker:"
call "ls"
call "just test" "just test" 1
call "ls"
COUNT=1
for run in second third; do
    sleep 1.1 # the marker holds the second it was written in, so a later run writes a different line
    COUNT=$((COUNT + 1))
    call "just test" "just test" 1
    call "ls"
    if vouched "$REPO" tests "$SHA" && [[ -z "$OUT" && "$(lines)" == "$COUNT" ]] && passes tests
    then ok "backgrounded, $run run: its new line is kept and the marker passes"; else bad "backgrounded $run run, $(lines) line(s): ${OUT:0:240}"; fi
done
rm -rf "$M" && call "ls"
pre toolu_fast "just test" 1
(cd "$REPO" && just test >/dev/null 2>&1)
post toolu_fast "just test" 1
if vouched "$REPO" tests "$SHA" && [[ -z "$OUT" ]]; then ok "a backgrounded run that finished before its audit is kept"; else bad "fast backgrounded run: ${OUT:0:240}"; fi
forge_tree; (cd "$REPO" && cp -r "$SRC/." .); call "ls"
if [[ "$OUT" == *"dropped: tests $SHA"* ]]; then ok "...and leaves no grant behind for a later line"; else bad "spare grant: ${OUT:0:240}"; fi
rm -rf "$M" && call "ls"
call "just test"
COUNT=1
for run in second third; do
    sleep 1.1
    COUNT=$((COUNT + 1))
    call "just test"
    if vouched "$REPO" tests "$SHA" && [[ -z "$OUT" && "$(lines)" == "$COUNT" ]] && passes tests
    then ok "foreground, $run run: its new line is kept and the marker passes"; else bad "foreground $run run, $(lines) line(s): ${OUT:0:240}"; fi
done
rm -rf "$M" && call "ls"

echo "Another call's audit landing before the stamping call's own holds the line pending, and only the owner accepts it:"
pre toolu_a "just test"
(cd "$REPO" && just test >/dev/null 2>&1)
pre toolu_b "ls"
post toolu_b "ls"
if [[ -z "$OUT" && -f "$M/tests/$SHA" && "$(lines)" == 0 ]]; then ok "the other call's audit takes the line out of the manifest, keeps the marker file and says nothing"
else bad "interleaved audit, $(lines) line(s): ${OUT:0:240}"; fi
if ! vouched "$REPO" tests "$SHA" && ! passes tests; then ok "...so a pending marker is not vouched and check_marker_for_head refuses it"; else bad "a pending marker passed"; fi
call "ls"
if [[ -z "$OUT" && -f "$M/tests/$SHA" ]] && ! passes tests; then ok "...and it stays pending across further audits while the stamper is open"; else bad "pending across audits: ${OUT:0:240}"; fi
post toolu_a "just test"
if vouched "$REPO" tests "$SHA" && [[ -z "$OUT" && "$(lines)" == 1 ]] && passes tests; then ok "...until the stamping call's own audit puts it back and accepts it"; else bad "own audit after another's: ${OUT:0:240}"; fi
forge_tree; (cd "$REPO" && cp -r "$SRC/." .); call "ls"
if [[ "$OUT" == *"dropped: tests $SHA"* ]]; then ok "...and once it is audited the snapshot authorizes nothing"; else bad "closed snapshot: ${OUT:0:240}"; fi
rm -rf "$M" && call "ls"
pre toolu_old "just test"
age_snapshots 14400
(cd "$REPO" && just test >/dev/null 2>&1)
call "ls"
if [[ "$OUT" == *"dropped: tests $SHA"* ]]; then ok "a stamper snapshot older than any call can run authorizes nothing"; else bad "aged snapshot: ${OUT:0:240}"; fi
rm -rf "$M"

echo "A snapshot whose call never reaches an audit authorizes nothing:"
for decoy in "tests|echo 'git push origin next'; just test" "review-shipping|echo 'git push origin next'; bash bin/review-marker.sh shipping $SHA"; do
    kind="${decoy%%|*}"; line="${decoy#*|}"
    N=$((N + 1)); pre "toolu_decoy_$N" "$line" # blocked by another gate, denied or cancelled: no audit follows
    if grep -q "\"$kind\"" "$LEDGER/ledger.json"; then ok "positive control: the $kind decoy left an open stamper snapshot"; else bad "the $kind decoy recorded no stamper"; fi
    call "true" "python3 -I bin/lib/marker_manifest.py stamp $REPO $kind $SHA"
    if [[ "$(lines)" == 0 ]] && ! vouched "$REPO" "$kind" "$SHA" && ! passes "$kind"; then ok "$kind stamped by hand under the stuck snapshot: line removed, check_marker_for_head refuses"
    else bad "stuck $kind snapshot, $(lines) line(s), $(passes "$kind" && echo passes || echo refused): ${OUT:0:240}"; fi
    stamp_by_hand "$kind"; call "ls"; call "ls"
    if [[ "$(lines)" == 0 ]] && ! passes "$kind"; then ok "...and again on a second attempt and later audits"; else bad "stuck $kind snapshot, second attempt: $(lines) line(s)"; fi
    age_snapshots 14400; call "ls"
    if [[ "$OUT" == *"dropped: $kind $SHA"* && ! -e "$M/$kind/$SHA" ]]; then ok "...and once the snapshot is older than the grant window the held line and marker are dropped, loudly"
    else bad "aged stuck $kind snapshot: ${OUT:0:240}"; fi
    rm -rf "$M" && call "ls"
done
pre toolu_c "just test"
stamp_by_hand tests; call "ls"
post toolu_c "just test"
rm -rf "$M" && call "ls"
stamp_by_hand tests; call "ls"
if [[ "$OUT" == *"dropped: tests $SHA"* && ! -e "$M/tests/$SHA" ]]; then ok "a closed snapshot holds nothing pending: a later line is dropped at once"; else bad "closed snapshot: ${OUT:0:240}"; fi
rm -rf "$M" && call "ls"

echo "PostToolUseFailure closes a failed stamper's snapshot at once:"
pre toolu_f "just test"
postfail toolu_f "just test"
[[ -z "$OUT" ]] && ok "the failure payload (an error, no tool_response) is audited without complaint" || bad "failure audit: ${OUT:0:240}"
stamp_by_hand tests; call "ls"
if [[ "$OUT" == *"dropped: tests $SHA"* && ! -e "$M/tests/$SHA" ]]; then ok "...and a line for that kind and SHA is then dropped, not held"; else bad "after a failed stamper: ${OUT:0:240}"; fi
pre toolu_g "just test; grep -q absent /dev/null"
(cd "$REPO" && just test >/dev/null 2>&1)
postfail toolu_g "just test; grep -q absent /dev/null"
if vouched "$REPO" tests "$SHA" && [[ -z "$OUT" ]] && passes tests; then ok "a stamper that stamped on a line whose last command failed keeps its line"; else bad "stamp then a failing read: ${OUT:0:240}"; fi
rm -rf "$M" && call "ls"
pre toolu_h "just test" 1
postfail toolu_h "just test" 1
stamp_by_hand tests; call "ls"
if [[ "$OUT" == *"dropped: tests $SHA"* ]]; then ok "a backgrounded stamper that failed to start leaves no grant"; else bad "grant after a failed launch: ${OUT:0:240}"; fi
stamp_by_hand tests
pre toolu_i "ls"; postfail toolu_i "ls"
if [[ "$OUT" == *'"hookEventName": "PostToolUseFailure"'* && "$OUT" == *"dropped: tests $SHA"* ]]; then ok "what a failure audit undoes is reported under its own event name"; else bad "failure event output: ${OUT:0:240}"; fi
rm -rf "$M" && call "ls"

echo "A read beside a stamper cannot write under its authority:"
mkdir -p "$M/review-shipping" "$M/code-review" && : > "$M/manifest" && call "ls"
FORGED_DIGEST="$(printf '1\n' | sha256sum | cut -d' ' -f1)"
printf "printf '1\\\\n' > .claude/\${X:-markers}/review-shipping/%s; printf 'review-shipping %s %s\\\\n' >> .claude/\${X:-markers}/manifest; bash bin/review-marker.sh shipping %s\n" \
    "$SHA" "$SHA" "$FORGED_DIGEST" "$SHA" > "$WORK/ride.sh"
for shell in bash zsh; do
    command -v "$shell" >/dev/null 2>&1 || { echo "  SKIP: $shell is not installed"; continue; }
    call "$(cat "$WORK/ride.sh")" "$shell $WORK/ride.sh"
    if [[ $PRE -eq 0 && "$OUT" == *"dropped: review-shipping $SHA"* && ! -e "$M/review-shipping/$SHA" ]] && ! passes review-shipping
    then ok "$shell: redirects through a path the shell builds, then a failing review-marker.sh: dropped and refused"
    else bad "$shell: a forged review-shipping marker rode the stamper (gate $PRE): ${OUT:0:240}"; fi
done
rm -rf "$M" && call "ls"
call "python3 -I bin/lib/marker_manifest.py stamp $REPO tests $SHA"
if [[ $PRE -eq 2 && ! -e "$M/tests/$SHA" ]]; then ok "marker_manifest.py stamp run from a Bash line is refused before it runs"; else bad "a hand-run stamp passed the gate (gate $PRE)"; fi
rm -rf "$M" && call "ls"

echo "A manifest that is not a regular file is said so, loudly:"
call "just test"
cp "$M/manifest" "$WORK/elsewhere-manifest" && rm "$M/manifest" && ln -s "$WORK/elsewhere-manifest" "$M/manifest"
call "ls"
if [[ "$OUT" == *"$M/manifest was a symbolic link"* && "$OUT" == *'"decision": "block"'* && ! -L "$M/manifest" && -s "$WORK/elsewhere-manifest" && ! -e "$M/tests/$SHA" ]] && ! passes tests
then ok "a symlinked manifest is named and removed (not its target), and the marker it listed goes"; else bad "symlinked manifest: ${OUT:0:300}"; fi
call "just test"
if vouched "$REPO" tests "$SHA" && [[ -z "$OUT" ]] && passes tests; then ok "...after which a stamp is kept again"; else bad "stamp after a symlinked manifest: ${OUT:0:240}"; fi
rm -f "$M/manifest" && ln -s "$WORK/elsewhere-manifest" "$M/manifest" && cp "$WORK/elsewhere-manifest" "$WORK/before"
ERR="$( (cd "$REPO" && python3 -I bin/lib/marker_manifest.py stamp "$REPO" tests "$SHA") 2>&1)"; RC=$?
if [[ $RC -ne 0 && "$ERR" == *"$M/manifest is not a regular file"* ]] && cmp -s "$WORK/elsewhere-manifest" "$WORK/before"
then ok "stamp refuses to append through a symlinked manifest, naming it"; else bad "stamp through a link: rc=$RC $ERR"; fi
rm -rf "$M" && mkdir -p "$M/manifest" && call "ls"
if [[ "$OUT" == *"$M/manifest is not a regular file"* && "$OUT" == *"remove it"* && $RC -ne 2 ]]; then ok "a manifest that is a directory is reported with what to do"; else bad "directory manifest: ${OUT:0:240}"; fi
rm -rf "$M" && call "ls"

echo "The manifest is read and rewritten under one hold of the stamp lock:"
pre toolu_lock "just test"
OTHER_SHA="$(printf '%040d' 7)"
mkdir -p "$M/tests" && printf 'code-review %s %064d\n' "$OTHER_SHA" 7 > "$M/manifest" # a line the audit must drop, so it rewrites
python3 -I -c 'import sys, time
sys.path.insert(0, sys.argv[3]); import file_lock
handle = open(sys.argv[1], "a+"); file_lock.lock(handle)
open(sys.argv[2], "w").close(); time.sleep(3)' "$M/.manifest.lock" "$WORK/locked" "$REPO/bin/lib" &
HOLDER=$!
while [[ ! -e "$WORK/locked" ]]; do sleep 0.1; done
(payload toolu_lock "just test" 0 | (cd "$REPO" && bash .claude/hooks/marker-audit.sh > "$WORK/lock.out" 2>&1)) &
AUDIT=$!
sleep 1.5 # the audit is now waiting on the lock; a stamp finishing meanwhile appends its line
WAITING=0; kill -0 "$AUDIT" 2>/dev/null && WAITING=1
date +%s > "$M/tests/$SHA" && printf 'tests %s %s\n' "$SHA" "$(sha256sum < "$M/tests/$SHA" | cut -d' ' -f1)" >> "$M/manifest"
wait "$HOLDER" "$AUDIT"
if [[ $WAITING -eq 1 ]] && vouched "$REPO" tests "$SHA" && grep -q "dropped: code-review $OTHER_SHA" "$WORK/lock.out" && [[ "$(lines)" == 1 ]] && passes tests
then ok "a stamp that lands while the audit waits is kept, the forged line beside it dropped"; else bad "stamp during an audit (audit still waiting: $WAITING), $(lines) line(s): $(head -c 240 "$WORK/lock.out")"; fi
rm -rf "$M" "$WORK/locked" && call "ls"

echo "The ledger's own state is sealed:"
call "just test"
python3 -c 'import json,sys; p=sys.argv[1]; d=json.load(open(p)); d["state"]["grants"].append({"root": "/x", "kind": "tests", "sha": "a", "expires": 9e9}); json.dump(d, open(p, "w"))' "$LEDGER/ledger.json"
call "ls"
if [[ "$OUT" == *"changed outside the hooks"* && "$(lines)" == 0 && ! -e "$M/tests/$SHA" ]] && ! passes tests
then ok "a ledger edited between calls is tampering: every line dropped, markers deleted"; else bad "tamper between calls: ${OUT:0:240}"; fi
call "ls"
[[ -z "$OUT" ]] && ok "...and the next audit finds a resealed, consistent ledger" || bad "after tamper: ${OUT:0:240}"
call "just test"
call "ls" "printf x >> $LEDGER/ledger.json"
if [[ "$OUT" == *"changed outside the hooks"* && "$(lines)" == 0 ]]; then ok "a ledger edited during a call is tampering"; else bad "tamper during a call: ${OUT:0:240}"; fi
rm -rf "$M"
call "just test"
python3 -c 'import json,sys; p=sys.argv[1]; d=json.load(open(p)); del d["seal"]; json.dump(d, open(p, "w"))' "$LEDGER/ledger.json"
call "ls"
if [[ "$OUT" == *"changed outside the hooks"* ]]; then ok "a removed seal is tampering"; else bad "removed seal: ${OUT:0:240}"; fi
rm -rf "$M"

echo "The state is one file, replaced whole:"
call "just test"
pre toolu_open "just test"
if [[ "$(ls "$LEDGER" | sort | tr '\n' ' ')" == "ledger.json lock " ]]; then ok "accepted and pending lines, grants and an open snapshot are one file beside the lock"
else bad "ledger files: $(ls "$LEDGER" | tr '\n' ' ')"; fi
BEFORE="$(inode "$LEDGER/ledger.json")"
call "ls"
if [[ "$(inode "$LEDGER/ledger.json")" != "$BEFORE" ]]; then ok "each write replaces the ledger with a new file by a rename, never in place"; else bad "the ledger was rewritten in place"; fi
printf '{"seal": "", "state": {"accepted": {}, "grants": [], "snapshots": {}}}' > "$LEDGER/ledger.json.tmp"
call "ls"
if [[ -z "$OUT" ]] && vouched "$REPO" tests "$SHA" && passes tests; then ok "a write killed before its rename is not tampering: the last whole state stands"
else bad "interrupted write: ${OUT:0:240}"; fi
post toolu_open "just test"
rm -rf "$M" && call "ls"

echo "No snapshot, no authority:"
(cd "$REPO" && just test >/dev/null 2>&1)
OUT="$(payload toolu_none "just test" 0 | (cd "$REPO" && bash .claude/hooks/marker-audit.sh 2>&1))"
if [[ "$OUT" == *"dropped: tests $SHA"* ]]; then ok "a stamp with no recorded call is dropped"; else bad "no snapshot: ${OUT:0:240}"; fi
rm -rf "$M"

echo "Every worktree, and links, are audited:"
mkdir -p "$WT/.claude/markers/tests" && date +%s > "$WT/.claude/markers/tests/$SHA"
printf 'tests %s %s\n' "$SHA" "$(sha256sum < "$WT/.claude/markers/tests/$SHA" | cut -d' ' -f1)" > "$WT/.claude/markers/manifest"
call "ls"
if [[ ! -e "$WT/.claude/markers/tests/$SHA" && "$OUT" == *"$WT"* ]]; then ok "a forged marker and line in another worktree are undone"; else bad "worktree: ${OUT:0:240}"; fi
mkdir -p "$WORK/fake/tests" && date +%s > "$WORK/fake/tests/$SHA"
printf 'tests %s %s\n' "$SHA" "$(sha256sum < "$WORK/fake/tests/$SHA" | cut -d' ' -f1)" > "$WORK/fake/manifest"
rm -rf "$M" && ln -s "$WORK/fake" "$M"
if ! passes tests; then ok "a linked markers directory carrying its own manifest is refused"; else bad "linked markers dir passed"; fi
call "ls"
if [[ ! -e "$M" && -f "$WORK/fake/tests/$SHA" && "$OUT" == *'"decision": "block"'* ]]; then ok "...and the audit removes the link, not its target"; else bad "linked markers dir: ${OUT:0:240}"; fi
mkdir -p "$WORK/elsewhere" && date +%s > "$WORK/elsewhere/$SHA" && mkdir -p "$M" && ln -s "$WORK/elsewhere" "$M/tests"
call "ls"
if [[ ! -e "$M/tests" && -f "$WORK/elsewhere/$SHA" ]]; then ok "a linked tests directory is removed, not its target"; else bad "linked tests dir: ${OUT:0:240}"; fi
rm -rf "$M"

echo "attest/ is outside the ledger:"
mkdir -p "$M/attest/$SHA" && printf 'x\n' > "$M/attest/$SHA/code.md"
call "ls"
[[ -f "$M/attest/$SHA/code.md" && -z "$OUT" ]] && ok "attestations are untouched" || bad "attest touched: ${OUT:0:240}"

echo "Fails loudly when it cannot run:"
mv "$REPO/bin/lib/marker_manifest.py" "$WORK/mm.py"
OUT="$(payload toolu_x ls 0 | (cd "$REPO" && bash .claude/hooks/marker-audit.sh 2>&1))"; RC=$?
if [[ $RC -eq 2 && "$OUT" == *"could not run"* ]]; then ok "the audit exits 2 with a reason"; else bad "audit without its library: rc=$RC ${OUT:0:200}"; fi
payload toolu_y ls 0 | (cd "$REPO" && bash .claude/hooks/marker-write-gate.sh >/dev/null 2>&1)
[[ $? -eq 2 ]] && ok "the gate blocks when it cannot record the call" || bad "gate passed without the ledger"
mv "$WORK/mm.py" "$REPO/bin/lib/marker_manifest.py"
mv "$REPO/bin/lib/file_lock.py" "$WORK/fl.py"
payload toolu_z ls 0 | (cd "$REPO" && bash .claude/hooks/marker-write-gate.sh >/dev/null 2>&1)
[[ $? -eq 2 ]] && ok "the gate blocks when the lock helper is missing" || bad "gate passed without the lock helper"
mv "$WORK/fl.py" "$REPO/bin/lib/file_lock.py"

echo ""
echo "marker-audit: $PASS passed, $FAIL failed"
(( FAIL == 0 ))
