#!/usr/bin/env bash
# Script-level tests for the Claude Code hooks and the commit-msg git hook.
#
# Builds a throwaway repo (a primary checkout on `next`, a linked worktree, fake origin branches),
# feeds each hook JSON payloads shaped like Claude Code's, and checks the exit code:
#   gates      blocked -> 2, allowed -> 0, empty/malformed payload -> 2
#   reminders  always 0; the reminder text appears only when the command matches
# It tests the hooks in THIS checkout, not the primary checkout's copies that settings.json runs.
#
# Usage: bash .claude/hooks/test/run.sh       Exit 0 when every case passes.
# shellcheck disable=SC2016  # payload commands are single-quoted on purpose: they are test input
set -uo pipefail

HOOKS="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GITHOOKS="$(cd "$HOOKS/../../.githooks" && pwd)"
SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT

pass=0
fail=0
failures=""

# --- fixture ---------------------------------------------------------------------------------------
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
PRIMARY="$SANDBOX/primary"
WT="$SANDBOX/wt"
BADWT="$SANDBOX/badwt"
{
    git init -q -b next "$PRIMARY"
    git -C "$PRIMARY" config user.name test
    git -C "$PRIMARY" config user.email test@example.com
    git -C "$PRIMARY" config commit.gpgsign false
    mkdir -p "$PRIMARY/backend/migrations"
    echo 'select 1;' >"$PRIMARY/backend/migrations/0001_init.sql"
    echo 'select 2;' >"$PRIMARY/backend/migrations/0002_users.sql"
    echo readme >"$PRIMARY/README.md"
    git -C "$PRIMARY" add README.md backend
    git -C "$PRIMARY" commit -qm init
    git -C "$PRIMARY" update-ref refs/remotes/origin/next HEAD
    git -C "$PRIMARY" tag v1.0
    # Another open branch that already took migration 3.
    git -C "$PRIMARY" checkout -q -b feat/9/other-thing
    echo 'select 3;' >"$PRIMARY/backend/migrations/0003_other.sql"
    git -C "$PRIMARY" add backend/migrations/0003_other.sql
    git -C "$PRIMARY" commit -qm other
    git -C "$PRIMARY" update-ref refs/remotes/origin/feat/9/other-thing HEAD
    git -C "$PRIMARY" checkout -q next
    git -C "$PRIMARY" branch -q -D feat/9/other-thing
    git -C "$PRIMARY" worktree add -q "$WT" -b feat/1/test-branch next
    git -C "$PRIMARY" worktree add -q "$BADWT" -b Bad_Branch next
} >/dev/null 2>&1 || { echo "fixture setup failed" >&2; exit 1; }
WT_SHA="$(git -C "$WT" rev-parse HEAD)"
MARKERS="$PRIMARY/.claude/markers/review-shipping"

# --- helpers ---------------------------------------------------------------------------------------
bash_payload() { jq -n --arg c "$1" --arg d "$2" '{hook_event_name: "PreToolUse", tool_name: "Bash", tool_input: {command: $c}, cwd: $d}'; }

# expect HOOK CODE DESCRIPTION PAYLOAD
expect() {
    local hook="$1" want="$2" desc="$3" payload="$4" got out
    out="$(printf '%s' "$payload" | bash "$HOOKS/$hook.sh" 2>&1)"
    got=$?
    if [[ "$got" == "$want" ]]; then
        pass=$((pass + 1))
    else
        fail=$((fail + 1))
        failures="$failures  FAIL $hook: $desc (want $want, got $got)"$'\n'"$(printf '%s\n' "$out" | sed 's/^/      /')"$'\n'
    fi
}
block() { expect "$1" 2 "$2" "$(bash_payload "$3" "${4:-$WT}")"; }
allow() { expect "$1" 0 "$2" "$(bash_payload "$3" "${4:-$WT}")"; }
malformed() {
    expect "$1" 2 "empty payload" ""
    expect "$1" 2 "not JSON" "this is not json"
    expect "$1" 2 "JSON without tool_input" '{"tool_name":"Bash"}'
    expect "$1" 2 "tool_input without command" '{"tool_name":"Bash","tool_input":{}}'
    expect "$1" 2 "non-string command" '{"tool_name":"Bash","tool_input":{"command":42}}'
}

# remind_case HOOK WANT_TEXT DESCRIPTION PAYLOAD — exit 0, and WANT_TEXT ("" = no output) in stdout.
remind_case() {
    local hook="$1" want="$2" desc="$3" payload="$4" got out ok=1
    out="$(printf '%s' "$payload" | bash "$HOOKS/$hook.sh" 2>/dev/null)"
    got=$?
    [[ $got -eq 0 ]] || ok=0
    if [[ -z "$want" ]]; then
        [[ -z "$out" ]] || ok=0
    else
        printf '%s' "$out" | jq -e --arg w "$want" '.hookSpecificOutput.additionalContext | contains($w)' >/dev/null 2>&1 || ok=0
    fi
    if [[ $ok -eq 1 ]]; then
        pass=$((pass + 1))
    else
        fail=$((fail + 1))
        failures="$failures  FAIL $hook: $desc (exit $got, output: ${out:0:200})"$'\n'
    fi
}
# remind_lacks HOOK TEXT DESCRIPTION PAYLOAD — exit 0, a reminder printed, and TEXT nowhere in it.
remind_lacks() {
    local hook="$1" text="$2" desc="$3" payload="$4" got out ok=1
    out="$(printf '%s' "$payload" | bash "$HOOKS/$hook.sh" 2>/dev/null)"
    got=$?
    [[ $got -eq 0 && -n "$out" ]] || ok=0
    [[ "$out" != *"$text"* ]] || ok=0
    if [[ $ok -eq 1 ]]; then
        pass=$((pass + 1))
    else
        fail=$((fail + 1))
        failures="$failures  FAIL $hook: $desc (exit $got, output: ${out:0:200})"$'\n'
    fi
}
remind_malformed() {
    remind_case "$1" "${2:-}" "empty payload" ""
    remind_case "$1" "${2:-}" "not JSON" "this is not json"
}

# --- git-safety-gate -------------------------------------------------------------------------------
G=git-safety-gate
block $G "push to next" 'git push origin next'
block $G "force-push to main" 'git push -f origin main'
block $G "push HEAD:next" 'git push origin HEAD:next'
block $G "force refspec +next" 'git push --force-with-lease origin +next'
block $G "push refs/heads/main" 'git push origin feat/1/test-branch:refs/heads/main'
block $G "push --all" 'git push --all origin'
block $G "bare push while on next" 'git push' "$PRIMARY"
block $G "push to next after a chain" 'cargo fmt && git add a.rs && git push origin next'
block $G "push inside \$(...)" 'echo $(git push origin next)'
block $G "gh pr merge" 'gh pr merge 12 --squash'
block $G "gh -R pr merge" 'gh -R VATUSA/OIS pr merge 12'
block $G "checkout -b in primary" 'git checkout -b feat/2/new-thing' "$PRIMARY"
block $G "switch -c in primary" 'git switch -c feat/2/new-thing' "$PRIMARY"
block $G "switch to another branch in primary" 'git switch feat/1/test-branch' "$PRIMARY"
block $G "checkout a commit in primary" 'git checkout v1.0' "$PRIMARY"
block $G "branch create in primary" 'git branch feat/2/new-thing' "$PRIMARY"
block $G "branch rename in primary" 'git branch -m renamed' "$PRIMARY"
block $G "cd into primary, then branch" "cd $PRIMARY && git checkout -b feat/2/x"
block $G "git -C primary switch -c" "git -C $PRIMARY switch -c feat/2/x"
block $G "cd to an unknowable dir keeps the last known one" 'cd "/no such/dir" && git checkout -b feat/2/x' "$PRIMARY"
block $G "git add -A" 'git add -A'
block $G "git add ." 'git add .'
block $G "git add --all" 'git add --all'
block $G "git add -Av" 'git add -Av'
allow $G "push a feature branch" 'git push -u origin feat/1/test-branch'
allow $G "bare push from a feature worktree" 'git push'
allow $G "push a branch whose name contains next" 'git push origin chore/7/next-steps'
allow $G "checkout -b in a worktree" 'git checkout -b feat/3/other-work'
allow $G "switch in a worktree" 'git switch next'
allow $G "restore a file with --" 'git checkout -- README.md' "$PRIMARY"
allow $G "restore a file without --" 'git checkout README.md' "$PRIMARY"
allow $G "switch to the current branch" 'git switch next' "$PRIMARY"
allow $G "worktree add from primary" 'git worktree add ../ois-wt/feat/4/z -b feat/4/z origin/next' "$PRIMARY"
allow $G "list branches in primary" 'git branch -vv' "$PRIMARY"
allow $G "delete a branch in primary" 'git branch -d old' "$PRIMARY"
allow $G "explicit git add" 'git add backend/src/a.rs web/src/b.ts'
# Wrapped or indirect pushes and merges (#744 review): next has no branch protection, so these
# shapes must not slip past.
block $G "push under timeout" 'timeout 600 git push origin next'
block $G "push under timeout with options" 'timeout -k 5 --preserve-status 600 git push origin next'
block $G "push under nice -n" 'nice -n 5 git push origin next'
block $G "push inside bash -c" "bash -c 'git push origin next'"
block $G "push inside sh -lc" 'sh -lc "cd /tmp && git push origin main"'
block $G "push inside eval (quoted)" "eval 'git push origin next'"
block $G "push inside eval (bare)" 'eval git push origin next'
block $G "push heads/next" 'git push origin heads/next'
block $G "push HEAD while on next" 'git push origin HEAD' "$PRIMARY"
block $G "push @ while on next" 'git push -u origin @' "$PRIMARY"
block $G "gh api PUT pulls/N/merge" 'gh api -X PUT repos/VATUSA/OIS/pulls/12/merge'
block $G "gh api --method=put /pulls/N/merge" 'gh api --method=put /repos/VATUSA/OIS/pulls/12/merge -f merge_method=squash'
block $G "gh api graphql mergePullRequest" "gh api graphql -f query='mutation { mergePullRequest(input: {pullRequestId: \"x\"}) { clientMutationId } }'"
allow $G "feature push inside bash -c" "bash -c 'git push -u origin feat/1/test-branch'"
allow $G "push HEAD from a feature worktree" 'git push -u origin HEAD'
allow $G "timeout on an unrelated command" 'timeout 600 cargo test --workspace'
allow $G "gh api GET merge status" 'gh api repos/VATUSA/OIS/pulls/12/merge'
allow $G "gh api read a PR" 'gh api repos/VATUSA/OIS/pulls/12'
allow $G "commit message that mentions a push to next" 'git commit -m "never git push origin next; gh pr merge is blocked"'
allow $G "heredoc body that mentions merging" "$(printf 'git commit -F - <<%sEOF%s\nchore: x\n\ngit push origin next && gh pr merge 3\nEOF' "'" "'")"
allow $G "gh pr view" 'gh pr view 12'
allow $G "unrelated command" 'ls -la'
malformed $G

# --- branch-naming-gate ----------------------------------------------------------------------------
G=branch-naming-gate
block $G "bare push from a badly named branch" 'git push' "$BADWT"
block $G "push -u of a badly named branch" 'git push -u origin Bad_Branch' "$BADWT"
block $G "missing issue number" 'git push origin HEAD:feat/save-event-replays'
block $G "unknown type" 'git push origin HEAD:feature/12/save-replays'
block $G "one-word description" 'git push origin HEAD:feat/12/replays'
block $G "five-word description" 'git push origin HEAD:feat/12/save-all-the-event-replays'
block $G "over 50 characters" 'git push origin HEAD:feat/12345/abcdefghijklmnopqrstu-vwxyzabcdefghijklmn'
allow $G "conforming branch" 'git push -u origin feat/1/test-branch'
allow $G "branch held in a variable" 'git push -u origin "$BRANCH"'
allow $G "bare push of a conforming branch" 'git push'
allow $G "rework branch" 'git push origin HEAD:chore/546/rework-d595743'
allow $G "delete a remote branch" 'git push origin --delete Bad_Branch'
allow $G "push a tag" 'git push origin v1.0'
allow $G "unrelated command" 'cargo test --workspace'
malformed $G

# --- attribution-gate ------------------------------------------------------------------------------
G=attribution-gate
printf 'fix: x\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\n' >"$WT/dirty-msg.txt"
printf 'fix: x\n\nCloses #1\n' >"$WT/clean-msg.txt"
printf '## Summary\nx\n\nhttps://claude.ai/code/session_01AbCdEf\n' >"$WT/dirty-body.md"
printf '## Summary\nx\n' >"$WT/clean-body.md"
block $G "commit -m with a Claude trailer" 'git commit -m "fix: x

Co-Authored-By: Claude <noreply@anthropic.com>"'
block $G "lower-case trailer" 'git commit -m "fix: x" -m "co-authored-by: claude"'
block $G "heredoc commit with a trailer" "$(printf 'git commit -F - <<%sEOF%s\nfix: x\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\nEOF' "'" "'")"
block $G "commit -F file with a trailer" 'git commit -F dirty-msg.txt'
block $G "commit --file=file with a trailer" 'git commit --file=dirty-msg.txt'
block $G "pr create --body with Generated with" 'gh pr create --title t --body "🤖 Generated with [Claude Code](https://claude.com/claude-code)"'
block $G "pr create --body-file with a session link" 'gh pr create --title t --body-file dirty-body.md'
block $G "pr edit -F with a session link" 'gh pr edit 5 -F dirty-body.md'
allow $G "clean commit -m" 'git commit -m "fix(flow): clamp descent ETA" -m "Closes #1"'
allow $G "clean commit -F" 'git commit -F clean-msg.txt'
allow $G "message file written by the same command" 'printf "fix: x\n" > new-msg.txt && git commit -F new-msg.txt'
allow $G "clean pr create" 'gh pr create --title t --body-file clean-body.md'
allow $G "trailer text outside a commit or PR" 'echo "Co-Authored-By: Claude" | wc -l'
allow $G "pr view" 'gh pr view 5'
# Bodies that reach gh/git without a plain path (#744 review).
printf 'fix: x\n\nCo-Authored-By: Someone <noreply@anthropic.com>\n' >"$WT/noreply-msg.txt"
block $G "noreply@anthropic.com alone" 'git commit -F noreply-msg.txt'
block $G "pr body from \$(cat file)" 'gh pr create --title t --body "$(cat dirty-body.md)"'
block $G "pr body from \$(< file)" 'gh pr edit 5 --body "$(< dirty-body.md)"'
block $G "commit message from \$(cat file)" 'git commit -m "$(cat dirty-msg.txt)"'
block $G "pr body piped to --body-file -" 'cat dirty-body.md | gh pr create --title t --body-file -'
block $G "pr body from /dev/stdin" 'gh pr create --title t --body-file /dev/stdin < clean-body.md'
block $G "commit message piped to -F -" 'cat clean-msg.txt | git commit -F -'
block $G "glued -Ffile" 'gh pr create --title t -Fdirty-body.md'
block $G "\$(cat file) that is missing" 'gh pr create --title t --body "$(cat no-such-body.md)"'
allow $G "pr body from \$(cat clean file)" 'gh pr create --title t --body "$(cat clean-body.md)"'
allow $G "commit -m from a heredoc substitution" "$(printf 'git commit -m "$(cat <<%sEOF%s\nfix: x\n\nCloses #1\nEOF\n)"' "'" "'")"
allow $G "body file written by the same heredoc command" "$(printf 'cat > new-body.md <<%sEOF%s\nclean\nEOF\ngh pr create --title t --body "$(cat new-body.md)"' "'" "'")"
malformed $G

# --- pre-pr-gate -----------------------------------------------------------------------------------
G=pre-pr-gate
block $G "no marker for HEAD" 'gh pr create --base next --title t --body b'
mkdir -p "$MARKERS"
echo "$(($(date +%s) - 8000))" >"$MARKERS/$WT_SHA"
block $G "marker older than 2h" 'gh pr create --base next --title t --body b'
echo "not-a-timestamp" >"$MARKERS/$WT_SHA"
block $G "unreadable marker" 'gh pr create --base next --title t --body b'
rm -f "$MARKERS/$WT_SHA"
date +%s >"$MARKERS/0000000000000000000000000000000000000000"
block $G "marker for a different commit" 'gh pr create --base next --title t --body b'
block $G "marker in the worktree's own .claude does not count" "mkdir -p .claude/markers/review-shipping && gh pr create --title t"
# The marker /review-before-shipping tells the reviewer to write is the one this gate reads: run that
# command's own snippet in the worktree, and the PR must pass (#744 review: nothing wrote it).
rm -f "$MARKERS/$WT_SHA"
writer="$(awk '/^```bash$/ {f = 1; next} /^```$/ {f = 0} f' "$HOOKS/../commands/review-before-shipping.md")"
(cd "$WT" && bash -c "$writer") >/dev/null 2>&1
allow $G "marker written by /review-before-shipping's snippet" 'gh pr create --base next --title t --body b'
date +%s >"$MARKERS/$WT_SHA"
allow $G "fresh marker for HEAD" 'gh pr create --base next --title t --body b'
allow $G "fresh marker, gh -R form" 'gh -R VATUSA/OIS pr create --base next --title t --body b'
allow $G "pr list" 'gh pr list --state open'
allow $G "unrelated command" 'git status --short'
malformed $G

# --- migration-gate --------------------------------------------------------------------------------
G=migration-gate
echo 'select 33;' >"$WT/backend/migrations/0003_mine.sql"
block $G "migration added in the same command collides" 'git add backend/migrations/0003_mine.sql && git commit -m "feat: x"'
git -C "$WT" add backend/migrations/0003_mine.sql
block $G "staged migration collides with an origin branch" 'git commit -m "feat: x"'
git -C "$WT" rm -q --cached backend/migrations/0003_mine.sql
mv "$WT/backend/migrations/0003_mine.sql" "$WT/backend/migrations/0004_mine.sql"
git -C "$WT" add backend/migrations/0004_mine.sql
allow $G "staged migration with a free number" 'git commit -m "feat: x"'
block $G "zero-padding does not hide a collision" 'git add backend/migrations/3_mine.sql && git commit -m x'
git -C "$WT" update-ref refs/remotes/origin/feat/1/test-branch "$(git -C "$WT" commit-tree -p HEAD -m own "$(git -C "$WT" write-tree)")"
allow $G "same file on your own remote branch" 'git commit -m "feat: x"'
git -C "$WT" rm -q --cached backend/migrations/0004_mine.sql
allow $G "commit without migrations" 'git commit -m "feat: x"'
allow $G "unrelated command" 'git status'
malformed $G

# --- reminders -------------------------------------------------------------------------------------
issue_payload() { jq -n --arg c "$1" --arg o "$2" '{hook_event_name: "PostToolUse", tool_name: "Bash", tool_input: {command: $c}, tool_response: {stdout: $o, stderr: ""}}'; }
R=issue-created-reminder
remind_case $R "board-status.sh 999" "after gh issue create" "$(issue_payload 'gh issue create --repo VATUSA/OIS --title t --body-file b.md' $'https://github.com/VATUSA/OIS/issues/999\n')"
remind_case $R "" "a grep that mentions gh issue create" "$(issue_payload 'grep -n "gh issue create" docs/github-issues.md' '')"
remind_case $R "" "gh issue view" "$(issue_payload 'gh issue view 5' '')"
remind_malformed $R

R=pr-created-reminder
remind_case $R "PR #1234" "after gh pr create" "$(issue_payload 'gh pr create --base next --title t --body b' $'https://github.com/VATUSA/OIS/pull/1234\n')"
remind_case $R "Testing Queue" "names the board column" "$(issue_payload 'gh pr create --base next --title t --body b' $'https://github.com/VATUSA/OIS/pull/1234\n')"
remind_case $R "ls-remote" "asks for the push proof" "$(issue_payload 'gh pr create --base next --title t --body b' '')"
remind_case $R "docs/github-issues.md § Comments" "points at the Moment 3 template" "$(issue_payload 'gh pr create --base next --title t --body b' '')"
remind_case $R "data path" "names the data path line" "$(issue_payload 'gh pr create --base next --title t --body b' '')"
remind_lacks $R "Drafted by Claude" "asks for no attribution footer" "$(issue_payload 'gh pr create --base next --title t --body b' '')"
remind_case $R "" "gh pr view" "$(issue_payload 'gh pr view 5' '')"
remind_malformed $R

R=plan-approved-reminder
remind_case $R "Moment 2" "after ExitPlanMode" '{"hook_event_name":"PostToolUse","tool_name":"ExitPlanMode","tool_input":{"plan":"x"}}'
remind_case $R "600 characters" "names the length cap" '{"tool_name":"ExitPlanMode","tool_input":{}}'
remind_case $R "Moment 2" "empty payload still reminds" ""
remind_lacks $R "Drafted by Claude" "asks for no attribution footer" '{"tool_name":"ExitPlanMode","tool_input":{}}'

edit_payload() { jq -n --arg f "$1" --arg n "$2" '{hook_event_name: "PostToolUse", tool_name: "Edit", tool_input: {file_path: $f, old_string: "a", new_string: $n}}'; }
R=client-regen-reminder
remind_case $R "codegen" "edit to router.rs" "$(edit_payload "$PRIMARY/backend/src/router.rs" 'x')"
remind_case $R "codegen" "edit to openapi.rs" "$(edit_payload "$PRIMARY/backend/src/openapi.rs" 'x')"
remind_case $R "codegen" "edit adding a ToSchema derive" "$(edit_payload "$PRIMARY/backend/src/handlers/flow.rs" '#[derive(Serialize, ToSchema)]')"
remind_case $R "codegen" "Write of a utoipa path" "$(jq -n --arg f "$PRIMARY/backend/src/handlers/x.rs" '{tool_name: "Write", tool_input: {file_path: $f, content: "#[utoipa::path(get)]"}}')"
remind_case $R "" "edit to a repo with no contract" "$(edit_payload "$PRIMARY/backend/src/repos/flow.rs" 'let x = 1;')"
remind_case $R "" "edit to the web app" "$(edit_payload "$PRIMARY/web/src/lib/flow.ts" 'ToSchema')"
remind_malformed $R

# --- commit-msg git hook ---------------------------------------------------------------------------
# A real commit in a throwaway repo, with core.hooksPath pointed at this checkout's .githooks.
REPO="$SANDBOX/commit-msg-repo"
git init -q "$REPO" && git -C "$REPO" config user.name test && git -C "$REPO" config user.email t@example.com
git -C "$REPO" config commit.gpgsign false && git -C "$REPO" config core.hooksPath "$GITHOOKS"
echo a >"$REPO/a" && git -C "$REPO" add a
hook_commit() {
    local want="$1" desc="$2" msg="$3" got
    # --no-verify would skip commit-msg too, so pre-commit runs; it passes on a repo with no .rs files.
    git -C "$REPO" commit -q -m "$msg" >/dev/null 2>&1
    got=$?
    if { [[ "$want" == reject ]] && [[ $got -ne 0 ]]; } || { [[ "$want" == accept ]] && [[ $got -eq 0 ]]; }; then
        pass=$((pass + 1))
    else
        fail=$((fail + 1))
        failures="$failures  FAIL commit-msg: $desc (want $want, git commit exit $got)"$'\n'
    fi
}
hook_commit reject "Co-Authored-By: Claude trailer" $'fix: x\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>'
hook_commit reject "Generated with Claude Code line" $'fix: x\n\n🤖 Generated with [Claude Code](https://claude.com/claude-code)'
hook_commit accept "clean message" $'fix: x\n\nCloses #1'

# --- review-scan.sh --------------------------------------------------------------------------------
# A miniature of OIS's layout: one base commit, then a branch that keeps every invariant (must scan
# clean) and a branch that breaks each one (must report each).
SCANNER="$HOOKS/../scripts/review-scan.sh"
SCAN="$SANDBOX/scan"
git init -q -b next "$SCAN" && git -C "$SCAN" config user.name test && git -C "$SCAN" config user.email t@example.com
git -C "$SCAN" config commit.gpgsign false
mkdir -p "$SCAN"/backend/src/{auth,repos,handlers} "$SCAN"/backend/migrations "$SCAN"/crates/ois-core/src
cat >"$SCAN/backend/src/auth/permissions.rs" <<'RS'
permission!(TmuTmiRead, ["tmu", "tmi"], Read);
permission!(TmuTmiUpdate, ["tmu", "tmi"], Update);
RS
cat >"$SCAN/crates/ois-core/src/catalog.rs" <<'RS'
pub fn default_roles() -> Vec<&'static str> {
    vec![
        SERVER_ADMIN_ROLE,
        "USER",
    ]
}

pub fn draft_new_permission_names() -> Vec<&'static str> {
    vec![
        "tmu.tmi.read",
        "tmu.tmi.update",
    ]
}
RS
cat >"$SCAN/backend/src/repos/access.rs" <<'RS'
pub const ASSIGNABLE_USER_ROLES: &[&str] = &[
    "USER",
];
RS
cat >"$SCAN/backend/migrations/0001_init.sql" <<'SQL'
insert into access.permissions (name, description) values
    ('tmu.tmi.read', 'Read TMIs'),
    ('tmu.tmi.update', 'Update TMIs');
insert into access.roles (name, description) values
    ('USER', 'Everyone');
SQL
cat >"$SCAN/backend/src/router.rs" <<'RS'
use crate::handlers::{jobs as jobs_handler, tmu};
pub fn build_router() -> Router {
    Router::new()
        .route("/api/v1/tmu/tmis", get(tmu::list_tmis))
        .route("/api/v1/tmu/tmis/{id}", patch(tmu::update_tmi))
}
RS
cat >"$SCAN/backend/src/openapi.rs" <<'RS'
#[openapi(
    paths(
        crate::handlers::tmu::list_tmis,
        crate::handlers::tmu::update_tmi,
    )
)]
pub struct ApiDoc;
RS
cat >"$SCAN/backend/src/handlers/tmu.rs" <<'RS'
pub async fn list_tmis(
    _permission: RequirePermission<TmuTmiRead>,
) -> Result<Json<Vec<Tmi>>, ApiError> {
    tmu_repo::list().await
}

pub async fn update_tmi(
    _permission: RequirePermission<TmuTmiUpdate>,
    Path(id): Path<String>,
) -> Result<Json<Tmi>, ApiError> {
    tmu_repo::update(&id).await
}

#[cfg(test)]
mod tests {
    #[test]
    fn t() {}
}
RS
git -C "$SCAN" add -A >/dev/null && git -C "$SCAN" commit -qm base

# Clean branch: a fully wired permission + route, an aliased router module, and test-only SQL/unwrap.
git -C "$SCAN" checkout -q -b clean
perl -pi -e 'print "permission!(TmuTmiCreate, [\"tmu\", \"tmi\"], Create);\n" if $. == 1' "$SCAN/backend/src/auth/permissions.rs"
perl -pi -e 's/^(\s+)"tmu.tmi.read",/$1"tmu.tmi.read",\n$1"tmu.tmi.create",/' "$SCAN/crates/ois-core/src/catalog.rs"
printf "insert into access.permissions (name, description) values\n    ('tmu.tmi.create', 'Create TMIs');\n" >"$SCAN/backend/migrations/0002_tmi_create.sql"
perl -pi -e 's/^(\s+)\.route\("\/api\/v1\/tmu\/tmis", get\(tmu::list_tmis\)\)/$1.route("\/api\/v1\/tmu\/tmis", get(tmu::list_tmis).post(tmu::create_tmi))\n$1.route("\/api\/v1\/jobs", get(jobs_handler::list_jobs))/' "$SCAN/backend/src/router.rs"
perl -pi -e 's/^(\s+)crate::handlers::tmu::list_tmis,/$1crate::handlers::tmu::list_tmis,\n$1crate::handlers::tmu::create_tmi,\n$1crate::handlers::jobs::list_jobs,/' "$SCAN/backend/src/openapi.rs"
perl -0pi -e 's/#\[cfg\(test\)\]/pub async fn create_tmi(\n    _permission: RequirePermission<TmuTmiCreate>,\n    Query(query): Query<CreateQuery>,\n) -> Result<Json<Tmi>, ApiError> {\n    tmu_repo::create().await\n}\n\nasync fn reload(pool: &sqlx::PgPool) -> Result<(), ApiError> {\n    tmu_repo::load(pool).await\n}\n\n#[cfg(test)]/; s/fn t\(\) \{\}/fn t() {\n        let n: i64 = sqlx::query_scalar("select 1").fetch_one(p).await.unwrap();\n    }/' "$SCAN/backend/src/handlers/tmu.rs"
git -C "$SCAN" add -A >/dev/null && git -C "$SCAN" commit -qm clean
scan_out="$(cd "$SCAN" && bash "$SCANNER" next 2>&1)"
scan_rc=$?
if [[ $scan_rc -eq 0 ]]; then
    pass=$((pass + 1))
else
    fail=$((fail + 1))
    failures="$failures  FAIL review-scan: a branch that keeps every invariant should scan clean (exit $scan_rc)"$'\n'"$(printf '%s\n' "$scan_out" | sed 's/^/      /')"$'\n'
fi

# Dirty branch: one break per check.
git -C "$SCAN" checkout -q -b dirty next
perl -pi -e 'print "permission!(FlowFooUpdate, [\"flow\", \"foo\"], Update);\n" if $. == 1' "$SCAN/backend/src/auth/permissions.rs"
perl -pi -e 's/^(\s+)"tmu.tmi.read",/$1"tmu.tmi.read",\n$1"flow.bar.read",/; s/^(\s+)"USER",/$1"USER",\n$1"OTHER_ROLE",/' "$SCAN/crates/ois-core/src/catalog.rs"
perl -pi -e 's/^(\s+)"USER",/$1"USER",\n$1"THIRD_ROLE",/' "$SCAN/backend/src/repos/access.rs"
printf "insert into access.permissions (name, description) values\n    ('flow.baz.read', 'x');\ninsert into access.roles (name, description) values\n    ('NEW_ROLE', 'x');\n" >"$SCAN/backend/migrations/0002_flow.sql"
echo "-- edited after it shipped" >>"$SCAN/backend/migrations/0001_init.sql"
perl -pi -e 's/^(\s+)\.route\("\/api\/v1\/tmu\/tmis", get\(tmu::list_tmis\)\)/$1.route("\/api\/v1\/tmu\/tmis", get(tmu::list_tmis))\n$1.route("\/api\/v1\/tmu\/purge", delete(tmu::purge_tmis))/' "$SCAN/backend/src/router.rs"
perl -pi -e 's/^(\s+)crate::handlers::tmu::list_tmis,/$1crate::handlers::tmu::list_tmis,\n$1crate::handlers::tmu::orphan_handler,/' "$SCAN/backend/src/openapi.rs"
perl -0pi -e 's/    _permission: RequirePermission<TmuTmiUpdate>,\n//; s/#\[cfg\(test\)\]/pub async fn purge_tmis(State(state): State<AppState>) -> Result<Json<()>, ApiError> {\n    let rows = sqlx::query("delete from tmu.tmis").execute(&state.db).await;\n    let n = rows.unwrap();\n    Ok(Json(()))\n}\n\n#[cfg(test)]/' "$SCAN/backend/src/handlers/tmu.rs"
git -C "$SCAN" add -A >/dev/null && git -C "$SCAN" commit -qm dirty
scan_out="$(cd "$SCAN" && bash "$SCANNER" next 2>&1)"
scan_rc=$?
scan_expect() {
    if [[ $scan_rc -eq 1 ]] && printf '%s\n' "$scan_out" | grep -qE "$1"; then
        pass=$((pass + 1))
    else
        fail=$((fail + 1))
        failures="$failures  FAIL review-scan: expected a finding matching /$1/ (exit $scan_rc)"$'\n'"$(printf '%s\n' "$scan_out" | sed 's/^/      /')"$'\n'
    fi
}
scan_expect '^backend/src/auth/permissions.rs:1: permission flow.foo.update has a marker but no "flow.foo.update" in'
scan_expect '^backend/src/auth/permissions.rs:1: permission flow.foo.update has a marker but no access.permissions row'
scan_expect '^crates/ois-core/src/catalog.rs:[0-9]+: permission flow.bar.read is in the catalog but no migration'
scan_expect '^crates/ois-core/src/catalog.rs:[0-9]+: role OTHER_ROLE is in default_roles\(\) but no migration'
scan_expect '^backend/src/repos/access.rs:3: role THIRD_ROLE is assignable but missing from default_roles'
scan_expect '^backend/src/repos/access.rs:3: role THIRD_ROLE is assignable but no migration'
scan_expect '^backend/migrations/0002_flow.sql:2: permission flow.baz.read is inserted but missing'
scan_expect '^backend/migrations/0002_flow.sql:4: role NEW_ROLE is inserted but missing from default_roles'
scan_expect '^backend/migrations/0002_flow.sql:4: role NEW_ROLE is not in ASSIGNABLE_USER_ROLES'
scan_expect '^backend/migrations/0001_init.sql:1: applied migration edited in place'
scan_expect '^backend/src/router.rs:[0-9]+: route tmu::purge_tmis is not registered in backend/src/openapi.rs'
scan_expect '^backend/src/handlers/tmu.rs:[0-9]+: mutating route \(delete\) tmu::purge_tmis takes no RequirePermission'
scan_expect '^backend/src/openapi.rs:[0-9]+: path tmu::orphan_handler is in the OpenAPI document but no route'
scan_expect '^backend/src/handlers/tmu.rs:[0-9]+: SQL in a handler'
scan_expect '^backend/src/handlers/tmu.rs:[0-9]+: unwrap\(\)/expect\(\) in a handler'
scan_expect '^backend/src/handlers/tmu.rs:8: RequirePermission<TmuTmiUpdate> was removed'
if (cd "$SCAN" && bash "$SCANNER" no-such-ref >/dev/null 2>&1); [[ $? -eq 2 ]]; then
    pass=$((pass + 1))
else
    fail=$((fail + 1))
    failures="$failures  FAIL review-scan: an unknown base ref must exit 2, not pass"$'\n'
fi

# --- cost on an unrelated Bash call (informational) -------------------------------------------------
# Claude Code runs a matcher's hooks in parallel, so the added latency is about the slowest single
# gate; the sequential sum is the worst case. Machine load moves these numbers, so they never fail.
TIMEFORMAT='%R'
for cmd in 'cargo test --workspace -- --test-threads=1' 'git status --short && git log --oneline -3'; do
    payload="$(bash_payload "$cmd" "$WT")"
    total="$({ time (for _ in 1 2 3 4 5 6 7 8 9 10; do
        for g in git-safety-gate branch-naming-gate attribution-gate pre-pr-gate migration-gate; do
            printf '%s' "$payload" | bash "$HOOKS/$g.sh" >/dev/null 2>&1
        done
    done); } 2>&1)"
    echo "timing ($cmd): all 5 gates in sequence ~$(awk -v t="$total" 'BEGIN { printf "%d", t * 100 }') ms per call"
done

# --- summary ---------------------------------------------------------------------------------------
if [[ $fail -gt 0 ]]; then
    printf '%s' "$failures"
fi
echo "hook tests: $pass passed, $fail failed"
[[ $fail -eq 0 ]]
