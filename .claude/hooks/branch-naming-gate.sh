#!/usr/bin/env bash
# shellcheck source-path=SCRIPTDIR
# shellcheck disable=SC2329  # inspect() and its helpers run through for_each_command
# PreToolUse(Bash) gate: a pushed branch is named `{feat|fix|chore}/{issue}/{2-4-word-desc}`, at
# most 50 characters (.claude/commands/start.md § 3), so every branch says which issue it serves.
# Pushes to main/next are git-safety-gate's call, and deleting or pushing tags is not naming, so
# those pass here. Exit 2 blocks (message on stderr); a payload it cannot read blocks too.
exec >&2
# shellcheck source=lib/tool-input.sh
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/tool-input.sh" || exit 2

gate_read_command || exit 2
case "$COMMAND" in *push*) ;; *) exit 0 ;; esac

NAME_RE='^(feat|fix|chore)/[0-9]+/[a-z0-9]+(-[a-z0-9]+){1,3}$'

check_name() {
    local name="${1#refs/heads/}"
    case "$name" in
        ''|main|next|refs/tags/*) return 0 ;;
        *__Q__*|*'$'*) return 0 ;;  # a variable or quoted prose: the name is not knowable here
    esac
    git -C "$GIT_DIR_AT" show-ref --verify --quiet "refs/tags/$name" 2>/dev/null && return 0
    if [[ ! "$name" =~ $NAME_RE ]]; then
        echo "BLOCKED: branch '$name' does not match {feat|fix|chore}/{issue}/{2-4-word-desc}."
        echo "  e.g. feat/47/save-event-replays, fix/569/duplicate-migration-check"
        echo "  Rename it: git branch -m '$name' <type>/<issue>/<short-desc>"
        return 2
    fi
    if [[ ${#name} -gt 50 ]]; then
        echo "BLOCKED: branch '$name' is ${#name} characters; the limit is 50."
        echo "  Shorten the description to 2-4 words: git branch -m '$name' <shorter-name>"
        return 2
    fi
    return 0
}

inspect() {
    git_parse || return 0
    [[ "$GIT_SUB" == push ]] || return 0
    local arg skip=0 positional=() refspec dst
    for arg in "${GIT_ARGS[@]}"; do
        case "$arg" in
            -d|--delete|--tags|--all|--mirror) return 0 ;;
            -o|--push-option|--repo|--receive-pack|--exec) skip=1; continue ;;
            -*) continue ;;
        esac
        if [[ $skip -eq 1 ]]; then skip=0; continue; fi
        positional[${#positional[@]}]="$arg"
    done
    if [[ ${#positional[@]} -le 1 ]]; then
        check_name "$(current_branch "$GIT_DIR_AT")"
        return
    fi
    for refspec in "${positional[@]:1}"; do
        refspec="${refspec#+}"
        case "$refspec" in :*) continue ;; esac  # `:branch` deletes it
        dst="${refspec##*:}"
        [[ "$dst" == HEAD ]] && dst="$(current_branch "$GIT_DIR_AT")"
        check_name "$dst" || return 2
    done
    return 0
}

for_each_command inspect || exit 2
exit 0
