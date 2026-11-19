"""Dataset fetch with sha256 verification. Pins live in pins.json; an unpinned download is kept
as <name>.unverified, its hash printed, and the call fails with 'PIN ME' unless
KF_ML_ALLOW_UNPINNED=1 (dev only).

2026-11-19 (standalone trust, #473): a network fetch goes through the repository's trust helper,
scripts/ci/kf_trust.py, for the dataset's artifact in scripts/ci/trust_manifest.json (`trust_artifact`
in pins.json): the helper checks the anchors and does the transfer (parallel and resumable for a pinned
file, the pin's `parallel` streams). This file holds no download primitive."""
from __future__ import annotations

import hashlib
import json
import os
import importlib.util
from pathlib import Path

PINS_FILE = Path(__file__).resolve().parent.parent / "pins.json"


class PinMe(Exception):
    pass


class HashMismatch(Exception):
    pass


def load_pins(path=None) -> dict:
    with open(path or PINS_FILE) as f:
        return json.load(f)


def _sha256(path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


def _md5(path) -> str:
    h = hashlib.md5(usedforsecurity=False)
    with open(path, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


class TrustHelperMissing(Exception):
    pass


def trust_helper():
    """scripts/ci/kf_trust.py of this checkout, loaded as a module; None outside a repository checkout."""
    path = Path(__file__).resolve().parents[3] / "scripts" / "ci" / "kf_trust.py"
    if not path.is_file():
        return None
    spec = importlib.util.spec_from_file_location("kf_trust", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def fetch(name: str, data_dir, pins=None, environ=None, opener=None, source_dir=None, trust=None) -> dict:
    """Return {'path', 'sha256', 'pinned'}; downloads on first use. source_dir: an offline copy (a wheelhouse's data/): the file is
    taken from there instead of the network and is verified against the pin exactly like a download."""
    environ = os.environ if environ is None else environ
    pins = pins if pins is not None else load_pins()
    ent = pins["datasets"][name]
    data_dir = Path(data_dir)
    data_dir.mkdir(parents=True, exist_ok=True)
    dest = data_dir / ent["filename"]
    unver = Path(str(dest) + ".unverified")
    want = ent.get("sha256")
    if not dest.exists():
        src = unver if unver.exists() else None
        if src is None:
            tmp = Path(str(dest) + ".part")
            op, target = opener, ent["url"]
            if source_dir is not None:
                local = Path(source_dir) / ent["filename"]
                if not local.is_file():
                    raise FileNotFoundError("offline: %s is not in %s" % (ent["filename"], source_dir))
                op, target = (lambda path: open(path, "rb")), str(local)
            if op is None:
                # the network: only through the trust helper, for the dataset's manifest artifact
                art = ent.get("trust_artifact")
                if not art:
                    raise PinMe("PIN ME: dataset %s has no trust_artifact in pins.json; add the artifact with its anchors to "
                                "scripts/ci/trust_manifest.json (no download outside the trust helper)" % name)
                kt = trust if trust is not None else trust_helper()
                if kt is None:
                    raise TrustHelperMissing("TRUST_HELPER_MISSING: %s needs scripts/ci/kf_trust.py (run kfml from a repository checkout) "
                                             "or an offline copy (--source-dir)" % name)
                kt.resolve(art, tmp, transport=kt.HttpTransport(streams=int(ent.get("parallel", 4))))
            else:
                with op(target) as r, open(tmp, "wb") as f:
                    while True:
                        b = r.read(1 << 20)
                        if not b:
                            break
                        f.write(b)
            src = tmp
        got = _sha256(src)
        if want:
            if got != want:
                src.unlink()
                raise HashMismatch("%s: sha256 %s != pinned %s (file deleted)" % (name, got, want))
            # 2026-11-19 (standalone trust): the vendor's own checksum, where the vendor publishes only an md5 (pins.json md5_source).
            vmd5 = ent.get("md5")
            if vmd5 and _md5(src) != vmd5:
                src.unlink()
                raise HashMismatch("%s: md5 != the vendor's %s (%s; file deleted)" % (name, vmd5, ent.get("md5_source", "?")))
            os.replace(src, dest)
        else:
            if src is not unver:
                os.replace(src, unver)
            if environ.get("KF_ML_ALLOW_UNPINNED") != "1":
                raise PinMe("PIN ME: dataset %s (%s) has no sha256 pin; downloaded sha256=%s. Put it in "
                            "scripts/kfml/pins.json (or KF_ML_ALLOW_UNPINNED=1, dev only)." % (name, ent["url"], got))
            os.replace(unver, dest)
            return {"path": str(dest), "sha256": got, "pinned": False}
    got = _sha256(dest)
    if want and got != want:
        dest.unlink()
        raise HashMismatch("%s: cached file sha256 %s != pinned %s (deleted)" % (name, got, want))
    if not want and environ.get("KF_ML_ALLOW_UNPINNED") != "1":
        raise PinMe("PIN ME: dataset %s has no sha256 pin; cached sha256=%s" % (name, got))
    return {"path": str(dest), "sha256": got, "pinned": bool(want)}
