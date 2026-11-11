"""Fetch one commit of the public repo and verify it. Only `git`, only into a temp dir under work_dir."""

import os
import subprocess
import tempfile


class FetchError(Exception):
    pass


def _git(args, cwd, timeout):
    env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": cwd, "GIT_TERMINAL_PROMPT": "0",
           "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull}
    done = subprocess.run(["git", *args], cwd=cwd, env=env, capture_output=True, text=True, timeout=timeout, check=False)
    if done.returncode != 0:
        raise FetchError("git %s failed: %s" % (args[0], done.stderr.strip()[:200]))
    return done.stdout.strip()


def fetch_commit(repo, commit, work_dir, timeout=900):
    """Return the checkout directory (caller removes it). Raises FetchError on any mismatch."""
    os.makedirs(work_dir, mode=0o700, exist_ok=True)
    dest = tempfile.mkdtemp(prefix="run-", dir=work_dir)
    try:
        _git(["init", "-q"], dest, 60)
        _git(["fetch", "-q", "--depth=1", repo, commit], dest, timeout)
        _git(["checkout", "-q", "--detach", "FETCH_HEAD"], dest, 300)
        head = _git(["rev-parse", "HEAD"], dest, 30)
    except (FetchError, subprocess.SubprocessError, OSError) as error:
        raise FetchError(str(error)) from error
    if head != commit:
        raise FetchError("commit mismatch: wanted %s, got %s" % (commit, head))
    return dest
