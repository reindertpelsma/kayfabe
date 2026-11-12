"""Fetch one commit of the allowlisted repo and verify it. Only `git`, only into a temp dir under work_dir.

Default: any SHA the Worker names (plain fetch, redirects off). strict (--trust-git-only only, see
profile_borrowed.py): master/next only plus an ancestry check. The tree key is recomputed in every profile.
"""

import os
import shutil
import subprocess
import tempfile

from . import profile_borrowed
from .treekey import commit_tree_key

BRANCHES = ("master", "next")


class FetchError(Exception):
    pass


def _git(args, cwd, timeout, raw=False, ok=(0,)):
    env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": cwd, "GIT_TERMINAL_PROMPT": "0",
           "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull}
    cmd = ["git", "-c", "http.followRedirects=false", "-c", "protocol.version=2", *args]
    done = subprocess.run(cmd, cwd=cwd, env=env, capture_output=True, timeout=timeout, check=False)
    if done.returncode not in ok:
        raise FetchError("git %s failed: %s" % (args[0], done.stderr.decode("utf-8", "replace").strip()[:200]))
    return done.stdout if raw else done.stdout.decode("utf-8", "replace").strip()


def fetch_commit(repo, commit, work_dir, tree_key, strict=False, timeout=900):
    """Return (checkout_dir, tree_key_mismatch). The caller removes the directory.

    Default (standard, container, owned): the Worker is trusted for what to run, so ANY commit SHA is fetched
    with a plain `git fetch origin <sha>` (redirects off); HEAD must equal the commit after checkout. A tree key
    that differs from the recomputed one is returned as `tree_key_mismatch` (the caller warns loudly), not refused.
    strict (only under --trust-git-only, the parked profile_borrowed mode): master/next-only fetch, ancestry check
    and a refusal on a tree key mismatch.
    """
    os.makedirs(work_dir, mode=0o700, exist_ok=True)
    dest = tempfile.mkdtemp(prefix="run-", dir=work_dir)
    try:
        _git(["init", "-q"], dest, 60)
        _git(["remote", "add", "origin", repo], dest, 60)
        if strict:
            profile_borrowed.fetch_and_check_ancestry(lambda a, **k: _git(a, dest, k.get("timeout", timeout)),
                                                      lambda b: _ancestor(dest, commit, b), commit, BRANCHES)
        else:
            _git(["fetch", "-q", "--depth=1", "origin", commit], dest, timeout)
        _git(["checkout", "-q", "--detach", commit], dest, 300)
        head = _git(["rev-parse", "HEAD"], dest, 30)
        if head != commit:
            raise FetchError("commit mismatch: wanted %s, got %s" % (commit, head))
        key = commit_tree_key(lambda a: _git(a, dest, 300, raw=True), commit)
        mismatch = key != tree_key
        if mismatch and strict:
            raise FetchError("tree key mismatch: offer %s, recomputed %s" % (tree_key, key))
    except (FetchError, subprocess.SubprocessError, OSError) as error:
        shutil.rmtree(dest, ignore_errors=True)
        raise FetchError(str(error)) from error
    return dest, mismatch


def _ancestor(dest, commit, branch):
    done = subprocess.run(["git", "merge-base", "--is-ancestor", commit, "origin/" + branch], cwd=dest,
                          capture_output=True, check=False,
                          env={"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": dest,
                               "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull})
    return done.returncode == 0
