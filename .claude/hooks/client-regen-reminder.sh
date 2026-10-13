#!/bin/bash
# PostToolUse hook (Edit, Write, MultiEdit): an edit that can move the API contract needs the typed
# client regenerated, or the web typecheck compiles against a stale contract and only CI's
# client-drift job notices (AGENTS.md -> "The API contract → typed client").
#
# Trigger: after an edit under backend/src/ to router.rs or openapi.rs, an edit whose text mentions
#          ToSchema or utoipa, or any edit to a backend model file that derives ToSchema (a field change
#          is a contract change, and a `///` doc comment on one is too, #591).
# Exit:    always 0 — ADVISORY, never a gate. The reminder goes back as additionalContext, the channel a
#          PostToolUse hook has to the agent; plain stdout goes to the debug log.
#
# The payload is read from stdin, never the environment, and python runs isolated (-I), so a json.py in
# the session cwd cannot stand in for the standard library. It fails OPEN: a payload it cannot read
# leaves it quiet.
#
# Tests: bin/lib/client-regen-reminder.test.sh
python3 -I -c '
import json, os, re, sys

try:
    payload = json.loads(sys.stdin.read() or "{}")
except ValueError:
    sys.exit(0)
if not isinstance(payload, dict):
    sys.exit(0)
tool_input = payload.get("tool_input") or {}
path = tool_input.get("file_path")
if not isinstance(path, str) or not re.search(r"(^|/)backend/src/", path):
    sys.exit(0)

why = ""
name = os.path.basename(path)
if name in ("router.rs", "openapi.rs"):
    why = "%s registers routes or OpenAPI paths" % name
else:
    texts = [tool_input.get(key) for key in ("content", "new_string", "old_string")]
    texts += [edit.get(key) for edit in (tool_input.get("edits") or []) if isinstance(edit, dict)
              for key in ("new_string", "old_string")]
    if any(isinstance(text, str) and re.search(r"ToSchema|utoipa", text) for text in texts):
        why = "the edit touches a ToSchema/utoipa annotation"
    elif re.search(r"(^|/)backend/src/models/", path):
        try:
            with open(path, errors="replace") as handle:
                if "ToSchema" in handle.read():
                    why = "%s holds ToSchema models" % name
        except OSError:
            pass
if not why:
    sys.exit(0)

reminder = ("CONTRACT REMINDER: %s, so the OpenAPI contract may have moved. Before calling this done,\n"
            "regenerate the typed client and typecheck against it:\n"
            "  OIS_OPENAPI_URL=http://127.0.0.1:3001/docs/api/v1/openapi.json pnpm --filter @ois/api-client codegen\n"
            "  pnpm typecheck\n"
            "A new endpoint needs both its router.rs route and its openapi.rs path. `just test` runs the same\n"
            "client-drift check CI does." % why)
print(json.dumps({"hookSpecificOutput": {"hookEventName": "PostToolUse", "additionalContext": reminder}}))
' 2>/dev/null || true
exit 0
