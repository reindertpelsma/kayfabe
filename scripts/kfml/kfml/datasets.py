"""Dataset fetch with sha256 verification. Pins live in pins.json; an unpinned download is kept
as <name>.unverified, its hash printed, and the call fails with 'PIN ME' unless
KF_ML_ALLOW_UNPINNED=1 (dev only)."""
from __future__ import annotations

import hashlib
import json
import os
import threading
import time
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


def download_parallel(url, dest, connections=6, urlopen=urllib.request.urlopen, retries=5, sleep=time.sleep, chunk=1 << 20):
    """Download URL into DEST with several ranged requests at once. Returns the byte count, or None when the server does not offer ranges
    (the caller then downloads sequentially). Why: the CIFAR-10 server gives about 45 KB/s per connection (MEASURED 2026-11-17, two boxes and
    the coordinator: a single download takes 15 to 60 minutes), and answers a burst of 8 or more connections with resets, so the default is 6.
    Nothing here is trusted: the caller verifies the sha256 pin of the finished file. A range that fails is retried from the byte it reached."""
    req = urllib.request.Request(url, method="HEAD")
    with urlopen(req) as r:
        length = int(r.headers.get("Content-Length") or 0)
        ranges_ok = "bytes" in (r.headers.get("Accept-Ranges") or "").lower()
    if not ranges_ok or length <= 0 or connections < 2:
        return None
    step = -(-length // connections)
    spans = [(i * step, min(length, (i + 1) * step) - 1) for i in range(connections) if i * step < length]
    fd = os.open(str(dest), os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o644)
    errors = []

    def one(span):
        pos, end = span
        for attempt in range(retries):
            try:
                rq = urllib.request.Request(url, headers={"Range": "bytes=%d-%d" % (pos, end)})
                with urlopen(rq) as r:
                    if getattr(r, "status", 206) != 206:
                        raise OSError("server ignored the Range header (HTTP %s)" % getattr(r, "status", "?"))
                    while pos <= end:
                        b = r.read(min(chunk, end - pos + 1))
                        if not b:
                            raise OSError("short read")
                        os.pwrite(fd, b, pos)
                        pos += len(b)
                return
            except Exception as e:  # noqa: BLE001  a reset, a timeout, a short read: retry this span from where it stopped
                last = e
                sleep(min(2 ** attempt, 10))
        errors.append("bytes %d-%d: %s" % (span[0], span[1], last))

    try:
        threads = [threading.Thread(target=one, args=(sp,)) for sp in spans]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
    finally:
        os.close(fd)
    if errors:
        raise OSError("parallel download failed: " + "; ".join(errors[:3]))
    return length


def fetch(name: str, data_dir, pins=None, environ=None, opener=None, source_dir=None) -> dict:
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
            op = opener or urllib.request.urlopen
            target = ent["url"]
            if source_dir is not None:
                local = Path(source_dir) / ent["filename"]
                if not local.is_file():
                    raise FileNotFoundError("offline: %s is not in %s" % (ent["filename"], source_dir))
                op, target = (lambda path: open(path, "rb")), str(local)
            done = None
            if source_dir is None and opener is None and ent.get("parallel", 1) > 1:
                done = download_parallel(target, tmp, ent["parallel"])
            if done is None:
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
