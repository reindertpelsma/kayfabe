"""kfml setup --venv DIR: the only command that installs anything, and it only touches DIR."""
from __future__ import annotations

import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
from pathlib import Path

from . import env as envmod

ROOT = Path(__file__).resolve().parent.parent
FLAVORS = {"cu128": "requirements.lock.txt", "cu126": "requirements.cu126.lock.txt"}


class SetupError(Exception):
    pass


class NoFlavor(SetupError):
    """No pinned wheel flavour can run on this box (driver too old / GPU not covered): NOT_RUN, never FAIL."""


def lock_path(flavor: str) -> Path:
    if flavor not in FLAVORS:
        raise SetupError("unknown flavor %r (choose %s)" % (flavor, "|".join(FLAVORS)))
    return ROOT / FLAVORS[flavor]


def has_hashes(lock_text: str) -> bool:
    return any("--hash=sha256:" in l for l in lock_text.splitlines() if not l.lstrip().startswith("#"))


INDEX_OPTION = re.compile(r"^\s*(--index-url|--extra-index-url|-i|--find-links|-f)\b")


def offline_lock(lock: Path, venv: Path) -> Path:
    """A copy of the hashed lock without its index options. MEASURED 2026-11-17 (kfml_run_20261117_second_die): pip honours an
    `--index-url` line INSIDE a requirements file even when `--no-index` is on the command line, so an offline install of the lock itself
    still contacted the indexes (and retried for minutes on a machine with no route). The requirement and hash lines are copied byte for byte."""
    keep = [l for l in lock.read_text().splitlines(True) if not INDEX_OPTION.match(l)]
    dest = venv / "kfml-cache" / "tmp" / "offline-lock.txt"
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_text("".join(keep))
    return dest


def check_target(venv: Path):
    if venv.exists() and not (venv / "pyvenv.cfg").exists() and any(venv.iterdir()):
        raise SetupError("%s exists, is not empty and is not a venv; refusing to touch it" % venv)


def install_cmd(venv: Path, lock: Path, hashed: bool, installer: str, find_links=None):
    """find_links: an offline wheelhouse directory: nothing is fetched (--no-index), every wheel must match its lock hash."""
    if installer == "uv":
        # unsafe-best-match: the lock names two indexes (the PyTorch index and PyPI) and torch==X+cuNNN exists only on the first; uv's default
        # first-index rule fails without it (measured 2026-11-17). The hashes, not the index order, are the integrity control.
        return ["uv", "pip", "sync", "--python", str(venv / "bin" / "python"), "--index-strategy", "unsafe-best-match"] + \
               (["--require-hashes"] if hashed else []) + \
               (["--no-index", "--find-links", str(find_links)] if find_links else []) + [str(lock)]
    return [str(venv / "bin" / "python"), "-m", "pip", "install", "--no-input"] + \
           (["--no-index", "--find-links", str(find_links)] if find_links else []) + \
           (["--require-hashes"] if hashed else []) + ["-r", str(lock)]


def safe_extract(tar_path: Path, dest: Path):
    with tarfile.open(tar_path) as tf:
        for m in tf.getmembers():
            p = os.path.realpath(dest / m.name)
            if not p.startswith(os.path.realpath(dest) + os.sep) or m.islnk() or m.issym() and \
                    os.path.isabs(m.linkname):
                raise SetupError("unsafe tar member %s" % m.name)
        tf.extractall(dest)


def merge_dir(src: Path, dst: Path):
    for p in sorted(src.rglob("*")):
        t = dst / p.relative_to(src)
        if p.is_dir():
            t.mkdir(parents=True, exist_ok=True)
        else:
            t.parent.mkdir(parents=True, exist_ok=True)
            if p.is_symlink():
                if not t.exists():
                    os.symlink(os.readlink(p), t)
            else:
                shutil.copy2(p, t)


def fetch_redist(dest: Path, flavor: str, pins: dict, fetch_fn=None) -> list:
    """The official NVIDIA redist archives (cuda_nvcc, cuda_cudart, cuda_cccl) of a flavour into DEST, sha256-verified against
    pins.json (a mismatch deletes the file and raises). Returns [{name, file, size, sha256}]."""
    import urllib.request
    red = pins["cuda_redist"][flavor]
    dest.mkdir(parents=True, exist_ok=True)
    out = []
    for name, c in sorted(red["components"].items()):
        arc = dest / os.path.basename(c["path"])
        if not arc.exists():
            (fetch_fn or (lambda u, d: urllib.request.urlretrieve(u, str(d))))(red["base_url"] + c["path"], arc)
        h = hashlib.sha256(arc.read_bytes()).hexdigest()
        if h != c["sha256"]:
            arc.unlink()
            raise SetupError("%s: sha256 %s != pinned %s" % (name, h, c["sha256"]))
        out.append({"name": name, "file": arc.name, "size": arc.stat().st_size, "sha256": h})
    return out


def install_nvcc(venv: Path, flavor: str, pins: dict, fetch_fn=None, archive_dir=None):
    """Official NVIDIA redist archives (cuda_nvcc, cuda_cudart, cuda_cccl), sha256 from pins.json, merged into
    <venv>/cuda_home. The pip nvidia-cuda-nvcc wheel has NO nvcc binary (measured: only ptxas, libnvvm, libdevice,
    headers), so it is not used. MEASURED (kfml_run_20261116): the assembled cu126 and cu128 trees compile load_inline
    extensions. archive_dir: archives already on disk (an offline wheelhouse's redist/), verified the same way and never fetched."""
    red = pins["cuda_redist"][flavor]
    home = venv / "cuda_home"
    stage = venv / "kfml-cache" / "tmp" / "nvcc-stage"
    if archive_dir is not None:
        def local(url, dest):
            src = Path(archive_dir) / os.path.basename(url)
            if not src.is_file():
                raise SetupError("offline: %s is not in %s" % (src.name, archive_dir))
            shutil.copyfile(src, dest)
        fetch_fn = local
    fetch_redist(stage, flavor, pins, fetch_fn)
    for name, c in sorted(red["components"].items()):
        arc = stage / os.path.basename(c["path"])
        ex = stage / ("x-" + name)
        shutil.rmtree(ex, ignore_errors=True)
        ex.mkdir()
        safe_extract(arc, ex)
        for top in ex.iterdir():
            merge_dir(top, home)
    if (home / "lib").exists() and not (home / "lib64").exists():
        os.symlink("lib", home / "lib64")
    return home


def resolve_flavor(flavor, pins=None, smi=None):
    """'auto' derives the flavour from this box (nvidia-smi: driver CUDA version, compute capability); an explicit
    name is used as given. Returns (flavor, reason). Raises NoFlavor when no pinned flavour fits."""
    if flavor != "auto":
        lock_path(flavor)
        return flavor, "explicit --flavor %s" % flavor
    from . import datasets
    pins = pins if pins is not None else datasets.load_pins()
    e = envmod.Env()
    smi = envmod.nvidia_smi() if smi is None else smi
    if not smi:
        raise SetupError("--flavor auto needs an NVIDIA GPU visible to nvidia-smi (none found); pass --flavor explicitly")
    e.driver, e.cc, e.driver_cuda = smi.get("driver"), smi.get("cc"), smi.get("driver_cuda")
    c = envmod.choose_flavor(e, pins)
    if c.flavor is None:
        raise (NoFlavor if e.flavor_blocked else SetupError)(c.reason)
    return c.flavor, c.reason


def nvcc_matches_flavor(flavor, pins, environ=None, root=None):
    """True when an nvcc already on the box has the flavour's CUDA major.minor (no redist download needed)."""
    got, _ = envmod.find_nvcc(environ, root)
    want = pins["flavors"][flavor]["cuda"]
    return bool(got) and tuple(got.split(".")[:2]) == tuple(want.split(".")[:2])


def setup(venv, flavor="auto", python=None, with_nvcc=False, allow_unhashed=False, installer="pip",
          pins=None, run=subprocess.run, fetch_fn=None, nvcc_if_needed=False, smi=None, wheelhouse=None):
    venv = Path(venv).resolve()
    flavor, flavor_reason = resolve_flavor(flavor, pins, smi)
    lock = lock_path(flavor)
    text = lock.read_text()
    hashed = has_hashes(text)
    if not hashed and not (allow_unhashed or os.environ.get("KF_ML_ALLOW_UNHASHED") == "1"):
        raise SetupError("%s has no hashes (HASHES TODO). Refusing to install an unhashed lock; "
                         "pass --allow-unhashed for dev use only." % lock.name)
    check_target(venv)
    if not (venv / "pyvenv.cfg").exists():
        p = run([python or sys.executable, "-m", "venv", str(venv)], capture_output=True, text=True)
        if p.returncode != 0:
            raise SetupError("venv creation failed: %s" % p.stderr[-400:])
    cenv = dict(os.environ)
    cenv.update(envmod.cache_env(venv))
    envmod.ensure_dirs(envmod.cache_env(venv))
    links = None
    if wheelhouse:
        if not hashed:
            raise SetupError("an offline install needs the hashed lock (it is the only thing that vouches for the copied wheels)")
        from . import wheelhouse as wh
        wh.verify(wheelhouse, flavor)  # every copied file against the manifest the host wrote
        links = wh.wheels_dir(wheelhouse, flavor)
    p = run(install_cmd(venv, offline_lock(lock, venv) if links else lock, hashed, installer, links), env=cenv)
    if p.returncode != 0:
        raise SetupError("dependency install failed (rc=%s)" % p.returncode)
    sha = hashlib.sha256(lock.read_bytes()).hexdigest()
    (venv / "kfml-lock.sha256").write_text("%s  %s\n" % (sha, lock.name))
    (venv / "kfml-setup.json").write_text(json.dumps(
        {"flavor": flavor, "flavor_reason": flavor_reason, "lock": lock.name, "lock_sha256": sha, "hashed": hashed, "installer": installer}) + "\n")
    if pins is None:
        from . import datasets
        pins = datasets.load_pins()
    nvcc = "none"
    if with_nvcc or nvcc_if_needed:
        if with_nvcc or not nvcc_matches_flavor(flavor, pins, None, venv):
            install_nvcc(venv, flavor, pins, fetch_fn, archive_dir=(Path(wheelhouse) / flavor / "redist") if wheelhouse else None)
            nvcc = "redist"
        else:
            nvcc = "system"
    return {"venv": str(venv), "flavor": flavor, "flavor_reason": flavor_reason, "lock_sha256": sha,
            "hashed": hashed, "nvcc": nvcc}
