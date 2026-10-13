#!/bin/bash
# ==============================================================================
# Tests for bin/pre-commit: the debug-instrumentation check, the files it hands
# rustfmt, gitleaks and the migration-version check.
#
#   bash bin/lib/pre-commit.test.sh
#
# Runs the hook in a scratch repository with stub rustfmt and gitleaks first on PATH
# and a stub .github/scripts/check-migration-versions.sh, each recording its
# arguments and scripted through STUB_* variables. The debug check reads the staged
# diff's added lines: a swap (one call removed, another added) must still fail.
# Exits non-zero on any failure.
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
HOOK="$ROOT/bin/pre-commit"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/pre-commit-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR
export GIT_AUTHOR_NAME=T GIT_AUTHOR_EMAIL=t@e.com GIT_COMMITTER_NAME=T GIT_COMMITTER_EMAIL=t@e.com
export GIT_CONFIG_NOSYSTEM=1 HOME="$WORK"

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

export STUB_LOG="$WORK/stub.log"
mkdir -p "$WORK/bin"
printf '#!/bin/bash\necho "rustfmt $*" >> "$STUB_LOG"\nexit "${STUB_RUSTFMT_RC:-0}"\n' > "$WORK/bin/rustfmt"
printf '#!/bin/bash\necho "gitleaks $*" >> "$STUB_LOG"\nexit "${STUB_GITLEAKS_RC:-0}"\n' > "$WORK/bin/gitleaks"
chmod +x "$WORK/bin/rustfmt" "$WORK/bin/gitleaks"

REPO="$WORK/repo"
mkdir -p "$REPO/backend/src" "$REPO/backend/migrations" "$REPO/web/src" "$REPO/.github/scripts"
printf '#!/bin/bash\necho "migrations $*" >> "$STUB_LOG"\necho "duplicate migration version 0042"\nexit "${STUB_MIGRATIONS_RC:-0}"\n' > "$REPO/.github/scripts/check-migration-versions.sh"
chmod +x "$REPO/.github/scripts/check-migration-versions.sh"
printf 'fn main() {\n    let a = 1;\n    dbg!(a);\n}\n' > "$REPO/backend/src/existing.rs"
printf 'export const a = 1;\n' > "$REPO/web/src/existing.ts"
printf 'create table t ();\n' > "$REPO/backend/migrations/0001_init.sql"
git -C "$REPO" init -q -b next && git -C "$REPO" add -A && git -C "$REPO" -c core.hooksPath=/dev/null commit -qm init

# hook_output -> the hook's output for what is staged now, color codes stripped; stub calls land in $STUB_LOG
hook_output() {
    : > "$STUB_LOG"
    (cd "$REPO" && PATH="$WORK/bin:$PATH" bash "$HOOK" 2>&1) | sed $'s/\033\\[[0-9;]*m//g'
}
# verdict CHECK -> that check's verdict for what is staged now
verdict() { hook_output | sed -n "s/.*$1\\.\\.\\. //p"; }
reset_repo() { git -C "$REPO" reset -q --hard && git -C "$REPO" clean -qfd; }
# expect WANT NAME — the debug check's verdict, then reset the index and tree to the committed state
expect() {
    local got
    got="$(verdict 'Debug instrumentation')"
    if [[ "$got" == "$1" ]]; then ok "$1: $2"; else bad "$2 (want $1, got '$got')"; fi
    reset_repo
}
same() { if [[ "$2" == "$3" ]]; then ok "$1"; else bad "$1 (got '$2', want '$3')"; fi; }

echo "Debug instrumentation, read from the added lines:"
printf 'fn f() {\n    dbg!(x);\n}\n' > "$REPO/backend/src/new.rs"; git -C "$REPO" add backend/src/new.rs
expect fail "a new dbg!() call"
printf 'fn main() {\n    let a = 1;\n    dbg!(a + 1);\n}\n' > "$REPO/backend/src/existing.rs"; git -C "$REPO" add backend/src/existing.rs
expect fail "a swap: one dbg!() removed, another added"
printf 'fn main() {\n    let a = 1;\n}\n' > "$REPO/backend/src/existing.rs"; git -C "$REPO" add backend/src/existing.rs
expect pass "only removing a dbg!()"
printf 'fn f() {\n    dbg! (x);\n}\n' > "$REPO/backend/src/new.rs"; git -C "$REPO" add backend/src/new.rs
expect fail "dbg! with a space before the paren"
printf 'fn f() {\n    my_dbg!(x);\n    let dbg = 1;\n    tracing::debug!(x);\n}\n' > "$REPO/backend/src/new.rs"; git -C "$REPO" add backend/src/new.rs
expect pass "my_dbg!(), a dbg variable and tracing::debug!() are not debug calls"
printf 'console.log(x);\n' > "$REPO/web/src/new.ts"; git -C "$REPO" add web/src/new.ts
expect fail "a new console.log() in .ts"
printf 'export function A() {\n  console.log("render");\n  return null;\n}\n' > "$REPO/web/src/new.tsx"; git -C "$REPO" add web/src/new.tsx
expect fail "a new console.log() in .tsx"
printf 'function f() {\n  debugger;\n}\n' > "$REPO/web/src/new.ts"; git -C "$REPO" add web/src/new.ts
expect fail "a bare debugger statement"
printf 'console.error(x);\nconsole.warn(x);\nconst debuggerPanel = 1;\n// open the debugger here\nmyconsole.log(x);\n' > "$REPO/web/src/new.ts"; git -C "$REPO" add web/src/new.ts
expect pass "console.error/warn, a debugger identifier, a comment and myconsole.log are not debug calls"
printf 'console.log(x);\n' > "$REPO/web/src/new.test.ts"; printf 'console.log(x);\n' > "$REPO/web/src/new.test.tsx"
mkdir -p "$REPO/backend/tests"; printf 'fn t() { dbg!(x); }\n' > "$REPO/backend/tests/scope.rs"; printf 'fn t() { dbg!(x); }\n' > "$REPO/backend/src/scope_tests.rs"
git -C "$REPO" add -A
expect pass "test files (*.test.ts, *.test.tsx, tests/, *_tests.rs) are left out"
printf 'dbg!(x)\nconsole.log(x)\n' > "$REPO/notes.md"; git -C "$REPO" add notes.md
expect pass "a file that is neither Rust nor TypeScript"
printf 'fn main() {\n    let a = 1;\n    dbg!(a);\n    dbg!(a + 2);\n}\n' > "$REPO/backend/src/existing.rs"
expect pass "an unstaged change to a tracked file is not checked"

echo "A file moved and edited in one commit is still checked (git lists it as a rename, status R):"
{ for n in 1 2 3 4 5 6 7 8 9; do printf 'pub const VALUE%s: &str = "a line long enough to keep the move a rename";\n' "$n"; done; } > "$REPO/backend/src/long.rs"
git -C "$REPO" add backend/src/long.rs && git -C "$REPO" -c core.hooksPath=/dev/null commit -qm long
# stage_move ADDED-LINE: backend/src/long.rs moved to backend/src/moved.rs with one line added.
stage_move() {
    git -C "$REPO" mv backend/src/long.rs backend/src/moved.rs
    printf '%s\n' "$1" >> "$REPO/backend/src/moved.rs"; git -C "$REPO" add -A
    if [[ "$(git -C "$REPO" diff --cached --name-status | cut -c1)" != R ]]; then bad "fixture: the staged move is not a rename"; fi
}
stage_move 'fn f() { dbg!(x); }'
expect fail "a dbg!() added to a moved file"
stage_move 'pub const VALUE10: u8 = 1;'
same "a moved-and-edited .rs file passes rustfmt" "$(verdict 'rustfmt (code style)')" "pass"
same "...and is the file rustfmt checks" "$(cat "$STUB_LOG" | grep '^rustfmt')" "rustfmt --edition 2024 --check -- backend/src/moved.rs"
reset_repo

echo "rustfmt sees the staged .rs files only:"
printf 'fn a() {}\n' > "$REPO/backend/src/a.rs"; printf 'fn b() {}\n' > "$REPO/backend/src/b.rs"; printf 'export const b = 1;\n' > "$REPO/web/src/b.ts"
git -C "$REPO" add backend/src/a.rs web/src/b.ts
same "a staged .rs file passes" "$(verdict 'rustfmt (code style)')" "pass"
same "...checked alone, edition 2024; the unstaged one and the .ts file dropped" "$(grep '^rustfmt' "$STUB_LOG")" "rustfmt --edition 2024 --check -- backend/src/a.rs"
export STUB_RUSTFMT_RC=1
same "a file rustfmt would reformat fails" "$(verdict 'rustfmt (code style)')" "fail (run \`just fmt\`, re-stage, commit)"
unset STUB_RUSTFMT_RC
reset_repo
printf 'export const b = 2;\n' > "$REPO/web/src/existing.ts"; git -C "$REPO" add web/src/existing.ts
same "no staged .rs skips rustfmt" "$(verdict 'rustfmt (code style)')" "skip (no Rust files)"
same "...without running it" "$(grep -c '^rustfmt' "$STUB_LOG")" "0"
reset_repo

echo "gitleaks scans the staged changes:"
printf 'export const b = 2;\n' > "$REPO/web/src/existing.ts"; git -C "$REPO" add web/src/existing.ts
same "a clean scan passes" "$(verdict 'Sensitive data check')" "pass"
same "...run on the staged changes, redacted" "$(grep '^gitleaks' "$STUB_LOG" | head -1)" "gitleaks protect --staged --redact --no-banner"
export STUB_GITLEAKS_RC=1
out="$(hook_output)"
same "a finding fails" "$(sed -n 's/.*Sensitive data check\.\.\. //p' <<<"$out")" "fail"
grep -q 'Pre-commit checks failed' <<<"$out" && ok "...and fails the commit" || bad "...and fails the commit: $out"
unset STUB_GITLEAKS_RC
mkdir -p "$WORK/nogitleaks"
for tool in git sed cat grep head dirname basename; do ln -sf "$(command -v "$tool")" "$WORK/nogitleaks/$tool"; done
ln -sf "$WORK/bin/rustfmt" "$WORK/nogitleaks/rustfmt"
got="$( (cd "$REPO" && PATH="$WORK/nogitleaks" /bin/bash "$HOOK" 2>&1) | sed $'s/\033\\[[0-9;]*m//g' | sed -n 's/.*Sensitive data check\.\.\. //p')"
case "$got" in "skip (gitleaks is not installed"*) ok "no gitleaks is a named skip, not a pass" ;; *) bad "no gitleaks (got '$got')" ;; esac
reset_repo

echo "The migration-version check runs when a migration is staged:"
printf 'create table u ();\n' > "$REPO/backend/migrations/0002_u.sql"; git -C "$REPO" add backend/migrations/0002_u.sql
same "a staged migration with unique versions passes" "$(verdict 'Migration versions')" "pass"
export STUB_MIGRATIONS_RC=1
out="$(hook_output)"
same "a duplicate version fails" "$(sed -n 's/.*Migration versions\.\.\. //p' <<<"$out")" "fail"
grep -q 'duplicate migration version 0042' <<<"$out" && ok "...and shows the script's output" || bad "...and shows the script's output: $out"
unset STUB_MIGRATIONS_RC
reset_repo
printf 'export const b = 2;\n' > "$REPO/web/src/existing.ts"; git -C "$REPO" add web/src/existing.ts
same "no staged migration skips the check" "$(verdict 'Migration versions')" "skip (no migration staged)"
same "...without running it" "$(grep -c '^migrations' "$STUB_LOG")" "0"
reset_repo

echo ""
echo "pre-commit: $PASS passed, $FAIL failed"
(( FAIL == 0 ))
