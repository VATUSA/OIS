#!/usr/bin/env bash
# shellcheck source-path=SCRIPTDIR
# PostToolUse(Edit|Write|MultiEdit) reminder: an edit that can move the API contract needs the typed
# client regenerated, or the web typecheck compiles against a stale contract and only CI's
# client-drift job notices (AGENTS.md § The API contract → typed client).
# It fires on router.rs and openapi.rs, on an edit whose text mentions ToSchema or utoipa, and on any
# edit to a backend model that derives ToSchema (a field change is a contract change, and a `///`
# doc comment on one is too).
# Advisory: always exits 0, and stays quiet on a payload it cannot read.
# shellcheck source=lib/tool-input.sh
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/tool-input.sh" 2>/dev/null || exit 0

INPUT="$(cat)"
FILE="$(printf '%s' "$INPUT" | hook_json '.tool_input.file_path // empty')"
case "$FILE" in */backend/src/*|backend/src/*) ;; *) exit 0 ;; esac

why=""
case "$FILE" in
    */router.rs|*/openapi.rs) why="${FILE##*/} registers routes or OpenAPI paths" ;;
esac
if [[ -z "$why" ]]; then
    TEXT="$(printf '%s' "$INPUT" | hook_json '[.tool_input.content, .tool_input.new_string, .tool_input.old_string, (.tool_input.edits // [] | .[] | .new_string, .old_string)] | map(select(. != null)) | join("\n")')"
    if printf '%s' "$TEXT" | grep -qE 'ToSchema|utoipa'; then
        why="the edit touches a ToSchema/utoipa annotation"
    elif [[ "$FILE" == */backend/src/models/* || "$FILE" == backend/src/models/* ]] && grep -q 'ToSchema' "$FILE" 2>/dev/null; then
        why="${FILE##*/} holds ToSchema models"
    fi
fi
[[ -n "$why" ]] || exit 0

remind "CONTRACT REMINDER: $why, so the OpenAPI contract may have moved. Before calling this done,
regenerate the typed client and typecheck against it:
  OIS_OPENAPI_URL=http://127.0.0.1:3001/docs/api/v1/openapi.json pnpm --filter @ois/api-client codegen
  pnpm typecheck
A new endpoint needs both its router.rs route and its openapi.rs path. \`just ci-full\` runs the same
client-drift check CI does."
