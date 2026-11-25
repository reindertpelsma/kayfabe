#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Fetch or check the pinned QEMU source tarball (STATUS: LIVE, 2026-11-15); stdlib only, works under python3 -I.

The pin is scripts/bench/qemu.pin (name, sha256, size). Every path that puts a QEMU tarball on disk goes through here and
fails CLOSED: the bytes are written to `<out>.unverified`, checked for size and sha256 against the pin, and only then renamed
to `<out>`. On any mismatch the unverified bytes are deleted and the exit status is non-zero; there is no fallback to an
unverified download and no switch that turns the check off. A local copy may be supplied with --from-file (the filesystem
form of the "artifacts pinnable" ruling, docs/OWNER_RULINGS.md section AR): it is hash-checked exactly like a download.

  qemu_source.py version                          print the pinned version (10.2.4)
  qemu_source.py fetch DEST_DIR [--from-file P]   print the verified path DEST_DIR/qemu-<ver>.tar.xz
  qemu_source.py verify FILE                      exit 0 only if FILE matches the pin

2026-11-17 (docs/design/V3_STANDALONE_TRUST.md, audit row 4): the transfer and the anchor check are scripts/ci/kf_trust.py's. The pin of
this file is the entry's `upstream-pin` (a source tarball whose sha256 was measured from download.qemu.org; scripts/ci/trust_manifest.json
holds the same value and trust_lint.py holds the two together). Behaviour is unchanged: the same refusals, the same names. A `--from-file`
copy is still held to the pin here (the owner has not ruled that this one flag should become a plain user input), and kf_trust.py records
it as `user` with the pin as `--expect-sha256`; the record is one line in trust.jsonl (KF_TRUST_EVIDENCE_DIR) and on stderr.
"""
import argparse
import hashlib
import os
from pathlib import Path
import re
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import kf_trust  # noqa: E402

PIN = Path(__file__).resolve().parents[1] / "bench" / "qemu.pin"
BASE_URL = "https://download.qemu.org/"
NAME_RE = re.compile(r"^qemu-([0-9]+(?:\.[0-9]+)+)\.tar\.xz$")
BLOCK = 1 << 20
TIMEOUT = 60


class Refused(Exception):
    pass


def read_pin(path=PIN):
    """Return (name, version, sha256, size) from the single pin line; anything else is refused."""
    try:
        lines = [l.split() for l in Path(path).read_text().splitlines() if l.strip() and not l.lstrip().startswith("#")]
    except OSError as e:
        raise Refused(f"cannot read pin {path}: {e.strerror}") from None
    if len(lines) != 1:
        raise Refused(f"pin {path} must hold exactly one entry, found {len(lines)}")
    f = lines[0]
    m = NAME_RE.match(f[0]) if len(f) == 3 else None
    kv = dict(x.split("=", 1) for x in f[1:] if "=" in x)
    if not m or set(kv) != {"sha256", "size"} or not re.fullmatch(r"[0-9a-f]{64}", kv["sha256"]) \
            or not re.fullmatch(r"[1-9][0-9]*", kv["size"]):
        raise Refused(f"pin {path} entry is not `qemu-<ver>.tar.xz sha256=<64 hex> size=<bytes>`")
    return f[0], m.group(1), kv["sha256"], int(kv["size"])


def check(path, sha256, size):
    """Raise Refused unless the file at path has exactly the pinned size and sha256."""
    try:
        if not Path(path).is_file():
            raise Refused(f"{path} is not a file")
        if os.path.getsize(path) != size:
            raise Refused(f"{path} is {os.path.getsize(path)} bytes, the pin says {size}")
        h = hashlib.sha256()
        with open(path, "rb") as f:
            for block in iter(lambda: f.read(BLOCK), b""):
                h.update(block)
    except OSError as e:
        raise Refused(f"cannot read {path}: {e.strerror}") from None
    if h.hexdigest() != sha256:
        raise Refused(f"{path} sha256 {h.hexdigest()} is not the pinned {sha256}")


def _discard(*paths):
    for p in paths:
        try:
            os.unlink(p)
        except FileNotFoundError:
            pass


def _manifest(name, sha256, size, base_url):
    """The pin file as a one-entry trust manifest: scripts/bench/qemu.pin stays the single place the value is written."""
    return {"schema": kf_trust.SCHEMA, "artifacts": {"qemu-source": {
        "kind": "file", "origin": base_url + name,
        "anchors": [{"class": kf_trust.CLASS_PIN, "sha256": sha256, "size": size, "pin_origin": "measured-from-upstream", "scope": "source",
                     "pin_source_url": BASE_URL + name, "status": "CARRIED", "carried_from": "scripts/bench/qemu.pin"}]}}}


def fetch(dest_dir, pin=PIN, from_file=None, base_url=BASE_URL):
    """Return the path of a pinned, verified tarball in dest_dir; raise Refused (leaving no file under that name) if it cannot."""
    name, _ver, sha256, size = read_pin(pin)
    dest = Path(dest_dir)
    out = dest / name
    try:
        dest.mkdir(parents=True, exist_ok=True)
    except OSError as e:
        raise Refused(f"cannot create {dest}: {e.strerror}") from None
    if from_file is None and out.exists():
        try:
            check(out, sha256, size)
            return out
        except Refused:
            _discard(out)  # a stale or damaged cached copy is never used and never trusted
    elif out.exists() or out.is_symlink():
        _discard(out)
    tmp = Path(str(out) + ".unverified")
    _discard(tmp)
    manifest = _manifest(name, sha256, size, base_url)
    try:
        if from_file is not None:
            check(from_file, sha256, size)  # the local copy is held to the pin exactly like a download
            kf_trust.resolve("qemu-source", out, manifest=manifest, user_source=str(from_file), expect_sha256=sha256, mode=kf_trust.MODE_GIT_ONLY)
        else:
            # the loopback exception exists for the tests that pass --base-url; the default origin is https only
            kf_trust.resolve("qemu-source", out, manifest=manifest, mode=kf_trust.MODE_GIT_ONLY,
                             transport=kf_trust.HttpTransport(allow_loopback_http=base_url != BASE_URL))
        check(out, sha256, size)
    except kf_trust.TrustError as e:
        _discard(tmp, out)
        raise Refused(f"obtaining {name} failed: {e.code}") from None
    except Refused:
        _discard(tmp, out)
        raise
    except (OSError, ValueError) as e:
        _discard(tmp, out)
        raise Refused(f"obtaining {name} failed: {type(e).__name__}") from None
    return out


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--pin", default=str(PIN), help="pin file (tests only)")
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("version")
    f = sub.add_parser("fetch")
    f.add_argument("dest_dir")
    f.add_argument("--from-file", help="local tarball; hash-checked like a download")
    f.add_argument("--base-url", default=BASE_URL, help="download base (tests only); the result is hash-checked either way")
    v = sub.add_parser("verify")
    v.add_argument("file")
    a = p.parse_args(argv)
    try:
        if a.cmd == "version":
            print(read_pin(a.pin)[1])
        elif a.cmd == "verify":
            _n, _v, sha256, size = read_pin(a.pin)
            check(a.file, sha256, size)
        else:
            print(fetch(a.dest_dir, a.pin, a.from_file, a.base_url))
    except Refused as e:
        print(f"QEMU_SOURCE_REFUSED: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
