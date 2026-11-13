"""Hardware/software description and its probe. The runner never imports torch itself:
torch facts come from a short-lived `_probe` subprocess."""
from __future__ import annotations

import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import sysconfig
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional, Tuple

from . import versions

# What NVIDIA documents as the minimum Linux driver for a CUDA toolkit release (CUDA release
# notes, "minor version compatibility" table). Only used to word a NOT_RUN reason; no test
# asserts or depends on a driver version. UNVERIFIED here: re-check against the release notes.
MIN_DRIVER_FOR_CUDA = {"12.6": "560.28", "12.8": "570.26", "12.9": "575.51", "13.0": "580.65"}
DRIVER_TOO_OLD = re.compile(
    r"driver on your system is too old|driver version is insufficient|forward compatibility", re.I)


@dataclass
class Env:
    python: str = ""
    platform: str = "linux"
    gpu_visible: bool = False  # nvidia-smi sees a GPU
    gpu_name: Optional[str] = None
    cc: Optional[Tuple[int, int]] = None
    vram_total_gb: Optional[float] = None
    vram_free_gb: Optional[float] = None
    driver: Optional[str] = None
    torch: Optional[str] = None
    cuda: Optional[str] = None  # torch.version.cuda
    cudnn: Optional[str] = None
    triton: Optional[str] = None
    cuda_available: bool = False
    cuda_error: Optional[str] = None
    nvcc: Optional[str] = None  # nvcc release major.minor
    nvcc_path: Optional[str] = None
    cxx: bool = False  # C compiler found
    python_h: bool = False
    disk_free_gb: Optional[float] = None
    modules: dict = field(default_factory=dict)
    lock_sha256: Optional[str] = None
    venv: Optional[str] = None

    def to_dict(self):
        d = dict(self.__dict__)
        return d


def sha256_file(path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


def venv_root(environ=None) -> Optional[Path]:
    """The directory that owns every cache: the active venv, or KF_ML_HOME (explicit)."""
    environ = os.environ if environ is None else environ
    if environ.get("KF_ML_HOME"):
        return Path(environ["KF_ML_HOME"]).resolve()
    if sys.prefix != getattr(sys, "base_prefix", sys.prefix):
        return Path(sys.prefix).resolve()
    return None


CACHE_VARS = ("HOME", "HF_HOME", "HF_HUB_CACHE", "TORCH_HOME", "XDG_CACHE_HOME", "XDG_CONFIG_HOME",
              "XDG_DATA_HOME", "PIP_CACHE_DIR", "UV_CACHE_DIR", "TRITON_CACHE_DIR",
              "TORCHINDUCTOR_CACHE_DIR", "TORCH_EXTENSIONS_DIR", "CUDA_CACHE_PATH", "TMPDIR", "TMP",
              "TEMP", "MPLCONFIGDIR", "PYTHONPYCACHEPREFIX", "KF_ML_DATA")


def cache_env(root: Path) -> dict:
    """Env vars for every test process: all caches, temp and data under <root>/kfml-cache."""
    c = Path(root) / "kfml-cache"
    m = {
        "HOME": c / "home",
        "HF_HOME": c / "hf",
        "HF_HUB_CACHE": c / "hf" / "hub",
        "TORCH_HOME": c / "torch",
        "XDG_CACHE_HOME": c / "xdg-cache",
        "XDG_CONFIG_HOME": c / "xdg-config",
        "XDG_DATA_HOME": c / "xdg-data",
        "PIP_CACHE_DIR": c / "pip",
        "UV_CACHE_DIR": c / "uv",
        "TRITON_CACHE_DIR": c / "triton",
        "TORCHINDUCTOR_CACHE_DIR": c / "inductor",
        "TORCH_EXTENSIONS_DIR": c / "torch-ext",
        "CUDA_CACHE_PATH": c / "nv-compute-cache",
        "TMPDIR": c / "tmp",
        "TMP": c / "tmp",
        "TEMP": c / "tmp",
        "MPLCONFIGDIR": c / "mpl",
        "PYTHONPYCACHEPREFIX": c / "pycache",
        "KF_ML_DATA": c / "data",
    }
    return {k: str(v) for k, v in m.items()}


def ensure_dirs(env: dict):
    for v in env.values():
        os.makedirs(v, exist_ok=True)


def assert_env_inside(env: dict, root, allowed_ro=()) -> list:
    """Return the offending (var, path) pairs: cache vars pointing outside root."""
    root = os.path.realpath(str(root))
    bad = []
    for k in CACHE_VARS:
        v = env.get(k)
        if v is None:
            continue
        rp = os.path.realpath(v)
        if rp != root and not rp.startswith(root + os.sep) and not any(
                rp == a or rp.startswith(a + os.sep) for a in allowed_ro):
            bad.append((k, v))
    return bad


def read_lock_sha(root: Optional[Path]) -> Optional[str]:
    if not root:
        return None
    p = Path(root) / "kfml-lock.sha256"
    try:
        return p.read_text().split()[0]
    except Exception:
        return None


def find_nvcc(environ=None, root: Optional[Path] = None):
    environ = os.environ if environ is None else environ
    cands = []
    if root and (Path(root) / "cuda_home" / "bin" / "nvcc").exists():
        cands.append(str(Path(root) / "cuda_home" / "bin" / "nvcc"))
    ch = environ.get("CUDA_HOME") or environ.get("CUDA_PATH")
    if ch:
        cands.append(str(Path(ch) / "bin" / "nvcc"))
    w = shutil.which("nvcc")
    if w:
        cands.append(w)
    for c in cands:
        if os.path.exists(c):
            try:
                out = subprocess.run([c, "--version"], capture_output=True, text=True, timeout=30).stdout
            except Exception:
                continue
            m = re.search(r"release (\d+\.\d+)", out)
            if m:
                return m.group(1), c
    return None, None


def probe_torch(python=None, environ=None, timeout=180) -> dict:
    """Run `python -I -m kfml._probe` style child; returns its JSON dict or {'error': ...}."""
    python = python or sys.executable
    env = dict(os.environ if environ is None else environ)
    here = str(Path(__file__).resolve().parent.parent)
    env["PYTHONPATH"] = here + os.pathsep + env.get("PYTHONPATH", "")
    try:
        p = subprocess.run([python, "-m", "kfml._probe"], capture_output=True, text=True,
                           timeout=timeout, env=env)
    except Exception as e:
        return {"error": "probe failed: %s" % e}
    for line in reversed(p.stdout.splitlines()):
        if line.startswith("{"):
            try:
                return json.loads(line)
            except ValueError:
                break
    return {"error": "probe produced no JSON (rc=%s): %s" % (p.returncode, p.stderr[-500:])}


def nvidia_smi() -> dict:
    exe = shutil.which("nvidia-smi")
    if not exe:
        return {}
    try:
        out = subprocess.run(
            [exe, "--query-gpu=name,driver_version,memory.total,memory.free,compute_cap",
             "--format=csv,noheader,nounits"], capture_output=True, text=True, timeout=30).stdout
    except Exception:
        return {}
    line = out.strip().splitlines()[0] if out.strip() else ""
    parts = [p.strip() for p in line.split(",")]
    if len(parts) < 5:
        return {}
    d = {"name": parts[0], "driver": parts[1]}
    try:
        d["total"] = float(parts[2]) / 1024.0
        d["free"] = float(parts[3]) / 1024.0
    except ValueError:
        pass
    cc = versions.parse(parts[4])
    if cc and len(cc) >= 2:
        d["cc"] = (cc[0], cc[1])
    return d


def probe_env(environ=None, python=None) -> Env:
    environ = os.environ if environ is None else environ
    root = venv_root(environ)
    e = Env(python=sys.version.split()[0], platform=sys.platform, venv=str(root) if root else None)
    smi = nvidia_smi()
    e.gpu_visible = bool(smi)
    e.driver = smi.get("driver")
    e.gpu_name, e.cc = smi.get("name"), smi.get("cc")
    e.vram_total_gb, e.vram_free_gb = smi.get("total"), smi.get("free")
    t = probe_torch(python, environ)
    e.torch, e.cuda, e.cudnn, e.triton = t.get("torch"), t.get("cuda"), t.get("cudnn"), t.get("triton")
    e.cuda_available, e.cuda_error = bool(t.get("cuda_available")), t.get("cuda_error") or t.get("error")
    e.modules = t.get("modules", {})
    if t.get("cuda_available"):  # torch's view wins (honours CUDA_VISIBLE_DEVICES)
        e.gpu_name = t.get("gpu_name") or e.gpu_name
        if t.get("cc"):
            e.cc = tuple(t["cc"])
        e.vram_total_gb = t.get("vram_total_gb", e.vram_total_gb)
        e.vram_free_gb = t.get("vram_free_gb", e.vram_free_gb)
    e.nvcc, e.nvcc_path = find_nvcc(environ, root)
    e.cxx = bool(shutil.which(environ.get("CC", "cc")) or shutil.which("gcc"))
    inc = sysconfig.get_paths().get("include")
    e.python_h = bool(inc and os.path.exists(os.path.join(inc, "Python.h")))
    try:
        e.disk_free_gb = shutil.disk_usage(str(root or ".")).free / 2**30
    except Exception:
        pass
    e.lock_sha256 = read_lock_sha(root)
    return e
