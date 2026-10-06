#!/usr/bin/env bash
# shellcheck source-path=SCRIPTDIR
# shellcheck disable=SC2329  # inspect() and its helpers run through for_each_command
# PreToolUse(Bash) gate: the git moves OIS never lets an agent make.
#   * push or force-push to `main` or `next` — work reaches them only through a reviewed PR;
#   * `gh pr merge` — a human merges;
#   * creating, renaming or switching a branch in the PRIMARY checkout — it stays on `next`, and
#     issue work happens in a worktree (`git worktree add` is how a branch is born);
#   * `git add -A` / `git add .` / `git add --all` — stage explicit paths, so nothing unreviewed rides
#     along.
# Rules: AGENTS.md § Git workflow, CLAUDE.md § Standing working agreements.
# Exit 2 blocks (message on stderr); a payload it cannot read blocks too.
exec >&2
# shellcheck source=lib/tool-input.sh
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/tool-input.sh" || exit 2

gate_read_command || exit 2
case "$COMMAND" in *git*|*gh*) ;; *) exit 0 ;; esac

PROTECTED_RE='^(refs/heads/)?(main|next)$'

block() {
    echo "BLOCKED: $1"
    shift
    local line
    for line in "$@"; do echo "  $line"; done
    return 2
}

check_push() {
    local arg positional=() i=0 refspec dst branch
    for arg in "${GIT_ARGS[@]}"; do
        case "$arg" in
            --all|--mirror)
                block "\`git push $arg\` would push main/next along with everything else." \
                    "Push the one feature branch: git push -u origin <branch>"
                return 2
                ;;
            -o|--push-option|--repo|--receive-pack|--exec) i=1; continue ;;
            -*) continue ;;
        esac
        if [[ $i -eq 1 ]]; then i=0; continue; fi
        positional[${#positional[@]}]="$arg"
    done
    # positional[0] is the remote; the rest are refspecs.
    if [[ ${#positional[@]} -le 1 ]]; then
        branch="$(current_branch "$GIT_DIR_AT")"
        if [[ "$branch" =~ $PROTECTED_RE ]]; then
            block "you are on '$branch' — a bare \`git push\` would push it." \
                "main and next change only through a reviewed PR. Push your feature branch from its worktree."
            return 2
        fi
        return 0
    fi
    for refspec in "${positional[@]:1}"; do
        refspec="${refspec#+}"
        dst="${refspec##*:}"
        if [[ "$dst" == HEAD ]]; then
            dst="$(current_branch "$GIT_DIR_AT")"
        fi
        if [[ "$dst" =~ $PROTECTED_RE ]]; then
            block "pushing to '${dst#refs/heads/}' is not allowed (refspec '$refspec')." \
                "main and next change only through a reviewed PR targeting next."
            return 2
        fi
    done
    return 0
}

check_branch_change() {
    is_primary_checkout "$GIT_DIR_AT" || return 0
    local here arg target="" create=0 positional=()
    here="$(current_branch "$GIT_DIR_AT")"
    case "$GIT_SUB" in
        switch)
            for arg in "${GIT_ARGS[@]}"; do
                case "$arg" in
                    -c|-C|--create|--force-create|--orphan|--detach|-d) create=1 ;;
                    -*) ;;
                    *) positional[${#positional[@]}]="$arg" ;;
                esac
            done
            target="${positional[0]:-}"
            [[ $create -eq 0 && -n "$here" && "$target" == "$here" ]] && return 0
            ;;
        checkout)
            for arg in "${GIT_ARGS[@]}"; do
                case "$arg" in
                    --) return 0 ;;  # `git checkout [<tree-ish>] -- <paths>` restores files
                    -b|-B|--orphan|--detach) create=1 ;;
                    -*) ;;
                    *) positional[${#positional[@]}]="$arg" ;;
                esac
            done
            if [[ $create -eq 0 ]]; then
                target="${positional[0]:-}"
                [[ -z "$target" || "$target" == "$here" ]] && return 0
                # Without `--`, `git checkout x` restores a file unless x names a commit.
                if [[ "$target" != - ]] &&
                    ! git -C "$GIT_DIR_AT" rev-parse --verify --quiet "$target^{commit}" >/dev/null 2>&1 &&
                    ! git -C "$GIT_DIR_AT" rev-parse --verify --quiet "refs/remotes/origin/$target" >/dev/null 2>&1; then
                    return 0
                fi
            fi
            ;;
        branch)
            for arg in "${GIT_ARGS[@]}"; do
                case "$arg" in
                    -m|-M|--move|-c|-C|--copy) create=1 ;;
                    -d|-D|--delete|-l|--list|-a|--all|-r|--remotes|-v|-vv|--verbose|--show-current|--contains|--no-contains|--merged|--no-merged|--points-at|--format=*|--sort=*|-u|--set-upstream-to=*|--unset-upstream|--edit-description)
                        return 0 ;;
                    -*) ;;
                    *) positional[${#positional[@]}]="$arg" ;;
                esac
            done
            [[ $create -eq 0 && ${#positional[@]} -eq 0 ]] && return 0
            ;;
        *) return 0 ;;
    esac
    block "\`git $GIT_SUB ${GIT_ARGS[*]}\` would create or switch a branch in the primary checkout ($GIT_DIR_AT)." \
        "The primary checkout stays on next. Do issue work in a worktree:" \
        "  git worktree add ../ois-wt/<branch> -b <branch> origin/next"
    return 2
}

check_add() {
    local arg
    for arg in "${GIT_ARGS[@]}"; do
        case "$arg" in
            --all|--no-ignore-removal|.|./|:/|:/.) ;;
            --*) continue ;;
            -*A*) ;;  # -A, or bundled like -Av
            *) continue ;;
        esac
        block "\`git add $arg\` stages everything in the tree." \
            "Stage explicit paths so nothing unreviewed is committed: git add path/one path/two"
        return 2
    done
    return 0
}

inspect() {
    if gh_is pr merge; then
        block "\`gh pr merge\` is not allowed. A human merges PRs after review."
        return 2
    fi
    git_parse || return 0
    case "$GIT_SUB" in
        push) check_push ;;
        switch|checkout|branch) check_branch_change ;;
        add) check_add ;;
        *) return 0 ;;
    esac
}

for_each_command inspect || exit 2
exit 0
