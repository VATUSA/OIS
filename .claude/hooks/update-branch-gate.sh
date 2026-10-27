#!/bin/bash
# PreToolUse hook: allow a PR's update-branch only when that PR's base is next (or main) and its head
# is not a protected branch.
#
# GitHub's update-branch merges the PR's OWN base into it, so on a PR based on
# another issue's branch it merges that issue's work in — the contamination
# branch-ancestry-gate.sh stops for a local merge. On a PR whose head is next or
# main (the release PR) it writes that protected branch, which no agent does.
# Every PR the command names is looked up on GitHub; anything unverifiable blocks.
#
# Blocking contract: only `exit 2` blocks, and Claude Code reads stderr then.
#
# Tests: bin/lib/update-branch-gate.test.sh
exec >&2

HOOK_LIB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/tool-input.sh"
if [[ ! -f "$HOOK_LIB" ]]; then
    echo "BLOCKED: cannot locate .claude/hooks/lib/tool-input.sh — refusing to pass the command unchecked."
    exit 2
fi
# shellcheck source=lib/tool-input.sh
. "$HOOK_LIB"

INPUT=$(cat)
COMMAND="$(printf '%s' "$INPUT" | require_tool_input_field command)" || exit 2

policy "$(dirname "${BASH_SOURCE[0]}")/lib/git_policy.py" update-branch
STATUS=$?
TARGETS="$POLICY_OUT"
if [[ $STATUS -ne 0 ]]; then
    echo "BLOCKED: ${POLICY_WHY:-the command could not be parsed}, so a PR update-branch in it cannot be checked."
    exit 2
fi
if [[ -z "$TARGETS" ]]; then
    exit 0
fi

while IFS= read -r target; do
    if [[ "$target" == "?" ]]; then
        echo "BLOCKED: this updates a PR branch the gate cannot verify (a variable, branch name, --rebase or GraphQL)."
        echo "Use: gh api -X PUT repos/<owner>/<repo>/pulls/<number>/update-branch -f expected_head_sha=<sha>"
        exit 2
    fi
    repo="${target%%#*}"
    number="${target##*#}"
    refs="$(gh pr view "$number" ${repo:+-R "$repo"} --json baseRefName,headRefName \
        -q '.baseRefName + " " + .headRefName' 2>/dev/null)"
    status=$?
    base="${refs%% *}"
    head="${refs#* }"
    if [[ $status -ne 0 || -z "$base" || "$refs" != *" "* || -z "$head" ]]; then
        echo "BLOCKED: could not read the base and head branches of PR #$number from GitHub, so its update-branch cannot be checked."
        exit 2
    fi
    if [[ "$base" != "next" && "$base" != "main" ]]; then
        echo "BLOCKED: PR #$number is based on '$base', not next. update-branch would merge '$base' into it,"
        echo "carrying another issue's work into this PR. Retarget the PR to next first, or leave it."
        exit 2
    fi
    if [[ "$head" == "next" || "$head" == "main" ]]; then
        echo "BLOCKED: PR #$number opens from '$head'. update-branch would merge '$base' into $head, a protected"
        echo "branch no agent writes. The release back-merge is a human's call."
        exit 2
    fi
done <<EOF
$TARGETS
EOF

exit 0
