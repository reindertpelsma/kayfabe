"""Per-invocation git trust for a checkout owned by another uid (python side of kf_git.sh).

`git -c safe.directory=<exact path>` for the one directory, never `*`, never a parent, never persisted.
Read-only queries only.
"""
import os
import subprocess

_BAD = set("*?[\n\r")


def safe_directory_args(path):
    """The exact `-c` argument pair for `path`; ValueError for anything that is not one plain absolute directory."""
    text = os.fspath(path)
    if not text or any(c in _BAD for c in text):
        raise ValueError("refusing an empty, pattern or multi-line safe.directory: " + repr(text))
    real = os.path.realpath(text)
    if real == "/" or not os.path.isabs(real) or not os.path.isdir(real) or any(c in _BAD for c in real):
        raise ValueError("refusing safe.directory for " + repr(real))
    return ["-c", "safe.directory=" + real]


def git_command(path, *args):
    return ["git", *safe_directory_args(path), "-C", os.path.realpath(os.fspath(path)), *args]


def git_output(path, *args):
    return subprocess.check_output(git_command(path, *args), text=True).strip()
