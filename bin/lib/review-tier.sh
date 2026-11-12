#!/bin/bash
# ==============================================================================
# Review tier: which lenses /code-review must run for a commit, decided from the paths it changes.
#
#   bash bin/lib/review-tier.sh [SHA]          # prints "<tier> <lens> <lens>..."
#   . bin/lib/review-tier.sh; review_tier SHA ROOT; review_lenses SHA ROOT
#
# The tiers are test-quality.md's blast-radius tiers, judged on `git diff --name-only <base>...SHA`,
# base = the merge base with origin/next (REVIEW_TIER_BASE overrides it for tests; a hook takes its
# environment from the harness). The highest tier any path reaches wins:
#   Light     code ac                  docs, guidance, test files
#   Standard  code ac security         gates, review commands and agents, tooling, the security rules
#   Full      code ac tests security   everything else (the bot included), and whenever the tier cannot be read
# Any tier adds lens tests when the diff changes a test file (test_file).
# The builder never declares the tier: an unresolvable base, an empty diff or an unknown path is Full.
# Backend source is Full: a path cannot tell whether a handler or repo change reaches scope or the contract.
# Bash 3.2 and BSD/GNU safe.
# ==============================================================================

# test_file PATH: true for a test file outside a crate's source (Light, like docs); puts lens tests in any tier.
test_file() {
    case "$1" in
        *.test.ts|*.test.tsx|tests/*|*/tests/*) return 0 ;;
        bin/lib/*.test.sh) case "${1#bin/lib/}" in */*) return 1 ;; esac; return 0 ;;
    esac
    return 1
}

# tier_of_path PATH: prints 1 (Light), 2 (Standard) or 3 (Full).
tier_of_path() {
    if test_file "$1"; then echo 1; return; fi
    case "$1" in
        .claude/hooks/*|.claude/settings.json|.claude/review-lenses.tsv) echo 2 ;;
        .claude/commands/code-review.md|.claude/commands/review-before-shipping.md|.claude/commands/ship.md) echo 2 ;;
        .claude/agents/*review*|.claude/agents/security-audit*) echo 2 ;;
        .claude/rules/secure-coding.md|.claude/rules/data-protection.md) echo 2 ;;
        .claude/*|CLAUDE.md|AGENTS.md|DESIGN.md|docs/*|docs-site/*) echo 1 ;;
        .githooks/*|bin/*|justfile) echo 2 ;;
        *) echo 3 ;;
    esac
}

# tier_name N: Light, Standard or Full.
tier_name() {
    case "$1" in 1) echo Light ;; 2) echo Standard ;; *) echo Full ;; esac
}

# tier_lenses TIER: the lenses that tier requires, space-separated.
tier_lenses() {
    case "$1" in
        Light) echo "code ac" ;;
        Standard) echo "code ac security" ;;
        *) echo "code ac tests security" ;;
    esac
}

# changed_paths SHA ROOT: SHA's changed paths against its base; fails when either cannot be read.
changed_paths() {
    local base
    base="$(git -C "$2" merge-base "${REVIEW_TIER_BASE:-origin/next}" "$1" 2>/dev/null)" && [ -n "$base" ] \
        && git -C "$2" diff --name-only --no-renames "$base" "$1" 2>/dev/null
}

# review_tier SHA [ROOT]: prints Light, Standard or Full for SHA's diff against its base.
review_tier() {
    local sha="$1" root="${2:-}" names path top=0 tier
    [ -n "$root" ] || root="$(git rev-parse --show-toplevel 2>/dev/null)"
    if ! names="$(changed_paths "$sha" "$root")" || [ -z "$names" ]; then
        echo Full
        return 0
    fi
    while IFS= read -r path; do
        [ -n "$path" ] || continue
        tier="$(tier_of_path "$path")"
        [ "$tier" -gt "$top" ] && top="$tier"
    done <<EOF
$names
EOF
    tier_name "$top"
}

# review_lenses SHA [ROOT]: the lenses SHA needs: its tier's, plus tests when it changes a test file.
review_lenses() {
    local sha="$1" root="${2:-}" lenses path
    [ -n "$root" ] || root="$(git rev-parse --show-toplevel 2>/dev/null)"
    lenses="$(tier_lenses "$(review_tier "$sha" "$root")")"
    case " $lenses " in *" tests "*) echo "$lenses"; return 0 ;; esac
    while IFS= read -r path; do
        if [ -n "$path" ] && test_file "$path"; then
            echo "code ac tests${lenses#code ac}"
            return 0
        fi
    done <<EOF
$(changed_paths "$sha" "$root")
EOF
    echo "$lenses"
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    sha="${1:-$(git rev-parse HEAD 2>/dev/null)}"
    echo "$(review_tier "$sha") $(review_lenses "$sha")"
fi
