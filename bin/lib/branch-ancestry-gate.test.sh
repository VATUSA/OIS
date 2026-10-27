#!/bin/bash
# ==============================================================================
# Tests for .claude/hooks/branch-ancestry-gate.sh.
#
#   bash bin/lib/branch-ancestry-gate.test.sh
#
# Feeds the gate Bash payloads and checks allow (exit 0) or block (exit 2).
# Pure shell; macOS bash 3.2 and WSL2. Exits non-zero on any failure.
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GATE="$ROOT/.claude/hooks/branch-ancestry-gate.sh"
# pull.rebase decides whether a bare-option pull rebases, so no global or system config may set it.
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

# expect WANT COMMAND — WANT is allow (exit 0) or block (exit 2)
expect() {
    local want="$1" cmd="$2" rc got
    printf '{"tool_input":{"command":"%s"}}' "$cmd" | bash "$GATE" >/dev/null 2>&1
    rc=$?
    case "$rc" in 0) got=allow ;; 2) got=block ;; *) got="rc=$rc" ;; esac
    if [[ "$got" == "$want" ]]; then ok "$want: $cmd"; else bad "$cmd (want $want, got $got)"; fi
}

expect allow "git merge origin/next"
expect allow "git merge next"
expect allow "git merge --ff-only origin/next"
expect block "git merge feat/312/other-work"
expect block "git rebase feat/312/other-work"
expect allow "git pull origin next"
expect block "git pull origin feat/312/other-work"
# main is the other allowed source: the release back-merge brings a hotfix on main into next.
expect allow "git merge origin/main"
expect allow "git merge main"
expect allow "git pull origin main"
expect allow "git pull --no-rebase origin main"
expect allow "git pull --rebase=false origin main"
# ...but only by a merge: a rebase replays the branch onto its source, and takes next alone.
expect block "git rebase origin/main"
expect block "git rebase main"
expect block "git rebase --onto origin/main origin/next"
expect block "git rebase --onto=main next"
expect block "git rebase --ont main origin/next"
expect block "git pull --rebase origin main"
expect block "git pull --rebase=merges origin main"
expect block "git pull -r origin main"
expect block "git pull --reb origin main"
expect block "git pull --no-rebase --rebase origin main"
# ...and a pull is a rebase when the line itself says so through config.
expect block "git -c pull.rebase=true pull origin main"
expect block "git -c pull.rebase pull origin main"
expect block "git -c Pull.Rebase=merges pull origin main"
expect block "git -c branch.feat/1/x.rebase=true pull origin main"
expect block "git --config-env=pull.rebase=R pull origin main"
expect block "GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=pull.rebase GIT_CONFIG_VALUE_0=true git pull origin main"
expect block "git config pull.rebase true && git pull origin main"
expect block "git -c pull.rebase=false -c pull.rebase=true pull origin main"
expect allow "git -c pull.rebase=false pull origin main"
expect allow "git -c pull.rebase=true pull --no-rebase origin main"
expect allow "git -c pull.rebase=true pull origin next"
expect allow "git rebase origin/next"
expect allow "git rebase --onto origin/next origin/next"
expect allow "git pull --rebase origin next"
expect allow "git pull -r origin next"
# The primary checkout is pulled by naming next (a bare pull there takes its upstream, which the gate resolves).
expect allow "git -C /abs/primary pull --ff-only origin next"
expect block "git -C /abs/primary pull --ff-only"
expect block "git merge origin/feat/312/other-work"
expect block "git merge origin/master"
# merge-base is a read, not a merge
expect allow "git merge-base HEAD origin/next"
expect allow "git merge-base --is-ancestor abc123 origin/next"
expect allow "git log HEAD --not origin/next --merges --oneline"
expect allow "git status"
# A redirected HOME or XDG_CONFIG_HOME brings in a global config the gate never reads (an alias, insteadOf).
expect block "HOME=/tmp/h git pull origin next"
expect block "XDG_CONFIG_HOME=/tmp/x git merge origin/next"
expect block "env HOME=/tmp/h git rebase origin/next"
expect block "export HOME=/tmp/h; git merge origin/next"
expect allow "echo \$HOME; git merge origin/next"

# expect_json WANT NAME COMMAND — the command may hold quotes and newlines
expect_json() {
    local want="$1" rc got
    python3 -c 'import json,sys; print(json.dumps({"tool_input": {"command": sys.argv[1]}}))' "$3" | bash "$GATE" >/dev/null 2>&1
    rc=$?
    case "$rc" in 0) got=allow ;; 2) got=block ;; *) got="rc=$rc" ;; esac
    if [[ "$got" == "$want" ]]; then ok "$want: $2"; else bad "$2 (want $want, got $got)"; fi
}
NL=$'\n'

echo "Parsed, not grepped:"
expect_json block "merge chained after cd"            "cd /repo && git merge feat/312/other-work"
expect_json block "merge after git -C"                "git -C /repo merge feat/312/other-work"
expect_json block "merge inside bash -c"              "bash -c \"git merge feat/312/other-work\""
expect_json block "merge behind timeout"              "timeout 5 git merge feat/312/other-work"
expect_json block "merge inside if"                   "if git merge feat/312/other-work; then :; fi"
expect_json block "merge behind xargs"                "echo x | xargs git merge feat/312/other-work"
expect_json block "merge across a line continuation"  "git merge \\${NL}feat/312/other-work"
expect_json block "second of two merge sources"       "git merge origin/next feat/312/other-work"
expect_json block "rebase --onto= a non-next branch"  "git rebase --onto=feat/312/other-work origin/next"
expect_json allow "rebase --onto=origin/next"         "git rebase --onto=origin/next origin/next"
expect_json block "rebase --ont= abbreviates --onto"  "git rebase --ont=feat/x origin/next"
expect_json block "rebase --onto's value is not the upstream" "git rebase --onto origin/next feat/x"
expect_json block "rebase --o takes a value like --onto" "git rebase --o origin/next feat/x"
expect_json allow "merge heads/next is next"          "git merge heads/next"
expect_json block "merge heads/feat is not"           "git merge heads/feat/x"
expect_json block "merge with -m before the source"   "git merge -m \"sync\" feat/312/other-work"
expect_json block "rebase --onto another branch"      "git rebase --onto feat/312/other-work origin/next"
expect_json block "pull --rebase of another branch"   "git pull --rebase origin feat/312/other-work"
expect_json block "unparseable line"                  "git merge \"feat/312/other-work"
expect_json allow "merge with -m before next"       "git merge -m \"sync\" origin/next"
expect_json allow "merge --abort"                     "git merge --abort"
expect_json allow "rebase --continue"                 "git rebase --continue"
expect_json block "echoed merge is read as a script"  "echo \"git merge feat/312/other-work\""
expect_json allow "commit message naming a merge"    "git commit -m \"docs: git merge feat/312/other-work\""
expect_json block "rebase --exec running a merge"    "git rebase --exec 'git merge feat/312/other-work' origin/next"
expect_json allow "heredoc naming a merge"            "cat > notes.md <<'EOF'${NL}git merge feat/312/other-work${NL}EOF"

# A pull, merge or rebase naming no source integrates the current branch's upstream, and `-` the
# previous branch, so the gate resolves them in the repository it runs in.
WORK="$(mktemp -d "${TMPDIR:-/tmp}/ancestry-gate-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT
REPO="$WORK/repo"
git init -q -b next "$REPO" && git -C "$REPO" config user.email t@t && git -C "$REPO" config user.name t
git -C "$REPO" commit -q --allow-empty -m init && git -C "$REPO" remote add origin "$WORK/nowhere"
git -C "$REPO" update-ref refs/remotes/origin/next HEAD
git -C "$REPO" update-ref refs/remotes/origin/main HEAD
git -C "$REPO" update-ref refs/remotes/origin/feat/312/other-work HEAD
git -C "$REPO" checkout -q -b feat/313/mine
git -C "$REPO" checkout -q -b feat/312/other-work && git -C "$REPO" checkout -q feat/313/mine
# expect_in WANT NAME COMMAND: the gate run from inside $REPO
expect_in() {
    local want="$1" rc got
    python3 -c 'import json,sys; print(json.dumps({"tool_input": {"command": sys.argv[1]}}))' "$3" \
        | (cd "$REPO" && bash "$GATE" >/dev/null 2>&1)
    rc=$?
    case "$rc" in 0) got=allow ;; 2) got=block ;; *) got="rc=$rc" ;; esac
    if [[ "$got" == "$want" ]]; then ok "$want: $2"; else bad "$2 (want $want, got $got)"; fi
}
upstream() { git -C "$REPO" config branch.feat/313/mine.remote origin; git -C "$REPO" config branch.feat/313/mine.merge "refs/heads/$1"; }

echo "No source, upstream origin/next:"
upstream next
expect_in allow "bare pull"                      "git pull"
expect_in allow "pull naming only the remote"   "git pull origin"
expect_in allow "bare merge"                     "git merge"
expect_in allow "bare rebase"                    "git rebase"
expect_in block "bare pull after a cd"           "cd /elsewhere && git pull"
expect_in block "bare merge in another repo"     "git -C /elsewhere merge"
expect_in block "merge of the previous branch"   "git merge -"
expect_in block "rebase onto the previous branch" "git rebase -"

echo "No source, upstream origin/main:"
upstream main
expect_in allow "bare pull"                      "git pull"
expect_in allow "bare merge"                     "git merge"
expect_in block "bare rebase onto main"          "git rebase"
expect_in block "pull --rebase from main"        "git pull --rebase"
expect_in allow "pull origin main, pull.rebase unset" "git pull origin main"
git -C "$REPO" config pull.rebase true
expect_in block "pull origin main, pull.rebase=true" "git pull origin main"
expect_in block "bare pull, pull.rebase=true"    "git pull"
expect_in allow "pull --no-rebase overrides pull.rebase" "git pull --no-rebase origin main"
expect_in allow "pull origin next, pull.rebase=true" "git pull origin next"
git -C "$REPO" config pull.rebase false
expect_in allow "pull origin main, pull.rebase=false" "git pull origin main"
git -C "$REPO" config --unset pull.rebase
git -C "$REPO" config branch.feat/313/mine.rebase true
expect_in block "pull origin main, branch.<current>.rebase=true" "git pull origin main"
git -C "$REPO" config pull.rebase true
git -C "$REPO" config branch.feat/313/mine.rebase false
expect_in allow "branch.<current>.rebase=false wins over pull.rebase=true" "git pull origin main"
git -C "$REPO" config --unset branch.feat/313/mine.rebase; git -C "$REPO" config --unset pull.rebase

echo "No source, upstream another feature branch:"
upstream feat/312/other-work
expect_in block "bare pull"                      "git pull"
expect_in block "pull --rebase"                  "git pull --rebase"
expect_in block "pull naming only the remote"   "git pull origin"
expect_in block "bare merge"                     "git merge"
expect_in block "merge with only options"        "git merge --no-edit --ff"
expect_in block "bare rebase"                    "git rebase"
expect_in block "rebase -i with no upstream"     "git rebase -i"
expect_in allow "rebase --onto next replays only this branch" "git rebase --onto origin/next"
expect_in allow "merge --abort"                  "git merge --abort"
expect_in allow "rebase --continue"              "git rebase --continue"
expect_in allow "named next still passes"        "git pull origin next"

echo "No source, no upstream:"
git -C "$REPO" config --unset branch.feat/313/mine.remote; git -C "$REPO" config --unset branch.feat/313/mine.merge
expect_in block "bare pull"                      "git pull"
expect_in allow "named next still passes"        "git rebase origin/next"

echo "git subtree pull, merge and add integrate the ref they name:"
for options in "-P a" "--prefix=a" "-dP a" "--pre a"; do
    expect_in block "subtree pull $options, a feature branch"  "git subtree pull $options origin feat/312/other-work"
    expect_in block "subtree $options pull, a feature branch"  "git subtree $options pull origin feat/312/other-work"
    expect_in block "subtree merge $options, a feature branch" "git subtree merge $options feat/312/other-work"
    expect_in block "subtree $options add, a feature branch"   "git subtree $options add origin feat/312/other-work"
    expect_in allow "subtree pull $options origin next"        "git subtree pull $options origin next"
    expect_in allow "subtree $options merge origin/next"       "git subtree $options merge origin/next"
    expect_in allow "subtree add $options origin next"         "git subtree add $options origin next"
done
expect_in block "subtree pull naming no ref"            "git subtree pull -P a origin"
expect_in block "subtree pull, a ref the shell builds"  "git subtree pull -P a origin \$B"
expect_in block "subtree merge of two commits"          "git subtree merge -P a origin/next feat/312/other-work"
expect_in block "...the feature branch first"           "git subtree merge -P a feat/312/other-work next"
expect_in block "git-subtree by its own name"           "git-subtree pull -P a origin feat/312/other-work"
expect_in block "subtree pull inside bash -c"           "bash -c 'git subtree -P a pull origin feat/312/other-work'"
expect_in allow "subtree split merges nothing"          "git subtree split -P a -b feat/312/other-work"
expect_in allow "subtree push is the push gate's"       "git subtree push -P a origin feat/312/other-work"
echo "zsh =git is git:"
expect_in block "zsh =git"                              "=git merge feat/312/other-work"
expect_in allow "=git merge origin/next"                "=git merge origin/next"

echo "A refused line says why:"
out="$(python3 -c 'import json,sys; print(json.dumps({"tool_name": "Bash", "tool_input": {"command": sys.argv[1]}}))' "git \$'merge' feat/x" \
    | (cd "$WORK" && bash "$GATE" 2>&1))"
rc=$?
if [[ $rc -eq 2 && "$out" == *"ANSI-C"* ]]; then ok "the parser's reason is in the message"; else bad "reason missing (rc=$rc: ${out:0:200})"; fi

echo ""
echo "branch-ancestry-gate: $PASS passed, $FAIL failed"
(( FAIL == 0 ))
