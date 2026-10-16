#!/bin/bash
# Deterministic pitfall scan for /code-review Phase 7.
#
# A script so it can run from a worktree session, where the harness refuses git fed
# to bash through a heredoc or $(...), and so the file list is word-split by bash
# rather than the zsh the Bash tool uses.
#
# Two layers. Each hit is a prompt to look, not proof: confirm it against the diff.
#
# 1. Sync invariants over the committed diff, BASE...HEAD, read from HEAD's tree (commit
#    first: the review is of a commit). Each prints `file:line: message`. They are the
#    mistakes that compile, pass the tests, and still break something (AGENTS.md):
#      * permission three-in-sync: a `permission!` marker, its catalog.rs string, an access.permissions row;
#      * role three-in-sync: an access.roles row, default_roles(), ASSIGNABLE_USER_ROLES;
#      * a handler registered in router.rs but not openapi.rs, or the reverse;
#      * SQL in handlers/ (it belongs in repos/);
#      * an applied migration edited, renamed or deleted in place (they are append-only);
#      * a mutating route whose handler takes no RequirePermission<P>, or a RequirePermission removed;
#      * unwrap()/expect() in handlers/.
#    Test code (`*_tests.rs`, and everything after a handler file's first `#[cfg(test)]`) is skipped.
#
# 2. Patterns over whole changed files (real file:line), across the union of committed-vs-
#    BASE, staged, unstaged and untracked, so a hit may be pre-existing. Each check prints
#    grep's `file:line:text` hits, then `^ message`. Only .rs, .ts and .tsx files are read,
#    so this script, the shell tests' fixtures (bin/lib/*.test.sh) and the guidance under
#    .claude/ never report themselves.
#
# Exit 0 with hits printed; exit 1 when the scan itself could not run (a failed git call,
# an unknown base, an unreadable file), so a partial scan never passes. The last line,
# `scan covered N file(s)`, is the positive control: 0 on a branch with changes means the
# file list was empty, not clean.
#
# Usage: bash bin/review-scan.sh [base-ref]    (from the checkout under review; default origin/next)
# Tests: bin/lib/review-scan.test.sh
set -uo pipefail

fail() { echo "SCAN FAILED: $*" >&2; exit 1; }

# git prints paths relative to the root, so run from there.
cd "$(git rev-parse --show-toplevel 2>/dev/null)" || fail "not inside a git checkout"
base="${1:-origin/next}"
mb="$(git merge-base "$base" HEAD 2>/dev/null)" || fail "no merge-base between $base and HEAD"

PERMS=backend/src/auth/permissions.rs
CATALOG=crates/ois-core/src/catalog.rs
ACCESS_REPO=backend/src/repos/access.rs
ROUTER=backend/src/router.rs
OPENAPI=backend/src/openapi.rs
HANDLERS=backend/src/handlers
MIGRATIONS=backend/migrations

tmp="$(mktemp -d)" || fail "mktemp"
trap 'rm -rf "$tmp"' EXIT
findings="$tmp/findings"
: >"$findings"

finding() { printf '%s:%s: %s\n' "$1" "$2" "$3" >>"$findings"; }

# Every added line as `file<TAB>new-line<TAB>text`, every removed one as `file<TAB>old-line<TAB>text`.
git diff -U0 --no-color --no-ext-diff "$mb" HEAD >"$tmp/diff" || fail "git diff $mb HEAD"
awk -v added="$tmp/added" -v removed="$tmp/removed" '
    /^diff --git / { file = ""; old = ""; next }
    /^--- a\// { old = substr($0, 7); next }
    /^--- / { old = ""; next }
    /^\+\+\+ b\// { file = substr($0, 7); next }
    /^\+\+\+ / { file = ""; next }
    /^@@ / {
        split($2, o, ","); oline = substr(o[1], 2) + 0
        split($3, a, ","); nline = substr(a[1], 2) + 0
        next
    }
    /^\+/ { if (file != "") print file "\t" nline "\t" substr($0, 2) > added; nline++; next }
    /^-/ { if (old != "") print old "\t" oline "\t" substr($0, 2) > removed; oline++; next }
' "$tmp/diff"
touch "$tmp/added" "$tmp/removed"

# head_has PATTERN PATH... — a fixed-string match anywhere in HEAD's copy of PATH(s).
head_has() { local p="$1"; shift; git grep -qF -e "$p" HEAD -- "$@" 2>/dev/null; }

# added_in PATH — the added lines of one file, as `line<TAB>text`.
added_in() { awk -F'\t' -v f="$1" '$1 == f { print $2 "\t" substr($0, length($1) + length($2) + 3) }' "$tmp/added"; }

# head_range PATH START_RE END_RE — `first last` line numbers of the block in HEAD's PATH.
head_range() {
    git show "HEAD:$1" 2>/dev/null | awk -v s="$2" -v e="$3" '
        start == 0 && $0 ~ s { start = NR; next }
        start > 0 && $0 ~ e { print start, NR; exit }'
}

# Regexes live in variables: bash 3.2 and 5 disagree about quoting inside a literal `=~` pattern.
MARKER_RE='permission!\([A-Za-z0-9_]+,[[:space:]]*\[([^]]*)\],[[:space:]]*([A-Za-z]+)\)'
PERM_STR_RE='"([a-z_]+(\.[a-z_]+)+)"'
ROLE_STR_RE='"([A-Z][A-Z0-9_]*)"'
PERM_NAME_RE='^[a-z_]+(\.[a-z_]+)+$'
ROLE_NAME_RE='^[A-Z][A-Z0-9_]*$'
COMMENT_RE='^[[:space:]]*//'
OPENAPI_PATH_RE='handlers::([a-z_][a-z0-9_]*)::([a-z_][a-z0-9_]*)'

# --- permissions: marker -> catalog + migration -------------------------------------------------
while IFS=$'\t' read -r line text; do
    [[ "$text" =~ $MARKER_RE ]] || continue
    segs="${BASH_REMATCH[1]}"
    action="$(printf '%s' "${BASH_REMATCH[2]}" | tr '[:upper:]' '[:lower:]')"
    name="$(printf '%s' "$segs" | tr -d '" ' | tr ',' '.').$action"
    head_has "\"$name\"" "$CATALOG" ||
        finding "$PERMS" "$line" "permission $name has a marker but no \"$name\" in $CATALOG (three-in-sync)"
    head_has "'$name'" "$MIGRATIONS" ||
        finding "$PERMS" "$line" "permission $name has a marker but no access.permissions row in any migration (three-in-sync)"
done < <(added_in "$PERMS")

# --- permissions + roles: catalog -> migration (and assignable roles) ----------------------------
roles_range="$(head_range "$CATALOG" 'fn default_roles' '^}')"
while IFS=$'\t' read -r line text; do
    if [[ "$text" =~ $PERM_STR_RE ]]; then
        name="${BASH_REMATCH[1]}"
        head_has "'$name'" "$MIGRATIONS" ||
            finding "$CATALOG" "$line" "permission $name is in the catalog but no migration inserts it into access.permissions (three-in-sync)"
    elif [[ -n "$roles_range" && "$text" =~ $ROLE_STR_RE ]]; then
        role="${BASH_REMATCH[1]}"
        read -r first last <<<"$roles_range"
        [[ $line -gt $first && $line -lt $last ]] || continue
        head_has "('$role'" "$MIGRATIONS" ||
            finding "$CATALOG" "$line" "role $role is in default_roles() but no migration inserts it into access.roles (three-in-sync)"
    fi
done < <(added_in "$CATALOG")

assignable_range="$(head_range "$ACCESS_REPO" 'pub const ASSIGNABLE_USER_ROLES' '^];')"
if [[ -n "$assignable_range" ]]; then
    read -r first last <<<"$assignable_range"
    while IFS=$'\t' read -r line text; do
        [[ $line -gt $first && $line -lt $last && "$text" =~ $ROLE_STR_RE ]] || continue
        role="${BASH_REMATCH[1]}"
        head_has "\"$role\"" "$CATALOG" ||
            finding "$ACCESS_REPO" "$line" "role $role is assignable but missing from default_roles() in $CATALOG (three-in-sync)"
        head_has "('$role'" "$MIGRATIONS" ||
            finding "$ACCESS_REPO" "$line" "role $role is assignable but no migration inserts it into access.roles (three-in-sync)"
    done < <(added_in "$ACCESS_REPO")
fi

# --- migrations: new rows -> catalog / default_roles / ASSIGNABLE_USER_ROLES ---------------------
# Rows inside an `insert into access.permissions|roles (...)` statement, up to its `;`.
while IFS= read -r file; do
    git show "HEAD:$file" 2>/dev/null | awk -v q="'" '
        BEGIN { row = "\\( *" q "[^" q "]+" q }
        tolower($0) ~ /insert into access\.permissions[ (]/ { table = "permission" }
        tolower($0) ~ /insert into access\.roles[ (]/ { table = "role" }
        table != "" && match($0, row) {
            v = substr($0, RSTART, RLENGTH); sub(/^\( */, "", v); gsub(q, "", v)
            print table "\t" NR "\t" v
        }
        index($0, ";") > 0 { table = "" }' >"$tmp/rows"
    while IFS=$'\t' read -r kind line value; do
        if [[ "$kind" == permission && "$value" =~ $PERM_NAME_RE ]]; then
            head_has "\"$value\"" "$CATALOG" ||
                finding "$file" "$line" "permission $value is inserted but missing from $CATALOG (three-in-sync)"
        elif [[ "$kind" == role && "$value" =~ $ROLE_NAME_RE ]]; then
            head_has "\"$value\"" "$CATALOG" ||
                finding "$file" "$line" "role $value is inserted but missing from default_roles() in $CATALOG (three-in-sync)"
            head_has "\"$value\"" "$ACCESS_REPO" ||
                finding "$file" "$line" "role $value is not in ASSIGNABLE_USER_ROLES ($ACCESS_REPO) — fine only for a machine role"
        fi
    done <"$tmp/rows"
done < <(git diff --name-only --diff-filter=A "$mb" HEAD -- "$MIGRATIONS/*.sql")

# --- migrations: applied ones are append-only ---------------------------------------------------
while IFS=$'\t' read -r status path renamed; do
    case "$status" in
        M) finding "$path" 1 "applied migration edited in place; add a new numbered migration instead" ;;
        D) finding "$path" 1 "applied migration deleted; migrations are append-only" ;;
        R*) finding "$path" 1 "applied migration renamed to ${renamed##*/}; sqlx treats that as a new version while the old one stays applied" ;;
    esac
done < <(git diff --name-status -M "$mb" HEAD -- "$MIGRATIONS")

# --- router.rs <-> openapi.rs ------------------------------------------------------------------
# Router imports may alias a module (`jobs as jobs_handler`); map each alias back to its module.
git show "HEAD:$ROUTER" 2>/dev/null | grep -oE '[a-z_]+ as [a-z_]+' | awk '{ print $3, $1 }' >"$tmp/aliases"
module_of() { local m; m="$(awk -v a="$1" '$1 == a { print $2; exit }' "$tmp/aliases")"; printf '%s' "${m:-$1}"; }
alias_of() { local a; a="$(awk -v m="$1" '$2 == m { print $1; exit }' "$tmp/aliases")"; printf '%s' "${a:-$1}"; }

ROUTE_RE='(get|post|put|patch|delete)\(([a-z_][a-z0-9_]*)::([a-z_][a-z0-9_]*)\)'
while IFS=$'\t' read -r line text; do
    rest="$text"
    while [[ "$rest" =~ $ROUTE_RE ]]; do
        method="${BASH_REMATCH[1]}" alias="${BASH_REMATCH[2]}" fn="${BASH_REMATCH[3]}"
        rest="${rest#*"${BASH_REMATCH[0]}"}"
        module="$(module_of "$alias")"
        git grep -qE "handlers::$module::$fn([^a-z0-9_]|\$)" HEAD -- "$OPENAPI" 2>/dev/null ||
            finding "$ROUTER" "$line" "route $module::$fn is not registered in $OPENAPI paths(...), so it never reaches the generated client (deliberate only for a non-JSON route like /metrics)"
        case "$method" in
            post|put|patch|delete) ;;
            *) continue ;;
        esac
        src="$HANDLERS/$module.rs"
        git cat-file -e "HEAD:$src" 2>/dev/null || src="$HANDLERS/$module/mod.rs"
        sig="$(git show "HEAD:$src" 2>/dev/null | awk -v fn="$fn" '
            start == 0 && $0 ~ ("async fn " fn "[(<]") { start = NR }
            start > 0 { text = text $0 "\n"; if (index($0, "{") > 0) { print start; printf "%s", text; exit } }')"
        [[ -n "$sig" ]] || continue
        if ! printf '%s' "$sig" | grep -q 'RequirePermission<'; then
            finding "$src" "${sig%%$'\n'*}" "mutating route ($method) $module::$fn takes no RequirePermission<P>; confirm its auth (session-only, HMAC, Actor) is deliberate"
        fi
    done
done < <(added_in "$ROUTER")

while IFS=$'\t' read -r line text; do
    [[ "$text" =~ $OPENAPI_PATH_RE ]] || continue
    module="${BASH_REMATCH[1]}" fn="${BASH_REMATCH[2]}"
    alias="$(alias_of "$module")"
    git grep -qE "(^|[^a-z0-9_])$alias::$fn([^a-z0-9_]|\$)" HEAD -- "$ROUTER" 2>/dev/null ||
        finding "$OPENAPI" "$line" "path $module::$fn is in the OpenAPI document but no route in $ROUTER serves it"
done < <(added_in "$OPENAPI")

# --- handlers: SQL, unwrap/expect, dropped RequirePermission ------------------------------------
while IFS= read -r file; do
    case "$file" in *_tests.rs) continue ;; esac
    test_from="$(git show "HEAD:$file" 2>/dev/null | grep -n -m1 '#\[cfg(test)\]' | cut -d: -f1)"
    test_from="${test_from:-999999999}"
    while IFS=$'\t' read -r line text; do
        [[ $line -lt $test_from ]] || continue
        [[ "$text" =~ $COMMENT_RE ]] && continue
        # Query calls are matched case-sensitively, so the `Query(q)` extractor is not one.
        if printf '%s' "$text" | grep -qE 'sqlx::(query|raw_sql)|(^|[^A-Za-z_])query(_as|_scalar)?(::<|\()|QueryBuilder' ||
            printf '%s' "$text" | grep -qiE '"[[:space:]]*(select|insert into|update [a-z_.]+ set|delete from|with [a-z_]+ as) '; then
            finding "$file" "$line" "SQL in a handler; move the query to repos/ (handlers stay thin)"
        fi
        if printf '%s' "$text" | grep -qE '\.unwrap\(\)|\.expect\('; then
            finding "$file" "$line" "unwrap()/expect() in a handler; return an ApiError instead of panicking the request"
        fi
    done < <(added_in "$file")
    # A RequirePermission line removed and not re-added in the same file.
    while IFS=$'\t' read -r oline text; do
        perm="$(printf '%s' "$text" | grep -oE 'RequirePermission<[A-Za-z0-9_]+>')" || continue
        added_in "$file" | grep -qF "$perm" ||
            finding "$file" "$oline" "$perm was removed (line $oline on $base); confirm the route is still gated"
    done < <(awk -F'\t' -v f="$file" '$1 == f { print $2 "\t" substr($0, length($1) + length($2) + 3) }' "$tmp/removed")
done < <(git diff --name-only --diff-filter=AM "$mb" HEAD -- "$HANDLERS/*.rs")


# Layer 1's findings, sorted by file then line.
if [[ -s "$findings" ]]; then
    sort -u "$findings" | sort -s -t: -k1,1 -k2,2n
fi

# --- layer 2: patterns over whole changed files ---------------------------------------------------
LIST="$tmp/list"
: >"$LIST"
for cmd in "diff $base...HEAD --name-only" "diff --name-only" "diff --cached --name-only" "ls-files --others --exclude-standard"; do
    # shellcheck disable=SC2086
    git $cmd >> "$LIST" || fail "git $cmd"
done

# Arrays, not word-split strings: tracked paths contain spaces and parentheses.
EXISTING=() RUST=() TS=() TSX=() TS_TESTS=()
while IFS= read -r f; do
    # Deleted files are still listed by the diffs.
    [[ -f "$f" ]] || continue
    EXISTING+=("$f")
    case "$f" in
        *.rs) RUST+=("$f") ;;
        # Declaration files are generated (packages/api-client's schema.d.ts) or types only.
        *.d.ts) ;;
        *.test.ts|*.test.tsx) TS+=("$f"); TS_TESTS+=("$f") ;;
        *.ts) TS+=("$f") ;;
        *.tsx) TS+=("$f"); TSX+=("$f") ;;
    esac
done < <(sort -u "$LIST")

ERRORS=0
# Per-checkout and gitignored; a forced add would ship one checkout's DATABASE_URL and secrets.
if grep -qxE '\.env(\.local)?' "$LIST"; then
    echo "CRITICAL: .env is in the diff — per-checkout and gitignored, must never be committed"
fi
# check MESSAGE GREP-ARGS... -- FILES...
# grep exits 1 on no match; 2 means it could not read something, which fails the scan.
check() {
    local message="$1"; shift
    grep "$@"
    case $? in
        0) echo "^ $message" ;;
        1) ;;
        *) echo "SCAN FAILED: grep $*" >&2; ERRORS=$((ERRORS + 1)) ;;
    esac
}

if (( ${#RUST[@]} )); then
    check "debug output must be removed" -HnE '\bdbg!\(' -- "${RUST[@]}"
    check "swallowed error — handle it, propagate it, or log it and surface it in the return value (code-quality.md)" \
        -HnE 'let _ = |\.ok\(\);' -- "${RUST[@]}"
    check "SQL built with format! — bind values as \$n; format! only for trusted code constants, with a comment saying so (database-postgres.md)" \
        -HniE 'format!\([[:space:]]*"[[:space:]]*(select|insert|update|delete|with) ' -- "${RUST[@]}"
    check "#[allow(...)] needs a stated reason; never silence a check to make it pass (code-quality.md)" \
        -HnE '#!?\[allow\(' -- "${RUST[@]}"
    check "test skeleton left in — a todo!() body or an #[ignore] with no reason, or a todo one (tdd-planner)" \
        -HnE '#\[ignore\]|#\[ignore *= *"todo|\btodo!\(' -- "${RUST[@]}"
    check "negative assertion — confirm a positive control in the same test (test-quality.md)" \
        -HnE 'assert!\([[:space:]]*!|assert!\(.*\.is_(none|empty)\(\)\)' -- "${RUST[@]}"
fi
if (( ${#TS[@]} )); then
    check "debug output must be removed" -HnE '\bconsole\.log[[:space:]]*\(|^[[:space:]]*debugger[[:space:]]*;?[[:space:]]*$' -- "${TS[@]}"
    check "@ts-ignore, @ts-expect-error and eslint-disable need a stated reason; never silence a check to make it pass (code-quality.md)" \
        -HnE '@ts-(ignore|expect-error)|eslint-disable' -- "${TS[@]}"
fi
if (( ${#TSX[@]} )); then
    check "dangerouslySetInnerHTML needs a stated reason the value is safe" -HnF 'dangerouslySetInnerHTML' -- "${TSX[@]}"
fi
if (( ${#TS_TESTS[@]} )); then
    check "focused, skipped or todo test left in a test file" -HnE '\b(it|test|describe)\.(only|skip|todo)\(' -- "${TS_TESTS[@]}"
    check "negative assertion — confirm a positive control in the same test (test-quality.md)" \
        -HnE '\.not\.to[A-Za-z]+\(|toHaveBeenCalledTimes\(0\)' -- "${TS_TESTS[@]}"
fi

if [[ -s "$findings" ]]; then
    echo "review-scan: $(sort -u "$findings" | wc -l | tr -d ' ') sync-invariant finding(s) vs $base"
else
    echo "review-scan: no sync-invariant finding vs $base"
fi
# Positive control: 0 on a branch with changes means the list was empty, not clean.
echo "scan covered ${#EXISTING[@]} file(s)"
if (( ERRORS )); then
    echo "SCAN FAILED: ${ERRORS} check(s) could not read their files" >&2
    exit 1
fi
exit 0
