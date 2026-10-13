#!/bin/bash
# ==============================================================================
# Tests for .claude/hooks/update-branch-gate.sh.
#
#   bash bin/lib/update-branch-gate.test.sh
#
# A stub `gh` answers `gh pr view <n> --json baseRefName,headRefName` with "BASE HEAD" from BASE_<n>
# and HEAD_<n> (BASE unset means GitHub failed; HEAD defaults to a feature branch), and records the
# repo it was asked about. Pure shell;
# macOS bash 3.2 and WSL2. Exits non-zero on any failure.
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GATE="$ROOT/.claude/hooks/update-branch-gate.sh"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/update-branch-gate-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

mkdir -p "$WORK/bin"
cat > "$WORK/bin/gh" <<'STUB'
#!/bin/bash
[[ "$1 $2" == "pr view" ]] || exit 1
number="$3"
repo=""
for ((i = 1; i <= $#; i++)); do
    [[ "${!i}" == "-R" ]] && { j=$((i + 1)); repo="${!j}"; }
done
printf '%s\n' "$repo" > "$GH_ASKED"
[[ -n "${STUB_ALWAYS_NEXT:-}" ]] && { echo "next feat/9/x"; exit 0; }
[[ -n "${STUB_NEXT_BUT_FAIL:-}" ]] && { echo "next feat/9/x"; exit 1; }
var="BASE_$number"
head="HEAD_$number"
[[ -n "${!var:-}" ]] || exit 1
printf '%s %s\n' "${!var}" "${!head:-feat/9/x}"
STUB
chmod +x "$WORK/bin/gh"
export GH_ASKED="$WORK/asked"
export BASE_101=next BASE_202=feat/312/other-work BASE_303=next BASE_505=main BASE_606=main HEAD_606=next \
    BASE_707=next HEAD_707=main

# expect WANT NAME COMMAND — WANT is allow (exit 0) or block (exit 2)
expect() {
    local want="$1" rc got
    python3 -c 'import json,sys; print(json.dumps({"tool_input": {"command": sys.argv[1]}}))' "$3" \
        | PATH="$WORK/bin:$PATH" bash "$GATE" >/dev/null 2>&1
    rc=$?
    case "$rc" in 0) got=allow ;; 2) got=block ;; *) got="rc=$rc" ;; esac
    if [[ "$got" == "$want" ]]; then ok "$want: $2"; else bad "$2 (want $want, got $got)"; fi
}

A=repos/VATUSA/OIS/pulls

echo "Base is next:"
expect allow "REST update-branch"                "gh api -X PUT $A/101/update-branch -f expected_head_sha=abc"
expect allow "leading slash"                     "gh api --method PUT /$A/101/update-branch"
expect allow "gh pr update-branch by number"     "gh pr update-branch 101"
expect allow "gh pr update-branch by URL"        "gh pr update-branch https://github.com/VATUSA/OIS/pull/101"
expect allow "two PRs, both on next"             "gh api -X PUT $A/101/update-branch && gh api -X PUT $A/303/update-branch"

echo "Base is main (a PR whose head is a feature branch):"
expect allow "REST update-branch"                "gh api -X PUT $A/505/update-branch"

echo "Head is a protected branch (the release PR), which update-branch would write:"
expect block "next into main's PR head"          "gh pr update-branch 606"
expect block "main as the head"                  "gh api -X PUT $A/707/update-branch"

echo "Base is another issue's branch:"
expect block "REST update-branch"                "gh api -X PUT $A/202/update-branch -f expected_head_sha=abc"
expect block "gh pr update-branch"               "gh pr update-branch 202"
expect block "second of two PRs"                 "gh api -X PUT $A/101/update-branch; gh api -X PUT $A/202/update-branch"
expect block "chained after cd"                  "cd /repo && gh api -X PUT $A/202/update-branch"
expect block "inside bash -c"                    "bash -c \"gh api -X PUT $A/202/update-branch\""
expect block "-iX bundle"                        "gh api -iX PUT $A/202/update-branch"
expect block "-iXPUT bundle"                     "gh api -iXPUT $A/202/update-branch"

echo "Unverifiable:"
expect block "GitHub lookup fails"               "gh api -X PUT $A/404/update-branch"
expect block "PR number in a variable"           "gh api -X PUT $A/\$n/update-branch"
expect block "gh pr update-branch by branch"     "gh pr update-branch feat/312/other-work"
expect block "gh pr update-branch, no PR"        "gh pr update-branch"
expect block "rebase instead of merge"           "gh pr update-branch 101 --rebase"
expect block "two PR numbers, the first on next" "gh pr update-branch 101 202"
expect block "GraphQL mutation"                  "gh api graphql -f query='mutation { updatePullRequestBranch(input: {pullRequestId: \"x\"}) { clientMutationId } }'"
expect block "unrecognized update-branch path"   "gh api -X PUT repos/x/pulls/101/update-branch/extra"
expect block "unparseable line"                  "gh api -X PUT $A/101/update-branch \"unterminated"

echo "Refused before GitHub is asked (a lookup answering next cannot rescue it):"
export STUB_ALWAYS_NEXT=1
expect block "PR number in a variable"           "gh api -X PUT $A/\$n/update-branch"
expect block "gh pr update-branch by branch"     "gh pr update-branch feat/312/other-work"
# A bare number is looked up in the hook's cwd; gh would resolve it in the repository the line points at.
expect block "#N after a cd"                     "cd /elsewhere && gh pr update-branch 101"
expect block "#N with GH_REPO"                   "GH_REPO=other-org/other-repo gh pr update-branch 101"
expect block "#N with GIT_DIR"                   "GIT_DIR=/elsewhere/.git gh pr update-branch 101"
expect block "#N in a REST path after a cd"      "cd /elsewhere && gh api -X PUT pulls/101/update-branch"
expect allow "positive control: #N in the cwd"   "gh pr update-branch 101"
expect allow "a named repo after a cd"           "cd /elsewhere && gh api -X PUT $A/101/update-branch"
unset STUB_ALWAYS_NEXT

echo "A failed lookup blocks even if it printed next:"
export STUB_NEXT_BUT_FAIL=1
expect block "gh exits non-zero"                 "gh api -X PUT $A/101/update-branch"
unset STUB_NEXT_BUT_FAIL

echo "Not an update-branch:"
expect allow "reading a PR"                      "gh pr view 202 --json baseRefName"
expect allow "other gh api call"                 "gh api repos/VATUSA/OIS/pulls/202"
expect allow "path quoted in a grep"             "grep -n 'pulls/202/update-branch' notes.md"
expect allow "plain command"                     "ls -la"

echo "Asks GitHub about the named repo:"
expect allow "REST path repo"                    "gh api -X PUT repos/other-org/other-repo/pulls/101/update-branch"
if [[ "$(cat "$GH_ASKED")" == "other-org/other-repo" ]]; then ok "looked up other-org/other-repo"; else bad "looked up '$(cat "$GH_ASKED")'"; fi
expect allow "--repo on gh pr"                   "gh pr update-branch 101 --repo other-org/other-repo"
if [[ "$(cat "$GH_ASKED")" == "other-org/other-repo" ]]; then ok "looked up --repo other-org/other-repo"; else bad "looked up '$(cat "$GH_ASKED")'"; fi
expect allow "PR URL in another repo"            "gh pr update-branch https://github.com/third-org/third-repo/pull/101"
if [[ "$(cat "$GH_ASKED")" == "third-org/third-repo" ]]; then ok "looked up the URL's third-org/third-repo"; else bad "looked up '$(cat "$GH_ASKED")'"; fi

echo "A refused line says why:"
out="$(python3 -c 'import json,sys; print(json.dumps({"tool_name": "Bash", "tool_input": {"command": sys.argv[1]}}))' "gh \$'pr' update-branch 5" \
    | (cd "$WORK" && bash "$GATE" 2>&1))"
rc=$?
if [[ $rc -eq 2 && "$out" == *"ANSI-C"* ]]; then ok "the parser's reason is in the message"; else bad "reason missing (rc=$rc: ${out:0:200})"; fi

echo ""
echo "update-branch-gate: $PASS passed, $FAIL failed"
(( FAIL == 0 ))
