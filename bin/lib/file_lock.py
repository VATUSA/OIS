"""An exclusive lock on an open lock file, on POSIX and on Windows: the marker ledger
(.claude/hooks/lib/marker_ledger.py) and the manifest (bin/lib/marker_manifest.py) take it.

  handle = open(path, "a+")
  file_lock.lock(handle)      # blocks until this process holds the lock
  ...
  file_lock.unlock(handle)

POSIX uses fcntl.flock. Windows has no fcntl, so it locks one byte at offset 0 with
msvcrt.locking. LK_NBLCK fails at once when another process holds the byte, and lock() retries
until it is held; LK_LOCK would wait a second between its own ten tries and then give up.
Python 3.8-compatible.

Tests: bin/lib/file-lock.test.sh
"""

import time

try:
    import fcntl
except ImportError:
    fcntl = None
    import msvcrt

RETRY_SECONDS = 0.05


def lock(handle):
    """Take the exclusive lock on HANDLE's file, waiting while another process holds it."""
    if fcntl is not None:
        fcntl.flock(handle, fcntl.LOCK_EX)
        return
    while True:
        handle.seek(0)
        try:
            msvcrt.locking(handle.fileno(), msvcrt.LK_NBLCK, 1)
            return
        except OSError:
            time.sleep(RETRY_SECONDS)


def unlock(handle):
    """Release the lock lock() took on HANDLE's file."""
    if fcntl is not None:
        fcntl.flock(handle, fcntl.LOCK_UN)
        return
    handle.seek(0)
    msvcrt.locking(handle.fileno(), msvcrt.LK_UNLCK, 1)
