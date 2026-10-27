"""The tests, review-shipping and code-review markers, and the manifest that vouches for them.

  python3 -I marker_manifest.py stamp ROOT KIND SHA     write the marker and its manifest line
  python3 -I marker_manifest.py vouched ROOT KIND SHA   exit 0 when the marker is vouched for, else 1

ROOT/.claude/markers/manifest holds one line per marker bin/run-tests.sh or bin/review-marker.sh wrote:

  <kind> <sha> <sha256 of the marker file>

A marker counts only when it is a regular file and the manifest, a regular file too, holds its exact
line. bin/check-marker.sh refuses any other at PR time, and the hooks' marker ledger
(.claude/hooks/lib/marker_ledger.py) keeps only the lines a stamping call accounts for and deletes,
through audit() here, every marker in every worktree the manifest does not vouch for. Only `stamp`
appends to the manifest; the ledger rewrites it when it reverts, holds back or restores a line.
Python 3.8-compatible.
"""

import contextlib
import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
import time

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)  # python3 -I leaves the script's own directory off sys.path; file_lock lives beside it

KINDS = ("tests", "review-shipping", "code-review")


def digest(path):
    """The SHA-256 of a regular file that is not a link, or None."""
    if os.path.islink(path) or not os.path.isfile(path):
        return None
    with open(path, "rb") as handle:
        return hashlib.sha256(handle.read()).hexdigest()


def manifest(markers):
    """The manifest's lines as (kind, sha, digest) tuples; none when it is missing or not a regular file."""
    path = os.path.join(markers, "manifest")
    if os.path.islink(path) or not os.path.isfile(path):
        return set()
    with open(path, errors="replace") as handle:
        return {tuple(line.split()) for line in handle if len(line.split()) == 3}


def real_markers(root):
    """ROOT/.claude/markers, or None when it or .claude is a link (whose manifest would be anyone's)."""
    markers = os.path.join(root, ".claude", "markers")
    return markers if os.path.realpath(markers) == os.path.join(os.path.realpath(root), ".claude", "markers") \
        else None


def vouched(root, kind, name, lines=None):
    """Whether ROOT's marker KIND/NAME is a regular file whose exact manifest line exists."""
    markers = real_markers(root)
    if markers is None or kind not in KINDS:
        return False
    found = digest(os.path.join(markers, kind, name))
    return found is not None and (kind, name, found) in (manifest(markers) if lines is None else lines)


@contextlib.contextmanager
def manifest_lock(markers):
    """An exclusive lock on MARKERS/.manifest.lock, held while the manifest is appended to or rewritten.
    file_lock is imported here, not at the top, so `vouched` needs no more than this file."""
    import file_lock
    with open(os.path.join(markers, ".manifest.lock"), "a+") as handle:
        file_lock.lock(handle)
        try:
            yield
        finally:
            file_lock.unlock(handle)


def stamp(root, kind, sha):
    """Write the marker and its line. The line goes first and the marker is moved into place after it,
    so the audit never sees the marker without its line."""
    if kind not in KINDS or len(sha) != 40 or set(sha) - set("0123456789abcdef"):
        raise ValueError("stamp needs a marker kind and a full 40-character SHA")
    markers = os.path.join(root, ".claude", "markers")
    os.makedirs(os.path.join(markers, kind), exist_ok=True)
    path = os.path.join(markers, "manifest")
    if os.path.lexists(path) and (os.path.islink(path) or not os.path.isfile(path)):
        raise ValueError("%s is not a regular file, so a line appended to it would vouch for nothing: remove it" % path)
    handle, temporary = tempfile.mkstemp(prefix=".stamp.", dir=markers)
    try:
        with os.fdopen(handle, "w") as stream:
            stream.write("%d\n" % int(time.time()))
        with manifest_lock(markers), open(path, "a") as stream:
            stream.write("%s %s %s\n" % (kind, sha, digest(temporary)))
        os.replace(temporary, os.path.join(markers, kind, sha))
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def worktrees(root):
    """Every checkout of ROOT's repository: the main checkout and each worktree."""
    try:
        result = subprocess.run(["git", "-C", root, "worktree", "list", "--porcelain"], stdout=subprocess.PIPE,
                                stderr=subprocess.DEVNULL, universal_newlines=True, timeout=10)
    except (OSError, subprocess.SubprocessError):
        return [root]
    paths = [line[len("worktree "):] for line in result.stdout.splitlines() if line.startswith("worktree ")]
    return [os.path.realpath(path) for path in paths or [root]]


def audit(roots, held=()):
    """Delete every entry in the three marker directories that is not vouched for, bar the (root, kind, name)
    markers in HELD, whose lines the ledger holds pending; return the deleted paths."""
    removed = []
    for root in roots:
        markers = os.path.join(root, ".claude", "markers")
        if real_markers(root) is None:
            os.unlink(markers if os.path.islink(markers) else os.path.join(root, ".claude"))
            removed.append(markers)
            continue
        lines = manifest(markers)
        for kind in KINDS:
            directory = os.path.join(markers, kind)
            if os.path.islink(directory):
                os.unlink(directory)
                removed.append(directory)
                continue
            if not os.path.isdir(directory):
                continue
            for name in sorted(os.listdir(directory)):
                path = os.path.join(directory, name)
                if vouched(root, kind, name, lines) or ((root, kind, name) in held and digest(path)):
                    continue
                if os.path.isdir(path) and not os.path.islink(path):
                    shutil.rmtree(path)
                else:
                    os.unlink(path)
                removed.append(path)
    return removed


def main():
    mode, args = (sys.argv[1], sys.argv[2:]) if len(sys.argv) > 1 else ("", [])
    if mode == "stamp" and len(args) == 3:
        try:
            stamp(*args)
        except ValueError as error:
            print("marker_manifest: %s" % error, file=sys.stderr)
            return 1
        return 0
    if mode == "vouched" and len(args) == 3:
        return 0 if vouched(*args) else 1
    print("usage: marker_manifest.py stamp|vouched ROOT KIND SHA", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
