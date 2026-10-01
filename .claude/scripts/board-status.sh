#!/usr/bin/env bash
# Move a VATUSA/OIS "Project 7" card to a named Status column.
# Usage: .claude/scripts/board-status.sh <issue-number> "<Status column>"
#   e.g. .claude/scripts/board-status.sh 47 "In build"
# Columns: Blocked · Triaging · To Do · Returned · In build · Post build ·
#          Testing Queue · In Test · Code Review · Shippable · Done
set -euo pipefail
issue="${1:?issue number required}"; col="${2:?status column required}"

PROJECT="PVT_kwDOAd0rh84Bi-Gk"                 # VATUSA project 7 node id
FIELD="PVTSSF_lADOAd0rh84Bi-Gkzhh0Mvg"         # its "Status" single-select field id

opt=$(gh api graphql -f query='query{organization(login:"VATUSA"){projectV2(number:7){field(name:"Status"){... on ProjectV2SingleSelectField{options{id name}}}}}}' \
  --jq ".data.organization.projectV2.field.options[] | select(.name==\"$col\") | .id")
# Resolve the item id from the ISSUE, never by listing the board. `gh project item-list` truncates at
# --limit silently, so a card past the cap is indistinguishable from one that isn't on the board at all
# -- that was #495, hit at 200 with ~170 Done rows eating the budget. Raising the cap only defers it,
# and the listing also drags down every card's full issue body to find one id (~800 KB vs ~120 bytes),
# which is what trips the Projects *secondary* rate limit and locks `gh project` out. Asking the issue
# which project items it belongs to has no limit to outgrow: `first: 100` is the API maximum and no
# issue is on 100 projects.
item=$(gh api graphql -f query="{repository(owner:\"VATUSA\",name:\"OIS\"){issue(number:$issue){projectItems(first:100){nodes{id project{number}}}}}}" \
  --jq '.data.repository.issue.projectItems.nodes[] | select(.project.number == 7) | .id')

[ -n "$opt" ]  || { echo "no such Status column: $col" >&2; exit 1; }
[ -n "$item" ] || { echo "issue #$issue is not on project 7 (or no such issue)" >&2; exit 1; }

gh api graphql -f query="mutation{updateProjectV2ItemFieldValue(input:{projectId:\"$PROJECT\",itemId:\"$item\",fieldId:\"$FIELD\",value:{singleSelectOptionId:\"$opt\"}}){projectV2Item{id}}}" >/dev/null
echo "moved #$issue → $col"
