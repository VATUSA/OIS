#!/usr/bin/env bash
# The AI-attribution patterns OIS never lets into a commit, PR, issue or comment. Sourced by
# .claude/hooks/attribution-gate.sh (the Claude hook) and .githooks/commit-msg (the git hook), so the
# two cannot drift apart. Nothing is credited to an agent: AGENTS.md § Git workflow.
# Sourced, never run; side-effect-free at source time.

# A "Drafted by Claude Code" footer counts only as a line of its own: markup, an emoji (raw, or
# escaped as `\uD83E` / `\U0001F916` / `\xf0`), a link or a parenthesised note, and punctuation may
# wrap it, but no words. So the footer is caught while prose that names it (a decision saying
# "remove the Drafted by Claude Code footer") still passes.
_ATTRIBUTION_PAD='([^[:alnum:]]|</?[a-z]+>|\\[uU][0-9a-f]{4,8}|\\x[0-9a-f]{2}|\\[rt])*'
_ATTRIBUTION_DRAFTED="^${_ATTRIBUTION_PAD}drafted (by|with)[[:space:]]+${_ATTRIBUTION_PAD}claude( code)?${_ATTRIBUTION_PAD}((\\([^)]*\\)|https?://[^[:space:]]*)${_ATTRIBUTION_PAD})?\$"

# One extended regex, matched case-insensitively, one line at a time.
ATTRIBUTION_RE='co-authored-by:[[:space:]]*claude([^a-z]|$)|noreply@anthropic\.com|generated (with|by) \[?claude code|claude\.ai/code/session'"|$_ATTRIBUTION_DRAFTED"

# attribution_hits FILE...
#
# Prints every line of FILE(s) that carries attribution, as `file:line:text`. Returns 0 when it found
# any, 1 when the files are clean, and 2 when a file could not be read. Pass `-` to read stdin.
#   * A literal `\n` (a JSON string, a printf format) is read as the line break it stands for, so a
#     footer escaped onto one line is still a line of its own.
#   * Text that parses as JSON is also checked string by string, decoded (`\uXXXX`, `\r`, a body
#     that is not the last key), when jq is installed.
#   * A code span (`...` within one line) is a quotation, not a credit, and is dropped first: prose
#     that names a form in backticks, as this repo writes them, passes.
attribution_hits() {
    local f label raw decoded text hits line rc=1
    for f in "$@"; do
        if [[ "$f" == - ]]; then
            label="(standard input)"
            raw="$(cat)" || { rc=2; continue; }
        elif [[ -f "$f" && -r "$f" ]]; then
            label="$f"
            raw="$(cat -- "$f" 2>/dev/null)" || { rc=2; continue; }
        else
            rc=2
            continue
        fi
        decoded=""
        if command -v jq >/dev/null 2>&1; then
            decoded="$(printf '%s\n' "$raw" | jq -r '.. | strings' 2>/dev/null)" || decoded=""
        fi
        if ! text="$(printf '%s\n%s\n' "$raw" "$decoded" | awk '{ gsub(/\\n/, "\n"); gsub(/`[^`\n]*`/, ""); print }' 2>/dev/null)"; then
            rc=2
            continue
        fi
        hits="$(printf '%s\n' "$text" | grep -niE "$ATTRIBUTION_RE" 2>/dev/null)"
        case $? in
            0)
                while IFS= read -r line; do printf '%s:%s\n' "$label" "$line"; done <<<"$hits"
                [[ $rc -eq 2 ]] || rc=0
                ;;
            1) ;;
            *) rc=2 ;;
        esac
    done
    return $rc
}
