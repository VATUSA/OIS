#!/usr/bin/env bash
# shellcheck source-path=SCRIPTDIR
# shellcheck disable=SC2329  # inspect() and its helpers run through for_each_command
# PreToolUse(Bash) gate: `gh pr create` only for a commit that /review-before-shipping reviewed.
#
# The review writes a marker named for the full SHA it reviewed:
#   <primary checkout>/.claude/markers/review-shipping/<40-char HEAD sha>
# holding the unix time it ran. It lives under the PRIMARY checkout's .claude (resolved through
# `git rev-parse --git-common-dir`), so a review run from any worktree is found from any other.
# A new commit moves HEAD off the marker and re-blocks until that commit is reviewed; a marker older
# than two hours is stale.
# Exit 2 blocks (message on stderr); a payload it cannot read blocks too.
exec >&2
# shellcheck source=lib/tool-input.sh
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/tool-input.sh" || exit 2

gate_read_command || exit 2
case "$COMMAND" in *gh*) ;; *) exit 0 ;; esac

MAX_AGE=7200

inspect() {
    gh_is pr create || return 0
    local sha marker stamp now age
    if ! sha="$(git -C "$SEG_DIR" rev-parse HEAD 2>/dev/null)" || [[ -z "$sha" ]]; then
        echo "BLOCKED: cannot resolve HEAD in $SEG_DIR, so cannot tell whether it was reviewed."
        return 2
    fi
    marker="$(primary_claude_dir "$SEG_DIR")/markers/review-shipping/$sha"
    if [[ ! -f "$marker" ]]; then
        echo "BLOCKED: commit ${sha:0:12} has not been reviewed (no marker at $marker)."
        echo "  Run /review-before-shipping on this HEAD; it writes the marker when the review is clean."
        echo "  If you committed a fix after reviewing, HEAD moved: review again."
        return 2
    fi
    stamp="$(cat "$marker" 2>/dev/null)"
    [[ "$stamp" =~ ^[0-9]+$ ]] || stamp=0
    now="$(date +%s)"
    age=$((now - stamp))
    if [[ $age -gt $MAX_AGE || $age -lt 0 ]]; then
        echo "BLOCKED: the review marker for ${sha:0:12} is stale or unreadable ($((age / 60)) min old; limit $((MAX_AGE / 60)))."
        echo "  Re-run /review-before-shipping."
        return 2
    fi
    return 0
}

for_each_command inspect || exit 2
exit 0
