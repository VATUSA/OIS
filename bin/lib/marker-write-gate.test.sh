#!/bin/bash
# ==============================================================================
# Tests for .claude/hooks/marker-write-gate.sh, the cheap first line of the marker
# guard: it refuses the writes it can read literally before a tool runs.
#
#   bash bin/lib/marker-write-gate.test.sh
#
# Writing a marker, the manifest or the marker ledger by a path the line spells out
# (an edit tool, a redirect, cp, a one-liner) must block, while reading them, the two
# scripts that stamp them and the lenses' attest/ files pass. Spellings the shell
# builds and trees copied in are the PostToolUse audit's (marker-audit.test.sh).
# Exits non-zero on any failure.
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GATE="$ROOT/.claude/hooks/marker-write-gate.sh"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/marker-write-gate-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

REPO="$WORK/repo"
mkdir -p "$REPO/bin" "$REPO/app" "$REPO/d" "$REPO/.claude/markers/tests" "$REPO/.claude/markers/attest"
git -C "$REPO" init -q
printf 'x\n' > "$REPO/f"
printf '#!/bin/bash\n' > "$REPO/bin/run-tests.sh"
printf '#!/bin/bash\n' > "$REPO/bin/review-marker.sh"
SHA=0123456789abcdef0123456789abcdef01234567
LEDGER="$REPO/.git/ois-marker-ledger"

# gate WANT NAME TOOL FIELD VALUE [CWD] — WANT is allow (exit 0) or block (exit 2)
gate() {
    local rc got cwd="${6:-$REPO}"
    python3 -c 'import json,sys; print(json.dumps({"tool_name": sys.argv[1], "tool_input": {sys.argv[2]: sys.argv[3]}, "cwd": sys.argv[4]}))' \
        "$3" "$4" "$5" "$cwd" | (cd "$cwd" && bash "$GATE" >/dev/null 2>&1)
    rc=$?
    case "$rc" in 0) got=allow ;; 2) got=block ;; *) got="rc=$rc" ;; esac
    if [[ "$got" == "$1" ]]; then ok "$1: $2"; else bad "$2 (want $1, got $got)"; fi
}
bash_gate() { gate "$1" "$2" Bash command "$2" "${3:-$REPO}"; }

echo "Edit tools writing a marker, the manifest or the ledger are blocked:"
gate block "Write tests marker"              Write file_path "$REPO/.claude/markers/tests/$SHA"
gate block "Write review-shipping marker"    Write file_path "$REPO/.claude/markers/review-shipping/$SHA"
gate block "Write code-review marker"        Write file_path "$REPO/.claude/markers/code-review/$SHA"
gate block "Edit, relative path"             Edit file_path ".claude/markers/tests/$SHA"
gate block "MultiEdit"                       MultiEdit file_path "$REPO/.claude/markers/tests/$SHA"
gate block "NotebookEdit notebook_path"      NotebookEdit notebook_path "$REPO/.claude/markers/tests/$SHA"
gate block "another checkout's marker"       Write file_path "/elsewhere/wt/.claude/markers/review-shipping/$SHA"
gate block "through attest/.."               Write file_path "$REPO/.claude/markers/attest/../tests/$SHA"
ln -s "$REPO/.claude/markers/tests" "$WORK/link"
gate block "through a symlinked directory"   Write file_path "$WORK/link/$SHA"
gate block "another letter case"             Write file_path "$REPO/.claude/Markers/Tests/$SHA"
gate block "Write the manifest"              Write file_path "$REPO/.claude/markers/manifest"
gate block "Edit the manifest, another case" Edit file_path ".claude/Markers/MANIFEST"
gate block "Write the ledger"                Write file_path "$LEDGER/ledger.json"
gate block "Write the ledger seal"           Write file_path "/any/repo/.git/ois-marker-ledger/seal"
echo "...but not an attestation or any other file:"
gate allow "Write an attestation"            Write file_path "$REPO/.claude/markers/attest/$SHA/code.md"
gate allow "a file named like the manifest"  Write file_path "$REPO/.claude/markers/attest/$SHA/manifest.md"
gate allow "Write app code"                  Write file_path "$REPO/backend/src/main.rs"
gate allow "a directory merely named alike"  Write file_path "$REPO/.claude/markers/testsuite/$SHA"
gate allow "a ledger-like name elsewhere"    Write file_path "$REPO/docs/ois-marker-ledgers.md"
gate allow "Read is not gated"               Read file_path "$REPO/.claude/markers/tests/$SHA"

echo "Bash writing a literal marker, manifest or ledger path is blocked:"
bash_gate block "touch .claude/markers/tests/$SHA"
bash_gate block "date +%s > .claude/markers/tests/$SHA"
bash_gate block "date +%s >.claude/markers/tests/$SHA"
bash_gate block "cat /dev/null > .claude/markers/review-shipping/$SHA"
bash_gate block "echo x >> .claude/markers/code-review/$SHA"
bash_gate block "echo tests $SHA x >> .claude/markers/manifest"
bash_gate block "echo {} > $LEDGER/ledger.json"
bash_gate block "rm -f .git/ois-marker-ledger/seal"
bash_gate block "ls >& .claude/markers/tests/$SHA"
bash_gate block "ls >| .claude/markers/tests/$SHA"
bash_gate block "ls &> .claude/markers/tests/$SHA"
bash_gate block "ls &>> .claude/markers/tests/$SHA"
bash_gate block "ls <> .claude/markers/tests/$SHA"
bash_gate block "echo \`touch .claude/markers/tests/$SHA\`"
bash_gate block "echo \$(touch .claude/markers/tests/$SHA)"
bash_gate block "find .claude/markers -execdir touch x \\;"
bash_gate block "find .claude/markers -exec touch {} \\;"
bash_gate block "find .claude/markers -ok touch {} \\;"
bash_gate block "find .claude/markers -okdir touch x \\;"
bash_gate block "find .claude/markers -name x -delete"
bash_gate block "find . -fprint .claude/markers/tests/$SHA"
bash_gate block "find . -fprint0 .claude/markers/tests/$SHA"
bash_gate block "find . -fprintf .claude/markers/tests/$SHA %p"
bash_gate block "find . -fls .claude/markers/tests/$SHA"
bash_gate block "cargo run -- touch .claude/markers/tests/$SHA"
bash_gate block "just test; touch .claude/markers/tests/$SHA"
bash_gate block "touch .claude/Markers/tests/$SHA"
bash_gate block "echo x | tee .claude/markers/tests/$SHA"
bash_gate block "cp /dev/null .claude/markers/tests/$SHA"
bash_gate block "cp /dev/null .claude/markers/\"tests\"/$SHA"
bash_gate block "mv /tmp/x .claude/markers/tests/$SHA"
bash_gate block "mv /tmp/mk/tests .claude/markers"
bash_gate block "ln -s /dev/null .claude/markers/tests/$SHA"
bash_gate block "install -D /dev/null .claude/markers/tests/$SHA"
bash_gate block "dd if=/dev/null of=.claude/markers/tests/$SHA"
bash_gate block "python3 -c \"open('.claude/markers/tests/$SHA', 'w')\""
bash_gate block "perl -e 'open(F, \">.claude/markers/tests/$SHA\")'"
bash_gate block "bash -c 'touch .claude/markers/tests/$SHA'"
bash_gate block "mkdir -p .claude/markers/tests && touch .claude/markers/tests/$SHA"
bash_gate block "cd .claude/markers && touch tests/$SHA"
mkdir -p "$WORK/elsewhere/bin" && printf '#!/bin/bash\n' > "$WORK/elsewhere/bin/run-tests.sh"
bash_gate block "bash $WORK/elsewhere/bin/run-tests.sh && ls .claude/markers/tests"
bash_gate block "cd .claude/markers/tests && bash bin/run-tests.sh"
echo "...and so is marker_manifest.py stamp run by hand, which names no marker path:"
bash_gate block "python3 -I bin/lib/marker_manifest.py stamp $REPO tests $SHA"
bash_gate block "python3 $REPO/bin/lib/marker_manifest.py stamp . review-shipping $SHA"
bash_gate block "ls; cd $REPO && python3 -I bin/lib/marker_manifest.py stamp . code-review $SHA"
bash_gate allow "python3 -I bin/lib/marker_manifest.py vouched $REPO tests $SHA"
bash_gate allow "git commit --allow-empty -m \"marker_manifest.py stamp is the scripts' call\""
bash_gate allow "grep -n stamp bin/lib/marker_manifest.py"
NL=$'\n'
bash_gate block "bash <<'EOF'${NL}touch .claude/markers/tests/$SHA${NL}EOF"
bash_gate block "cat > .claude/markers/tests/$SHA <<'EOF'${NL}x${NL}EOF"
MSG="$(python3 -c 'import json,sys; print(json.dumps({"tool_name": "Bash", "tool_input": {"command": sys.argv[1]}, "cwd": sys.argv[2]}))' \
    "touch .claude/markers/tests/$SHA" "$REPO" | (cd "$REPO" && bash "$GATE" 2>&1))"
[[ "$MSG" == *"names a marker, the manifest or the marker ledger"* ]] && ok "the block says what the line names" || bad "message: $MSG"

echo "Reading markers, the stamping scripts and attestations pass:"
bash_gate allow "ls .claude/markers/tests/"
bash_gate allow "ls -la .claude/markers/review-shipping 2>/dev/null"
bash_gate allow "cat .claude/markers/tests/$SHA"
bash_gate allow "cat .claude/markers/manifest"
bash_gate allow "test -f .claude/markers/tests/$SHA && echo yes"
bash_gate allow "[ -f .claude/markers/review-shipping/$SHA ] || echo missing"
bash_gate allow "find .claude/markers -name '$SHA'"
bash_gate allow "bash bin/run-tests.sh"
bash_gate allow "bash bin/review-marker.sh shipping $SHA && ls .claude/markers/review-shipping/"
bash_gate allow "bin/run-tests.sh; ls .claude/markers/tests | wc -l"
bash_gate allow "just test && ls .claude/markers/tests"
bash_gate allow "mkdir -p .claude/markers/attest/$SHA"
bash_gate allow "cd $REPO; git status; ls .claude/markers/attest/"
bash_gate allow "cd $REPO && echo x > .claude/markers/attest/$SHA/code.md"
bash_gate allow "cd .claude/markers/attest && touch $SHA.md"
bash_gate allow "git status"
bash_gate allow "git commit --allow-empty -m \"stop writing tests markers by hand\""
bash_gate allow "git commit --allow-empty -m \"\$(cat <<'EOF'${NL}Stop hand-written .claude/markers/tests files${NL}EOF${NL})\""
bash_gate allow "cat > notes.md <<'EOF'${NL}.claude/markers/tests/$SHA${NL}EOF"
bash_gate allow "cp a.txt b.txt"
bash_gate allow "cp -r /tmp/mk/. docs/"
echo "...and spellings the shell builds pass here, for the audit to judge the outcome:"
bash_gate allow "touch .claude/\$M/tests/$SHA"
bash_gate allow "cp -r /tmp/mk/. ."
bash_gate allow "cd $WORK && touch link/$SHA"

echo "Fails closed:"
printf 'not json' | (cd "$REPO" && bash "$GATE" >/dev/null 2>&1)
[[ $? -eq 2 ]] && ok "block an unparseable payload" || bad "unparseable payload was not blocked"
python3 -c 'import json; print(json.dumps({"tool_name": "Bash", "tool_input": {"command": "ls"}}))' \
    | (cd "$REPO" && PATH=/nonexistent /bin/bash "$GATE" >/dev/null 2>&1)
[[ $? -eq 2 ]] && ok "block when python3 is missing" || bad "python3 missing was not blocked"
bash_gate block "echo \"an unclosed quote .claude/markers/tests/$SHA"
bash_gate block "cat <<'EOF'${NL}never closed .claude/markers/tests/$SHA"
# The payload's cwd, not the hook process's, is where relative words resolve.
python3 -c 'import json,sys; print(json.dumps({"tool_name": "Bash", "tool_input": {"command": sys.argv[1]}, "cwd": sys.argv[2]}))' \
    "touch link/$SHA" "$WORK" | (cd "$REPO" && bash "$GATE" >/dev/null 2>&1)
[[ $? -eq 2 ]] && ok "block: a relative word resolved against the payload cwd" || bad "payload cwd was not used"
python3 -c 'import json,sys; print(json.dumps({"tool_name": "Bash", "tool_input": {"command": sys.argv[1]}, "cwd": sys.argv[2]}))' \
    "touch other/$SHA" "$WORK" | (cd "$REPO" && bash "$GATE" >/dev/null 2>&1)
[[ $? -eq 0 ]] && ok "allow: positive control, an unlinked relative word" || bad "unlinked relative word was blocked"

echo "What a call may stamp (lib/marker_write.py stampers):"
git -C "$REPO" -c user.name=t -c user.email=t@e commit -q --allow-empty -m init
HEAD_SHA="$(git -C "$REPO" rev-parse HEAD)"
stampers() { (cd "$REPO" && python3 -I -c 'import sys; sys.path.insert(0, sys.argv[1]); import marker_write as m
print(" ".join("%s:%s" % (kind, sha[:7]) for _root, kind, sha in m.stampers(sys.argv[2], sys.argv[3])))' \
    "$ROOT/.claude/hooks/lib" "$1" "$REPO"); }
expect_stamps() { local got; got="$(stampers "$2" 2>/dev/null)" || got="crashed"; [[ "$got" == "$1" ]] && ok "stamps '$1': $2" || bad "$2 (want '$1', got '$got')"; }
expect_stamps "tests:${HEAD_SHA:0:7}" "just test"
expect_stamps "tests:${HEAD_SHA:0:7}" "just test 2>&1 | tail -20"
expect_stamps "tests:${HEAD_SHA:0:7}" "just test > /tmp/out.txt 2>&1"
expect_stamps "tests:${HEAD_SHA:0:7}" "bash bin/run-tests.sh"
expect_stamps "code-review:${SHA:0:7}" "bash bin/review-marker.sh code-review $SHA"
expect_stamps "code-review:${SHA:0:7} review-shipping:${SHA:0:7}" "bash bin/review-marker.sh shipping $SHA"
expect_stamps "" "bash bin/run-tests.sh --filter=X"
expect_stamps "" "bash bin/review-marker.sh shipping ${SHA:0:12}"
expect_stamps "" "bash $WORK/elsewhere/bin/run-tests.sh"
expect_stamps "" "just test; touch x"
echo "...and exactly \`cd ABSDIR && just test\`, for the checkout at ABSDIR:"
expect_stamps "tests:${HEAD_SHA:0:7}" "cd $REPO && just test"
expect_stamps "tests:${HEAD_SHA:0:7}" "cd $REPO && just test > /tmp/out.txt 2>&1"
OTHER="$WORK/other-checkout"
git init -q "$OTHER" && git -C "$OTHER" -c user.name=t -c user.email=t@e commit -q --allow-empty -m other
got="$(cd "$REPO" && python3 -I -c 'import sys; sys.path.insert(0, sys.argv[1]); import marker_write as m
print(" ".join("%s %s %s" % found for found in m.stampers(sys.argv[2], sys.argv[3])))' \
    "$ROOT/.claude/hooks/lib" "cd $OTHER && just test" "$REPO")"
if [[ "$got" == "$(cd "$OTHER" && pwd -P) tests $(git -C "$OTHER" rev-parse HEAD)" ]]; then ok "the stamp is for ABSDIR's checkout and HEAD, not the cwd's"
else bad "cd to another checkout (got '$got')"; fi
expect_stamps "" "cd . && just test"
expect_stamps "" "cd \$PWD && just test"
expect_stamps "" "cd ~ && just test"
expect_stamps "" "cd $REPO/bin && just test"
expect_stamps "" "cd -P $REPO && just test"
expect_stamps "" "cd $WORK && just test"
expect_stamps "" "cd $REPO; just test"
expect_stamps "" "cd $REPO || just test"
expect_stamps "" "cd $REPO && just test | tail -5"
expect_stamps "" "cd $REPO && just test && touch x"
expect_stamps "" "touch x && cd $REPO && just test"
expect_stamps "" "cd $REPO && X=1 just test"
expect_stamps "" "cd $REPO && env just test"
expect_stamps "" "cd $REPO && timeout 600 just test"
expect_stamps "" "cd $REPO && bash bin/run-tests.sh"
expect_stamps "" "cd $REPO && just test rust -p ois-backend"
expect_stamps "" "pushd $REPO && just test"
expect_stamps "" "timeout 600 just test"
echo "...and exactly \`cd ABSDIR && bash bin/review-marker.sh KIND SHA\`, with ABSDIR's own script:"
expect_stamps "code-review:${SHA:0:7} review-shipping:${SHA:0:7}" "cd $REPO && bash bin/review-marker.sh shipping $SHA"
expect_stamps "code-review:${SHA:0:7}" "cd $REPO && sh bin/review-marker.sh code-review $SHA"
expect_stamps "code-review:${SHA:0:7}" "cd $REPO && bin/review-marker.sh code-review $SHA"
expect_stamps "" "cd $REPO && bash $WORK/elsewhere/bin/review-marker.sh shipping $SHA"
expect_stamps "" "bash $WORK/elsewhere/bin/review-marker.sh shipping $SHA"
expect_stamps "" "cd $REPO && bash bin/review-marker.sh bogus $SHA"
expect_stamps "" "cd $REPO && bash bin/review-marker.sh shipping ${SHA:0:12}"
expect_stamps "" "cd $REPO/bin && bash $REPO/bin/review-marker.sh code-review $SHA"
expect_stamps "code-review:${SHA:0:7}" "cd $REPO && bash $REPO/bin/review-marker.sh code-review $SHA"
expect_stamps "code-review:${SHA:0:7}" "cd $REPO && bash bin/review-marker.sh code-review $SHA > /tmp/out.txt 2>&1"
expect_stamps "" "cd $REPO && bash bin/review-marker.sh code-review $SHA > .claude/out.txt"
expect_stamps "" "cd $REPO && bash bin/review-marker.sh code-review $SHA | tail -5"
expect_stamps "" "cd $REPO && bash"
expect_stamps "" "X=1 just test"
expect_stamps "" "just test \$(touch x)"
expect_stamps "" "echo \`touch x\` && just test"
expect_stamps "" "cat <(touch x); just test"
expect_stamps "" "cat < <(touch x); just test"
expect_stamps "" "bash <<'EOF'${NL}just test${NL}EOF"
expect_stamps "" "find . -delete; just test"
expect_stamps "" "just test-rust"
echo "...beside reads that only print, with redirects to literal paths outside .claude/ and the git directory:"
expect_stamps "tests:${HEAD_SHA:0:7}" "just test > /tmp/out.txt 2>&1; echo \"exit=\$?\"; grep Tests: /tmp/out.txt"
expect_stamps "tests:${HEAD_SHA:0:7}" "just test 2>&1 | tail -5; echo \${PIPESTATUS[0]}"
expect_stamps "tests:${HEAD_SHA:0:7}" "just test > out.txt"
expect_stamps "tests:${HEAD_SHA:0:7}" "just test >> /tmp/out.txt 2> /dev/null"
expect_stamps "tests:${HEAD_SHA:0:7}" "just test; ls .claude/markers/tests/*; cat .claude/markers/manifest"
RIDE="printf '1\\n' > .claude/\${X:-markers}/review-shipping/$SHA; printf 'review-shipping $SHA 0\\n' >> .claude/\${X:-markers}/manifest"
expect_stamps "" "$RIDE; bash bin/review-marker.sh shipping $SHA"
expect_stamps "" "$RIDE; just test"
ln -s "$REPO/.claude/markers/manifest" "$WORK/into-claude"
mkdir -p "$WORK/plain"
for target in ".claude/x.log" ".claude/markers/manifest" "$REPO/.claude/markers/manifest" ".git/ois-marker-ledger/ledger.json" \
    "$LEDGER/ledger.json" ".git/x" "$WORK/into-claude" '$HOME/x.log' '"$OUT"' '~/x.log' '.claud?/markers/manifest' \
    '.claude/{markers,x}/manifest' '.claud*/markers/manifest' "\$'.claude/markers/manifest'" '=x' '! /tmp/x.log' '!/tmp/x.log' '.Claude/markers/manifest' 'a/../.claude/x' '`pwd`/x'; do
    expect_stamps "" "just test > $target"
    expect_stamps "" "echo x >> $target; just test"
    expect_stamps "" "just test 2>&1 | tail -5 &> $target"
done
expect_stamps "" "just test >& .claude/x.log"
expect_stamps "" "just test >| .claude/x.log"
expect_stamps "" "just test <> .claude/x.log"
expect_stamps "" "just test > /tmp/a.log > .claude/x.log"
expect_stamps "" "just test >"
expect_stamps "" "just test > >(cat)"
expect_stamps "" "cd $REPO && just test > .claude/x.log"
expect_stamps "" "cd $REPO && just test > \$X"
expect_stamps "tests:${HEAD_SHA:0:7}" "just test > $WORK/plain/out.log"
expect_stamps "" "just test > $OTHER/.git/x.log"
expect_stamps "" "just test > /elsewhere/ois-marker-ledger/x.log"
git init -q --separate-git-dir "$WORK/sep-git" "$WORK/sep" && git -C "$WORK/sep" -c user.name=t -c user.email=t@e commit -q --allow-empty -m sep
count_in_sep() { (cd "$WORK/sep" && python3 -I -c 'import sys; sys.path.insert(0, sys.argv[1]); import marker_write as m
print(len(m.stampers(sys.argv[2], sys.argv[3])))' "$ROOT/.claude/hooks/lib" "$1" "$WORK/sep"); }
[[ "$(count_in_sep "just test > $WORK/sep-git/x.log")" == 0 ]] && ok "stamps '': a redirect into a git directory kept apart from its checkout" || bad "a redirect into a separate git directory stamps"
[[ "$(count_in_sep "just test > $WORK/plain/out.log")" == 1 ]] && ok "positive control: that checkout stamps with a redirect elsewhere" || bad "the separate-git-dir checkout does not stamp"
echo "...and inside an issue worktree, which lives under the main checkout's .claude/worktrees:"
WT="$REPO/.claude/worktrees/wt"
git -C "$REPO" worktree add -q -b wt-branch "$WT"
count_in_wt() { (cd "$WT" && python3 -I -c 'import sys; sys.path.insert(0, sys.argv[1]); import marker_write as m
print(len(m.stampers(sys.argv[2], sys.argv[3])))' "$ROOT/.claude/hooks/lib" "$1" "$WT"); }
for line in "just test > target/suite.log 2>&1" "just test > $WT/suite.log" "cd $WT && just test > suite.log"; do
    [[ "$(count_in_wt "$line")" == 1 ]] && ok "stamps in a worktree: $line" || bad "does not stamp in a worktree: $line"
done
for line in "just test > .claude/x.log" "just test > $REPO/.claude/markers/manifest" "just test > $REPO/.claude/x.log"; do
    [[ "$(count_in_wt "$line")" == 0 ]] && ok "stamps '' in a worktree: $line" || bad "stamps in a worktree: $line"
done
expect_stamps "" "cd $WT && just test > ../../markers/x.log"
git -C "$REPO" worktree remove --force "$WT" && git -C "$REPO" branch -q -D wt-branch
expect_stamps "" "echo \$X; just test"
expect_stamps "" "echo \"\${X:=y}\"; just test"
expect_stamps "" "printf %s \"\$(date)\"; just test"
expect_stamps "" "./ls; just test"
expect_stamps "" "/tmp/bin/cat x; just test"
expect_stamps "" "=ls; just test"
expect_stamps "" "just test | tee /tmp/out.log"
expect_stamps "" "just test; rm -f out.txt"
expect_stamps "" "ls *(.); just test"

echo "The scripts that stamp markers still do (their own suites):"
for suite in run-tests review-marker; do
    summary="$(bash "$ROOT/bin/lib/$suite.test.sh" 2>&1 | tail -1)"
    if [[ "$summary" == *" 0 failed"* ]]; then ok "$suite: $summary"; else bad "$suite: $summary"; fi
done

echo ""
echo "marker-write-gate: $PASS passed, $FAIL failed"
(( FAIL == 0 ))
