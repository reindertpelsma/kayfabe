"""Dataset fetch with sha256 verification. Pins live in pins.json; an unpinned download is kept
as <name>.unverified, its hash printed, and the call fails with 'PIN ME' unless
KF_ML_ALLOW_UNPINNED=1 (dev only)."""
from __future__ import annotations

import hashlib
import json
import os
import urllib.request
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


def fetch(name: str, data_dir, pins=None, environ=None, opener=None) -> dict:
    """Return {'path', 'sha256', 'pinned'}; downloads on first use."""
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
            op = opener or urllib.request.urlopen
            with op(ent["url"]) as r, open(tmp, "wb") as f:
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
