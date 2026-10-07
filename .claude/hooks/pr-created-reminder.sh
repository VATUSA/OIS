#!/usr/bin/env bash
# shellcheck source-path=SCRIPTDIR
# shellcheck disable=SC2329  # inspect() and its helpers run through for_each_command
# PostToolUse(Bash) reminder: what has to happen once `gh pr create` succeeded (Moment 3).
# The blocking decision belongs to pre-pr-gate.sh; this only says what comes next.
# Advisory: always exits 0, and stays quiet on a payload it cannot read.
# shellcheck source=lib/tool-input.sh
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/tool-input.sh" 2>/dev/null || exit 0

INPUT="$(cat)"
COMMAND="$(printf '%s' "$INPUT" | hook_json '.tool_input.command // empty')"
[[ "$COMMAND" == *pr* ]] || exit 0

FOUND=0
inspect() { gh_is pr create && FOUND=1; return 0; }
for_each_command inspect
[[ $FOUND -eq 1 ]] || exit 0

# Read the PR number from what gh printed. Never predict it: a concurrent session takes "the next one".
PR="$(printf '%s' "$INPUT" | hook_json '.tool_response | if type == "object" then (.stdout // "") else tostring end' |
    grep -oE '/pull/[0-9]+' | tail -1)"
PR="${PR##*/}"
PR="${PR:-<number from gh output>}"

remind "ISSUE REMINDER, Moment 3 (work complete) for PR #$PR:
1. Prove the push: \`git ls-remote origin refs/heads/<branch>\` must print \`git rev-parse HEAD\`.
2. Post ONE comment on the issue, at most 1,200 characters; assert the
   length before posting. Draft to the template in docs/github-issues.md § Comments: the three
   Moments (check steps naming their mode, blast radius, data path, deploy note). No attribution
   footer or \"Drafted by\" line.
3. Move the card: .claude/scripts/board-status.sh <n> \"Testing Queue\". Never Shippable or Done.
See .claude/commands/ship.md § Step 7."
