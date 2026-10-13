#!/bin/bash
# ==============================================================================
# Tests for .claude/hooks/master-protection-gate.sh.
#
#   bash bin/lib/master-protection-gate.test.sh
#
# Builds a throwaway repo and feeds the gate Bash payloads from a feature branch
# and from next. Every way of writing next must block; data that only
# mentions a command must not. Exits non-zero on any failure.
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GATE="$ROOT/.claude/hooks/master-protection-gate.sh"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/next-gate-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR
export GIT_AUTHOR_NAME=T GIT_AUTHOR_EMAIL=t@e.com GIT_COMMITTER_NAME=T GIT_COMMITTER_EMAIL=t@e.com
export GIT_CONFIG_NOSYSTEM=1 HOME="$WORK"

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

REPO="$WORK/repo"
git init -q -b next "$REPO"
git -C "$REPO" -c core.hooksPath=/dev/null commit -q --allow-empty -m init

# says BRANCH COMMAND TEXT — the gate blocks and its message carries TEXT (the parser's reason)
says() {
    local out
    git -C "$REPO" checkout -q -B "$1"
    out="$(cd "$REPO" && python3 -c 'import json,sys; print(json.dumps({"tool_input": {"command": sys.argv[1]}}))' "$2" \
        | bash "$GATE" 2>&1)"
    if [[ $? -eq 2 && "$out" == *"$3"* ]]; then ok "says '$3': $2"; else bad "says '$3': $2 (got: ${out:0:160})"; fi
}

# expect WANT BRANCH COMMAND — WANT is allow (exit 0) or block (exit 2)
expect() {
    local want="$1" branch="$2" rc got
    git -C "$REPO" checkout -q -B "$branch"
    (cd "$REPO" && python3 -c 'import json,sys; print(json.dumps({"tool_input": {"command": sys.argv[1]}}))' "$3" \
        | bash "$GATE" >/dev/null 2>&1)
    rc=$?
    case "$rc" in 0) got=allow ;; 2) got=block ;; *) got="rc=$rc" ;; esac
    if [[ "$got" == "$want" ]]; then ok "$want on $branch: $3"; else bad "on $branch: $3 (want $want, got $got)"; fi
}

F=feat/312/season-delta-check
NL=$'\n'

echo "Writing next is blocked from any branch:"
expect block "$F" "git push origin next"
expect block "$F" "git push origin HEAD:next"
expect block "$F" "git push origin +next"
expect block "$F" "git push origin refs/heads/next"
expect block "$F" "git push origin :next"
expect block "$F" "git push --all origin"
expect block "$F" "git push --mirror origin"
expect block "$F" "git -C . push origin next"
expect block "$F" "cd . && git push origin next"
expect block "$F" "bash -c \"git push origin next\""
expect block "$F" "git push --repo origin origin next"
expect block "$F" "git push \"origin next"
expect block "$F" "git push --repo origin next"
expect block "$F" "timeout 5 git push origin next"
expect block "$F" "nice -n 5 git push origin next"
expect block "$F" "if git push origin next; then :; fi"
expect block "$F" "echo x | xargs git push origin next"
expect block "$F" "git push -u origin \\${NL}next"
expect block "$F" "git --attr-source HEAD push origin next"
expect block next "git push origin {fd}>/dev/null"
expect block "$F" "echo \"a\\\\${NL}\"; git push origin next; echo \"b\\\\${NL}\""
expect block "$F" "bash --rcfile -xc -c 'git push origin next'"
expect block "$F" "# it's${NL}git push origin next # '"
expect block "$F" "git -c alias.p='!git push origin next' p"
expect block "$F" "flock /tmp/l -c 'git push origin next'"
expect block "$F" "bash <<< 'git push origin next'"
expect block "$F" "git config alias.p '!git push origin next' && git p"
expect block "$F" "git -c include.path=/tmp/c p"
expect block "$F" "GIT_CONFIG_GLOBAL=/tmp/c git p"
expect block "$F" "cat <<'W' | bash
git push origin next
W"
expect block "$F" "cat <<'W' |& bash
git push origin next
W"
expect block "$F" "cat <<< 'git push origin next' | bash"
expect block "$F" "cat <<< 'git push origin next' | if true; then bash; fi"
expect block "$F" "flock /tmp/l bash <<< 'git push origin next'"
expect block "$F" "cat <<'W' | ( bash )
git push origin next
W"
expect block "$F" "cat <<'W' | (bash)
git push origin next
W"
expect block "$F" "g\$'it' push origin next"
expect block "$F" "cat <<'W' | bash; echo done
git push origin next
W"
expect block "$F" "git -C /tmp/r config alias.ship '!git push origin next' && git -C /tmp/r ship"
expect block "$F" "GIT_CONFIG_KEY_0=alias.p GIT_CONFIG_VALUE_0='!git push origin next' git p"
expect block "$F" "echo \`echo #\`; git push origin next"
expect block "$F" "echo \$'\\' #'; git push origin next"
expect block next "git push --repo origin"

expect block "$F" "git push --branches origin"
expect block "$F" "git push origin HEAD:heads/next"
expect block "$F" "git push origin 'refs/heads/*:refs/heads/*'"
expect block "$F" "git push origin :"
expect block "$F" "git push origin ''"
expect block "$F" "git push origin HEAD:\$B"
expect block "$F" "git push origin \$B"
expect block "$F" "git push origin HEAD:\$(echo next)"
expect block "$F" "git push origin HEAD:\`echo next\`"
expect block "$F" "git push origin HEAD:{next,x}"
expect block "$F" "git push origin HEAD:mai?"
expect block "$F" "git push origin 'HEAD:ma[i]n'"
expect block "$F" "git push origin HEAD:~next"
expect block "$F" "git push origin HEAD:"
expect block "$F" "git -c remote.origin.push=HEAD:refs/heads/next push"
expect block "$F" "git -c push.default=upstream push"
expect block "$F" "GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=remote.origin.push GIT_CONFIG_VALUE_0=HEAD:refs/heads/next git push"
expect block "$F" "git config remote.origin.push HEAD:refs/heads/next && git push"
expect block "$F" "GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=\$K GIT_CONFIG_VALUE_0=HEAD:refs/heads/next git push"
expect block "$F" "git -c remote.origin.mirror=true push origin"

# git accepts any unique prefix of a long option: `--m` is --mirror, `--o=` is --onto.
echo "Abbreviated push-everything options are read as git reads them:"
expect block "$F" "git push --mirr origin"
expect block "$F" "git push --m origin"
expect block "$F" "git push --br origin"
expect block "$F" "git push --al=x origin"
expect allow "$F" "git push --dry-run origin $F"

# Saved config steers a later bare `git push` that no gate can connect back to this line.
echo "git config naming saved push or merge config is refused on its own line:"
expect block "$F" "git config remote.origin.push HEAD:refs/heads/next"
expect block "$F" "git config set remote.origin.push HEAD:refs/heads/next"
expect block "$F" "git config --add remote.origin.push HEAD:refs/heads/next"
expect block "$F" "git config --replace-all remote.origin.push HEAD:refs/heads/next"
expect block "$F" "git config --file .git/config remote.origin.push HEAD:refs/heads/next"
expect block "$F" "git config remote.pushDefault origin"
expect block "$F" "git config remote.origin.mirror true"
expect block "$F" "git config push.default upstream"
expect block "$F" "git config branch.$F.merge refs/heads/next"
expect block "$F" "git config branch.$F.pushRemote origin"
expect block "$F" "echo x | xargs git config remote.origin.push"
expect block "$F" "git config include.path /tmp/push.cfg"
expect block "$F" "git config includeIf.gitdir:~/.path /tmp/push.cfg"
expect block "$F" "git config remote.origin.fetch +refs/heads/feat/1/x:refs/remotes/origin/next"
expect block "$F" "git config --add remote.origin.fetch +refs/heads/feat/1/x:refs/remotes/origin/next"
expect block "$F" "git config core.hooksPath /dev/null"
expect allow "$F" "git config --get remote.origin.fetch"
expect allow "$F" "git config --get core.hooksPath"
expect block "$F" "git config remote.origin.pushurl git@github.com:VATUSA/OIS.git"
expect block "$F" "git config branch.$F.remote origin"
expect block "$F" "git config url.git@x:.pushInsteadOf https://github.com/"
expect block "$F" "git config -e"
expect block "$F" "git config --edit"
expect allow "$F" "git config user.name T"
expect allow "$F" "git config --get remote.origin.url"
expect block "$F" "git config url.x.insteadOf https://github.com/VATUSA/OIS"
echo "...however git config's options are abbreviated or bundled:"
expect block "$F" "git config --edi"
expect block "$F" "git config --ed"
expect block "$F" "GIT_EDITOR=true git config --edi"
expect block "$F" "git config -le"
expect block "$F" "git config --ad remote.origin.push HEAD:refs/heads/next"
expect block "$F" "git config --replace remote.origin.push HEAD:refs/heads/next"
expect block "$F" "git config --get push.default --add remote.origin.push HEAD:refs/heads/next"
expect block "$F" "git config --rename-section branch.$F branch.next"
expect block "$F" "git config --remove-sec remote.origin"
echo "...while reading saved push config passes:"
expect allow "$F" "git config --get push.default"
expect allow "$F" "git config --get-all remote.origin.push"
expect allow "$F" "git config --get-regexp '^remote\\..*\\.push'"
expect allow "$F" "git config --ge push.default"
expect allow "$F" "git config get remote.origin.push"
expect allow "$F" "git config --list"
expect allow "$F" "git config -l"
expect allow "$F" "git config --file .git/config --get remote.origin.push"
echo "...and a write passes only for a key on the allowlist:"
expect block "$F" "git config --rename-section core.x core.y"
expect block "$F" "git config --rename-section foo alias"
expect block "$F" "git config rename-section foo alias"
expect block "$F" "git config remove-section remote.origin"
expect block "$F" "git config --remove-section color"
expect block "$F" "GIT_EDITOR=true git config edit"
expect block "$F" "git config core.fsmonitor 'git push origin HEAD:next #'"
expect block "$F" "git config core.editor vim"
expect allow "$F" "git config core.sshCommand 'ssh -i k'"
expect block "$F" "git config sequence.editor vim"
expect block "$F" "git config credential.helper store"
expect block "$F" "git config diff.external x"
expect block "$F" "git config gpg.program x"
expect block "$F" "git config set alias.p '!git push origin next'"
expect block "$F" "git config unset push.default"
expect block "$F" "git config \"\$K\" x"
expect block "$F" "git config user.\$X x"
expect block "$F" "git config -f .git/config remote.origin.push HEAD:refs/heads/next"
expect allow "$F" "git config user.name 'T T'"
expect allow "$F" "git config set user.email t@e.com"
expect allow "$F" "git config --global color.diff.meta 'blue bold'"
expect allow "$F" "git config core.autocrlf false"
expect allow "$F" "git config pull.rebase true"
expect allow "$F" "git config --unset user.signingkey"
expect allow "$F" "git config remote.origin.push"
expect allow "$F" "git config --file .git/config get remote.origin.push"
expect allow "$F" "git config -f .git/config remote.origin.url"
expect allow "$F" "git config -f .git/config user.name T"
expect block "$F" "git config user.names x"
expect block "$F" "git config --rename-section color.a color.b"
expect block "$F" "git config rename-section color.a color.b"
expect allow "$F" "git config list --show-origin"
echo "...and so does a -c, --config-env or GIT_CONFIG_KEY_<n> override:"
expect block "$F" "git -c core.pager=less log"
expect block "$F" "git -c core.fsmonitor='git push origin HEAD:next #' status"
expect block "$F" "git -c credential.helper='!git push origin HEAD:next' fetch"
expect block "$F" "git --config-env=core.editor=E commit"
expect block "$F" "git --config-env core.editor=E commit"
expect block "$F" "GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=core.fsmonitor GIT_CONFIG_VALUE_0=x git status"
expect block "$F" "export GIT_CONFIG_KEY_0=core.fsmonitor GIT_CONFIG_COUNT=1; git status"
expect block "$F" "GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=\"\$K\" GIT_CONFIG_VALUE_0=x git status"
expect block "$F" "GIT_CONFIG_COUNT=2 GIT_CONFIG_KEY_0=user.name GIT_CONFIG_VALUE_0=x git status"
expect block "$F" "GIT_CONFIG_COUNT=\$N git status"
expect block "$F" "GIT_CONFIG_PARAMETERS=\"'alias.p'='!x'\" git p"
expect allow "$F" "git -c user.name=T -c user.email=t@e.com log -1"
expect allow "$F" "GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=user.name GIT_CONFIG_VALUE_0=x git log -1"
says "$F" "git config core.editor vim" "core.editor\` is not one of them"
echo "core.sshCommand and GIT_SSH_COMMAND pass only as plain ssh with harmless options:"
expect allow "$F" "git -c core.sshCommand=\"ssh -o ServerAliveInterval=20 -o ServerAliveCountMax=60 -o TCPKeepAlive=yes\" push origin $F"
expect allow "$F" "git -c core.sshCommand=\"ssh -oServerAliveInterval=20 -p 22 -i ~/.ssh/id -F /dev/null\" fetch"
expect allow "$F" "GIT_SSH_COMMAND=\"ssh -o ServerAliveInterval=20\" git push origin $F"
expect allow "$F" "git config core.sshCommand \"ssh -o ServerAliveInterval=20\""
expect allow "$F" "git config --unset core.sshCommand"
expect allow "$F" "GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=core.sshCommand GIT_CONFIG_VALUE_0=\"ssh -o TCPKeepAlive=yes\" git log -1"
for bad in "ssh -o ProxyCommand=nc" "ssh -o proxycommand=nc" "ssh -oLocalCommand=x" "ssh -o PermitLocalCommand=yes" \
    "ssh -o KnownHostsCommand=x" "ssh -o RemoteCommand=x" "ssh -o Match=exec" "./evil.sh" "ssh; ./evil" \
    "ssh -o X=\$(evil)" "ssh -v" "ssh -p x" "ssh -o" "ssh -o ServerAliveInterval" "ssh -i"; do
    expect block "$F" "git -c core.sshCommand=\"$bad\" push origin $F"
done
expect block "$F" "git -c core.sshCommand push origin $F"
expect block "$F" "git --config-env=core.sshCommand=E push origin $F"
expect block "$F" "GIT_SSH_COMMAND=./evil.sh git push origin $F"
expect block "$F" "export GIT_SSH_COMMAND=\"ssh -o ProxyCommand=x\"; git push origin $F"
expect block "$F" "git config core.sshCommand ./evil.sh"
expect block "$F" "git config set core.sshCommand"
expect block "$F" "GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=core.sshCommand GIT_CONFIG_VALUE_0=./evil git log -1"
says "$F" "git -c core.pager=less log" "sets core.pager for this run"
echo "A mirror remote saves push-everything config for a later push:"
expect block "$F" "git remote add --mirror backup https://example.invalid/x.git"
expect block "$F" "git remote add --mirror=push backup https://example.invalid/x.git"
expect block "$F" "git remote add --mir=fetch backup https://example.invalid/x.git"
expect block "$F" "git remote add backup https://example.invalid/x.git --mirror=push"
expect block "$F" "git remote -v add --mirror=push backup https://example.invalid/x.git"
expect block "$F" "git remote --verbose add backup --mir https://example.invalid/x.git"
expect allow "$F" "git remote add backup https://example.invalid/x.git"
expect allow "$F" "git remote -v"
echo "git's exec-path binaries are git subcommands:"
GIT_EXEC="$(git --exec-path)"
expect block "$F" "git-config remote.origin.push HEAD:refs/heads/next"
expect block "$F" "$GIT_EXEC/git-config remote.origin.push HEAD:refs/heads/next"
expect block "$F" "$GIT_EXEC/git-config --edi"
expect block "$F" "git-push origin next"
expect block "$F" "$GIT_EXEC/git-push origin HEAD:next"
expect block next "git-push"
expect block "$F" "xargs git-push origin next"
expect block "$F" "git-remote add --mirror=push backup https://example.invalid/x.git"
expect allow "$F" "git-push origin $F"
expect allow "$F" "git-config --get push.default"
echo "Plumbing pushes are refused outright:"
expect block "$F" "git send-pack origin HEAD:refs/heads/next"
expect block "$F" "git-send-pack https://github.com/VATUSA/OIS.git HEAD:refs/heads/next"
expect block "$F" "git http-push https://example.invalid/x.git next"
echo "A push on a line that points git at another global config is refused:"
expect block "$F" "HOME=$WORK/h git push"
expect block "$F" "XDG_CONFIG_HOME=$WORK/x git push"
expect block "$F" "env HOME=$WORK/h git push origin $F"
expect block "$F" "env XDG_CONFIG_HOME=$WORK/x git push origin $F"
expect block "$F" "export HOME=$WORK/h; git push origin $F"
expect block "$F" "HOME=$WORK/h; git push origin $F"
expect block "$F" "HOME=$WORK/h bash -c 'git push origin $F'"
expect block "$F" "GIT_CONFIG_GLOBAL=$WORK/c git push origin $F"
expect allow "$F" "echo \$HOME && git push origin $F"
expect block "$F" "HOME=$WORK/h git status && git push origin $F"
echo "...however the assignment is spelled, and only an assignment counts:"
expect block "$F" "H\"\"OME=$WORK/h git push origin $F"
expect block "$F" "H\\OME=$WORK/h git push origin $F"
expect block "$F" "declare -x XDG_CONFIG_HOME=$WORK/x; git push origin $F"
expect block "$F" "read HOME < f; git push origin $F"
expect block "$F" "read GIT_CONFIG_GLOBAL < f; export GIT_CONFIG_GLOBAL; git push origin $F"
expect block "$F" "read GIT_CONFIG_SYSTEM < f; export GIT_CONFIG_SYSTEM; git push origin $F"
expect block "$F" "env -u HOME git push origin $F"
expect block "$F" "env --unset=HOME git push origin $F"
expect block "$F" "unset XDG_CONFIG_HOME; git push origin $F"
expect block "$F" "sudo HOME=$WORK/h git push origin $F"
expect block "$F" "eval 'export HOME=$WORK/h'; git push origin $F"
expect block "$F" "bash <<'W'${NL}HOME=$WORK/h git push origin $F${NL}W"
expect allow "$F" "git commit -m \"Fix HOME page\" && git push origin $F"
expect allow "$F" "echo HOME && git push origin $F"
expect block "$F" "export -n HOME; git push origin $F"
expect block "$F" "declare +x HOME; git push origin $F"
expect block "$F" "typeset +x HOME; git push origin $F"
expect block "$F" "f() { local HOME; git push origin $F; }; f"
expect allow "$F" "HOME=$WORK/h git status"

echo "From next, pushing the current branch is blocked:"
expect block next "git push"
expect block next "git push origin"
expect block next "git push -u origin HEAD"
expect block next "git push origin @"
expect block next "git push -u origin @"

echo "main is protected as next is (the release PR promotes next to it; no agent writes it):"
expect block "$F" "git push origin main"
expect block "$F" "git push origin HEAD:main"
expect block "$F" "git push origin +main"
expect block "$F" "git push origin refs/heads/main"
expect block "$F" "git push origin HEAD:heads/main"
expect block "$F" "git push origin :main"
expect block "$F" "bash -c 'git push origin main'"
expect block "$F" "git subtree push --prefix=docs origin main"
expect block main "git push"
expect block main "git push -u origin HEAD"
expect block "$F" "gh api -X PATCH repos/VATUSA/OIS/git/refs/heads/main -f sha=abc"
expect allow main "git push origin $F"
expect allow "$F" "git push origin fix/289/main-menu-fix"

# The hook reads the branch where it runs; a line that runs git elsewhere must name what it pushes.
ONMAIN="$WORK/onmain"
git clone -q "$REPO" "$ONMAIN" && git -C "$ONMAIN" checkout -q next
echo "A push that runs in another checkout, leaving the branch to git, is blocked:"
expect block "$F" "cd $ONMAIN && git push"
expect block "$F" "cd $ONMAIN && git push -u origin HEAD"
expect block "$F" "cd $ONMAIN && git push origin @"
expect block "$F" "cd $ONMAIN; git push origin"
expect block "$F" "git -C $ONMAIN push"
expect block "$F" "git -C $ONMAIN push -u origin HEAD"
expect block "$F" "git --git-dir=$ONMAIN/.git push"
expect block "$F" "GIT_DIR=$ONMAIN/.git git push"
expect block "$F" "env --chdir=$ONMAIN git push"
expect block "$F" "env -C $ONMAIN git push"
expect block "$F" "env -C$ONMAIN git push"
expect block "$F" "env --chd=$ONMAIN git push"
expect block "$F" "env --ch $ONMAIN git push"
expect block "$F" "env --c=$ONMAIN git push"
expect block "$F" "env --c $ONMAIN git push"
expect block "$F" "nice env --c=$ONMAIN git push"
expect allow "$F" "cargo fmt --all -- --check && git push origin $F"
expect block "$F" "env -iC$ONMAIN PATH=/usr/bin:/bin git push"
expect block "$F" "nice env -C$ONMAIN git push"
expect block "$F" "timeout 9 env -C$ONMAIN git push"
expect block "$F" "env -S 'git push origin next'"
expect allow "$F" "env -C$ONMAIN git push origin $F"
expect allow "$F" "env -u X git push origin $F"
expect block "$F" "find $ONMAIN/.git -maxdepth 0 -execdir git push \\;"
expect block "$F" "find $ONMAIN/.git -maxdepth 0 -okdir git push \\;"
expect block "$F" "find $ONMAIN/.git -maxdepth 0 -execdir git push origin HEAD \\;"
expect block "$F" "find $ONMAIN/.git -maxdepth 0 -execdir git push origin @ \\;"
expect block "$F" "echo origin | xargs git push"
expect block "$F" "sudo -u nobody git push"
expect allow "$F" "find $ONMAIN/.git -maxdepth 0 -execdir git push origin $F \\;"
expect allow "$F" "echo origin | xargs git push origin $F"
expect block "$F" "pushd $ONMAIN && git push"
expect block "$F" "bash -c 'cd $ONMAIN && git push'"
echo "...but one naming its branch, or only mentioning cd, passes:"
expect allow "$F" "cd $ONMAIN && git push origin $F"
expect allow "$F" "git -C $ONMAIN push -u origin $F"
expect allow "$F" "git commit -m \"cd fix\" --allow-empty && git push -u origin HEAD"
expect allow "$F" "git -c user.name=x push origin $F"

echo "Feature pushes pass:"
expect allow "$F" "git push -u origin HEAD"
expect allow "$F" "git push origin $F"
expect allow "$F" "git push origin fix/289/next-menu-fix"
expect allow next "git push origin $F"
expect allow "$F" "git push -o next origin $F"

echo "PR merges are blocked:"
expect block "$F" "gh pr merge 2684"
expect block "$F" "cd . && gh pr merge 2684 --squash"

echo "PR merges by another spelling or through the API are blocked:"
expect block "$F" "gh pr --repo VATUSA/OIS merge 12"
expect block "$F" "gh pr -R VATUSA/OIS merge 12 --squash"
expect block "$F" "gh api -X PUT repos/VATUSA/OIS/pulls/12/merge"
expect block "$F" "gh api --method=PUT /repos/VATUSA/OIS/pulls/12/merge -f merge_method=squash"
expect block "$F" "gh api repos/VATUSA/OIS/merges -f base=next -f head=$F"
expect block "$F" "gh api -XPATCH repos/VATUSA/OIS/git/refs/heads/next -f sha=abc"
expect block "$F" "gh alias set pm 'pr merge'"
expect block "$F" "gh -R VATUSA/OIS pr merge 12"
expect block "$F" "gh --repo=VATUSA/OIS pr merge 12 --squash"
expect block "$F" "gh -R VATUSA/OIS api -X PUT repos/VATUSA/OIS/pulls/12/merge"
echo "Any other gh api write fails closed:"
expect block "$F" "gh api -X PUT repos/VATUSA/OIS/contents/backend/src/main.rs -f message=x -f content=eA== -f branch=next"
expect block "$F" "gh api -X PATCH repos/VATUSA/OIS/git/refs/heads/next -f sha=abc"
expect block "$F" "gh api repos/VATUSA/OIS/git/refs -f ref=refs/heads/next -f sha=abc"
expect block "$F" "gh api -X POST repositories/123456/merges -f base=next -f head=x"
expect block "$F" "gh api -X PUT https://api.github.com/repos/VATUSA/OIS/pulls/12/merge"
expect block "$F" "gh api -X PUT /REPOS/VATUSA/OIS/PULLS/12/MERGE?x=1"
expect block "$F" "gh api --method put repos/VATUSA/OIS/pulls/12/merge"
expect block "$F" "gh api -X \$M repos/VATUSA/OIS/pulls/12/merge"
expect block "$F" "gh api repos/VATUSA/OIS/issues -f title=x -f body=y"
expect block "$F" "gh api repos/VATUSA/OIS/pulls/12/comments -f body=x"
expect block "$F" "gh api -X PUT repos/VATUSA/OIS/issues/12/comments -f body=x"
echo "...except an issue comment through REST, github-issues.md's fallback when GraphQL is throttled, and its edit:"
expect allow "$F" "gh api repos/VATUSA/OIS/issues/12/comments -f body=x"
expect allow "$F" "gh api --method POST repos/VATUSA/OIS/issues/12/comments --input c.json --jq .html_url"
expect allow "$F" "gh api -X PATCH repos/VATUSA/OIS/issues/comments/12 -F body=@plan.md"
expect allow "$F" "gh api --method PATCH /repos/VATUSA/OIS/issues/comments/12 -f body=x"
expect allow "$F" "gh issue comment 12 --repo VATUSA/OIS --edit-last --body-file m3.md"
expect block "$F" "gh api -X PATCH repos/VATUSA/OIS/issues/comments/IC_kwDOabc -f body=x"
expect block "$F" "gh api -X PATCH repos/VATUSA/OIS/issues/12 -f body=x"
expect block "$F" "gh api -X PATCH repos/VATUSA/OIS/issues/comments/12/reactions -f content=x"
expect block "$F" "gh api -X DELETE repos/VATUSA/OIS/issues/comments/12"
expect block "$F" "gh api -X PATCH repos/VATUSA/OIS/pulls/comments/12 -f body=x"
expect block "$F" "gh api -X DELETE repos/VATUSA/OIS/git/refs/heads/next"
expect block "$F" "gh api -XDELETE repos/VATUSA/OIS/git/refs/heads/next"
expect block "$F" "gh api --method=DELETE repos/VATUSA/OIS/git/refs/heads/next"
expect block "$F" "gh api repos/VATUSA/OIS/merges -fbase=next -fhead=$F"
expect block "$F" "gh api repos/VATUSA/OIS/merges --field=base=next --raw-field=head=$F"
echo "Each field flag alone makes gh POST:"
expect block "$F" "gh api repos/VATUSA/OIS/merges -F base=next"
expect block "$F" "gh api repos/VATUSA/OIS/merges --field base=next"
expect block "$F" "gh api repos/VATUSA/OIS/merges --raw-field base=next"
expect block "$F" "gh api repos/VATUSA/OIS/merges --input body.json"
expect allow "$F" "gh api repos/VATUSA/OIS/merges --jq .x"
echo "Short-flag bundles are read as gh reads them:"
expect block "$F" "gh api -iX PUT repos/VATUSA/OIS/pulls/12/merge"
expect block "$F" "gh api -iXPUT repos/VATUSA/OIS/pulls/12/merge"
expect block "$F" "gh api -X=PUT repos/VATUSA/OIS/pulls/12/merge"
expect block "$F" "gh api -iX PUT repos/VATUSA/OIS/contents/backend/src/main.rs -f message=x -f content=eA== -f branch=next"
expect block "$F" "gh api -i repos/VATUSA/OIS/merges -fbase=next -fhead=$F"
expect block "$F" "gh api -iz repos/VATUSA/OIS/pulls/12/merge"
expect allow "$F" "gh api -i repos/VATUSA/OIS/pulls/12"
echo "GraphQL: every mutation, and anything the gate cannot read whole, is refused:"
expect block "$F" "gh api graphql"
expect block "$F" "gh api graphql -f query='mutation { mergePullRequest(input: {pullRequestId: \"x\"}) { clientMutationId } }'"
expect block "$F" "gh api graphql -f query='mutation { enablePullRequestAutoMerge(input: {pullRequestId: \"x\"}) { clientMutationId } }'"
expect block "$F" "gh api graphql -f query='mutation { createCommitOnBranch(input: {}) { commit { oid } } }'"
expect block "$F" "gh api graphql -f query='mutation{updateProjectV2ItemFieldValue(input:{projectId:\"P\",itemId:\"I\"}){projectV2Item{id}}}'"
expect block "$F" "gh api /GRAPHQL -f query='MUTATION { mergePullRequest(input: {}) { clientMutationId } }'"
expect block "$F" "gh api https://api.github.com/graphql?x=1 -f query='mutation { mergeBranch(input: {}) { clientMutationId } }'"
expect block "$F" "gh api graphql -f query='Mutation { mergeBranch(input: {}) { clientMutationId } }'"
expect block "$F" "gh api graphql --input q.json"
expect block "$F" "gh api graphql --input - <<< 'query { viewer { login } }'"
expect block "$F" "gh api graphql -F query=@q.graphql"
expect block "$F" "gh api graphql -F query=@-"
expect block "$F" "gh api graphql -f query=\"\$Q\""
expect block "$F" "gh api graphql -f query=\"\$(cat q.graphql)\""
expect block "$F" "gh api graphql -f query=\"\`cat q.graphql\`\""
expect block "$F" "gh api graphql -F query=<(cat q.graphql)"
expect block "$F" "gh api graphql -f query='{ viewer { login } }' -f=query='mutation { x }'"
expect block "$F" "gh api graphql -F=query='mutation { x }'"
expect block "$F" "gh api graphql -f query=mutation{mergeBranch,x}"
expect block "$F" "gh api graphql -f query=mut*tion"
expect block "$F" "gh api graphql -f query='{ viewer }' -f x=muta{tion,}"
expect block "$F" "gh api graphql -f query=~/q.graphql"
expect block "$F" "gh api graphql -f q='{ viewer { login } }'"
expect block "$F" "gh api graphql -f query=\$'mutation { x }'"
expect block "$F" "Q='mutation { x }'; gh api graphql -f query=\"\$Q\""
expect block "$F" "bash -c \"gh api graphql -f query='mutation { mergeBranch(input: {}) { clientMutationId } }'\""
echo "...while a literal read passes, and bin/board-status.sh makes the board's mutation inside the script:"
expect allow "$F" "gh api graphql -f query='{ viewer { login } }'"
expect allow "$F" "gh api graphql --paginate -f query='query { viewer { login } }' --jq .data"
expect allow "$F" "gh api graphql -f query='query(\$n: Int!) { repository(owner: \"VATUSA\", name: \"OIS\") { issue(number: \$n) { projectItems(first: 10) { nodes { id } } } } }' -F n=47"
expect allow "$F" "gh api graphql -f query='{ repository(owner: \"VATUSA\", name: \"OIS\") { issue(number: 47) { title } } }' --jq .data"
expect allow "$F" "bash bin/board-status.sh 47 \"In build\""
expect allow "$F" "bin/board-status.sh 47 \"Post build\""
expect allow "$F" "gh api -X PUT repos/VATUSA/OIS/pulls/12/update-branch -f expected_head_sha=abc"
expect allow "$F" "gh api --paginate repos/VATUSA/OIS/pulls/12/comments"
expect allow "$F" "gh api -X GET repos/VATUSA/OIS/issues -f state=open"
expect allow "$F" "gh api repos/VATUSA/OIS/pulls/12"
expect allow "$F" "gh api repos/VATUSA/OIS/pulls/12/merge"

echo "A quoted string holding a command is read as one, wherever it sits:"
expect block "$F" "git rebase --exec 'git push origin HEAD:next' origin/next"
expect block "$F" "git rebase --exec='git push origin HEAD:next' origin/next"
expect block "$F" "git rebase -x'git push origin HEAD:next' origin/next"
expect block "$F" "GIT_EDITOR='git push origin HEAD:next; true' git commit"
expect block "$F" "watch -g 'git push origin HEAD:next'"
expect block "$F" "ssh host 'git push origin HEAD:next'"
expect block "$F" "tmux new-session -d 'git push origin HEAD:next'"
expect block "$F" "git submodule foreach 'git push origin HEAD:next'"
expect block "$F" "git filter-branch --tree-filter 'git push origin HEAD:next'"
expect block "$F" "git difftool -x 'git push origin HEAD:next'"
expect block "$F" "npx -c 'git push origin HEAD:next'"
expect block "$F" "X='git push origin HEAD:next' make"
expect block "$F" "ssh host 'gh pr merge 12'"
expect block "$F" "echo \"git push origin next\""
expect block "$F" "grep -n \"gh pr merge\" .claude/hooks/*.sh"
expect block "$F" "echo \"don't git push origin next\""
expect block "$F" "echo \"don't git-push origin next\""
expect block next "tool 'x;git-push'"
expect block "$F" "ssh host 'git config core.editor vim'"
expect allow "$F" "echo \"don't do it\""
echo "...except a commit or PR message, a heredoc written to a file, and text naming nothing:"
expect allow "$F" "git commit -m \"docs: git push origin next is blocked\""
expect allow "$F" "git commit -m 'docs: gh pr merge is blocked'"
expect allow "$F" "git tag -a v1 -m 'git push origin next'"
expect allow "$F" "gh pr create --title 'gate: git push' --body 'git push origin next is blocked'"
expect allow "$F" "gh issue comment 1 -b 'gh pr merge is blocked'"
expect allow "$F" "cat > notes.md <<'EOF'${NL}git push origin next${NL}gh pr merge 1${NL}EOF"
expect allow "$F" "git log --format='%h %s' -3"
echo "ANSI-C quoting is refused only on a line that could run git:"
expect block "$F" "git \$'push' origin next"
expect block "$F" "IFS=\$'\\n'; git status"
expect allow "$F" "IFS=\$'\\n'; for f in a b; do echo \"\$f\"; done"
says "$F" "git \$'push' origin next" "ANSI-C"
echo "git subtree push is a push to the branch it names:"
expect block "$F" "git subtree push --prefix=docs origin next"
expect block "$F" "git subtree push -P docs origin refs/heads/next"
expect block "$F" "git-subtree push --prefix docs origin next"
expect block "$F" "git subtree push --prefix=docs origin \$B"
expect block "$F" "git subtree push --prefix=docs origin"
expect block "$F" "git subtree push --prefix=docs origin $F extra"
expect allow "$F" "git subtree push --prefix=docs origin $F"
echo "...with its options before the subcommand too:"
expect block "$F" "git subtree --prefix=docs push origin next"
expect block "$F" "git subtree -P docs push origin next"
expect block "$F" "git subtree -q --pre docs push origin refs/heads/next"
expect block "$F" "git subtree --annotate x -P docs push origin next"
expect block "$F" "git-subtree -P docs push origin next"
expect allow "$F" "git subtree --prefix=docs push origin $F"
expect allow "$F" "git subtree -P docs push origin $F"
expect allow "$F" "git subtree --prefix=docs split --branch docs-only"
expect allow "$F" "git subtree -P push split -b docs-only"
echo "...bundled, as git rev-parse --parseopt reads them: a value letter last takes the next word, earlier the rest:"
for options in "-dP docs" "-qP docs" "-dqP docs" "-dPdocs" "-db x -P docs" "-dm x -P docs" "-mP -P docs" "-Pm" \
    "-dSP -P docs" "-S -P docs" "-P docs --" "-m x -P docs" "-b x -P docs" "--message x -P docs" "--branch x -P docs" \
    "--onto x -P docs" "--annotate x -P docs" "--prefix docs" "--mes x --bra y --ont z --ann w --pre docs"; do
    expect block "$F" "git subtree $options push origin next"
    expect allow "$F" "git subtree $options push origin $F"
done
# Each option that takes a value, driven on its own: unread, its value would be taken for the subcommand.
for option in --prefix --message --branch --onto --annotate; do
    expect block "$F" "git subtree $option x --prefix=docs push origin next"
    expect block "$F" "git subtree ${option:0:5} x --prefix=docs push origin next"
    expect allow "$F" "git subtree $option x --prefix=docs push origin $F"
done
for letter in P m b; do
    expect block "$F" "git subtree -$letter x --prefix=docs push origin next"
    expect block "$F" "git subtree -d$letter x --prefix=docs push origin next"
    expect allow "$F" "git subtree -d$letter x --prefix=docs push origin $F"
done
expect block "$F" "git subtree push --prefix=docs \$R $F"
expect block "$F" "git subtree push --prefix=docs next $F"
expect block "$F" "git subtree push origin next -dP docs"
expect block "$F" "git subtree push -dP docs origin next"
expect block "$F" "git subtree -dP docs push -- origin next"
expect block "$F" "bash -c 'git subtree -dqP docs push origin next'"
expect allow "$F" "git subtree -dP push split -b docs-only"
expect allow "$F" "git subtree -db push -P docs split"
echo "zsh =cmd is cmd, and precommand modifiers and quoting hide nothing:"
expect block "$F" "=git push origin next"
expect block next "=git push"
expect block "$F" "=gh pr merge 5"
expect block "$F" "command =git push origin next"
expect block "$F" "sudo -n =git push origin next"
expect block "$F" "=git subtree -P docs push origin next"
expect block "$F" "noglob git push origin next"
expect block "$F" "nocorrect git push origin next"
expect block "$F" "- git push origin next"
expect block "$F" "builtin git push origin next"
expect block "$F" "repeat 1 git push origin next"
expect block "$F" "cat =(git push origin next)"
expect block "$F" "\\git push origin next"
expect block "$F" "'git' push origin next"
expect block "$F" "g''it push origin next"
expect allow "$F" "=git push origin $F"
expect allow "$F" "noglob git push origin $F"
expect allow "$F" "- git push origin $F"
expect allow "$F" "=git status"
echo "...and ordinary lines with a built word pass:"
for line in "git push origin $F" "git -C . status" "git -C \"\$dir\" push origin $F" \
    "git --git-dir=\"\$dir/.git\" push origin $F" "git --git-dir \"\$dir/.git\" push origin $F" 'echo "$greeting world"' 'cp "$a" "$b"' 'ls *.md' 'cp a.{txt,bak}' \
    'case $x in *) echo y;; esac' '[ -f a ] && echo y' 'x=$(git rev-parse HEAD) && echo "$x"' 'gh pr view $n' \
    'for f in $list; do echo "$f"; done' 'echo ==== ; ls' 'git log --format=%h -3 "$ref"' 'git commit -m "use $x here"' \
    "echo '{\"a\":1,\"b\":\"git push origin next\"}' > /tmp/x.json"; do
    expect allow "$F" "$line"
done
echo "A symlink to git under another name is git:"
ln -s "$(command -v git)" "$WORK/g"
expect block "$F" "$WORK/g push origin next"
expect block "$F" "$WORK/g config core.editor vim"
expect allow "$F" "$WORK/g push origin $F"
expect allow "$F" "gh pr view 2684"
expect allow "$F" "gh run list"
expect allow "$F" "git status"

# The primary checkout stays on next: a branch is born in a worktree. $REPO is a primary checkout, so every
# expect above ran in one; the worktree cases run with in_dir.
WT="$WORK/wt"
git -C "$REPO" worktree add -q -b feat/1/wt-branch "$WT" 2>/dev/null
git -C "$REPO" tag v1.0
# in_dir WANT DIR COMMAND — the gate run from DIR, whatever branch it is on
in_dir() {
    local rc got
    (cd "$2" && python3 -c 'import json,sys; print(json.dumps({"tool_input": {"command": sys.argv[1]}}))' "$3" \
        | bash "$GATE" >/dev/null 2>&1)
    rc=$?
    case "$rc" in 0) got=allow ;; 2) got=block ;; *) got="rc=$rc" ;; esac
    if [[ "$got" == "$1" ]]; then ok "$1 in ${2##*/}: $3"; else bad "in ${2##*/}: $3 (want $1, got $got)"; fi
}
echo "Creating, renaming or switching a branch in the primary checkout is blocked:"
expect block next "git checkout -b feat/2/new-thing"
expect block next "git checkout -B feat/2/new-thing"
expect block next "git switch -c feat/2/new-thing"
expect block next "git switch --create feat/2/new-thing"
expect block next "git switch $F"
expect block next "git switch -"
expect block next "git checkout -"
expect block next "git checkout v1.0"
expect block next "git checkout --detach"
expect block next "git branch feat/2/new-thing"
expect block next "git branch -m renamed"
expect block next "git branch -M next renamed"
expect block next "git branch --move renamed"
expect block next "git branch -c copied"
expect block next "cd $REPO && git checkout -b feat/2/x"
expect block next "git -C $REPO switch -c feat/2/x"
expect block next "bash -c 'git checkout -b feat/2/x'"
expect block next "echo x | xargs git checkout -b"
expect block next "cd \"\$WT\" && git checkout -b feat/2/x"
expect block next "cd '/no such/dir' && git checkout -b feat/2/x"
expect block next "cd '/no such/dir'; git switch -c feat/2/x"
says next "git checkout -b feat/2/x" "primary checkout"
echo "...and so is gh pr checkout, which switches the checkout it runs in, in any spelling:"
for line in "gh pr checkout 812" "gh pr checkout https://github.com/VATUSA/OIS/pull/812" "gh -R VATUSA/OIS pr checkout 812" \
    "gh pr -R VATUSA/OIS checkout 812" "gh pr checkout -R VATUSA/OIS 812" "gh --repo=VATUSA/OIS pr checkout 812" \
    "gh co 812" "gh -R VATUSA/OIS co 812" "GH_REPO=VATUSA/OIS gh pr checkout 812" "bash -c 'gh pr checkout 812'" \
    "cd $REPO && gh pr checkout 812" "cd \"\$WT\" && gh pr checkout 812" "env -C $WT gh pr checkout 812"; do
    expect block next "$line"
done
says next "gh pr checkout 812" "gh pr checkout"
echo "...a bare trailing -- still switches (it only marks a revision):"
git -C "$REPO" update-ref refs/remotes/origin/main HEAD
for line in "git checkout -b x --" "git checkout -B x --" "git checkout $F --" "git checkout --orphan z --" \
    "git checkout --detach --" "git checkout --track origin/main --" "git checkout v1.0 --"; do
    expect block next "$line"
done
expect allow next "git checkout -- README.md"
expect allow next "git checkout $F -- README.md"
echo "...and a restore from another tree-ish may not overwrite .claude/, the hooks every session runs:"
for line in "git checkout $F -- .claude" "git checkout $F -- .claude/hooks/master-protection-gate.sh" "git checkout $F ." \
    "git checkout $F -- ." "git checkout v1.0 -- :/" "git checkout $F -- '*.sh'" "git checkout $F -- ':!README.md'" \
    "git restore --source=$F .claude" "git restore --source $F -- ." "git restore -s $F ." "git restore -Ws$F .claude" \
    "git restore --sou=$F .claude/hooks" "git checkout $F --pathspec-from-file=paths.txt --"; do
    expect block next "$line"
done
says next "git checkout $F -- .claude" "overwrite the primary checkout's .claude/"
for line in "git checkout HEAD -- .claude" "git checkout -- .claude" "git checkout @ -- ." "git restore .claude" \
    "git restore --source=HEAD -- ." "git restore --staged ." "git checkout $F -- web/src/a.ts" \
    "git restore --source=$F README.md"; do
    expect allow next "$line"
done
in_dir allow "$WT" "git checkout $F -- .claude"
in_dir allow "$WT" "git restore --source=$F ."
expect allow next "cd $WT && gh pr checkout 812"
expect allow next "gh pr view 812"
expect allow next "gh pr diff 812"
expect allow next "gh pr list --search checkout"
echo "...while restoring files, staying put, listing and deleting pass:"
expect allow next "git checkout -- README.md"
expect allow next "git checkout README.md"
expect allow next "git checkout HEAD -- README.md"
expect allow next "git checkout HEAD README.md"
expect allow next "git checkout v1.0 README.md backend/src/a.rs"
expect allow next "git checkout $F README.md"
expect allow next "git switch next"
expect allow next "git checkout next"
expect allow next "git branch -vv"
expect allow next "git branch --show-current"
expect allow next "git branch -d old"
expect allow next "git branch -D old"
expect allow next "git branch --list 'feat/*'"
expect allow next "git branch -r --contains HEAD"
expect allow next "git worktree add ../ois-wt/feat/4/z -b feat/4/z origin/next"
expect allow next "git commit -m \"never git checkout -b in the primary\""
echo "...and in a linked worktree the same commands pass (/start renames worktree-* there):"
in_dir allow "$WT" "git checkout -b feat/3/other-work"
in_dir allow "$WT" "git switch -c feat/3/other-work"
in_dir allow "$WT" "git branch -m worktree-fix+289+x feat/1/wt-renamed"
in_dir allow "$WT" "git switch next"
in_dir allow "$WT" "gh pr checkout 812"
in_dir allow "$WT" "gh co 812"
in_dir block "$WT" "cd $REPO && git switch -c q"
in_dir block "$WT" "git -C $REPO checkout -b q"
expect allow next "cd $WT && git checkout -b feat/5/x"
expect allow next "git -C $WT switch -c feat/5/x"

echo "Staging the whole tree is blocked; explicit paths pass:"
for line in "git add -A" "git add ." "git add --all" "git add -Av" "git add --al" "git add -- ." "git add ./" \
    "git add :/" "git stage -A" "git add --no-ignore-removal" "cargo fmt && git add -A && git commit -m x" \
    "bash -c 'git add .'" "git -C $WT add -A"; do
    expect block "$F" "$line"
done
for line in "git add backend/src/a.rs web/src/b.ts" "git add -p backend/src/a.rs" \
    "git commit -m \"never git add -A\"" "git add ./backend/src/a.rs" "git add -- backend/src/a.rs"; do
    expect allow "$F" "$line"
done
says "$F" "git add -A" "stages everything"
echo "...and so is -u/--update with no pathspec, a bare * pathspec, and git commit -a/--all:"
for line in "git add -u" "git add --update" "git add --up" "git add -uv" "git stage -u" "git add *" "git add -- *" \
    "git add -u ." "git commit -a" "git commit -am x" "git commit -a -m x" "git commit --all -m x" "git commit --al -m x" \
    "git commit -m x -a" "git commit -vam x" "git commit --amend -a --no-edit" "git commit -q -a" \
    "git commit -F msg.txt -a" "git commit --message x --all" "bash -c 'git commit -am x'" \
    "git add backend/a.rs && git commit -am x" "git -C $WT commit -am x"; do
    expect block "$F" "$line"
done
for line in "git add ':!README.md'" "git add ':^README.md'" "git add ':(exclude)README.md'" \
    "git add -- ':(top,exclude)web' ':!README.md'" "git stage ':!x'" "git commit -m x -- ':!README.md'" \
    "git commit -m x ." "git commit -m x -- :/" "git commit -m x '*'"; do
    expect block "$F" "$line"
done
says "$F" "git add ':!README.md'" "excludes paths and nothing else"
for line in "git add backend/ ':!backend/x.rs'" "git add -- web ':(exclude)web/dist'" "git commit -m x -- backend/a.rs ':!web'"; do
    expect allow "$F" "$line"
done
says "$F" "git commit -am x" "commits every tracked change"
says "$F" "git add -u" "with no pathspec stages every tracked change"
for line in "git add -u backend/" "git add -u -- backend/src/a.rs" "git add --update web/src" \
    "git add -u --pathspec-from-file=paths.txt" "git commit -m x" "git commit --amend --no-edit" \
    "git commit -ma" "git commit -m -a" "git commit --message -a" "git commit -F -a" "git commit -m \"git commit -a\"" \
    "git commit --allow-empty -m x" "git commit -sm x" "git commit -S -m x" "git commit -Sa -m x" \
    "git commit -- backend/src/a.rs" "git add 'backend/*.rs'"; do
    expect allow "$F" "$line"
done

# A json.py in the session cwd must not stand in for the standard library in the gate's own python.
echo "A planted json.py in the cwd does not disarm the gate:"
printf 'raise SystemExit(0)\n' > "$REPO/json.py"
expect block "$F" "gh pr merge 2684"
expect block "$F" "git push origin next"
rm -f "$REPO/json.py"

echo ""
echo "master-protection-gate: $PASS passed, $FAIL failed"
(( FAIL == 0 ))
