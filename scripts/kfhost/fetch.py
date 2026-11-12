"""Fetch one commit of the allowlisted repo and verify it. Only `git`, only into a temp dir under work_dir.

strict (every profile but owned): fetch ONLY refs/heads/master and refs/heads/next (never refs/pull/* or an
arbitrary SHA) and require the commit to be an ancestor of one of them. In every profile the tree key is
recomputed on the host and must equal the offer's.
"""

import os
import shutil
import subprocess
import tempfile

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


def fetch_commit(repo, commit, work_dir, tree_key, strict=True, timeout=900):
    """Return the checkout directory (caller removes it). Raises FetchError on any mismatch."""
    os.makedirs(work_dir, mode=0o700, exist_ok=True)
    dest = tempfile.mkdtemp(prefix="run-", dir=work_dir)
    try:
        _git(["init", "-q"], dest, 60)
        _git(["remote", "add", "origin", repo], dest, 60)
        if strict:
            got = []
            for b in BRANCHES:  # only these two refs; a missing branch is tolerated if the other exists
                try:
                    _git(["fetch", "-q", "--no-tags", "--filter=blob:none", "origin",
                          "+refs/heads/%s:refs/remotes/origin/%s" % (b, b)], dest, timeout)
                    got.append(b)
                except FetchError:
                    pass
            if not got:
                raise FetchError("could not fetch refs/heads/master or refs/heads/next")
            if not any(_ancestor(dest, commit, b) for b in got):
                raise FetchError("commit %s is not reachable from origin/master or origin/next" % commit)
        else:
            _git(["fetch", "-q", "--depth=1", "origin", commit], dest, timeout)
        _git(["checkout", "-q", "--detach", commit], dest, 300)
        head = _git(["rev-parse", "HEAD"], dest, 30)
        if head != commit:
            raise FetchError("commit mismatch: wanted %s, got %s" % (commit, head))
        key = commit_tree_key(lambda a: _git(a, dest, 300, raw=True), commit)
        if key != tree_key:
            raise FetchError("tree key mismatch: offer %s, recomputed %s" % (tree_key, key))
    except (FetchError, subprocess.SubprocessError, OSError) as error:
        shutil.rmtree(dest, ignore_errors=True)
        raise FetchError(str(error)) from error
    return dest


def _ancestor(dest, commit, branch):
    done = subprocess.run(["git", "merge-base", "--is-ancestor", commit, "origin/" + branch], cwd=dest,
                          capture_output=True, check=False,
                          env={"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": dest,
                               "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull})
    return done.returncode == 0
