"""The shared GPU lock, openable by whichever user runs the suite (python side of kf_lock.sh).

One path for every user keeps the mutual exclusion between users. `open(path, "a")` is O_CREAT, which
fs.protected_regular refuses on another user's file in a sticky /tmp even at mode 0666; flock(2) needs only a
read-only descriptor. So: create the file once with mode 0666 if it is missing, then open it read-only.
"""
import fcntl
import os
import stat

DEFAULT = "/tmp/kayfabe-fastguest.lock"


def ensure_lock_file(path, opener=os.open):
    """Create the lock file mode 0666 if absent. A create refused because another user won the race is fine."""
    try:
        old = os.umask(0)
        try:
            fd = opener(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o666)
        finally:
            os.umask(old)
        os.close(fd)
    except (FileExistsError, PermissionError):
        pass


def open_lock(path=DEFAULT, opener=os.open):
    """Return a read-only fd for `path` (creating it first if needed); refuse a symlink or non-regular file."""
    ensure_lock_file(path, opener)
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    if not stat.S_ISREG(os.fstat(fd).st_mode):
        os.close(fd)
        raise OSError("lock path is not a regular file: " + path)
    return fd


def acquire(path=DEFAULT, blocking=True, opener=os.open):
    """Open and flock; return the fd (close it to release). BlockingIOError when not blocking and held."""
    fd = open_lock(path, opener)
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | (0 if blocking else fcntl.LOCK_NB))
    except BaseException:
        os.close(fd)
        raise
    return fd


def open_lock_file(path=DEFAULT, opener=os.open):
    """open_lock as a file object, for `with ... as lock: fcntl.flock(lock, ...)` callers."""
    return os.fdopen(open_lock(path, opener), "r")
