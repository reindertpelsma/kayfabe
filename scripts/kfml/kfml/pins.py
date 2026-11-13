"""Upstream code pins: vendored NOTICE check, or clone-at-pin + verify + patch, all in a fresh temp dir."""
from __future__ import annotations

import hashlib
import os
import subprocess
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Optional

ROOT = Path(__file__).resolve().parent.parent


class PinError(Exception):
    pass


@dataclass
class Resolved:
    path: str
    commit: str
    patch_sha256: Optional[str]


def check_vendored(up, root=ROOT) -> Resolved:
    d = Path(root) / "vendor" / up.vendor_dir
    notice = d / "NOTICE"
    if not notice.is_file():
        raise PinError("vendored %s: %s missing" % (up.name, notice))
    text = notice.read_text()
    if up.commit not in text or up.repo_url not in text:
        raise PinError("vendored %s: NOTICE must name %s and commit %s" % (up.name, up.repo_url, up.commit))
    if not any((d / n).is_file() for n in ("LICENSE", "LICENSE.txt", "LICENSE.md", "COPYING")):
        raise PinError("vendored %s: LICENSE file missing" % up.name)
    return Resolved(str(d), up.commit, None)


def _git(args, cwd, run=subprocess.run):
    p = run(["git"] + list(args), cwd=cwd, capture_output=True, text=True, timeout=1800)
    if p.returncode != 0:
        raise PinError("git %s failed: %s" % (" ".join(args), (p.stderr or p.stdout).strip()[-400:]))
    return p.stdout.strip()


def clone_at_pin(up, tmp_root, root=ROOT, run=subprocess.run) -> Resolved:
    """Fresh clone dir under tmp_root; HEAD must equal the pin; the patch (if any) must apply."""
    work = tempfile.mkdtemp(prefix="kfml-%s-" % up.name, dir=str(tmp_root))
    _git(["init", "-q"], work, run)
    _git(["remote", "add", "origin", up.repo_url], work, run)
    try:
        _git(["fetch", "-q", "--depth", "1", "origin", up.commit], work, run)
    except PinError:
        try:
            _git(["fetch", "-q", "origin"], work, run)
        except PinError as e:
            raise PinError("cannot fetch %s for pin %s: %s" % (up.repo_url, up.commit, e))
    try:
        _git(["checkout", "-q", "--detach", up.commit], work, run)
    except PinError:
        raise PinError("pin %s of %s not found in the repository" % (up.commit, up.repo_url))
    head = _git(["rev-parse", "HEAD"], work, run)
    if head != up.commit:
        raise PinError("refusing %s: HEAD %s != pin %s" % (up.name, head, up.commit))
    psha = None
    if up.patch:
        pp = Path(root) / up.patch
        if not pp.is_file():
            raise PinError("patch %s missing" % pp)
        _git(["apply", "--check", str(pp)], work, run)  # raises -> refused
        _git(["apply", str(pp)], work, run)
        psha = hashlib.sha256(pp.read_bytes()).hexdigest()
    return Resolved(work, head, psha)


def resolve(up, tmp_root, root=ROOT, run=subprocess.run) -> Optional[Resolved]:
    if up.role == "vendored":
        return check_vendored(up, root)
    if up.role == "fetched":
        return clone_at_pin(up, tmp_root, root, run)
    return Resolved("", up.commit, None)  # derived: credit only
