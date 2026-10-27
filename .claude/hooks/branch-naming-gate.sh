#!/bin/bash
# PreToolUse hook: Validate branch naming convention before push.
#
# Blocking contract: Claude Code blocks a tool call ONLY on `exit 2`, reading
# stderr. Exit 1 is a non-blocking error and the push proceeds anyway — this gate
# shipped with `exit 1` + stdout and therefore blocked nothing. `exec >&2` keeps
# every message on the stream Claude reads.
#
# Tests: bin/lib/branch-naming-gate.test.sh
exec >&2

HOOK_LIB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/tool-input.sh"
if [[ ! -f "$HOOK_LIB" ]]; then
    echo "BLOCKED: cannot locate .claude/hooks/lib/tool-input.sh — refusing to pass the command unchecked."
    exit 2
fi
# shellcheck source=lib/tool-input.sh
. "$HOOK_LIB"

INPUT=$(cat)

# Fails closed: a parse fault used to print '' and exit 0, allowing the push.
COMMAND="$(printf '%s' "$INPUT" | require_tool_input_field command)" || exit 2

# Only check push commands. The line is parsed, so a heredoc or quoted argument naming
# the command passes while `git -C dir push` and `cd x && git push` are checked; a line
# the parser cannot read, or refuses, blocks with its reason.
LIB="$(dirname "${BASH_SOURCE[0]}")/lib"
policy "$LIB/command_match.py" git push
case $? in
    0) ;;
    1) exit 0 ;;
    *)
        echo "BLOCKED: ${POLICY_WHY:-the command could not be parsed}, so a push in it cannot be checked."
        exit 2
        ;;
esac

# The branch is read where the push runs: the hook's cwd, or DIR for `cd DIR && git push`. Any other
# cd, -C, --git-dir or GIT_DIR on the line (or a parse failure) means the gate cannot tell which branch.
policy "$LIB/git_policy.py" push-checkout && WHERE="$POLICY_OUT" || WHERE="?"
case "$WHERE" in
    '') ;;
    '='*) cd "${WHERE#=}" 2>/dev/null || { echo "BLOCKED: cannot cd to ${WHERE#=} to check its branch name."; exit 2; } ;;
    *)
        echo "BLOCKED: this line changes directory or repository before git push, so the branch name cannot be checked."
        echo "Run git push from the checkout, or as \`cd DIR && git push ...\`."
        exit 2
        ;;
esac

# check_name NAME — 0 when NAME is next, main or follows the convention; prints why and returns 2 otherwise.
check_name() {
    local name="$1"
    # next and main are master-protection-gate.sh's to refuse, not a naming question.
    if [[ "$name" == "next" ]] || [[ "$name" == "main" ]]; then
        return 0
    fi
    # {type}/{issue-or-hash}/{description}, the description 2-4 hyphenated words. The
    # middle segment is REQUIRED: an issue number where there is an issue, a short hash
    # where there is not, so every branch says what it belongs to. Kept byte-identical
    # to bin/pre-push -- two gates disagreeing is what let the issue-numbered format be
    # blocked for months.
    if [[ "$name" =~ ^(feat|fix|chore)/[a-z0-9]+/[a-z0-9]+(-[a-z0-9]+){1,3}$ ]]; then
        if [[ ${#name} -gt 50 ]]; then
            echo "BLOCKED: Branch name too long (${#name} chars, max 50): $name"
            echo "Keep the description to 2-4 words. Example: fix/569/duplicate-migration-check"
            echo "Rename with: git branch -m $name <shorter-name>"
            return 2
        fi
        return 0
    fi
    echo "BLOCKED: Branch '$name' does not match naming convention: {type}/{issue-or-hash}/{description}, a 2-4 word description"
    echo "Examples: feat/47/save-event-replays, fix/569/duplicate-migration-check, chore/a1b2c3/update-deps"
    echo "The middle segment is required: the issue number, or a short hash when there is no issue."
    if [[ "$name" == worktree-* ]]; then
        echo "A worktree-* branch is the name EnterWorktree gave it: rename it before the first push, as /start does."
    fi
    echo "Keep it SHORT (max 50 chars, 2-4 word description)."
    echo "Rename with: git branch -m $name {type}/{issue-or-hash}/{description}"
    return 2
}

BRANCH=$(git rev-parse --abbrev-ref HEAD 2>/dev/null)
check_name "$BRANCH" || exit 2

# A refspec naming its destination (`git push origin HEAD:feat/save-replays`) creates that branch on origin,
# whatever this checkout is on, so each named destination is held to the same convention. Tags, deletions
# and names the shell builds are not branch names this gate can judge.
policy "$LIB/git_policy.py" push-destinations
if [[ $? -ne 0 ]]; then
    echo "BLOCKED: ${POLICY_WHY:-the command could not be parsed}, so the branches it pushes cannot be checked."
    exit 2
fi
while IFS= read -r destination; do
    [[ -n "$destination" ]] || continue
    git show-ref --verify --quiet "refs/tags/$destination" 2>/dev/null && continue
    check_name "$destination" || exit 2
done <<EOF
$POLICY_OUT
EOF
exit 0
