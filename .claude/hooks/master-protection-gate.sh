#!/bin/bash
# PreToolUse hook: Block pushing to next or main and merging PRs from the CLI or the GitHub API; block
# creating, renaming or switching a branch in the primary checkout (gh pr checkout too), and staging the whole tree.
#
# Blocking contract: Claude Code blocks a tool call ONLY on `exit 2`, and reads
# stderr when it does — stdout is discarded. Any other non-zero code, including
# 1, is a non-blocking error and the tool runs anyway. `exec >&2` sends
# everything below to stderr so a message can't land on the wrong stream by
# mistake; command substitutions set up their own fd 1 and are unaffected.
#
# next is the integration branch every PR targets and main is promoted from it by a release PR a
# human opens: both change only through a reviewed PR a human merges. The GitHub Actions workflows
# in .github/workflows/ are CI's, not this gate's.
#
# Tests: bin/lib/master-protection-gate.test.sh
exec >&2

HOOK_LIB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/tool-input.sh"
if [[ ! -f "$HOOK_LIB" ]]; then
    echo "BLOCKED: cannot locate .claude/hooks/lib/tool-input.sh — refusing to pass the command unchecked."
    exit 2
fi
# shellcheck source=lib/tool-input.sh
. "$HOOK_LIB"

INPUT=$(cat)

# Fails closed: a parse fault used to print '' and exit 0, which matched none of
# the patterns below and allowed the command through.
COMMAND="$(printf '%s' "$INPUT" | require_tool_input_field command)" || exit 2

# Parsed fine, but there is no command to inspect — nothing to block.
if [[ -z "$COMMAND" ]]; then
    exit 0
fi

LIB="$(dirname "${BASH_SOURCE[0]}")/lib"

# The line is parsed, not grepped: a heredoc or quoted argument naming a command is
# data, while `git -C dir push`, `bash -c`, `eval` and `$(...)` still count. Any exit
# other than 0 from the parser (unparseable line, python3 missing) blocks.

# invoked WORD... -> 0 runs it, 1 does not, anything else: block, with the parser's reason in POLICY_WHY
invoked() {
    policy "$LIB/command_match.py" "$@"
}

# --- Block 1: Never push to next or main (git push, or git subtree push, which pushes to a named branch) ---
invoked git push
PUSH=$?
if [[ $PUSH -eq 1 ]]; then
    invoked git subtree push
    PUSH=$?
fi
case $PUSH in
    0)
        CURRENT_BRANCH=$(git rev-parse --abbrev-ref HEAD 2>/dev/null)
        policy "$LIB/git_policy.py" protected-push "$CURRENT_BRANCH"
        STATUS=$?
        REASON="$POLICY_OUT"
        if [[ $STATUS -ne 0 ]]; then
            REASON="${POLICY_WHY:-the push could not be parsed}, so it cannot be shown to leave next and main alone"
        fi
        if [[ -n "$REASON" ]]; then
            echo ""
            echo "BLOCKED: Pushing directly to next or main is not allowed ($REASON)."
            echo "Push a feature branch and open a PR: gh pr create --base next"
            exit 2
        fi
        ;;
    1) ;;
    *) echo "BLOCKED: ${POLICY_WHY:-the command could not be parsed}; refusing to pass it unchecked."; exit 2 ;;
esac

# --- Block 2: Never merge a PR via CLI ---
invoked gh pr merge
case $? in
    1) ;;
    0)
        echo ""
        echo "BLOCKED: Merging PRs via CLI is not allowed."
        echo "PRs are merged manually through the GitHub UI after human review."
        exit 2
        ;;
    *)
        echo ""
        echo "BLOCKED: ${POLICY_WHY:-the command could not be parsed}, so a gh pr merge in it cannot be ruled out."
        echo "PRs are merged manually through the GitHub UI after human review."
        exit 2
        ;;
esac

# --- Block 3: nor through the API. Any gh api write (REST non-GET, GraphQL mutation or an unreadable
# GraphQL document) can merge or write next or main, so only the allowlist in git_policy.py passes, and
# GraphQL only as a literal read. bin/board-status.sh makes the board's one mutation inside the script ---
policy "$LIB/git_policy.py" api-write
STATUS=$?
REASON="$POLICY_OUT"
if [[ $STATUS -ne 0 || -n "$REASON" ]]; then
    echo ""
    echo "BLOCKED: GitHub API writes are not allowed from here (${REASON:-${POLICY_WHY:-the command could not be parsed}})."
    echo "PRs are merged manually through the GitHub UI after human review; use gh pr/issue commands and"
    echo "bin/board-status.sh for everything else, or ask the user to add the endpoint to API_WRITE_ALLOWLIST."
    exit 2
fi

# --- Block 4: the primary checkout stays on next. A branch is born in a worktree (`git worktree add -b`,
# or EnterWorktree), never by creating, renaming or switching one in the primary checkout ---
policy "$LIB/git_policy.py" primary-branch
STATUS=$?
REASON="$POLICY_OUT"
if [[ $STATUS -ne 0 || -n "$REASON" ]]; then
    echo ""
    echo "BLOCKED: ${REASON:-${POLICY_WHY:-the command could not be parsed}, so a branch change in it cannot be ruled out}."
    echo "The primary checkout stays on next. Do issue work in a worktree (/start, or"
    echo "git worktree add <path> -b <branch> origin/next) and rename or switch branches there."
    exit 2
fi

# --- Block 5: stage explicit paths, so nothing unreviewed rides along in a commit (git add -A, ., *, a bare
# -u, git commit -a; git_policy.py stages_everything) ---
policy "$LIB/git_policy.py" stage-all
STATUS=$?
REASON="$POLICY_OUT"
if [[ $STATUS -ne 0 || -n "$REASON" ]]; then
    echo ""
    echo "BLOCKED: ${REASON:-${POLICY_WHY:-the command could not be parsed}, so staging the whole tree cannot be ruled out}."
    echo "Stage explicit paths so nothing unreviewed is committed: git add path/one path/two, then"
    echo "git commit without -a."
    exit 2
fi

exit 0
