#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Box side, stdlib only: verify a stored-zip propimage whose big member was fetched alone (adobe_part2/dl_member.py) without a second
96 GB download or an unzip copy. It fetches the bytes before and after the member by ranged GET, checks that the local header names the
member as stored at the given offset, hashes head + member file + tail and compares the result with the zip pin, then extracts the small
members (state.json, manifest.json, ...) from a sparse copy of head and tail and checks every member against manifest.json.
Writes the unpack stamp that adobe_part2/hold_run.py reads. Never prints the URL.

usage: zip_verify.py URLFILE MEMBER_FILE ZIP_SHA256 OUTDIR
"""
import hashlib
import json
import os
import struct
import sys
import time
import urllib.request
import zipfile
from pathlib import Path


def get(url, lo, hi):
    req = urllib.request.Request(url, headers={"Range": "bytes=%d-%d" % (lo, hi)})
    with urllib.request.urlopen(req, timeout=120) as r:
        if r.status != 206:
            raise SystemExit("range status %s" % r.status)
        total = int(r.headers["Content-Range"].rsplit("/", 1)[1])
        return r.read(), total


def sha_file(path, h=None):
    h = h or hashlib.sha256()
    with open(path, "rb") as f:
        while True:
            b = f.read(8 << 20)
            if not b:
                return h
            h.update(b)


def main():
    url_file, member, want_zip, outdir = sys.argv[1:5]
    url = Path(url_file).read_text().strip()
    out = Path(outdir)
    head, total = get(url, 0, 4095)
    sig, _v, _fl, method, _t, _d, _crc, csize, _usize, nlen, xlen = struct.unpack("<IHHHHHIIIHH", head[:30])
    name = head[30:30 + nlen].decode()
    off = 30 + nlen + xlen
    msize = os.path.getsize(member)
    if sig != 0x04034B50 or method != 0 or name != Path(member).name:
        raise SystemExit("first member is not %s stored: sig=%x method=%d name=%s" % (Path(member).name, sig, method, name))
    print("member", name, "stored at offset", off, "size", msize, "zip size", total, flush=True)
    tail, _ = get(url, off + msize, total - 1)
    t0 = time.time()
    h = hashlib.sha256(head[:off])
    sha_file(member, h)
    h.update(tail)
    ok = h.hexdigest() == want_zip
    print("ZIP_SHA256", "OK" if ok else "MISMATCH", h.hexdigest()[:16], "secs=%.0f" % (time.time() - t0), flush=True)
    if not ok:
        return 3
    sparse = out / ".sparse.zip"
    with open(sparse, "wb") as f:
        f.truncate(total)
        f.seek(0)
        f.write(head[:off])
        f.seek(off + msize)
        f.write(tail)
    with zipfile.ZipFile(sparse) as z:
        names = z.namelist()
        for n in names:
            if n == name:
                continue
            if "/" in n or n.startswith(".") or z.getinfo(n).file_size > 64 << 20:
                raise SystemExit("unexpected member %r" % n)
            (out / n).write_bytes(z.read(n))
    sparse.unlink()
    man = json.loads((out / "manifest.json").read_text())
    members = man["members"]
    bad = []
    for n, want in members.items():
        p = out / n
        got = hashlib.sha256(p.read_bytes()).hexdigest() if n != name else None
        if n == name:
            continue
        if got != want:
            bad.append(n)
    print("members", names, "manifest_mismatch", bad, "qcow2_in_manifest", members.get(name, "")[:16], flush=True)
    if bad:
        return 4
    (out / ".verified").write_text(json.dumps({"sha256": members[name], "size": msize, "at": time.strftime("%FT%TZ", time.gmtime())}))
    print("VERIFY_EXIT rc=0", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
