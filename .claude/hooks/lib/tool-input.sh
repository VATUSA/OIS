#!/usr/bin/env bash
# Shared plumbing for the Claude Code hooks in .claude/hooks/. Sourced, never run; side-effect-free
# at source time. Needs `jq`, `awk` and `git`, and nothing newer than macOS's stock bash 3.2.
#
# The two kinds of hook fail in opposite directions:
#   * a GATE (PreToolUse, exit 2 blocks) must fail CLOSED: one that cannot read its input blocks,
#     because "could not parse" and "parsed, nothing dangerous" must never look the same;
#   * a REMINDER (PostToolUse, advisory) must fail OPEN: it protects nothing, so it must never be the
#     reason a call stalls.
# Claude Code blocks a tool call only on exit 2, and reads stderr when it does. Exit 1 is a
# non-blocking error and the call runs anyway, so a gate that means "no" must exit 2.

HOOK_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# gate_read_command
#
# For gates. Reads the hook payload on stdin and sets COMMAND (tool_input.command) and HOOK_CWD (the
# payload's cwd, else $PWD). Returns 2, with the reason on stderr, when the payload is empty, is not
# JSON, or has no string command. Call as:  gate_read_command || exit 2
gate_read_command() {
    local out
    # The trailing "x" survives command substitution's newline stripping, so a command that ends in
    # a newline (or is empty) keeps its exact text.
    if ! out="$(jq -ej '
        if (.tool_input | type) == "object" and (.tool_input.command | type) == "string"
        then ((.cwd // "") | tostring) + "\n" + .tool_input.command + "x"
        else error("no tool_input.command") end' 2>/dev/null)"; then
        echo "BLOCKED: this gate could not parse its tool input (empty or malformed payload, or jq is missing)." >&2
        echo "A gate that cannot read the command must not pass it unchecked." >&2
        return 2
    fi
    HOOK_CWD="${out%%$'\n'*}"
    COMMAND="${out#*$'\n'}"
    COMMAND="${COMMAND%x}"
    [[ -n "$HOOK_CWD" && -d "$HOOK_CWD" ]] || HOOK_CWD="$PWD"
    return 0
}

# hook_json FILTER  (payload on stdin)
#
# For reminders. Prints the jq FILTER's raw result, or nothing when the payload cannot be read.
# Always returns 0: a reminder that cannot parse simply stays quiet.
hook_json() {
    jq -r "$1" 2>/dev/null || true
}

# remind TEXT
#
# Hands TEXT to Claude after the tool ran, then exits 0. PostToolUse discards plain stdout, so the
# text travels as hookSpecificOutput.additionalContext, which the harness adds to Claude's context.
remind() {
    jq -n --arg ctx "$1" \
        '{hookSpecificOutput: {hookEventName: "PostToolUse", additionalContext: $ctx}}' 2>/dev/null
    exit 0
}

# hook_segments COMMAND
#
# Prints the simple commands COMMAND runs, one per line, with heredoc bodies and quoted prose
# removed. See segments.awk for what that does and does not handle.
hook_segments() {
    printf '%s\n' "$1" | awk -f "$HOOK_LIB_DIR/segments.awk"
}

# hook_words SEGMENT
#
# Splits SEGMENT into the array WORDS and drops what only wraps a command (`env X=1`, `sudo`,
# `time`, `timeout 600`, `nice -n 5`, `eval`, `if`/`then`, `{`), so WORDS[0] is the program that
# actually runs.
hook_words() {
    local raw w started=0 wrapper="" skip_next=0
    WORDS=()
    read -r -a raw <<<"$1"
    for w in "${raw[@]}"; do
        if [[ $started -eq 0 ]]; then
            if [[ $skip_next -eq 1 ]]; then skip_next=0; continue; fi
            # `timeout [opts] DURATION cmd` and `nice [-n N] cmd`: drop their options and argument.
            case "$wrapper" in
                timeout)
                    case "$w" in
                        -s|-k|--signal|--kill-after) skip_next=1; continue ;;
                        -*) continue ;;
                        *) wrapper=""; continue ;;  # the duration
                    esac
                    ;;
                nice)
                    case "$w" in
                        -n|--adjustment) skip_next=1; continue ;;
                        -*) continue ;;
                        *) wrapper="" ;;
                    esac
                    ;;
            esac
            case "$w" in
                '{'|'}'|'!'|if|then|else|elif|do|while|until|time|command|builtin|exec|nohup|sudo|env|eval) continue ;;
                timeout|nice) wrapper="$w"; continue ;;
            esac
            [[ "$w" =~ ^[A-Za-z_][A-Za-z0-9_]*= ]] && continue
            started=1
        fi
        WORDS[${#WORDS[@]}]="$w"
    done
}

# resolve_dir BASE DIR
#
# Prints DIR made absolute against BASE. An unknown DIR (empty, quoted prose, or a variable) leaves
# BASE as the best available guess.
resolve_dir() {
    local base="$1" dir="$2"
    case "$dir" in
        ''|*__Q__*|*'$'*) printf '%s' "$base" ;;
        \~) printf '%s' "$HOME" ;;
        \~/*) printf '%s' "$HOME/${dir#\~/}" ;;
        /*) printf '%s' "$dir" ;;
        *) printf '%s' "$base/$dir" ;;
    esac
}

# for_each_command CALLBACK
#
# Runs CALLBACK once for each simple command in $COMMAND, with WORDS set and SEG_DIR holding the
# directory it would run in (a `cd` earlier in the same line moves it). CALLBACK's return value is
# collected: for_each_command returns 2 if any call did.
for_each_command() {
    local callback="$1" seg dir="${HOOK_CWD:-$PWD}" status=0
    # fd 3, so a callback that reads stdin cannot swallow the remaining commands.
    while IFS= read -r seg <&3; do
        hook_words "$seg"
        [[ ${#WORDS[@]} -gt 0 ]] || continue
        case "${WORDS[0]}" in
            cd|pushd)
                if [[ ${#WORDS[@]} -gt 1 ]]; then
                    dir="$(resolve_dir "$dir" "${WORDS[1]}")"
                else
                    dir="$HOME"
                fi
                continue
                ;;
        esac
        SEG_DIR="$dir"
        "$callback" || status=2
    done 3< <(hook_segments "$COMMAND")
    return $status
}

# git_parse
#
# When WORDS is a git invocation, sets GIT_SUB (the subcommand), GIT_ARGS (the words after it) and
# GIT_DIR_AT (SEG_DIR moved by any `-C`), and returns 0. Returns 1 for anything else.
git_parse() {
    [[ ${#WORDS[@]} -gt 0 && "${WORDS[0]##*/}" == git ]] || return 1
    local i=1 n=${#WORDS[@]}
    GIT_DIR_AT="$SEG_DIR"
    GIT_SUB=""
    GIT_ARGS=()
    while [[ $i -lt $n ]]; do
        case "${WORDS[$i]}" in
            -C) GIT_DIR_AT="$(resolve_dir "$GIT_DIR_AT" "${WORDS[$((i + 1))]:-}")"; i=$((i + 2)) ;;
            -c|--git-dir|--work-tree|--namespace|--exec-path) i=$((i + 2)) ;;
            -*) i=$((i + 1)) ;;
            *) GIT_SUB="${WORDS[$i]}"; i=$((i + 1)); break ;;
        esac
    done
    [[ -n "$GIT_SUB" ]] || return 1
    while [[ $i -lt $n ]]; do
        GIT_ARGS[${#GIT_ARGS[@]}]="${WORDS[$i]}"
        i=$((i + 1))
    done
    return 0
}

# gh_is SUBCOMMAND ACTION
#
# Returns 0 when WORDS is `gh SUBCOMMAND ACTION ...` (repo flags such as `-R x` may come first) and
# sets GH_ARGS to the words after ACTION.
gh_is() {
    [[ ${#WORDS[@]} -gt 0 && "${WORDS[0]##*/}" == gh ]] || return 1
    local i=1 n=${#WORDS[@]} got=0
    while [[ $i -lt $n && $got -lt 2 ]]; do
        case "${WORDS[$i]}" in
            -R|--repo) i=$((i + 2)); continue ;;
            -*) i=$((i + 1)); continue ;;
        esac
        if [[ $got -eq 0 ]]; then
            [[ "${WORDS[$i]}" == "$1" ]] || return 1
        else
            [[ "${WORDS[$i]}" == "$2" ]] || return 1
        fi
        got=$((got + 1))
        i=$((i + 1))
    done
    [[ $got -eq 2 ]] || return 1
    GH_ARGS=()
    while [[ $i -lt $n ]]; do
        GH_ARGS[${#GH_ARGS[@]}]="${WORDS[$i]}"
        i=$((i + 1))
    done
    return 0
}

# current_branch DIR — prints the checked-out branch in DIR, or nothing when detached / not a repo.
current_branch() {
    git -C "$1" symbolic-ref --quiet --short HEAD 2>/dev/null
}

# is_primary_checkout DIR — 0 when DIR is in the primary checkout rather than a linked worktree.
is_primary_checkout() {
    local dirs git_dir common_dir
    dirs="$(git -C "$1" rev-parse --path-format=absolute --git-dir --git-common-dir 2>/dev/null)" || return 1
    git_dir="${dirs%%$'\n'*}"
    common_dir="${dirs#*$'\n'}"
    [[ -n "$git_dir" && "$git_dir" == "$common_dir" ]]
}

# primary_claude_dir DIR — the primary checkout's .claude, wherever DIR's worktree lives.
primary_claude_dir() {
    local common
    common="$(git -C "$1" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)" || return 1
    printf '%s/.claude' "$(dirname "$common")"
}
