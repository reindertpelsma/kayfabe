"""kfml bundle: reproducible tarball + manifest. Never uploads. Wheels are included only when their
licence derives to the allowlist AND they contain nothing NVIDIA; everything else is fetch-only."""
from __future__ import annotations

import gzip
import hashlib
import io
import json
import os
import tarfile
from pathlib import Path

from . import licenses

ROOT = Path(__file__).resolve().parent.parent
SKIP_DIRS = {"__pycache__", ".git"}


def _sha(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def tree_files(root=ROOT):
    out = []
    for dp, dn, fn in os.walk(root):
        dn[:] = sorted(d for d in dn if d not in SKIP_DIRS)
        for f in sorted(fn):
            if f.endswith(".pyc"):
                continue
            p = Path(dp) / f
            out.append(("kfml/" + p.relative_to(root).as_posix(), p))
    return sorted(out)


def dataset_components(pins: dict):
    comps = []
    for name, d in sorted(pins.get("datasets", {}).items()):
        lic = d.get("license", "UNKNOWN")
        redist = bool(d.get("redistribution_ok")) and lic in licenses.ALLOW
        comps.append({"kind": "dataset", "name": name, "version": d.get("filename"), "license_spdx": lic,
                      "sha256": d.get("sha256"), "source_url": d["url"], "redistributable": redist,
                      "fetch_only_reason": "" if redist else "licence does not clearly allow redistribution; "
                      "fetched from the official URL with the sha256 pin"})
    return comps


def build(out_dir, wheels_dir=None, pins=None, upstreams=(), root=ROOT):
    out_dir = Path(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    comps, members = [], []
    for arc, p in tree_files(root):
        members.append((arc, p.read_bytes(), 0o755 if os.access(p, os.X_OK) else 0o644))
    for up in upstreams:
        comps.append({"kind": "source", "name": up.name, "version": up.commit, "license_spdx": up.license,
                      "sha256": None, "source_url": up.repo_url, "role": up.role,
                      "redistributable": up.vendored and up.license in licenses.ALLOW,
                      "fetch_only_reason": "" if up.vendored else "not vendored: " + up.role})
    if wheels_dir:
        for w in sorted(Path(wheels_dir).glob("*.whl")):
            data = w.read_bytes()
            s = licenses.scan_wheel(str(w))
            c = {"kind": "wheel", "name": s["name"], "version": s["version"], "filename": s["filename"],
                 "tags": s["tags"], "license_spdx": s["license_spdx"], "license_evidence": s["license_evidence"],
                 "sha256": _sha(data), "bundled_libs": s["bundled_libs"], "nvidia_origin_libs": s["nvidia_origin_libs"],
                 "redistributable": s["redistributable"], "fetch_only_reason": s["fetch_only_reason"],
                 "source_url": "https://pypi.org/project/%s/%s/" % (s["name"], s["version"].split("+")[0])}
            comps.append(c)
            if s["redistributable"]:
                members.append(("wheels/" + w.name, data, 0o644))
    comps.extend(dataset_components(pins or {}))
    comps.sort(key=lambda c: (c["kind"], c["name"], c.get("version") or ""))
    manifest = {"bundle_format": 1, "components": comps}
    mb = (json.dumps(manifest, indent=1, sort_keys=True) + "\n").encode()
    members.append(("MANIFEST.json", mb, 0o644))
    members.sort(key=lambda m: m[0])
    raw = io.BytesIO()
    with tarfile.open(fileobj=raw, mode="w", format=tarfile.GNU_FORMAT) as tf:
        for arc, data, mode in members:
            ti = tarfile.TarInfo(arc)
            ti.size, ti.mtime, ti.mode = len(data), 0, mode
            ti.uid = ti.gid = 0
            ti.uname = ti.gname = ""
            tf.addfile(ti, io.BytesIO(data))
    gz = io.BytesIO()
    with gzip.GzipFile(fileobj=gz, mode="wb", mtime=0, filename="") as g:
        g.write(raw.getvalue())
    tar_path = out_dir / "kfml-bundle.tar.gz"
    tar_path.write_bytes(gz.getvalue())
    (out_dir / "MANIFEST.json").write_bytes(mb)
    sha = _sha(gz.getvalue())
    (out_dir / "kfml-bundle.tar.gz.sha256").write_text("%s  kfml-bundle.tar.gz\n" % sha)
    return {"path": str(tar_path), "sha256": sha, "manifest": manifest}
