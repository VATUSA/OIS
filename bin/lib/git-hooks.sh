#!/bin/bash
# ==============================================================================
# Helpers sourced by bin/pre-push. Side-effect free when sourced; bash 3.2 and
# BSD/GNU safe. Tests: bin/lib/git-hooks.test.sh.
# ==============================================================================

# pushed_ref — reads pre-push stdin ("<local ref> <local sha> <remote ref>
# <remote sha>" per line) and prints "<local sha> <branch>" for the branch update
# the checks judge: the one branch that is not main or next, else the protected
# one. Returns 1 when no branch is updated, 3 when more than one other branch is
# (the checks judge one commit, so a second branch would go unchecked).
pushed_ref() {
    local lref lsha rref rsha main="" branch=""
    while read -r lref lsha rref rsha; do
        case "$lsha" in *[!0]*) ;; *) continue ;; esac
        case "$rref" in
            refs/heads/main|refs/heads/next) [ -n "$main" ] || main="$lsha ${rref#refs/heads/}" ;;
            refs/heads/*) [ -z "$branch" ] || return 3; branch="$lsha ${rref#refs/heads/}" ;;
        esac
    done
    [ -n "$branch$main" ] || return 1
    echo "${branch:-$main}"
}

# tests_marker_for SHA — 0 when bin/run-tests.sh stamped a clean full-suite pass
# for SHA in this checkout and the marker manifest vouches for it: the check
# `gh pr create` is gated on (bin/check-marker.sh).
tests_marker_for() {
    local lib
    lib="$(dirname "${BASH_SOURCE[0]}")/../check-marker.sh"
    [ -n "${1:-}" ] && [ -f "$lib" ] || return 1
    # shellcheck source=../check-marker.sh
    . "$lib"
    check_marker_for_head tests "Tests" "just test" "$1" >/dev/null
}
