"""Borrowed mode (`--trust-git-only`): the daemon trusts only the public git repository, not the Worker.

STATUS: PARKED (2026-11-12), owner decision pending. The mode-specific code lives in this module, in
downloads.py (repo-pinned download policy, attestation interface) and in the `borrowed` branches of
profile.py / daemon.py / jobs.py / fetch.py, so it can move to a follow-up branch in one step. While ENABLED is
False the daemon refuses to start with --trust-git-only (a Worker-added `borrowed` restriction, which only
tightens, is still applied).
"""

ENABLED = False
