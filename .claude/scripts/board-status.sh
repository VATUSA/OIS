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
item=$(gh project item-list 7 --owner VATUSA --format json --limit 200 \
  | jq -r ".items[] | select(.content.number==$issue) | .id")

[ -n "$opt" ]  || { echo "no such Status column: $col" >&2; exit 1; }
[ -n "$item" ] || { echo "issue #$issue is not on the board" >&2; exit 1; }

gh api graphql -f query="mutation{updateProjectV2ItemFieldValue(input:{projectId:\"$PROJECT\",itemId:\"$item\",fieldId:\"$FIELD\",value:{singleSelectOptionId:\"$opt\"}}){projectV2Item{id}}}" >/dev/null
echo "moved #$issue → $col"
