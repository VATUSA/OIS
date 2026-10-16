"""Which manifest lines a tool call may add: the marker ledger behind marker-write-gate.sh (before a call)
and marker-audit.sh (after it).

  printf '%s' "$PAYLOAD" | python3 -I marker_ledger.py snapshot   # PreToolUse: record what this call may stamp
  printf '%s' "$PAYLOAD" | python3 -I marker_ledger.py audit      # PostToolUse[Failure]: keep or revert what it did

bin/run-tests.sh and bin/review-marker.sh add a `<kind> <sha> <sha256>` line to .claude/markers/manifest
for each marker they write (bin/lib/marker_manifest.py). Any process can append a line, so the ledger
records which lines the hooks accepted, per checkout. Before each call the snapshot notes what that call
may stamp (marker_write.stampers): a line that is nothing but a stamper and reads, the stamper being
`just test`, this checkout's bin/run-tests.sh (tests for HEAD) or bin/review-marker.sh KIND SHA, with
no cd, or exactly `cd ABSDIR && just test` for the checkout at ABSDIR.

A new line is accepted in two ways only: by the audit of the very call whose snapshot covers its kind and
SHA, or by a grant. A run_in_background stamper leaves a one-shot grant for each line its own audit did
not already keep, so a re-run on a SHA that is already stamped is covered too. Another call's snapshot
accepts nothing: when an audit meets a line that only a different call's open snapshot could explain (a
foreground stamper whose own audit has not run yet), it takes the line out of the manifest and holds it
as pending, leaving the marker file, so no gate reads it as vouched. The owning call's audit puts the
line back and accepts it; once that snapshot is closed or older than GRANT_SECONDS the pending line is
dropped. Anything else puts the manifest back as the ledger had it, and every marker the manifest neither
vouches for nor holds pending is deleted, in every worktree. A snapshot whose call never reaches an audit
(blocked by another gate, denied, cancelled) therefore authorizes nothing.

The state is one file under the git common dir, replaced by a rename so a hook killed part-way leaves the
last whole state; it is written only here and is sealed: a ledger that changed outside these hooks is
tampering, and the hooks then trust nothing, emptying every manifest.

Residual, by design: this guards against an agent's mistake or shortcut. A deliberate multi-step forgery
of the ledger and its seal is outside it, as is a forged line that lands while the matching stamper's own
call is still running, or while a grant for it is open. Python 3.8-compatible.
"""

import hashlib
import json
import os
import subprocess
import sys
import time

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
MAIN_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(HERE)))
sys.path.insert(0, HERE)
sys.path.insert(1, os.path.join(MAIN_ROOT, "bin", "lib"))
import file_lock  # noqa: E402
import marker_manifest  # noqa: E402
from marker_write import LEDGER_DIR, payload_cwd, stampers  # noqa: E402

# How long a backgrounded stamper's grant lasts, and how long another call's open snapshot keeps a line
# pending: a full suite finishes well inside it.
GRANT_SECONDS = 3 * 3600
SNAPSHOT_SECONDS = 24 * 3600  # when an unaudited snapshot is forgotten; it authorizes nothing meanwhile


def state_dir(root):
    """LEDGER_DIR under the repository's git common dir, shared by every worktree."""
    try:
        result = subprocess.run(["git", "-C", root, "rev-parse", "--path-format=absolute", "--git-common-dir"],
                                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, universal_newlines=True,
                                timeout=10)
    except (OSError, subprocess.SubprocessError):
        raise RuntimeError("git cannot find the common dir for %s" % root)
    if result.returncode != 0 or not result.stdout.strip():
        raise RuntimeError("git cannot find the common dir for %s" % root)
    return os.path.join(result.stdout.strip(), LEDGER_DIR)


def seal(state):
    """A SHA-256 over the state as the hooks last left it."""
    return hashlib.sha256(json.dumps(state, sort_keys=True).encode()).hexdigest()


def write_atomic(path, text):
    temporary = path + ".tmp"
    with open(temporary, "w") as handle:
        handle.write(text)
    os.replace(temporary, path)


class Ledger(object):
    """The sealed state (accepted and pending lines, grants, open snapshots) in one file, opened under an
    exclusive lock. A missing file is a new ledger, which has accepted nothing; one that is not
    `{"seal", "state"}` with a matching seal is tampering."""

    def __init__(self, root):
        directory = state_dir(root)
        os.makedirs(directory, exist_ok=True)
        self.lock = open(os.path.join(directory, "lock"), "a+")
        file_lock.lock(self.lock)
        self.path = os.path.join(directory, "ledger.json")
        state, self.tampered = {}, False
        if os.path.lexists(self.path):
            try:
                with open(self.path) as handle:
                    stored = json.load(handle)
                state = stored["state"]
                self.tampered = stored["seal"] != seal(state)
            except (OSError, ValueError, KeyError, TypeError):
                self.tampered = True
        if self.tampered:
            state = {}
        self.accepted = dict(state.get("accepted", {}))
        self.pending = dict(state.get("pending", {}))
        self.grants = [grant for grant in state.get("grants", []) if grant["expires"] > time.time()]
        self.snapshots = dict(state.get("snapshots", {}))

    def close(self, save=True):
        if save:
            now = time.time()
            state = {"accepted": self.accepted,
                     "pending": {root: held for root, held in self.pending.items() if held},
                     "grants": self.grants,
                     "snapshots": {key: snap for key, snap in self.snapshots.items()
                                   if now - snap["at"] <= SNAPSHOT_SECONDS}}
            write_atomic(self.path, json.dumps({"seal": seal(state), "state": state}, sort_keys=True))
        file_lock.unlock(self.lock)
        self.lock.close()


def read_manifest(path):
    """(the manifest's lines in order, a fault or None, whether it can be rewritten). A symbolic link is
    never read: it is removed, and nothing behind it is trusted. Anything else that is not a regular file
    cannot be replaced safely, so it is reported and left."""
    if os.path.islink(path):
        os.unlink(path)
        return [], "%s was a symbolic link, which is never read as a manifest: removed it (not its target), " \
                   "so no marker it listed is vouched for" % path, True
    if not os.path.lexists(path):
        return [], None, True
    if not os.path.isfile(path):
        return [], "%s is not a regular file, so no marker here can be vouched for: remove it" % path, False
    with open(path, errors="replace") as handle:
        return [line.rstrip("\n") for line in handle if line.strip()], None, True


def pair_of(line):
    """(kind, sha) of a `<kind> <sha> <sha256>` line, or None."""
    fields = tuple(line.split())
    return fields[:2] if len(fields) == 3 else None


def reconcile(ledger, root, own, elsewhere):
    """Settle ROOT's manifest against the ledger, holding the lock a stamp appends under from the read to
    the rewrite, so a stamp landing meanwhile is not overwritten.

    A new line, or one held pending, is accepted when its pair is in OWN (what the audited call's snapshot
    covers) or a grant accounts for it; held pending, out of the manifest, when only ELSEWHERE (other
    calls' open snapshots) could explain it; and dropped otherwise. A line removed since is left removed:
    it can only unvouch a marker. Returns what was undone and the pairs accepted on OWN's account."""
    markers = os.path.join(root, ".claude", "markers")
    if marker_manifest.real_markers(root) is None or not os.path.isdir(markers):
        ledger.accepted[root], ledger.pending[root] = [], []  # no directory, or a linked one the audit removes
        return [], set()
    with marker_manifest.manifest_lock(markers):
        path = os.path.join(markers, "manifest")
        current, fault, writable = read_manifest(path)
        accepted = [line for line in ledger.accepted.get(root, []) if line in current]
        held = {entry["line"]: entry["at"] for entry in ledger.pending.get(root, [])}
        undone, kept, pending = ([fault] if fault else []), set(), []
        for line in [line for line in current if line not in accepted] + [line for line in held if line not in current]:
            if line in accepted:
                continue
            pair = pair_of(line)
            grant = next((g for g in ledger.grants
                          if pair and (g["root"], g["kind"], g["sha"]) == (root,) + pair), None)
            if pair and pair in own:
                accepted.append(line)
                kept.add(pair)
            elif grant:
                ledger.grants.remove(grant)
                accepted.append(line)
            elif pair and pair in elsewhere:
                pending.append({"line": line, "at": held.get(line, time.time())})
            else:
                undone.append("dropped: %s" % line)
        if writable and (accepted != current or ledger.tampered):
            write_atomic(path, "".join(line + "\n" for line in accepted))
    ledger.accepted[root], ledger.pending[root] = accepted, pending
    return undone, kept


def snapshot(payload):
    """Record what this call may stamp, keyed by its tool_use_id."""
    tool_use_id = payload.get("tool_use_id")
    if not isinstance(tool_use_id, str) or not tool_use_id:
        return
    cwd = payload_cwd(payload)
    tool_input = payload.get("tool_input") or {}
    found = stampers(str(tool_input.get("command") or ""), cwd) if payload.get("tool_name") == "Bash" else []
    ledger = Ledger(MAIN_ROOT)
    ledger.snapshots[tool_use_id] = {"stampers": [[os.path.realpath(root), kind, sha] for root, kind, sha in found],
                                     "background": bool(tool_input.get("run_in_background")), "at": time.time()}
    ledger.close(save=not ledger.tampered)  # a broken seal is left for the audit to see


def audit(payload):
    """Close this call's snapshot, keep or revert the manifest changes since the last audit, delete
    unvouched markers, and say what was undone. PostToolUseFailure runs it too, so a stamper that failed
    closes its snapshot at once; a call that failed to start leaves no grant."""
    event = payload.get("hook_event_name")
    event = event if event in ("PostToolUse", "PostToolUseFailure") else "PostToolUse"
    ledger = Ledger(MAIN_ROOT)
    notes = []
    try:
        if ledger.tampered:
            notes.append("the marker ledger under the git common dir was changed outside the hooks, so no "
                         "manifest line is trusted")
        tool_use_id = payload.get("tool_use_id")
        own = ledger.snapshots.pop(tool_use_id, None) if isinstance(tool_use_id, str) else None
        stamped = {tuple(item) for item in own["stampers"]} if own else set()
        now = time.time()
        # A stamper another call started and has not audited yet may be about to accept a new line: hold it.
        open_elsewhere = {tuple(item) for snap in ledger.snapshots.values() if now - snap["at"] <= GRANT_SECONDS
                          for item in snap["stampers"]}
        roots = marker_manifest.worktrees(MAIN_ROOT)
        kept, held = set(), set()
        for root in roots:
            undone, pairs = reconcile(ledger, root, {item[1:] for item in stamped if item[0] == root},
                                      {item[1:] for item in open_elsewhere if item[0] == root})
            notes += ["%s %s" % (root, item) for item in undone]
            kept |= {(root,) + pair for pair in pairs}
            held |= {(root,) + pair_of(entry["line"]) for entry in ledger.pending[root]}
        if own and own["background"] and event == "PostToolUse":  # a line this audit did not keep is still to come
            ledger.grants += [{"root": root, "kind": kind, "sha": sha, "expires": now + GRANT_SECONDS}
                              for root, kind, sha in sorted(stamped - kept)]
        removed = marker_manifest.audit(roots, held)
    finally:
        ledger.close()
    if not notes and not removed:
        return None
    reason = ("Marker audit: %s%s. Only bin/run-tests.sh (just test) and bin/review-marker.sh write markers "
              "and their manifest lines, on a call that is nothing but that command and reads; the PR gates "
              "refuse any other. Run the suite or finish the review instead; on a branch whose scripts predate the "
              "manifest, git merge origin/next first." % (
                  "; ".join(notes) if notes else "manifest unchanged",
                  ("; deleted %d marker(s) with no manifest line: %s" % (len(removed), ", ".join(removed)))
                  if removed else ""))
    print("marker-audit: %s" % reason, file=sys.stderr)
    return {"decision": "block", "reason": reason,
            "hookSpecificOutput": {"hookEventName": event, "additionalContext": reason}}


def main():
    mode = sys.argv[1] if len(sys.argv) == 2 else ""
    payload = json.loads(sys.stdin.read() or "{}")
    if not isinstance(payload, dict):
        raise ValueError("the payload is not a JSON object")
    if mode == "snapshot":
        snapshot(payload)
        return 0
    if mode == "audit":
        output = audit(payload)
        if output:
            print(json.dumps(output))
        return 0
    print("usage: marker_ledger.py snapshot|audit < PAYLOAD", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
