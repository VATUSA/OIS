#!/usr/bin/env bash
# shellcheck source-path=SCRIPTDIR
# shellcheck disable=SC2329  # inspect() and its helpers run through for_each_command
# PreToolUse(Bash) gate: a new migration's number must not already belong to a different migration
# on any origin branch.
#
# sqlx applies two files that share a version and then fails on `_sqlx_migrations`' primary key,
# leaving the database half-migrated (#569). .github/scripts/check-migration-versions.sh catches a
# duplicate inside one tree; this catches the collision with another open branch before it is even
# committed (AGENTS.md § Conventions & gotchas, "Picking a migration number is contended").
#
# Fires on `git commit`. It checks the migrations already staged plus any that a `git add` earlier
# in the same command line stages. It reads local `refs/remotes/origin/*` only, with no fetch, so it
# is as fresh as your last `git fetch`. Your own branch's remote copy is skipped.
# Exit 2 blocks (message on stderr); a payload it cannot read blocks too.
exec >&2
# shellcheck source=lib/tool-input.sh
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/tool-input.sh" || exit 2

gate_read_command || exit 2
case "$COMMAND" in *commit*) ;; *) exit 0 ;; esac

MIGRATIONS=backend/migrations
CANDIDATES=()
COMMIT_DIR=""

add_candidate() {
    local name="${1##*/}"
    [[ "$name" =~ ^[0-9]+_.*\.sql$ ]] && CANDIDATES[${#CANDIDATES[@]}]="$name"
}

inspect() {
    git_parse || return 0
    local arg
    case "$GIT_SUB" in
        add)
            for arg in "${GIT_ARGS[@]}"; do
                case "$arg" in *"$MIGRATIONS"/*) add_candidate "$arg" ;; esac
            done
            ;;
        commit) COMMIT_DIR="$GIT_DIR_AT" ;;
    esac
    return 0
}

for_each_command inspect
[[ -n "$COMMIT_DIR" ]] || exit 0

while IFS= read -r staged; do
    add_candidate "$staged"
done < <(git -C "$COMMIT_DIR" diff --cached --name-only --diff-filter=AR -- "$MIGRATIONS" 2>/dev/null)
[[ ${#CANDIDATES[@]} -gt 0 ]] || exit 0

own="$(current_branch "$COMMIT_DIR")"

# Every origin branch's migrations directory, as `<tree sha> <ref>`. Most branches share a few trees,
# so each distinct tree is listed once.
trees="$(git -C "$COMMIT_DIR" for-each-ref --format='%(refname):backend/migrations %(refname:lstrip=3)' refs/remotes/origin |
    grep -v ' HEAD$' |
    git -C "$COMMIT_DIR" cat-file --batch-check='%(objectname) %(objecttype) %(rest)' |
    awk -v own="$own" '$2 == "tree" && $3 != own { print $1, $3 }' |
    sort -u -k1,1)"

collisions=""
while read -r tree ref; do
    [[ -n "$tree" ]] || continue
    while IFS= read -r existing; do
        for candidate in "${CANDIDATES[@]}"; do
            [[ "$existing" == "$candidate" ]] && continue
            if [[ $((10#${existing%%_*})) -eq $((10#${candidate%%_*})) ]]; then
                collisions="$collisions  $candidate collides with $existing on origin/$ref"$'\n'
            fi
        done
    done < <(git -C "$COMMIT_DIR" ls-tree --name-only "$tree" 2>/dev/null | grep -E '^[0-9]+_')
done <<<"$trees"

if [[ -n "$collisions" ]]; then
    echo "BLOCKED: a staged migration reuses a version another branch already took:"
    printf '%s' "$collisions" | sort -u
    echo "Renumber yours upward, past the highest number on next AND every open branch (a gap is"
    echo "harmless, a repeat half-migrates the database). See AGENTS.md § Conventions & gotchas."
    exit 2
fi
exit 0
