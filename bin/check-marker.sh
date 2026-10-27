#!/bin/bash
# Shared marker validation for pre-PR gate hooks.
#
#   check_marker_for_head <type> <label> <rerun_cmd> <sha>
#     SHA-keyed marker. Looks for .claude/markers/<type>/<SHA> in the current
#     checkout, vouched for by its .claude/markers/manifest line (bin/lib/marker_manifest.py).
#     The filename is the proof of which commit
#     was reviewed or tested, so parallel worktrees never overwrite each other and
#     a moved HEAD invalidates the marker automatically (no time-based staleness needed).
#
#   $1 - marker type (e.g., "review-shipping", "tests")
#   $2 - human-readable label (e.g., "Review")
#   $3 - command to re-run (e.g., "/review-before-shipping")
#   $4 - commit to check (pr_head_sha's PR_HEAD_SHA); required, so an unresolved PR head
#        is refused rather than checked against whatever HEAD has
#
# Returns 0 on pass, 1 on fail (with a BLOCKED message on stdout).
check_marker_for_head() {
    local TYPE="$1"
    local LABEL="$2"
    local RERUN_CMD="$3"

    local REPO_ROOT
    REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null)"
    if [[ -z "$REPO_ROOT" ]]; then
        echo "BLOCKED: Not inside a git repository — cannot verify ${LABEL}."
        return 1
    fi

    local HEAD_SHA="${4:-}"
    if [[ -z "$HEAD_SHA" ]]; then
        echo "BLOCKED: no PR head commit to check — cannot verify ${LABEL}."
        return 1
    fi

    local MARKER="$REPO_ROOT/.claude/markers/$TYPE/$HEAD_SHA"

    if [[ ! -f "$MARKER" ]]; then
        echo "BLOCKED: ${LABEL} has not run for the PR head commit (${HEAD_SHA:0:8})."
        echo "Run: ${RERUN_CMD}"
        return 1
    fi

    # The marker counts only with the manifest line its stamping script wrote beside it.
    local MANIFEST_LIB
    MANIFEST_LIB="$(dirname "${BASH_SOURCE[0]}")/lib/marker_manifest.py"
    if [[ ! -f "$MANIFEST_LIB" ]]; then
        echo "BLOCKED: cannot load bin/lib/marker_manifest.py, so the ${LABEL} marker cannot be verified."
        return 1
    fi
    if ! python3 -I "$MANIFEST_LIB" vouched "$REPO_ROOT" "$TYPE" "$HEAD_SHA"; then
        echo "BLOCKED: the ${LABEL} marker for ${HEAD_SHA:0:8} has no matching line in .claude/markers/manifest,"
        echo "so bin/run-tests.sh or bin/review-marker.sh did not write it (a branch whose scripts predate the"
        echo "manifest writes none: git merge origin/next first). Run: ${RERUN_CMD}"
        return 1
    fi

    return 0
}

# pr_head_sha <branch>: sets PR_HEAD_SHA to the commit `gh pr create --head <branch>` opens the PR
# on, or the current branch's when <branch> is empty. gh opens it on origin's copy, so a local tip
# that differs from origin/<branch> is refused rather than guessed between.
# Returns 0 on success, 1 with a BLOCKED message on stdout.
pr_head_sha() {
    local head="$1" branch local_sha remote_sha
    PR_HEAD_SHA=""
    if [[ -z "$head" ]]; then
        local_sha="$(git rev-parse --verify --quiet 'HEAD^{commit}' 2>/dev/null)"
        branch="$(git symbolic-ref --quiet --short HEAD 2>/dev/null)"
    else
        branch="$head"
        local_sha="$(git rev-parse --verify --quiet "refs/heads/${head}^{commit}" 2>/dev/null)"
    fi
    if [[ -n "$branch" ]]; then
        remote_sha="$(git rev-parse --verify --quiet "refs/remotes/origin/${branch}^{commit}" 2>/dev/null)"
    fi
    if [[ -n "$local_sha" && -n "$remote_sha" && "$local_sha" != "$remote_sha" ]]; then
        echo "BLOCKED: origin/$branch is at ${remote_sha:0:8} but the local branch is at ${local_sha:0:8}."
        echo "gh opens the PR on origin's commit; push (or fetch) until they match, then retry."
        return 1
    fi
    PR_HEAD_SHA="${local_sha:-$remote_sha}"
    if [[ -z "$PR_HEAD_SHA" ]]; then
        echo "BLOCKED: cannot resolve the commit the PR would open on (${head:-HEAD})."
        return 1
    fi
    return 0
}
