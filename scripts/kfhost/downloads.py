"""Download policy. `borrowed` distrusts us: a download is allowed only if it can be verified against the
GitHub repository, never against anything the Worker or a seeder says (url/sha256/size from them are NEVER
enough). Other profiles keep using Worker-published records. All fetches go through scripts/ci/kf_fetch.py.
"""

import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

from .profile import Refused

KF_FETCH = Path(__file__).resolve().parents[1] / "ci" / "kf_fetch.py"
# Tracked files of the verified checkout that pin (url, sha256) pairs. Add manifests here, not by globbing.
PIN_FILES = (
    "scripts/ci/setup-cache.lock.json",
    "scripts/bench/windows/appmatrix/manifest.json",
    "ci/vast-broker/planner/fixtures/official-driver-pins.json",
)
SHA = re.compile(r"[0-9a-f]{64}\Z")
ATTESTATION_UNAVAILABLE = "attestation verification not available: artifact refused for profile borrowed"


def _walk(node, out):
    if isinstance(node, dict):
        url, sha = node.get("url"), node.get("sha256")
        if isinstance(url, str) and isinstance(sha, str) and SHA.fullmatch(sha):
            size = node.get("size")
            out.setdefault((url, sha), set()).add(size if type(size) is int else None)
        for v in node.values():
            _walk(v, out)
    elif isinstance(node, list):
        for v in node:
            _walk(v, out)


def pins_from_checkout(path):
    """{(url, sha256): {sizes}} collected from the known pin files of the checkout at `path`."""
    out = {}
    root = Path(path).resolve()
    for rel in PIN_FILES:
        f = (root / rel).resolve()
        if root not in f.parents or not f.is_file():
            continue
        try:
            _walk(json.loads(f.read_text(encoding="utf-8")), out)
        except (OSError, ValueError, RecursionError):
            continue
    return out


def verify_attestation(artifact_path, sha256):
    """GitHub attestation with signer identity pinned to repo + workflow. NOT implemented: always False."""
    return False


class DownloadPolicy:
    def __init__(self, profile, checkout, log=print, run=subprocess.run):
        self.profile, self.checkout, self.log, self.run = profile, checkout, log, run

    def check(self, url, sha256, size=None, source="worker", attestation_path=None):
        """Raise Refused unless this triple may be downloaded under the profile."""
        if self.profile != "borrowed":
            return
        if attestation_path is not None:
            if not verify_attestation(attestation_path, sha256):
                self.log(ATTESTATION_UNAVAILABLE)
                raise Refused(ATTESTATION_UNAVAILABLE)
            return
        sizes = pins_from_checkout(self.checkout).get((url, sha256))
        if sizes is None:
            why = "url/sha256 from the %s is not pinned in the verified checkout; borrowed refuses it" % source
            self.log(why)
            raise Refused(why)
        known = {s for s in sizes if s is not None}
        if size is not None and known and size not in known:
            raise Refused("size does not match the pin in the verified checkout")

    def fetch(self, url, sha256, size, out, source="worker"):
        """Policy check, then kf_fetch (SHA-256 pinned, no redirects). Returns the exit code."""
        self.check(url, sha256, size, source)
        argv = [sys.executable, "-I", str(KF_FETCH), url, str(out), "--sha256", sha256]
        if size is not None:
            argv += ["--size", str(size)]
        with tempfile.TemporaryDirectory() as cwd:
            return self.run(argv, cwd=cwd, check=False).returncode
