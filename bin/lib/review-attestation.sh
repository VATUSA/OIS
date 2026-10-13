#!/bin/bash
# ==============================================================================
# Review attestation: proof that every review lens checked every rule heading it owns.
#
#   bash bin/lib/review-attestation.sh check [SHA]    # prints gaps; exit 0 complete, 1 not
#   bash bin/lib/review-attestation.sh prior [SHA]    # prints PRIOR_SHA; exit 1 on a first round
#   . bin/lib/review-attestation.sh; check_attestation_for_head [SHA]
#
# Each lens in .claude/review-lenses.tsv writes
#   .claude/markers/attest/<SHA>/<lens>.md
# holding one table row per `## `/`### ` heading of every file it owns, read from the
# reviewed commit (`git show <SHA>:<file>`), with the lens map taken from the checker's
# own checkout (main's, when a hook runs it; REVIEW_LENSES_MAP overrides it for tests):
#   | <file> | <heading text> | Checked | <file:line or reason> |
# Status is Checked, N/A or Finding; the last cell must not be empty. A lens owning
# file "-" (intent) needs at least one row. A lens owning "@ac" needs one
#   | AC-COUNT | <n> | Checked | <issue number, fetch time> |
# row and exactly n `| AC | ... |` rows, every one Checked: an unmet AC is a gap.
#
# Repeat rounds (/code-review Phase 3): `prior` prints the nearest commit at or before SHA
# whose set is complete in this checkout, so each checkout's rounds are one caller's. A row
# the delta left alone is carried as `Carried-from <sha>: <evidence>`; one missing the sha
# or the evidence counts as empty. Lines outside the table are not rows.
#
# A tests-only delta: the caller may copy a prior attestation for a lens in ATTEST_CARRY_LENSES
# and open it with `Carried-from <sha>: tests-only delta`. That file passes only when <sha> is
# a strict ancestor of SHA with a complete set, `git diff --name-only --no-renames <sha> SHA` is
# non-empty and every path is a test file (attest_test_path; a file moved into one lists its old
# path too), and the rest of the file is <sha>'s attestation byte for byte.
#
# Sourced by .claude/hooks/pre-pr-gate.sh. Fails closed: a missing map, file or
# attestation is a gap. Bash 3.2 and BSD/GNU safe: no arrays, no GNU-only flags.
# ==============================================================================

ATTEST_SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# attest_rows FILE: "file<TAB>heading<TAB>status<TAB>evidence" per table row, cells trimmed.
attest_rows() {
    awk -F'|' '
        /^[[:space:]]*\|/ {
            n = 0
            for (i = 2; i < NF; i++) {
                c = $i
                gsub(/^[[:space:]]+|[[:space:]]+$/, "", c)
                cell[++n] = c
            }
            if (n >= 4) {
                if (cell[4] ~ /^Carried-from/ && cell[4] !~ /^Carried-from [0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]*: *[^ ]/) { cell[4] = "" }
                print cell[1] "\t" cell[2] "\t" cell[3] "\t" cell[4]
            }
        }' "$1"
}

# attest_headings SHA FILE ROOT: each `## `/`### ` heading text of FILE at SHA, outside code fences.
attest_headings() {
    git -C "$3" show "$1:$2" 2>/dev/null | awk '
        /^ ? ? ?(```|~~~)/ { fence = !fence; next }
        !fence && /^###? / { sub(/^###? /, ""); sub(/[[:space:]]+$/, ""); print }'
}

# Lenses that read no test file, so a tests-only delta cannot change what they attest
# (.claude/commands/code-review.md -> "A tests-only delta").
ATTEST_CARRY_LENSES="code security"

# attest_test_path PATH: 0 when PATH holds only tests, so a change to it is a tests-only delta.
# A vitest file (*.test.ts, *.test.tsx), anything under a tests/ directory, or a shell test of the
# tooling (bin/lib/*.test.sh). An inline Rust `#[cfg(test)] mod tests` does not qualify: its file
# also holds the production code the code and security lenses attested.
attest_test_path() {
    case "$1" in
        *.test.ts|*.test.tsx|tests/*|*/tests/*) return 0 ;;
        bin/lib/*.test.sh) case "${1#bin/lib/}" in */*) return 1 ;; esac; return 0 ;;
    esac
    return 1
}

# attest_non_test_paths: reads paths on stdin, prints each one attest_test_path refuses.
attest_non_test_paths() {
    local path
    while IFS= read -r path; do
        attest_test_path "$path" || printf '%s\n' "$path"
    done
}

# carried_copy_gap SHA ROOT LENS: prints one GAP when LENS's attestation for SHA opens with
# `Carried-from <sha>: tests-only delta` and is not a valid carry; prints nothing otherwise.
carried_copy_gap() {
    local sha="$1" root="$2" lens="$3" file first="" prior full head names outside
    file="$root/.claude/markers/attest/$sha/$lens.md"
    IFS= read -r first < "$file"
    case "$first" in 'Carried-from '*': tests-only delta') ;; *) return 0 ;; esac
    prior="${first#Carried-from }"
    prior="${prior%: tests-only delta}"
    case " $ATTEST_CARRY_LENSES " in *" $lens "*) ;; *)
        echo "GAP: lens '$lens' reads test files and cannot be carried over a tests-only delta; re-dispatch it."
        return 0 ;;
    esac
    case "$prior" in ''|*[!0-9a-f]*|?|??|???|????|?????|??????)
        echo "GAP: lens '$lens' is carried from '$prior', which is not a commit of 7 to 40 hex characters."
        return 0 ;;
    esac
    full="$(git -C "$root" rev-parse --verify --quiet "$prior^{commit}" 2>/dev/null)"
    head="$(git -C "$root" rev-parse --verify --quiet "$sha^{commit}" 2>/dev/null)"
    if [ -z "$full" ] || [ -z "$head" ]; then
        echo "GAP: lens '$lens' is carried from $prior, which does not resolve to a commit."
        return 0
    fi
    if [ "$full" = "$head" ] || ! git -C "$root" merge-base --is-ancestor "$full" "$head" 2>/dev/null; then
        echo "GAP: lens '$lens' is carried from ${full:0:8}, which is not an earlier commit on ${head:0:8}'s history."
        return 0
    fi
    # --no-renames lists a move's source too, so moving production code into a test path is not a tests-only delta.
    if ! names="$(git -C "$root" diff --name-only --no-renames "$full" "$head" 2>/dev/null)" || [ -z "$names" ]; then
        echo "GAP: lens '$lens' is carried from ${full:0:8}, but no file differs between it and ${head:0:8}."
        return 0
    fi
    outside="$(printf '%s\n' "$names" | attest_non_test_paths)"
    if [ -n "$outside" ]; then
        echo "GAP: lens '$lens' is carried from ${full:0:8}, but the delta is not tests-only: $(printf '%s\n' "$outside" | head -1)"
        return 0
    fi
    if ! check_attestation "$full" "$root" >/dev/null 2>&1; then
        echo "GAP: lens '$lens' is carried from ${full:0:8}, whose own attestation set is not complete."
        return 0
    fi
    if ! tail -n +2 "$file" | cmp -s - "$root/.claude/markers/attest/$full/$lens.md"; then
        echo "GAP: lens '$lens' is carried from ${full:0:8}, but its rows differ from that commit's attestation."
    fi
}

# check_attestation SHA [ROOT]: prints each gap; returns 0 when none.
check_attestation() {
    local sha="$1" root="${2:-}" map dir lens file rows heading out gaps=0 seen carried

    [ -n "$root" ] || root="$(git rev-parse --show-toplevel 2>/dev/null)"
    if [ -z "$root" ] || [ -z "$sha" ]; then
        echo "GAP: cannot resolve the repository or commit to check."
        return 1
    fi
    # The checker's own copy (main's, for a hook), never the branch's, so a branch cannot drop a lens.
    # REVIEW_LENSES_MAP exists for tests; a hook takes its environment from the harness.
    map="${REVIEW_LENSES_MAP:-$ATTEST_SELF_DIR/../../.claude/review-lenses.tsv}"
    if [ ! -f "$map" ]; then
        echo "GAP: .claude/review-lenses.tsv not found."
        return 1
    fi
    dir="$root/.claude/markers/attest/$sha"

    seen=""
    carried=""
    while IFS="$(printf '\t')" read -r lens file || [ -n "$lens" ]; do
        case "$lens" in ''|'#'*) continue ;; esac
        if [ ! -f "$dir/$lens.md" ]; then
            case " $seen " in *" $lens "*) ;; *)
                echo "GAP: no attestation for lens '$lens' ($dir/$lens.md)."
                seen="$seen $lens"
                gaps=$((gaps + 1)) ;;
            esac
            continue
        fi
        rows="$(attest_rows "$dir/$lens.md")"
        case " $carried " in *" $lens "*) ;; *)
            carried="$carried $lens"
            out="$(carried_copy_gap "$sha" "$root" "$lens")"
            if [ -n "$out" ]; then
                printf '%s\n' "$out"
                gaps=$((gaps + 1))
            fi ;;
        esac

        if [ -z "$file" ]; then
            echo "GAP: lens '$lens' has no file column in the lens map."
            gaps=$((gaps + 1))
            continue
        fi
        if [ "$file" = "-" ]; then
            if ! printf '%s\n' "$rows" | awk -F'\t' '($3 == "Checked" || $3 == "N/A" || $3 == "Finding") && $4 != "" { found = 1 } END { exit !found }'; then
                echo "GAP: lens '$lens' has no row with a status and evidence."
                gaps=$((gaps + 1))
            fi
            continue
        fi
        if [ "$file" = "@ac" ]; then
            out="$(printf '%s\n' "$rows" | L="$lens" awk -F'\t' '
                $1 == "AC-COUNT" {
                    counts++
                    if ($2 ~ /^[0-9]+$/ && $3 == "Checked" && $4 != "") { want = $2 + 0 } else { bad = 1 }
                }
                $1 == "AC" {
                    got++
                    if ($3 != "Checked" || $4 == "") { printf "GAP: lens %s: AC not met: %s\n", ENVIRON["L"], $2 }
                }
                END {
                    if (counts != 1 || bad) { printf "GAP: lens %s needs one | AC-COUNT | <n> | Checked | <issue, fetch time> | row.\n", ENVIRON["L"] }
                    else if (got != want) { printf "GAP: lens %s attests %d AC rows; the section it fetched has %d.\n", ENVIRON["L"], got, want }
                }')"
            if [ -n "$out" ]; then
                printf '%s\n' "$out"
                gaps=$((gaps + $(printf '%s\n' "$out" | wc -l)))
            fi
            continue
        fi

        if ! git -C "$root" cat-file -e "$sha:$file" 2>/dev/null; then
            echo "GAP: $file does not exist at $sha, so lens '$lens' cannot be attested."
            gaps=$((gaps + 1))
            continue
        fi
        while IFS= read -r heading; do
            [ -n "$heading" ] || continue
            if ! printf '%s\n' "$rows" | F="$file" H="$heading" awk -F'\t' '
                    $1 == ENVIRON["F"] && $2 == ENVIRON["H"] && ($3 == "Checked" || $3 == "N/A" || $3 == "Finding") && $4 != "" { found = 1 }
                    END { exit !found }'; then
                echo "GAP: lens '$lens' has no row for $file | $heading"
                gaps=$((gaps + 1))
            fi
        done <<EOF
$(attest_headings "$sha" "$file" "$root")
EOF
    done < "$map"

    [ "$gaps" -eq 0 ]
}

# prior_attested_sha [SHA] [ROOT]: the nearest commit at or before SHA (default HEAD) whose set
# is complete in ROOT's checkout. Prints it; returns 1 when there is none, which is a first round.
prior_attested_sha() {
    local sha="${1:-}" root="${2:-}" commit
    [ -n "$root" ] || root="$(git rev-parse --show-toplevel 2>/dev/null)"
    [ -n "$root" ] || return 1
    [ -n "$sha" ] || sha="HEAD"
    while IFS= read -r commit; do
        [ -d "$root/.claude/markers/attest/$commit" ] || continue
        if check_attestation "$commit" "$root" >/dev/null 2>&1; then
            echo "$commit"
            return 0
        fi
    done <<EOF
$(git -C "$root" rev-list "$sha" 2>/dev/null)
EOF
    return 1
}

# check_attestation_for_head [SHA]: the pre-pr-gate entry point, SHA defaulting to HEAD. Prints BLOCKED on failure.
check_attestation_for_head() {
    local sha="${1:-}" out total
    [ -n "$sha" ] || sha="$(git rev-parse HEAD 2>/dev/null)"
    if out="$(check_attestation "$sha")"; then
        return 0
    fi
    echo "BLOCKED: the review lenses did not attest every rule heading for ${sha:0:8}."
    printf '%s\n' "$out" | head -40
    total="$(printf '%s\n' "$out" | wc -l | tr -d ' ')"
    [ "$total" -le 40 ] || echo "... and $((total - 40)) more: bash bin/lib/review-attestation.sh check"
    echo "Re-run /code-review (its lens phase writes .claude/markers/attest/<SHA>/<lens>.md)."
    return 1
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    case "${1:-}" in
        check)
            sha="${2:-$(git rev-parse HEAD 2>/dev/null)}"
            if check_attestation "$sha"; then
                echo "attestation complete for ${sha:0:8}"
                exit 0
            fi
            exit 1
            ;;
        prior)
            if prior_attested_sha "${2:-}"; then
                exit 0
            fi
            echo "no complete attestation set at or before ${2:-HEAD} in this checkout: a first round" >&2
            exit 1
            ;;
        *)
            echo "usage: bash bin/lib/review-attestation.sh check|prior [SHA]" >&2
            exit 2
            ;;
    esac
fi
