"""Borrowed mode (`--trust-git-only`): the daemon trusts only the public git repository, not the Worker.

STATUS: PARKED (2026-11-12), owner decision pending. The mode-specific code lives in this module, in
downloads.py (repo-pinned download policy, attestation interface) and in the `borrowed` branches of
profile.py / daemon.py / jobs.py / fetch.py, so it can move to a follow-up branch in one step. While ENABLED is
False the daemon refuses to start with --trust-git-only (a Worker-added `borrowed` restriction, which only
tightens, is still applied).
"""

ENABLED = False


def fetch_and_check_ancestry(git, is_ancestor, commit, branches):
    """Under --trust-git-only only: fetch ONLY refs/heads/master and refs/heads/next (never refs/pull/* or an
    arbitrary SHA) and require `commit` to be an ancestor of one of them. `git(args)` runs git in the fresh repo
    (remote `origin` set); `is_ancestor(branch)` answers merge-base --is-ancestor. Raises the caller's
    FetchError type through the late import below."""
    from .fetch import FetchError

    got = []
    for b in branches:  # a missing branch is tolerated if the other exists
        try:
            git(["fetch", "-q", "--no-tags", "--filter=blob:none", "origin",
                 "+refs/heads/%s:refs/remotes/origin/%s" % (b, b)])
            got.append(b)
        except FetchError:
            pass
    if not got:
        raise FetchError("could not fetch refs/heads/master or refs/heads/next")
    if not any(is_ancestor(b) for b in got):
        raise FetchError("commit %s is not reachable from origin/master or origin/next" % commit)
