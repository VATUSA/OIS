#!/bin/bash
# ==============================================================================
# Tests for bin/lib/review-attestation.sh.
#
#   bash bin/lib/review-attestation.test.sh
#
# Builds a scratch repository with a lens map and two rule files, then checks which
# attestation sets pass, which carried rows count, and which commit `prior` names as a
# repeat round's baseline. Exits non-zero on any failure.
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
LIB="$ROOT/bin/lib/review-attestation.sh"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/review-attestation-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

REPO="$WORK/repo"
mkdir -p "$REPO/rules"
cd "$REPO" || exit 1
git init -q .
git config user.email t@example.com
git config user.name t
printf 'code\trules/a.md\ntests\trules/b.md\nintent\t-\nac\t@ac\n' > "$WORK/lenses.tsv"
cat > rules/a.md <<'EOF'
# Title is not a lens heading
## Alpha — with a dash
text
### Beta `code` and \Throwable
```
## Inside a fence is not a heading
```
~~~
## Inside a tilde fence is not a heading
~~~
   ```
## Inside an indented fence is not a heading
   ```
EOF
printf '## Gamma \t\n' > rules/b.md
git add . && git commit -qm init
SHA="$(git rev-parse HEAD)"
ATT="$REPO/.claude/markers/attest/$SHA"

write_complete() {
    rm -rf "$REPO/.claude/markers"
    mkdir -p "$ATT"
    cat > "$ATT/code.md" <<'EOF'
| File | Heading | Status | Evidence |
|---|---|---|---|
| rules/a.md | Alpha — with a dash | Checked | backend/src/foo.rs:12 |
| rules/a.md | Beta `code` and \Throwable | N/A | no cache writes in the diff |
EOF
    printf '| rules/b.md | Gamma | Finding | MAJOR web/src/foo.test.ts:9 |\n' > "$ATT/tests.md"
    printf '| Scope | App\\Banner | Finding | MAJOR no AC asks for it |\n' > "$ATT/intent.md"
    cat > "$ATT/ac.md" <<'EOF'
| AC-COUNT | 2 | Checked | #1 fetched 2026-10-08T10:00Z |
| AC | 1. Admin sees the banner | Checked | web/src/pages/banner.tsx:4 |
| AC | 2. Banner hides after its end date | Checked | web/src/pages/banner.test.tsx:30 |
EOF
}

# expect WANT(0 complete | 1 gaps) NAME
expect() {
    local got
    (cd "$REPO" && REVIEW_LENSES_MAP="$WORK/lenses.tsv" bash "$LIB" check "$SHA" >"$WORK/out" 2>&1)
    got=$?
    if [[ "$got" == "$1" ]]; then ok "$2"; else bad "$2 (want $1, got $got): $(head -3 "$WORK/out")"; fi
}

echo "Complete:"
write_complete; expect 0 "every heading attested; fenced (backtick, tilde, indented) and h1 lines ignored; an intent row may be a Finding"

echo "Carried rows (a repeat round):"
# carried EVIDENCE: the Alpha row's evidence replaced.
carried() { write_complete; sed "s#| backend/src/foo.rs:12 |#| $1 |#" "$ATT/code.md" > "$WORK/t" && mv "$WORK/t" "$ATT/code.md"; grep -qF "| $1 |" "$ATT/code.md" || bad "carried fixture not written: $1"; }
write_complete
printf 'Carried-from an earlier round, see the report\n%s\n' "$(cat "$ATT/code.md")" > "$ATT/code.md"
expect 0 "a prose line above the table is not a row and does not hide its rows"
carried 'Carried-from 0123abc: backend/src/foo.rs:12'
expect 0 "a carried row keeps its evidence after the commit"
carried 'Carried-from 0123abc0123abc0123abc0123abc0123abc0123abc0: backend/src/foo.rs:12'
expect 0 "a carried row may name the full 40-character commit"
carried 'Carried-from 0123abc:'
expect 1 "a carried row with no evidence after the commit"
carried 'Carried-from: backend/src/foo.rs:12'
expect 1 "a carried row naming no commit"
carried 'Carried-from 0123ab: backend/src/foo.rs:12'
expect 1 "a carried row naming fewer than seven hex characters"
carried 'Carried-from earlier: backend/src/foo.rs:12'
expect 1 "a carried row whose commit is not hex"
write_complete; printf '| Scope | 1 | Checked | Carried-from 0123abc: |\n' > "$ATT/intent.md"
expect 1 "an intent row carried with no evidence"
write_complete; printf '| Scope | 1 | Checked | Carried-from 0123abc: AC 2 |\n' > "$ATT/intent.md"
expect 0 "an intent row carried with its evidence"

printf '# a comment line\n\ncode\trules/a.md\ntests\trules/b.md\nintent\t-\nac\t@ac\n' > "$WORK/lenses.tsv"
write_complete; expect 0 "comment and blank lines in the lens map are skipped"
printf 'code\trules/a.md\ntests\trules/b.md\nintent\t-\nac\t@ac\n' > "$WORK/lenses.tsv"

printf '## \n## Gamma \t\n' > rules/b.md && git commit -qam bare && SHA="$(git rev-parse HEAD)" && ATT="$REPO/.claude/markers/attest/$SHA"
write_complete; expect 0 "a bare heading line asks for no row"
printf 'code\trules/a.md\ntests\trules/b.md\nintent\t-\nintent\t-\nac\t@ac\n' > "$WORK/lenses.tsv"
write_complete; rm "$ATT/intent.md"
(cd "$REPO" && REVIEW_LENSES_MAP="$WORK/lenses.tsv" bash "$LIB" check "$SHA" >"$WORK/out" 2>&1)
[ "$(grep -c "lens 'intent'" "$WORK/out")" = 1 ] && ok "a lens listed twice reports its missing file once" || bad "duplicate lens gaps: $(grep -c "lens 'intent'" "$WORK/out")"
printf 'code\trules/a.md\ntests\trules/b.md\nintent\t-\nac\t@ac\n' > "$WORK/lenses.tsv"

printf '    ```\n## Epsilon\n' >> rules/b.md && git commit -qam epsilon && SHA="$(git rev-parse HEAD)" && ATT="$REPO/.claude/markers/attest/$SHA"
write_complete; expect 1 "a fence marker indented four spaces is not a fence, so the heading after it is still required"
printf '| rules/b.md | Epsilon | Checked | x.rs:1 |\n' >> "$ATT/tests.md"; expect 0 "...and attesting it completes the set"
git checkout -q HEAD~1 -- rules/b.md && git commit -qam no-epsilon && SHA="$(git rev-parse HEAD)" && ATT="$REPO/.claude/markers/attest/$SHA"

echo "Command line:"
write_complete
(cd "$REPO" && REVIEW_LENSES_MAP="$WORK/lenses.tsv" bash "$LIB" check >"$WORK/out" 2>&1) && grep -q "complete for ${SHA:0:8}" "$WORK/out" && ok "check with no SHA checks HEAD" || bad "check with no SHA: $(head -2 "$WORK/out")"
(cd "$REPO" && bash "$LIB" chek >/dev/null 2>&1); got=$?
[ "$got" = 2 ] && ok "an unknown subcommand exits 2" || bad "unknown subcommand exited $got"

echo "Gaps:"
write_complete; rm "$ATT/tests.md"
expect 1 "a lens file missing"
write_complete; sed '/Beta/d' "$ATT/code.md" > "$WORK/t" && mv "$WORK/t" "$ATT/code.md"
expect 1 "one heading missing"
write_complete; sed 's/| N\/A |/| Skipped |/' "$ATT/code.md" > "$WORK/t" && mv "$WORK/t" "$ATT/code.md"
expect 1 "unknown status"
write_complete; sed 's/| backend\/src\/foo.rs:12 |/|  |/' "$ATT/code.md" > "$WORK/t" && mv "$WORK/t" "$ATT/code.md"
expect 1 "empty evidence"
write_complete; printf '| Comment | x | Checked | y |\n' > "$ATT/intent.md"
expect 0 "intent lens needs any one row, not an AC row"
write_complete; : > "$ATT/intent.md"
expect 1 "intent lens with no rows"
write_complete; printf '| Scope | 1 | Bogus | x.rs:1 |\n' > "$ATT/intent.md"
expect 1 "intent row with an unknown status"
write_complete; printf '| Scope | 1 | Checked | x.rs:1\n' > "$ATT/intent.md"
expect 1 "a row missing its closing pipe is not a row"
write_complete; printf '| Doc | docs/x.md | N/A | no claim changed |\n' > "$ATT/intent.md"
expect 0 "intent row with status N/A"
write_complete; sed 's/^| rules\/a.md | Beta/note | rules\/a.md | Beta/' "$ATT/code.md" > "$WORK/t" && mv "$WORK/t" "$ATT/code.md"
grep -q '^note | rules/a.md | Beta' "$ATT/code.md" || bad "prose-row fixture not written"
expect 1 "a pipe-bearing prose line is not a table row"
write_complete; printf '| Scope | 1 | Checked |  |\n' > "$ATT/intent.md"
expect 1 "intent row with empty evidence"
write_complete; sed 's/| rules\/a.md | Beta `code` and \\Throwable | N\/A | no cache writes in the diff |/| rules\/a.md | Beta `code` and \\Throwable | N\/A |/' "$ATT/code.md" > "$WORK/t" && mv "$WORK/t" "$ATT/code.md"
grep -q 'Throwable | N/A |$' "$ATT/code.md" || bad "three-cell fixture not written"
expect 1 "a three-cell row does not inherit the previous row's evidence"
printf 'code\trules/a.md\ncode\trules/missing.md\ntests\trules/b.md\nintent\t-\nac\t@ac\n' > "$WORK/lenses.tsv"
write_complete; expect 1 "an owned file missing at the commit"
printf 'code\trules/a.md\ncode\ntests\trules/b.md\nintent\t-\nac\t@ac\n' > "$WORK/lenses.tsv"
write_complete; expect 1 "a lens-map line with no file column"
printf 'code\trules/a.md\ntests\trules/b.md\nintent\t-\nac\t@ac\n' > "$WORK/lenses.tsv"
write_complete; sed 's/Alpha — with a dash/Alpha - with a dash/' "$ATT/code.md" > "$WORK/t" && mv "$WORK/t" "$ATT/code.md"
expect 1 "heading text paraphrased"
write_complete; sed 's/rules\/a.md | Alpha/rules\/b.md | Alpha/' "$ATT/code.md" > "$WORK/t" && mv "$WORK/t" "$ATT/code.md"
expect 1 "heading attested against the wrong file"

echo "Acceptance-criteria lens (@ac):"
# ac_fixture COUNT-ROW [AC-ROWS...]: replace ac.md with exactly these rows.
ac_fixture() { write_complete; printf '%s\n' "$@" > "$ATT/ac.md"; }
C2='| AC-COUNT | 2 | Checked | #1 fetched 2026-10-08T10:00Z |'
A1='| AC | 1. Admin sees the banner | Checked | banner.tsx:4 |'
A2='| AC | 2. Banner hides after its end date | Checked | banner.test.tsx:30 |'
ac_fixture "$C2" "$A1"
expect 1 "fewer AC rows than the fetched section has (a stale list)"
grep -q 'attests 1 AC rows; the section it fetched has 2' "$WORK/out" && ok "...and the gap names both counts" || bad "count gap message: $(head -2 "$WORK/out")"
ac_fixture "$C2" "$A1" "$A2" '| AC | 3. Extra | Checked | x.rs:1 |'
expect 1 "more AC rows than the fetched section has"
ac_fixture "$C2" "$A1" '| AC | 2. Banner hides after its end date | Finding | CRITICAL no end-date check |'
expect 1 "an unmet AC blocks"
grep -q 'AC not met: 2. Banner hides after its end date' "$WORK/out" && ok "...and the gap names the unmet AC" || bad "unmet message: $(head -2 "$WORK/out")"
ac_fixture "$C2" "$A1" '| AC | 2. Banner hides after its end date | N/A | out of scope |'
expect 1 "N/A is not a way to skip an AC"
ac_fixture "$C2" "$A1" '| AC | 2. Banner hides after its end date | Checked |  |'
expect 1 "an AC row with empty evidence"
ac_fixture "$A1" "$A2"
expect 1 "no AC-COUNT row"
ac_fixture "$C2" "$C2" "$A1" "$A2"
expect 1 "two AC-COUNT rows"
ac_fixture '| AC-COUNT | two | Checked | #1 |' "$A1" "$A2"
expect 1 "a non-numeric AC-COUNT"
ac_fixture '| AC-COUNT | 2x | Checked | #1 fetched 2026-10-08T10:00Z |' "$A1" "$A2"
expect 1 "an AC-COUNT with trailing text after the number"
ac_fixture '| AC-COUNT | 2 | Finding | #1 fetched 2026-10-08T10:00Z |' "$A1" "$A2"
expect 1 "an AC-COUNT whose status is not Checked"
ac_fixture "$C2" "$A1" "$A2"
expect 0 "positive control: the same rows with a Checked count pass"
ac_fixture '| AC-COUNT | 2 | Checked |  |' "$A1" "$A2"
expect 1 "an AC-COUNT with no issue or fetch time"
ac_fixture "$C2" "$A1" '| AC | 2. Banner hides after its end date | Checked | Carried-from 0123abc: banner.test.tsx:30 |'
expect 0 "an AC row carried with its evidence"
ac_fixture "$C2" "$A1" '| AC | 2. Banner hides after its end date | Checked | Carried-from 0123abc: |'
expect 1 "an AC row carried with no evidence"
ac_fixture '| AC-COUNT | 0 | Checked | #1 fetched 2026-10-08T10:00Z, no section |'
expect 0 "a ticket with no section attests a count of 0"
write_complete; rm "$ATT/ac.md"
expect 1 "the ac lens's file missing"

echo "Lens map without a final newline:"
printf 'code\trules/a.md\ntests\trules/b.md\nintent\t-\nac\t@ac' > "$WORK/lenses.tsv"
write_complete; expect 0 "complete with a newline-less map"
rm "$ATT/ac.md"
expect 1 "the last lens is still required"
printf 'code\trules/a.md\ntests\trules/b.md\nintent\t-\nac\t@ac\n' > "$WORK/lenses.tsv"

echo "Reads the reviewed commit, not the working tree:"
write_complete; printf '## Delta\n' >> rules/b.md
expect 0 "an uncommitted new heading is not required"
git checkout -q -- rules/b.md
printf '## Delta\n' >> rules/b.md && git commit -qam delta
SHA="$(git rev-parse HEAD)"; ATT="$REPO/.claude/markers/attest/$SHA"
write_complete; expect 1 "a committed new heading is required"

echo "The checker's own lens map wins over the branch's:"
LAYOUT="$WORK/layout"
mkdir -p "$LAYOUT/bin/lib" "$LAYOUT/.claude" "$REPO/.claude"
cp "$LIB" "$LAYOUT/bin/lib/review-attestation.sh"
printf 'code\trules/a.md\ntests\trules/b.md\nintent\t-\nac\t@ac\n' > "$LAYOUT/.claude/review-lenses.tsv"
printf 'code\trules/a.md\n' > "$REPO/.claude/review-lenses.tsv"
write_complete; rm "$ATT/intent.md"
(cd "$REPO" && bash "$LAYOUT/bin/lib/review-attestation.sh" check "$SHA" >"$WORK/out" 2>&1)
if [[ $? == 1 ]] && grep -q "lens 'intent'" "$WORK/out"; then ok "a lens the branch's map drops is still required"; else bad "branch map dropped a lens: $(head -2 "$WORK/out")"; fi
rm "$REPO/.claude/review-lenses.tsv"

echo "More than 40 gaps says how many were left out:"
i=0; : > "$WORK/many.md"; while [ $i -lt 45 ]; do printf '## H%s\n' "$i" >> "$WORK/many.md"; i=$((i + 1)); done
cp "$WORK/many.md" "$REPO/rules/many.md" && git -C "$REPO" add rules/many.md && git -C "$REPO" commit -qm many
SHA="$(git -C "$REPO" rev-parse HEAD)"; ATT="$REPO/.claude/markers/attest/$SHA"
printf 'code\trules/many.md\n' > "$WORK/lenses.tsv"
mkdir -p "$ATT" && printf '| x | y | Checked | z |\n' > "$ATT/code.md"
(cd "$REPO" && REVIEW_LENSES_MAP="$WORK/lenses.tsv" bash -c '. "$1"; check_attestation_for_head' _ "$LIB" >"$WORK/out" 2>&1)
if grep -q 'and 5 more' "$WORK/out"; then ok "the 41st to 45th gaps are counted"; else bad "dropped gaps not counted: $(tail -2 "$WORK/out")"; fi
grep -q '^BLOCKED: the review lenses did not attest every rule heading' "$WORK/out" && ok "an incomplete attestation is reported as BLOCKED" || bad "no BLOCKED headline: $(head -1 "$WORK/out")"
[ "$(grep -c '^GAP:' "$WORK/out")" = 40 ] && ok "exactly 40 gaps are shown when there are more" || bad "shown gaps: $(grep -c '^GAP:' "$WORK/out")"
head -40 "$WORK/many.md" > "$REPO/rules/many.md" && git -C "$REPO" commit -qam forty
SHA="$(git -C "$REPO" rev-parse HEAD)"; ATT="$REPO/.claude/markers/attest/$SHA"
mkdir -p "$ATT" && printf '| x | y | Checked | z |\n' > "$ATT/code.md"
(cd "$REPO" && REVIEW_LENSES_MAP="$WORK/lenses.tsv" bash -c '. "$1"; check_attestation_for_head' _ "$LIB" >"$WORK/out" 2>&1)
if grep -q 'H39' "$WORK/out" && ! grep -q 'more:' "$WORK/out"; then ok "exactly 40 gaps are all shown, with no count line"; else bad "40-gap boundary: $(tail -2 "$WORK/out")"; fi
printf 'code\trules/a.md\ntests\trules/b.md\nintent\t-\nac\t@ac\n' > "$WORK/lenses.tsv"

echo "prior (a repeat round's baseline):"
PR="$WORK/prior"
mkdir -p "$PR/rules"
git -C "$PR" init -q .
git -C "$PR" config user.email t@example.com
git -C "$PR" config user.name t
printf 'code\trules/a.md\n' > "$WORK/prior.tsv"
printf '## Alpha\n' > "$PR/rules/a.md"
git -C "$PR" add rules && git -C "$PR" commit -qm c1
C1="$(git -C "$PR" rev-parse HEAD)"
git -C "$PR" commit -q --allow-empty -m c2; C2SHA="$(git -C "$PR" rev-parse HEAD)"
git -C "$PR" commit -q --allow-empty -m c3; C3="$(git -C "$PR" rev-parse HEAD)"
git -C "$PR" checkout -q -b side "$C1" && git -C "$PR" commit -q --allow-empty -m side
SIDE="$(git -C "$PR" rev-parse HEAD)"
git -C "$PR" checkout -q -
# attest_at SHA [EVIDENCE]: a one-row set for SHA; an empty EVIDENCE makes it incomplete.
attest_at() { mkdir -p "$PR/.claude/markers/attest/$1" && printf '| rules/a.md | Alpha | Checked | %s |\n' "${2-x.rs:1}" > "$PR/.claude/markers/attest/$1/code.md"; }
# expect_prior WANT-SHA-OR-EMPTY NAME [SHA-ARG]
expect_prior() {
    local got rc
    got="$(cd "$PR" && REVIEW_LENSES_MAP="$WORK/prior.tsv" bash "$LIB" prior ${3:+"$3"} 2>"$WORK/err")"; rc=$?
    if [[ -n "$1" && "$rc" == 0 && "$got" == "$1" ]] || [[ -z "$1" && "$rc" == 1 && -z "$got" ]]; then ok "$2"; else bad "$2 (want '${1:0:8}', got '${got:0:8}' exit $rc)"; fi
}
expect_prior "" "no attestation set is a first round"
grep -q 'a first round' "$WORK/err" && ok "...and says so on stderr" || bad "first-round message: $(cat "$WORK/err")"
attest_at "$C1"
expect_prior "$C1" "the one complete set behind HEAD"
attest_at "$C2SHA"
expect_prior "$C2SHA" "the nearest of two complete sets"
attest_at "$C2SHA" ""
expect_prior "$C1" "an incomplete nearer set is passed over for the complete one behind it"
attest_at "$C3"
expect_prior "$C3" "a complete set at HEAD itself (a dirty-checkout round)"
expect_prior "$C1" "a named commit never takes a later commit's set" "$C2SHA"
rm -rf "$PR/.claude/markers"; attest_at "$SIDE"
expect_prior "" "a complete set on a commit HEAD does not descend from is not a baseline"
attest_at "$C1"
got="$(cd "$WORK" && GIT_CEILING_DIRECTORIES="$WORK" REVIEW_LENSES_MAP="$WORK/prior.tsv" bash "$LIB" prior 2>/dev/null)"; rc=$?
[[ "$rc" == 1 && -z "$got" ]] && ok "outside a repository there is no baseline" || bad "prior outside a repository: '$got' exit $rc"

echo "A tests-only carry (a copied attestation):"
TC="$WORK/carry"
mkdir -p "$TC/rules" "$TC/backend/src" "$TC/backend/tests" "$TC/web/src" "$TC/bin/lib/sub"
git -C "$TC" init -q .
git -C "$TC" config user.email t@example.com
git -C "$TC" config user.name t
printf 'code\trules/a.md\ntests\trules/b.md\n' > "$WORK/carry.tsv"
printf '## Alpha\n' > "$TC/rules/a.md"; printf '## Gamma\n' > "$TC/rules/b.md"
printf 'a\n' > "$TC/backend/src/foo.rs"; printf 'a\n' > "$TC/backend/src/foo_tests.rs"; printf 'a\n' > "$TC/backend/tests/scope.rs"
printf 'a\n' > "$TC/web/src/foo.test.ts"; printf 'a\n' > "$TC/web/src/bar.test.tsx"; printf 'a\n' > "$TC/bin/lib/x.test.sh"
printf 'a\n' > "$TC/bin/lib/sub/y.test.sh"; printf 'a\n' > "$TC/web/src/foo.ts"
git -C "$TC" add . && git -C "$TC" commit -qm prior
TP="$(git -C "$TC" rev-parse HEAD)"
git -C "$TC" checkout -q -b side && printf 's\n' > "$TC/web/src/side.test.ts" && git -C "$TC" add . && git -C "$TC" commit -qm side
TSIDE="$(git -C "$TC" rev-parse HEAD)"
git -C "$TC" checkout -q -
# Every kind of test path changes at once: vitest .ts and .tsx, a tests/ directory, a tooling shell test.
printf 'b\n' | tee "$TC/web/src/foo.test.ts" "$TC/web/src/bar.test.tsx" "$TC/backend/tests/scope.rs" > "$TC/bin/lib/x.test.sh"
git -C "$TC" commit -qam tests-only
TH="$(git -C "$TC" rev-parse HEAD)"
git -C "$TC" commit -q --allow-empty -m empty; TEMPTY="$(git -C "$TC" rev-parse HEAD)"
# carry_set AT FROM [LENS]: complete sets at FROM and AT; at AT, LENS (default code) is a copy of FROM's under the carry line.
carry_set() {
    local at="$1" from="$2" lens="${3:-code}" c
    rm -rf "$TC/.claude/markers"
    for c in "$from" "$at"; do
        mkdir -p "$TC/.claude/markers/attest/$c"
        printf '| rules/a.md | Alpha | Checked | backend/src/foo.rs:1 |\n' > "$TC/.claude/markers/attest/$c/code.md"
        printf '| rules/b.md | Gamma | Checked | web/src/foo.test.ts:1 |\n' > "$TC/.claude/markers/attest/$c/tests.md"
    done
    { printf 'Carried-from %s: tests-only delta\n' "$from"; cat "$TC/.claude/markers/attest/$from/$lens.md"; } > "$TC/.claude/markers/attest/$at/$lens.md"
}
# expect_carry WANT NAME AT [MESSAGE-FRAGMENT]
expect_carry() {
    local got
    (cd "$TC" && REVIEW_LENSES_MAP="$WORK/carry.tsv" bash "$LIB" check "$3" >"$WORK/out" 2>&1); got=$?
    if [[ "$got" == "$1" ]] && { [[ -z "${4:-}" ]] || grep -qF "$4" "$WORK/out"; }; then ok "$2"; else bad "$2 (want $1 ${4:+and '$4'}, got $got): $(head -2 "$WORK/out")"; fi
}
# touch_commit PATH: a commit off TH changing PATH alone; sets TAT. Run from TH each time.
touch_commit() {
    git -C "$TC" checkout -q --detach "$TH" && printf 'c\n' >> "$TC/$1" && git -C "$TC" commit -qam "touch $1"
    TAT="$(git -C "$TC" rev-parse HEAD)"
}
carry_set "$TH" "$TP"
expect_carry 0 "a copy of the prior attestation over a delta of vitest .ts/.tsx files, a tests/ directory and a tooling shell test" "$TH"
carry_set "$TH" "$TP"; sed "1s/.*/Carried-from ${TP:0:7}: tests-only delta/" "$TC/.claude/markers/attest/$TH/code.md" > "$WORK/t" && mv "$WORK/t" "$TC/.claude/markers/attest/$TH/code.md"
expect_carry 0 "...naming the prior commit by its first seven characters" "$TH"
carry_set "$TH" "$TP" tests
expect_carry 1 "a lens that reads test files cannot be carried" "$TH" "cannot be carried"
carry_set "$TH" "$TP"; sed "1s/.*/Carried-from 0123abc: tests-only delta/" "$TC/.claude/markers/attest/$TH/code.md" > "$WORK/t" && mv "$WORK/t" "$TC/.claude/markers/attest/$TH/code.md"
expect_carry 1 "a carry line naming a commit that does not exist" "$TH" "does not resolve"
carry_set "$TH" "$TP"; sed "1s/.*/Carried-from HEAD~1: tests-only delta/" "$TC/.claude/markers/attest/$TH/code.md" > "$WORK/t" && mv "$WORK/t" "$TC/.claude/markers/attest/$TH/code.md"
expect_carry 1 "a carry line naming a ref, not a hex commit" "$TH" "7 to 40 hex"
carry_set "$TH" "$TSIDE"
expect_carry 1 "a prior commit the reviewed one does not descend from" "$TH" "not an earlier commit"
carry_set "$TEMPTY" "$TH"
expect_carry 1 "a prior commit with no file changed since" "$TEMPTY" "no file differs"
# Each of these holds production code, or is not a test file this checker knows.
for path in backend/src/foo.rs backend/src/foo_tests.rs web/src/foo.ts bin/lib/sub/y.test.sh; do
    touch_commit "$path"
    carry_set "$TAT" "$TH"
    expect_carry 1 "a delta touching $path is not tests-only" "$TAT" "not tests-only: $path"
done
git -C "$TC" checkout -q --detach "$TH"
git -C "$TC" mv backend/src/foo.rs backend/tests/foo.rs && git -C "$TC" commit -qm "move production code under tests"; TMOVE="$(git -C "$TC" rev-parse HEAD)"
same="$(git -C "$TC" diff --name-only "$TH" "$TMOVE" | tr '\n' ' ')"
[[ "$same" == "backend/tests/foo.rs " ]] && ok "positive control: git lists only the destination of that move" || bad "the move fixture is not a rename: '$same'"
carry_set "$TMOVE" "$TH"
expect_carry 1 "a delta that moves a file from backend/src/ into backend/tests/" "$TMOVE" "not tests-only: backend/src/foo.rs"
git -C "$TC" mv web/src/foo.test.ts web/src/moved.test.ts && git -C "$TC" commit -qm "move within tests"; TWITHIN="$(git -C "$TC" rev-parse HEAD)"
carry_set "$TWITHIN" "$TMOVE"
expect_carry 0 "a move from one test file to another is still a tests-only delta" "$TWITHIN"
carry_set "$TH" "$TP"; rm "$TC/.claude/markers/attest/$TP/tests.md"
expect_carry 1 "a prior set that is not complete" "$TH" "not complete"
carry_set "$TH" "$TP"; printf '| rules/a.md | Alpha | Checked | written by hand |\n' >> "$TC/.claude/markers/attest/$TH/code.md"
expect_carry 1 "a copy whose rows differ from the prior attestation" "$TH" "rows differ"
carry_set "$TH" "$TP"; printf '| rules/a.md | Alpha | Checked | backend/src/foo.rs:1 |\n' > "$TC/.claude/markers/attest/$TH/code.md"
expect_carry 0 "positive control: the same commit with a lens-written file passes without a carry line" "$TH"
carry_set "$TH" "$TP"
got="$(cd "$TC" && REVIEW_LENSES_MAP="$WORK/carry.tsv" bash "$LIB" prior "$TH" 2>/dev/null)"
[[ "$got" == "$TH" ]] && ok "a set holding a valid carry is a baseline for the next round" || bad "prior over a carried set: '${got:0:8}'"

echo "Fails closed:"
write_complete
(cd "$REPO" && REVIEW_LENSES_MAP="$WORK/absent.tsv" bash "$LIB" check "$SHA" >/dev/null 2>&1) && bad "missing lens map passed" || ok "missing lens map refused"
(cd "$WORK" && GIT_CEILING_DIRECTORIES="$WORK" REVIEW_LENSES_MAP="$WORK/lenses.tsv" bash "$LIB" check deadbeef >"$WORK/out" 2>&1)
if [[ $? == 1 ]] && grep -q 'cannot resolve' "$WORK/out"; then ok "outside a repository refused"; else bad "outside a repository: $(head -2 "$WORK/out")"; fi
(cd "$REPO" && REVIEW_LENSES_MAP="$WORK/lenses.tsv" bash -c '. "$1"; check_attestation ""' _ "$LIB" >"$WORK/out" 2>&1)
if [[ $? == 1 ]] && grep -q 'cannot resolve' "$WORK/out"; then ok "an empty commit refused"; else bad "empty commit: $(head -2 "$WORK/out")"; fi

echo ""
echo "review-attestation: $PASS passed, $FAIL failed"
(( FAIL == 0 ))
