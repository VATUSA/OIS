#!/bin/bash
# PreToolUse hook: Block PR creation unless /review-before-shipping passed for the
# EXACT commit the PR opens on and every review lens attested every rule heading it owns.
#
# The marker is SHA-keyed (.claude/markers/review-shipping/<sha>): a new
# commit moves HEAD, so the marker no longer matches and the gate re-blocks
# until that commit is reviewed. The commit is the PR's head: the current
# branch, or the branch `--head` names, as origin holds it.
#
# Blocking contract: Claude Code blocks a tool call ONLY on `exit 2`, reading
# stderr. Exit 1 is a non-blocking error and the tool runs anyway. `exec >&2`
# also captures check-marker.sh, which echoes its BLOCKED messages to stdout.
exec >&2

HOOK_LIB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/tool-input.sh"
if [[ ! -f "$HOOK_LIB" ]]; then
    echo "BLOCKED: cannot locate .claude/hooks/lib/tool-input.sh — refusing to pass the command unchecked."
    exit 2
fi
# shellcheck source=lib/tool-input.sh
. "$HOOK_LIB"

INPUT=$(cat)

# Fails closed: a parse fault used to print '' and exit 0, allowing PR creation.
COMMAND="$(printf '%s' "$INPUT" | require_tool_input_field command)" || exit 2

# Only gate PR creation (gh pr create, its aliases, or a gh api call that opens a PR). git_policy.py
# parses the line, so a heredoc or quoted argument mentioning the command does not trip the gate.
# It prints nothing for no PR, `=BRANCH` for the head (`=` alone: the current branch) or `?REASON`,
# reading the repository in the current directory. Any other exit (unparseable, no python3) blocks.
pr_head_of_command() {
    policy "$HOOK_DIR/lib/git_policy.py" pr-head && PR_HEAD="$POLICY_OUT" && return 0
    echo "BLOCKED: ${POLICY_WHY:-the command could not be parsed}, so a PR it opens cannot be checked."
    exit 2
}
HOOK_DIR="$(dirname "${BASH_SOURCE[0]}")"
HOOK_DIR="$(cd "$HOOK_DIR" && pwd)"
pr_head_of_command
[[ -n "$PR_HEAD" ]] || exit 0

# The session cwd is shared across worktrees, so `cd DIR && gh pr create` is checked in DIR. command_match.py
# follows only that exact shape; git_policy.py pr-head refuses any other cd, -C or GIT_DIR on the line.
CD_DIR="$(printf '%s' "$COMMAND" | python3 -I "$HOOK_DIR/lib/command_match.py" --cd-target gh pr create 2>/dev/null)"
if [[ -n "$CD_DIR" ]]; then
    cd "$CD_DIR" 2>/dev/null || { echo "BLOCKED: cannot cd to $CD_DIR to check its review marker."; exit 2; }
    pr_head_of_command
fi
case "$PR_HEAD" in
    '?'*) echo "BLOCKED: ${PR_HEAD#\?}."; exit 2 ;;
    '='*) ;;
    *) echo "BLOCKED: unexpected PR head '$PR_HEAD' from git_policy.py."; exit 2 ;;
esac

# Libraries come from beside this hook, which settings.json always runs from the primary
# checkout, never from the branch under review: a branch must not ship its own lenient checker.
MAIN_ROOT="$(cd "$HOOK_DIR/../.." && pwd)"

CHECK_MARKER="$MAIN_ROOT/bin/check-marker.sh"
if [[ ! -f "$CHECK_MARKER" ]]; then
    echo "pre-pr-gate: cannot locate bin/check-marker.sh — refusing to pass the PR unchecked."
    exit 2
fi
# shellcheck source=../../bin/check-marker.sh
source "$CHECK_MARKER"
pr_head_sha "${PR_HEAD#=}" || exit 2

CLAUDE_ONLY_LIB="$MAIN_ROOT/bin/lib/claude-only-change.sh"
if [[ -f "$CLAUDE_ONLY_LIB" ]]; then
    # shellcheck source=../../bin/lib/claude-only-change.sh
    . "$CLAUDE_ONLY_LIB"
    if claude_only_change "$PR_HEAD_SHA"; then
        echo "pre-pr-gate: skipped — the PR changes only agent guidance."
        exit 0
    fi
fi

check_marker_for_head "review-shipping" "Review" "/review-before-shipping" "$PR_HEAD_SHA" || exit 2

# The marker says the review ran; the attestation says every lens checked every rule heading.
ATTESTATION="$MAIN_ROOT/bin/lib/review-attestation.sh"
if [[ ! -f "$ATTESTATION" ]]; then
    echo "pre-pr-gate: cannot locate bin/lib/review-attestation.sh — refusing to pass the PR unchecked."
    exit 2
fi
# shellcheck source=../../bin/lib/review-attestation.sh
source "$ATTESTATION"
if ! check_attestation_for_head "$PR_HEAD_SHA"; then
    # The hook is the primary checkout's copy, but the review ran with the branch's own commands, which write
    # attestations only once the branch has the lens map.
    if ! git cat-file -e "$PR_HEAD_SHA:.claude/review-lenses.tsv" 2>/dev/null; then
        echo "This branch predates .claude/review-lenses.tsv: git merge origin/next, then re-run /review-before-shipping."
    fi
    exit 2
fi
exit 0
