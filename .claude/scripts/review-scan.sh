#!/usr/bin/env bash
# OIS pitfall scan over the committed diff: the mistakes that compile, pass the tests, and still
# break something. Each check maps to a rule in AGENTS.md:
#   * permission three-in-sync: a `permission!` marker, its catalog.rs string, an access.permissions row;
#   * role three-in-sync: an access.roles row, default_roles(), ASSIGNABLE_USER_ROLES;
#   * a handler registered in router.rs but not openapi.rs, or the reverse;
#   * SQL in handlers/ (it belongs in repos/);
#   * an applied migration edited, renamed or deleted in place (they are append-only);
#   * a mutating route whose handler takes no RequirePermission<P>, or a RequirePermission removed;
#   * unwrap()/expect() in handlers/.
# Test code (`*_tests.rs`, and everything after a handler file's first `#[cfg(test)]`) is skipped.
#
# Usage: .claude/scripts/review-scan.sh [base-ref]     (default origin/next)
# Scans base...HEAD: commit first, the review is of a commit. Reads HEAD's tree, not the worktree.
# Prints `file:line: message` per finding. Exit 0 = clean, 1 = findings, 2 = the scan could not run
# (so a partial scan never passes). A finding is a prompt to look, not proof: confirm each one.
set -uo pipefail

fail() { echo "SCAN FAILED: $*" >&2; exit 2; }

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
        if printf '%s' "$text" | grep -qiE 'sqlx::|QueryBuilder|"[[:space:]]*(select|insert into|update [a-z_.]+ set|delete from|with [a-z_]+ as) '; then
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

files="$(git diff --name-only "$mb" HEAD | wc -l | tr -d ' ')"
if [[ -s "$findings" ]]; then
    sort -t: -k1,1 -k2,2n -u "$findings"
    echo "review-scan: $(sort -u "$findings" | wc -l | tr -d ' ') finding(s) across $files changed file(s) vs $base"
    exit 1
fi
echo "review-scan: clean, $files changed file(s) vs $base"
exit 0
