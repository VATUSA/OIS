#!/bin/bash
# ==============================================================================
# Review tier: which lenses /code-review must run for a commit, decided from the paths it changes.
#
#   bash bin/lib/review-tier.sh [SHA]          # prints "<tier> <lens> <lens>..."
#   . bin/lib/review-tier.sh; review_tier SHA ROOT; tier_lenses TIER
#
# The tiers are test-quality.md's blast-radius tiers, judged on `git diff --name-only <base>...SHA`,
# base = the merge base with origin/next (REVIEW_TIER_BASE overrides it for tests; a hook takes its
# environment from the harness). The highest tier any path reaches wins:
#   Light     code ac                  docs, guidance, test files
#   Standard  code ac security         the bot, gates, review commands and agents, and tooling
#   Full      code ac tests security   everything else, and whenever the tier cannot be read
# The builder never declares the tier: an unresolvable base, an empty diff or an unknown path is Full.
# Backend source is Full: a path cannot tell whether a handler or repo change reaches scope or the contract.
# Bash 3.2 and BSD/GNU safe.
# ==============================================================================

# tier_of_path PATH: prints 1 (Light), 2 (Standard) or 3 (Full).
tier_of_path() {
    case "$1" in
        *.test.ts|*.test.tsx|tests/*|*/tests/*) echo 1; return ;;
        bin/lib/*.test.sh) case "${1#bin/lib/}" in */*) ;; *) echo 1; return ;; esac ;;
    esac
    case "$1" in
        .claude/hooks/*|.claude/settings.json|.claude/review-lenses.tsv) echo 2 ;;
        .claude/commands/code-review.md|.claude/commands/review-before-shipping.md|.claude/commands/ship.md) echo 2 ;;
        .claude/agents/*review*|.claude/agents/security-audit*) echo 2 ;;
        .claude/*|CLAUDE.md|AGENTS.md|DESIGN.md|docs/*|docs-site/*) echo 1 ;;
        .githooks/*|bin/*|justfile) echo 2 ;;
        discord/*) echo 2 ;;
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

# review_tier SHA [ROOT]: prints Light, Standard or Full for SHA's diff against its base.
review_tier() {
    local sha="$1" root="${2:-}" base names path top=0 tier
    [ -n "$root" ] || root="$(git rev-parse --show-toplevel 2>/dev/null)"
    base="$(git -C "$root" merge-base "${REVIEW_TIER_BASE:-origin/next}" "$sha" 2>/dev/null)"
    if [ -z "$base" ] || ! names="$(git -C "$root" diff --name-only --no-renames "$base" "$sha" 2>/dev/null)" \
            || [ -z "$names" ]; then
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

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    sha="${1:-$(git rev-parse HEAD 2>/dev/null)}"
    tier="$(review_tier "$sha")"
    echo "$tier $(tier_lenses "$tier")"
fi
