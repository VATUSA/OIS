#!/bin/bash
# ==============================================================================
# Tests for which commands .claude/hooks/pre-pr-gate.sh gates.
#
#   bash bin/lib/pre-pr-gate.test.sh
#
# Command matching runs the gate from a scratch directory outside any git repo,
# where the review marker cannot be found: a command the gate lets through exits 0
# before the check, and one it gates exits 2. The marker and attestation cases run
# inside a scratch repository. Exits non-zero on any failure.
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GATE="$ROOT/.claude/hooks/pre-pr-gate.sh"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/pre-pr-gate-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

# expect WANT(0 passed through | 2 gated) NAME COMMAND
expect() {
    local got
    python3 -c 'import json,sys; print(json.dumps({"tool_name": "Bash", "tool_input": {"command": sys.argv[1]}}))' "$3" \
        | (cd "$WORK" && GIT_CEILING_DIRECTORIES="$WORK" bash "$GATE" >/dev/null 2>&1)
    got=$?
    if [[ "$got" == "$1" ]]; then ok "$2"; else bad "$2 (want $1, got $got)"; fi
}

NL=$'\n'
PR='gh pr create --base next --title x'

echo "Gated:"
expect 2 "bare"                        "$PR"
expect 2 "chained after cd"            "cd /repo && $PR"
expect 2 "inside bash -c"              "bash -c \"$PR\""
expect 2 "unparseable line"            "echo \"$PR"
expect 2 "the pr new alias"            "gh pr new --base next --title x"
expect 2 "-R before the subcommand"    "gh pr -R VATUSA/OIS create --title x"
expect 2 "-R before the group"          "gh -R VATUSA/OIS pr create --title x"
expect 2 "gh api POST to pulls"        "gh api repos/VATUSA/OIS/pulls -f head=feat/1/x -f base=next -f title=x"
expect 2 "gh api -X POST to pulls"     "gh api -X POST /repos/{owner}/{repo}/pulls --input body.json"
expect 2 "gh api --input alone POSTs to pulls" "gh api repos/VATUSA/OIS/pulls --input body.json"
expect 2 "GraphQL createPullRequest"   "gh api graphql -f query='mutation { createPullRequest(input: {}) { clientMutationId } }'"
expect 0 "gh api reading pulls"        "gh api repos/VATUSA/OIS/pulls"
expect 2 "gh api POST to a full-URL pulls" "gh api -X POST https://api.github.com/repos/VATUSA/OIS/pulls?x=1 -f head=x"
expect 2 "gh api POST to repositories/<id>/pulls" "gh api repositories/123456/pulls -f head=x"
expect 2 "createPullRequest via /graphql, any case" "gh api /graphql -f query='mutation { CreatePullRequest(input: {}) { clientMutationId } }'"

echo "A quoted string naming the command reads as a run (a grep would too):"
expect 2 "quoted in a grep"            "grep -n \"gh pr create\" .claude/hooks/*.sh"
expect 2 "ssh remote command"          "ssh host 'gh pr create --base next'"

echo "Passed through:"
expect 0 "other command"               "gh pr view 2684"
expect 0 "commit message"              "git commit -m \"docs: gh pr create --base next\""
expect 0 "heredoc written to a file"   "cat > body.md <<'EOF'${NL}$PR${NL}EOF"
expect 0 "inside an echoed JSON string" "echo '{\"tool_input\":{\"command\":\"$PR\"}}' | bash .claude/hooks/pre-pr-gate.sh"

echo "Review marker and attestation, in a scratch repository:"
REPO="$WORK/repo"
mkdir -p "$REPO/rules" && cd "$REPO" && git init -q . && git config user.email t@example.com && git config user.name t
printf '## Alpha\n' > rules/a.md && git add . && git commit -qm init && git branch -q -M next
git update-ref refs/remotes/origin/next HEAD && git checkout -qb feat/1/reviewed
printf 'x\n' > app.rs && git add app.rs && git commit -qm app
SHA="$(git rev-parse HEAD)"
git remote add origin git@github.com:VATUSA/OIS.git
printf 'code\trules/a.md\nintent\t-\n' > "$WORK/lenses.tsv"
gate_in_repo() {
    python3 -c 'import json,sys; print(json.dumps({"tool_name": "Bash", "tool_input": {"command": sys.argv[1]}}))' "$PR" \
        | (cd "$REPO" && REVIEW_LENSES_MAP="$WORK/lenses.tsv" bash "$GATE" >/dev/null 2>&1)
}
MARKERS="$REPO/.claude/markers"
stamp_marker() { python3 -I "$ROOT/bin/lib/marker_manifest.py" stamp "$@"; }
mkdir -p "$MARKERS/review-shipping" && : > "$MARKERS/review-shipping/$SHA"
mkdir -p "$MARKERS/attest/$SHA"
printf '| rules/a.md | Alpha | Checked | app.rs:1 |\n' > "$MARKERS/attest/$SHA/code.md"
printf '| AC | 1 | Checked | app.rs:1 |\n' > "$MARKERS/attest/$SHA/intent.md"
gate_in_repo; got=$?; [[ $got == 2 ]] && ok "a hand-written marker with no manifest line is gated" || bad "hand-written marker (want 2, got $got)"
rm -rf "$MARKERS/attest/$SHA" "$MARKERS/review-shipping/$SHA"
stamp_marker "$REPO" review-shipping "$SHA"
gate_in_repo; got=$?; [[ $got == 2 ]] && ok "marker without attestation is gated" || bad "marker without attestation (want 2, got $got)"
mkdir -p "$MARKERS/attest/$SHA"
printf '| rules/a.md | Alpha | Checked | app.rs:1 |\n' > "$MARKERS/attest/$SHA/code.md"
printf '| AC | 1 | Checked | app.rs:1 |\n' > "$MARKERS/attest/$SHA/intent.md"
gate_in_repo; got=$?; [[ $got == 0 ]] && ok "marker with complete attestation passes" || bad "complete attestation (want 0, got $got)"
rm "$MARKERS/attest/$SHA/intent.md"
gate_in_repo; got=$?; [[ $got == 2 ]] && ok "one lens missing is gated" || bad "one lens missing (want 2, got $got)"

# The session cwd is often another checkout (the next repo, or the last worktree entered), so a leading
# `cd DIR && gh pr create` must be checked in DIR.
gate_from_outside() {
    python3 -c 'import json,sys; print(json.dumps({"tool_name": "Bash", "tool_input": {"command": sys.argv[1]}}))' "$1" \
        | (cd "$WORK" && GIT_CEILING_DIRECTORIES="$WORK" REVIEW_LENSES_MAP="$WORK/lenses.tsv" bash "$GATE" >/dev/null 2>&1)
}
printf '| AC | 1 | Checked | app.rs:1 |\n' > "$MARKERS/attest/$SHA/intent.md"
gate_from_outside "cd $REPO && $PR"; got=$?
[[ $got == 0 ]] && ok "cd into a reviewed checkout is checked there" || bad "cd into reviewed checkout (want 0, got $got)"
gate_from_outside "cd \"$REPO\" && $PR"; got=$?
[[ $got == 0 ]] && ok "a double-quoted cd path is checked there" || bad "quoted cd path (want 0, got $got)"
gate_from_outside "cd $REPO && cd $WORK && $PR"; got=$?
[[ $got == 2 ]] && ok "a second cd before the PR falls back to the session cwd" || bad "second cd (want 2, got $got)"
gate_from_outside "cd $REPO && GIT_DIR=$WORK/unreviewed/.git $PR"; got=$?
[[ $got == 2 ]] && ok "GIT_DIR on the PR command is not checked in the cd target" || bad "cd && GIT_DIR (want 2, got $got)"
gate_from_outside "cd $REPO && GIT_WORK_TREE=$WORK/unreviewed $PR"; got=$?
[[ $got == 2 ]] && ok "GIT_WORK_TREE on the PR command is not checked in the cd target" || bad "cd && GIT_WORK_TREE (want 2, got $got)"
gate_from_outside "cd $REPO; $PR"; got=$?
[[ $got == 2 ]] && ok "cd joined by ; is not followed" || bad "cd with ; (want 2, got $got)"
gate_from_outside "cd $WORK/missing && $PR"; got=$?
[[ $got == 2 ]] && ok "cd into a missing directory is gated" || bad "missing cd dir (want 2, got $got)"
UNREVIEWED="$WORK/unreviewed"
git clone -q "$REPO" "$UNREVIEWED" && git -C "$UNREVIEWED" update-ref refs/remotes/origin/next HEAD~1
gate_from_outside "cd $REPO && $PR; cd $UNREVIEWED && $PR"; got=$?
[[ $got == 2 ]] && ok "a second PR from another checkout is not covered by the first cd" || bad "second PR elsewhere (want 2, got $got)"
gate_from_outside "cd $REPO && $PR && pushd $UNREVIEWED && $PR"; got=$?
[[ $got == 2 ]] && ok "a pushd after the PR is not followed" || bad "pushd (want 2, got $got)"
gate_from_outside "cd '$REPO' && $PR"; got=$?
[[ $got == 0 ]] && ok "a single-quoted cd path is checked there" || bad "single-quoted cd path (want 0, got $got)"
gate_from_outside "cd $REPO && $PR --body \"\$(cat <<'EOF'${NL}Summary${NL}EOF${NL})\""; got=$?
[[ $got == 0 ]] && ok "a heredoc PR body keeps the cd followed" || bad "heredoc body (want 0, got $got)"
# bash expands `$NOPE`; a literal directory of that name must not stand in for where gh really runs.
cp -R "$REPO" "$WORK/\$NOPE"
gate_from_outside "cd $WORK/\$NOPE && $PR"; got=$?
[[ $got == 2 ]] && ok "a cd path bash would expand is not followed" || bad "expandable cd path (want 2, got $got)"
rm "$MARKERS/review-shipping/$SHA"
gate_from_outside "cd $REPO && $PR"; got=$?
[[ $got == 2 ]] && ok "cd into an unreviewed checkout is gated" || bad "cd into unreviewed checkout (want 2, got $got)"
stamp_marker "$REPO" review-shipping "$SHA"

# The markers must describe the commit the PR opens on, not whatever the checkout has at HEAD.
gate_cmd() {
    python3 -c 'import json,sys; print(json.dumps({"tool_name": "Bash", "tool_input": {"command": sys.argv[1]}}))' "$1" \
        | (cd "$REPO" && REVIEW_LENSES_MAP="$WORK/lenses.tsv" bash "$GATE" >/dev/null 2>&1)
}
expect_cmd() {
    local got
    gate_cmd "$3"; got=$?
    if [[ "$got" == "$1" ]]; then ok "$2"; else bad "$2 (want $1, got $got)"; fi
}
git branch -q feat/2/unreviewed next && git checkout -q feat/2/unreviewed && printf 'evil\n' > evil.rs \
    && git add evil.rs && git commit -qm evil && git checkout -q feat/1/reviewed
echo "PR head, in a scratch repository:"
expect_cmd 0 "positive control: the reviewed branch itself"    "$PR"
expect_cmd 2 "--head an unreviewed branch"                     "$PR --head feat/2/unreviewed"
expect_cmd 2 "-H an unreviewed branch"                         "$PR -H feat/2/unreviewed"
expect_cmd 2 "-dH bundle naming an unreviewed branch"          "$PR -dH feat/2/unreviewed"
expect_cmd 2 "--head= an unreviewed branch"                    "$PR --head=feat/2/unreviewed"
expect_cmd 2 "-H attached to an unreviewed branch"             "$PR -Hfeat/2/unreviewed"
expect_cmd 2 "--head on a fork"                                "$PR --head someone:feat/1/reviewed"
expect_cmd 2 "--repo another repository"                       "$PR --repo someone/fork"
expect_cmd 2 "GH_REPO on the line"                             "GH_REPO=someone/fork $PR"
expect_cmd 2 "a gh pr create flag the gate does not know"      "$PR -z"
expect_cmd 0 "--repo this checkout's origin"                   "$PR --repo VATUSA/OIS"
expect_cmd 0 "--head owner:branch on this checkout's origin"   "$PR --head VATUSA:feat/1/reviewed"
expect_cmd 0 "-t value holding an H is a title, not a head"    "gh pr create -tHello --base next"
git checkout -q feat/2/unreviewed
expect_cmd 0 "--head the reviewed branch from an unreviewed HEAD" "$PR --head feat/1/reviewed"
git checkout -q feat/1/reviewed
git update-ref refs/remotes/origin/feat/1/reviewed next
expect_cmd 2 "origin's copy of the branch is not the reviewed commit" "$PR"
git update-ref refs/remotes/origin/feat/1/reviewed HEAD
expect_cmd 0 "origin's copy is the reviewed commit"            "$PR"
git update-ref -d refs/remotes/origin/feat/1/reviewed
expect_cmd 2 "PRs from two branches on one line"              "$PR --head feat/1/reviewed && $PR"

# gate_msg COMMAND: run from $REPO, keeping the gate's message; sets got and MSG.
gate_msg() {
    MSG="$(python3 -c 'import json,sys; print(json.dumps({"tool_name": "Bash", "tool_input": {"command": sys.argv[1]}}))' "$1" \
        | (cd "$REPO" && REVIEW_LENSES_MAP="$WORK/lenses.tsv" bash "$GATE" 2>&1 >/dev/null))"
    got=$?
}
echo "The block names the head the PR opens on, so a misread -H cannot pass for a check:"
UNREVIEWED_SHORT="$(git rev-parse --short=8 feat/2/unreviewed)"
for form in "-H feat/2/unreviewed" "-dH feat/2/unreviewed" "-Hfeat/2/unreviewed"; do
    gate_msg "$PR $form"
    if [[ $got == 2 && "$MSG" == *"($UNREVIEWED_SHORT)"* ]]; then ok "$form is checked as feat/2/unreviewed"; else bad "$form (want 2 naming $UNREVIEWED_SHORT, got $got: $MSG)"; fi
done
echo "A failed attestation on a branch with no lens map says to merge next:"
mv "$MARKERS/attest/$SHA/intent.md" "$WORK/intent.md"
gate_msg "$PR"
if [[ $got == 2 && "$MSG" == *"git merge origin/next"* ]]; then ok "no lens map: says to merge origin/next"; else bad "no lens map (want 2 and the merge hint, got $got: $MSG)"; fi
git checkout -q -b feat/5/with-map feat/1/reviewed && printf 'code\trules/a.md\n' > .claude/review-lenses.tsv \
    && git add .claude/review-lenses.tsv && git commit -qm map
MAPPED="$(git rev-parse HEAD)"
stamp_marker "$REPO" review-shipping "$MAPPED"
gate_msg "$PR"
if [[ $got == 2 && "$MSG" == *"did not attest"* && "$MSG" != *"git merge origin/next"* ]]; then ok "with a lens map: no merge hint"; else bad "with a lens map (want 2, the attestation block and no merge hint, got $got: $MSG)"; fi
git checkout -q feat/1/reviewed
mv "$WORK/intent.md" "$MARKERS/attest/$SHA/intent.md"
gate_msg "$PR --head 'feat/1/reviewed;x'"
if [[ $got == 2 && "$MSG" == *"is not a branch name"* ]]; then ok "a --head that is not a branch name is refused as such"; else bad "--head 'feat/1/reviewed;x' (got $got: $MSG)"; fi

# The session cwd here is the REVIEWED checkout; a line moving to an unreviewed one must not borrow its marker.
echo "A line that moves directory before the PR, from a reviewed cwd:"
expect_cmd 0 "positive control: the reviewed checkout itself"  "$PR"
expect_cmd 0 "positive control: cd DIR && gh pr create"        "cd $REPO && $PR"
expect_cmd 2 "cd DIR; gh pr create"                            "cd $UNREVIEWED; $PR"
expect_cmd 2 "cd DIR && git push && gh pr create"              "cd $UNREVIEWED && git push -u origin feat/1/reviewed && $PR"
expect_cmd 2 "pushd DIR && gh pr create"                       "pushd $UNREVIEWED && $PR"
expect_cmd 2 "GIT_DIR on the PR line"                          "GIT_DIR=$UNREVIEWED/.git $PR"
expect_cmd 2 "a subshell cd"                                   "(cd $UNREVIEWED && $PR)"
expect_cmd 0 "a cd only named in the title is data"            "gh pr create --base next --title 'cd x'"
expect_cmd 2 "find -execdir runs the PR in another checkout"   "find $UNREVIEWED/.git -maxdepth 0 -execdir $PR \\;"
expect_cmd 2 "find -okdir runs the PR in another checkout"     "find $UNREVIEWED/.git -maxdepth 0 -okdir $PR \\;"
expect_cmd 2 "env -C/x runs gh in another checkout"            "env -C$UNREVIEWED $PR"
expect_cmd 2 "env --chd=/x runs gh in another checkout"        "env --chd=$UNREVIEWED $PR"
expect_cmd 2 "cd DIR && env -C/x gh pr create"                 "cd $REPO && env -C$UNREVIEWED $PR"
expect_cmd 0 "positive control: env A=b gh pr create"          "env A=b $PR"
expect_cmd 0 "positive control: cd DIR && env A=b gh pr create" "cd $REPO && env A=b $PR"

echo "Guidance-only exemption, judged on the PR head:"
git checkout -q -b chore/3/guidance next && mkdir -p .claude/rules && printf 'x\n' > .claude/rules/x.md \
    && git add .claude/rules/x.md && git commit -qm rules
expect_cmd 0 "a rules-only branch skips the gate"              "$PR"
expect_cmd 2 "a rules-only HEAD does not exempt --head another branch" "$PR --head feat/2/unreviewed"
git checkout -q -b chore/4/lenses next && printf 'x\n' > .claude/review-lenses.tsv \
    && git add .claude/review-lenses.tsv && git commit -qm lenses
expect_cmd 2 "a branch editing the lens map is gated"          "$PR"
git checkout -q feat/1/reviewed

# A copy of the hook tree without the attestation library must refuse, not pass.
COPY="$WORK/copy"
mkdir -p "$COPY/.claude/hooks" "$COPY/bin/lib"
cp -R "$ROOT/.claude/hooks/lib" "$COPY/.claude/hooks/lib"
cp "$GATE" "$COPY/.claude/hooks/pre-pr-gate.sh"
cp "$ROOT/bin/check-marker.sh" "$COPY/bin/check-marker.sh"
cp "$ROOT/bin/lib/claude-only-change.sh" "$COPY/bin/lib/claude-only-change.sh"
python3 -c 'import json,sys; print(json.dumps({"tool_name": "Bash", "tool_input": {"command": sys.argv[1]}}))' "$PR" \
    | (cd "$REPO" && bash "$COPY/.claude/hooks/pre-pr-gate.sh" >/dev/null 2>"$WORK/err")
got=$?
if [[ $got == 2 ]] && grep -q 'cannot load bin/lib/marker_manifest.py' "$WORK/err"; then ok "a missing manifest library is gated"; else bad "missing manifest library (want 2 and the guard's message, got $got: $(cat "$WORK/err"))"; fi
cp "$ROOT/bin/lib/marker_manifest.py" "$COPY/bin/lib/marker_manifest.py"
python3 -c 'import json,sys; print(json.dumps({"tool_name": "Bash", "tool_input": {"command": sys.argv[1]}}))' "$PR" \
    | (cd "$REPO" && bash "$COPY/.claude/hooks/pre-pr-gate.sh" >/dev/null 2>"$WORK/err")
got=$?
if [[ $got == 2 ]] && grep -q 'cannot locate bin/lib/review-attestation.sh' "$WORK/err"; then ok "a missing attestation library is gated"; else bad "missing attestation library (want 2 and the guard's message, got $got)"; fi
# ...and without the marker checker.
cp "$ROOT/bin/lib/review-attestation.sh" "$COPY/bin/lib/review-attestation.sh"
rm "$COPY/bin/check-marker.sh"
python3 -c 'import json,sys; print(json.dumps({"tool_name": "Bash", "tool_input": {"command": sys.argv[1]}}))' "$PR" \
    | (cd "$REPO" && REVIEW_LENSES_MAP="$WORK/lenses.tsv" bash "$COPY/.claude/hooks/pre-pr-gate.sh" >/dev/null 2>"$WORK/err")
got=$?
if [[ $got == 2 ]] && grep -q 'cannot locate bin/check-marker.sh' "$WORK/err"; then ok "a missing check-marker.sh is gated"; else bad "missing check-marker.sh (want 2 and the guard's message, got $got)"; fi
cd "$ROOT" || exit 1

echo "A refused line says why:"
out="$(python3 -c 'import json,sys; print(json.dumps({"tool_name": "Bash", "tool_input": {"command": sys.argv[1]}}))' "gh \$'pr' create" \
    | (cd "$WORK" && bash "$GATE" 2>&1))"
rc=$?
if [[ $rc -eq 2 && "$out" == *"ANSI-C"* ]]; then ok "the parser's reason is in the message"; else bad "reason missing (rc=$rc: ${out:0:200})"; fi

echo ""
echo "pre-pr-gate: $PASS passed, $FAIL failed"
(( FAIL == 0 ))
