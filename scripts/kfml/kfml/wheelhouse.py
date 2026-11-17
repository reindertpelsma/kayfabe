"""Offline wheelhouse: the environment a machine without internet (the open Linux guest of the CI payload) installs from.

Built ON THE HOST at run time from the repository's hashed lock (`pip download --require-hashes`, official indexes only), plus the
official NVIDIA redist archives for nvcc (sha256 from pins.json); copied into the guest; installed there with
`pip install --no-index --find-links <wheelhouse> --require-hashes` (setup_venv.setup(wheelhouse=...)). Nothing here is ever uploaded or
mirrored: it is a transient directory on the box (NVIDIA binaries are never stored by us). Layout:

    DIR/MANIFEST.json                 {schema, built_by_lock_sha256 per flavour, python, files: {rel: {size, sha256}}}
    DIR/<flavor>/wheels/*.whl         the closure of the flavour's lock for the guest's CPython tag
    DIR/<flavor>/redist/*.tar.xz      cuda_nvcc, cuda_cudart, cuda_cccl of that flavour (optional)
    DIR/data/<filename>               the pinned datasets, fetched and verified on the host (optional)

`verify` (stdlib only, runs in the guest) re-hashes every file against the manifest, so a truncated or tampered copy is refused before
pip sees it; pip then checks each wheel against the lock hash a second time.
"""
from __future__ import annotations

import hashlib
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

SCHEMA = "kfml.wheelhouse/1"
PLATFORMS = ("manylinux_2_28_x86_64", "manylinux_2_17_x86_64", "manylinux2014_x86_64", "linux_x86_64")


class WheelhouseError(Exception):
    pass


def wheels_dir(root, flavor) -> Path:
    return Path(root) / flavor / "wheels"


def py_tag(version: str):
    """'3.12' or '3.12.3' -> ('3.12', 'cp312')."""
    m = re.fullmatch(r"(\d+)\.(\d+)(?:\.\d+)?", version or "")
    if not m:
        raise WheelhouseError("bad python version %r (want 3.12)" % version)
    return "%s.%s" % m.groups(), "cp%s%s" % m.groups()


def download_cmd(python, lock: Path, dest: Path, guest_python=None):
    """pip download of the hashed lock. For another interpreter than the host's (the guest's) the tag, ABI and platform are named
    explicitly and only binary wheels are accepted: nothing is built, nothing runs setup.py."""
    cmd = [python, "-m", "pip", "download", "--no-input", "--disable-pip-version-check", "--require-hashes",
           "--only-binary=:all:", "-d", str(dest), "-r", str(lock)]
    if guest_python:
        ver, tag = py_tag(guest_python)
        cmd += ["--python-version", ver, "--implementation", "cp", "--abi", tag]
        for p in PLATFORMS:
            cmd += ["--platform", p]
    return cmd


def sha256_file(path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


def _scan(root: Path, sub: str) -> dict:
    out = {}
    base = root / sub
    if not base.is_dir():
        return out
    for p in sorted(base.rglob("*")):
        if p.is_file():
            out[p.relative_to(root).as_posix()] = {"size": p.stat().st_size, "sha256": sha256_file(p)}
    return out


def build(out, flavors, pins, guest_python=None, python=None, run=subprocess.run, with_redist=True, datasets=False,
          fetch_fn=None, lock_for=None, now=time.time) -> dict:
    """Fill OUT for each flavour (idempotent: pip skips what is there) and write MANIFEST.json. Returns the manifest."""
    from . import datasets as ds
    from . import setup_venv
    out = Path(out)
    python = python or sys.executable
    lock_for = lock_for or setup_venv.lock_path
    t0 = now()
    manifest = {"schema": SCHEMA, "python": guest_python or "%d.%d" % sys.version_info[:2], "flavors": {}, "files": {}}
    for fl in flavors:
        lock = lock_for(fl)
        text = lock.read_text()
        if not setup_venv.has_hashes(text):
            raise WheelhouseError("%s has no hashes: a wheelhouse is only built from a hashed lock" % lock.name)
        wd = wheels_dir(out, fl)
        wd.mkdir(parents=True, exist_ok=True)
        p = run(download_cmd(python, lock, wd, guest_python))
        if p.returncode != 0:
            raise WheelhouseError("pip download failed for %s (rc=%s)" % (fl, p.returncode))
        manifest["flavors"][fl] = {"lock": lock.name, "lock_sha256": hashlib.sha256(lock.read_bytes()).hexdigest(),
                                   "cuda": pins["flavors"][fl]["cuda"]}
        if with_redist:
            setup_venv.fetch_redist(out / fl / "redist", fl, pins, fetch_fn)
    if datasets:
        for name, ent in sorted(pins["datasets"].items()):
            ds.fetch(name, out / "data", pins)
    for sub in sorted(manifest["flavors"]) + (["data"] if datasets else []):
        manifest["files"].update(_scan(out, sub))
    manifest["total_bytes"] = sum(f["size"] for f in manifest["files"].values())
    manifest["build_s"] = round(now() - t0, 1)
    tmp = out / "MANIFEST.json.tmp"
    tmp.write_text(json.dumps(manifest, indent=1, sort_keys=True) + "\n")
    os.replace(tmp, out / "MANIFEST.json")
    return manifest


def load(root) -> dict:
    p = Path(root) / "MANIFEST.json"
    try:
        doc = json.loads(p.read_text())
    except (OSError, ValueError) as e:
        raise WheelhouseError("no readable MANIFEST.json in %s (%s)" % (root, e))
    if doc.get("schema") != SCHEMA:
        raise WheelhouseError("manifest schema %r, want %s" % (doc.get("schema"), SCHEMA))
    return doc


def verify(root, flavor=None) -> dict:
    """Re-hash every file of the manifest (or only those under <flavor>/ and data/). Missing, short or altered = WheelhouseError; a file
    in the wheels directory that the manifest does not list is refused too (pip would otherwise be offered it)."""
    root = Path(root)
    doc = load(root)
    if flavor is not None and flavor not in doc["flavors"]:
        raise WheelhouseError("the wheelhouse has no flavour %s (has %s)" % (flavor, ", ".join(sorted(doc["flavors"]))))
    want = {k: v for k, v in doc["files"].items() if flavor is None or k.startswith(flavor + "/") or k.startswith("data/")}
    for rel, meta in sorted(want.items()):
        p = root / rel
        if not p.is_file():
            raise WheelhouseError("missing %s" % rel)
        if p.stat().st_size != meta["size"] or sha256_file(p) != meta["sha256"]:
            raise WheelhouseError("%s does not match the manifest (size or sha256)" % rel)
    for fl in ([flavor] if flavor else doc["flavors"]):
        d = wheels_dir(root, fl)
        for p in d.glob("*") if d.is_dir() else ():
            rel = p.relative_to(root).as_posix()
            if rel not in doc["files"]:
                raise WheelhouseError("unlisted file in the wheels directory: %s" % rel)
    return doc
