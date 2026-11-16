#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free, network-free contracts for scripts/bench/assets/build_test_assets.py.

Falsifier: a manifest with a duplicate name, a non-https URL, a disallowed licence or a
share-alike licence without the flag validates; verify accepts a changed byte, an extra
file, or a missing file; a name that escapes the destination is accepted; the pure-Python
ISO loses a file name or a byte.

Run: python3 -m unittest scripts.ci.test_test_assets
 or: python3 -m unittest discover -s scripts/ci -p test_test_assets.py
"""

import contextlib
import copy
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts" / "bench" / "assets"))
import build_test_assets as B  # noqa: E402


def sha(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def entry(name, data=None, **kw):
    e = {
        "name": name,
        "url": "https://example.org/" + name.replace("/", "_"),
        "license": "CC0-1.0",
        "license_url": "https://creativecommons.org/publicdomain/zero/1.0/",
        "share_alike": False,
        "attribution": "Someone (CC0)",
        "source_page": "https://example.org/page",
        "sha256": sha(data) if data is not None else None,
        "size": len(data) if data is not None else None,
        "purpose": "unit test",
    }
    e.update(kw)
    return e


def manifest(*assets):
    return {"schema": B.SCHEMA, "assets": list(assets)}


class Fixture(unittest.TestCase):
    def setUp(self):
        self._td = tempfile.TemporaryDirectory()
        self.tmp = Path(self._td.name)
        self.addCleanup(self._td.cleanup)
        self.files = {"a.bin": b"alpha" * 1000, "sub/b.txt": b"beta\n", "sub/c.dat": os.urandom(5000)}
        self.m = manifest(*[entry(n, d) for n, d in self.files.items()])
        self.mpath = self.tmp / "m.json"
        self.mpath.write_text(json.dumps(self.m))
        self.dir = self.tmp / "bundle"
        for n, d in self.files.items():
            p = self.dir / n
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_bytes(d)

    def run_main(self, *argv):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            rc = B.main(["--manifest", str(self.mpath), *map(str, argv)])
        return rc, out.getvalue(), err.getvalue()


class ManifestSchema(unittest.TestCase):
    def bad(self, m, text):
        with self.assertRaises(B.AssetError) as cm:
            B.validate_manifest(m)
        self.assertIn(text, str(cm.exception))

    def test_valid_minimal(self):
        self.assertEqual(len(B.validate_manifest(manifest(entry("x.jpg", b"1")))), 1)

    def test_unpinned_is_valid_but_half_pinned_is_not(self):
        B.validate_manifest(manifest(entry("x.jpg")))
        self.bad(manifest(entry("x.jpg", sha256=sha(b"1"))), "pinned together")

    def test_wrong_schema(self):
        m = manifest(entry("x"))
        m["schema"] = "other/1"
        self.bad(m, "schema must be")

    def test_duplicate_names_even_by_case(self):
        self.bad(manifest(entry("x.jpg"), entry("x.jpg")), "duplicate name")
        self.bad(manifest(entry("x.jpg"), entry("X.JPG")), "duplicate name")

    def test_unsafe_names(self):
        for n in ("../x", "/abs", "a/../b", "a/b/c", "a\\b", ".hidden", "a b", "x/", "MANIFEST.json",
                  "licenses.txt", "a/..", "a./b", ""):
            with self.subTest(name=n):
                self.bad(manifest(entry(n)), "assets[0]")

    def test_non_https_urls(self):
        for k in ("url", "license_url", "source_page"):
            for u in ("http://example.org/x", "ftp://example.org/x", "file:///etc/passwd", "https:///x", "x"):
                with self.subTest(key=k, url=u):
                    self.bad(manifest(entry("x", **{k: u})), "https URL")

    def test_licences(self):
        for lic in ("CC-BY-NC-4.0", "CC-BY-ND-4.0", "CC-BY-NC-SA-4.0", "Proprietary", "", "GPL-2.0-only"):
            with self.subTest(lic=lic):
                self.bad(manifest(entry("x", license=lic)), "not allowed")
        for lic in sorted(B.ALLOWED_LICENSES):
            sa = "-SA-" in lic
            B.validate_manifest(manifest(entry("x", license=lic, share_alike=sa)))

    def test_share_alike_needs_flag_both_ways(self):
        self.bad(manifest(entry("x", license="CC-BY-SA-4.0", share_alike=False)), "disagrees")
        self.bad(manifest(entry("x", license="CC-BY-4.0", share_alike=True)), "disagrees")
        self.bad(manifest(entry("x", share_alike="no")), "boolean")

    def test_sha256_shape(self):
        for s in ("abc", "G" * 64, "A" * 64, sha(b"1") + "0", 5):
            with self.subTest(s=s):
                self.bad(manifest(entry("x", sha256=s, size=1)), "sha256 must be")

    def test_size_shape(self):
        for z in (0, -1, 1.5, True, "5"):
            with self.subTest(size=z):
                self.bad(manifest(entry("x", sha256=sha(b"1"), size=z)), "size must be")

    def test_size_limits(self):
        self.bad(manifest(entry("x", sha256=sha(b"1"), size=B.MAX_FILE + 1)), "per-file limit")
        big = [entry(f"f{i}", sha256=sha(bytes([i])), size=B.MAX_FILE) for i in range(3)]
        self.bad(manifest(*big), "bundle limit")

    def test_missing_and_extra_keys(self):
        e = entry("x")
        del e["attribution"]
        self.bad(manifest(e), "keys must be exactly")
        e = entry("x")
        e["extra"] = 1
        self.bad(manifest(e), "keys must be exactly")

    def test_empty_attribution_and_purpose(self):
        self.bad(manifest(entry("x", attribution="  ")), "attribution")
        self.bad(manifest(entry("x", purpose="")), "purpose")

    def test_shipped_manifest_is_valid(self):
        m, assets = B.load_manifest(B.DEFAULT_MANIFEST)
        self.assertGreaterEqual(len(assets), 10)
        for a in assets:
            self.assertNotRegex(a["license"], r"NC|ND")
        self.assertEqual(m["schema"], B.SCHEMA)


class Verify(Fixture):
    def test_clean_directory_passes(self):
        self.assertEqual(B.verify_dir(self.mpath, self.dir), [])
        rc, out, _ = self.run_main("verify", "--dir", self.dir)
        self.assertEqual(rc, 0)
        self.assertIn("ok", out)

    def test_changed_byte(self):
        p = self.dir / "a.bin"
        b = bytearray(p.read_bytes())
        b[10] ^= 1
        p.write_bytes(bytes(b))
        problems = B.verify_dir(self.mpath, self.dir)
        self.assertEqual(len(problems), 1)
        self.assertIn("a.bin: sha256", problems[0])
        self.assertEqual(self.run_main("verify", "--dir", self.dir)[0], 1)

    def test_changed_size(self):
        (self.dir / "sub/b.txt").write_bytes(b"beta\nmore")
        self.assertIn("size", B.verify_dir(self.mpath, self.dir)[0])

    def test_extra_file(self):
        (self.dir / "extra.txt").write_text("x")
        (self.dir / "sub" / "deep.txt").write_text("x")
        problems = B.verify_dir(self.mpath, self.dir)
        self.assertEqual(sorted(p.split(":")[0] for p in problems), ["extra.txt", "sub/deep.txt"])

    def test_missing_file(self):
        (self.dir / "sub/c.dat").unlink()
        problems = B.verify_dir(self.mpath, self.dir)
        self.assertEqual(len(problems), 1)
        self.assertIn("sub/c.dat: missing", problems[0])

    def test_symlink_is_not_a_file(self):
        (self.dir / "a.bin").unlink()
        (self.dir / "a.bin").symlink_to(self.dir / "sub" / "b.txt")
        self.assertTrue(any("a.bin" in p for p in B.verify_dir(self.mpath, self.dir)))

    def test_unpinned_entry_fails_verify(self):
        self.mpath.write_text(json.dumps(manifest(entry("a.bin"))))
        self.assertIn("not pinned", B.verify_dir(self.mpath, self.dir)[0])

    def test_generated_files_are_allowed(self):
        (self.dir / "LICENSES.txt").write_text("x")
        (self.dir / "MANIFEST.json").write_text("{}")
        self.assertEqual(B.verify_dir(self.mpath, self.dir), [])


class Licenses(Fixture):
    def test_content(self):
        m = manifest(
            entry("a.bin", self.files["a.bin"], license="CC-BY-4.0", attribution="Ann (CC BY 4.0)"),
            entry("sub/b.txt", self.files["sub/b.txt"], license="CC-BY-SA-4.0", share_alike=True,
                  license_url="https://creativecommons.org/licenses/by-sa/4.0/"),
            entry("sub/c.dat", self.files["sub/c.dat"], source_page="https://example.org/src"),
        )
        self.mpath.write_text(json.dumps(m))
        rc, _, _ = self.run_main("licenses", "--dir", self.dir)
        self.assertEqual(rc, 0)
        text = (self.dir / "LICENSES.txt").read_text()
        for needle in ("a.bin", "CC-BY-4.0", "Ann (CC BY 4.0)", "sub/b.txt", "CC-BY-SA-4.0",
                       "https://creativecommons.org/licenses/by-sa/4.0/", "https://example.org/src",
                       "Share-alike:  YES", "Share-alike:  no", "Source page:", "Attribution:"):
            self.assertIn(needle, text)
        self.assertEqual(text.count("File:         "), 3)
        self.assertEqual(json.loads((self.dir / "MANIFEST.json").read_text())["schema"], B.SCHEMA)
        self.assertEqual(B.verify_dir(self.mpath, self.dir), [])


class PathTraversal(Fixture):
    def test_safe_join(self):
        for n in ("../x", "/etc/passwd", "a/../../x", "a/b/c"):
            with self.subTest(n=n):
                with self.assertRaises(B.AssetError):
                    B.safe_join(self.dir, n)
        self.assertEqual(B.safe_join(self.dir, "sub/b.txt"), self.dir / "sub" / "b.txt")

    def test_manifest_with_traversal_is_refused_by_every_command(self):
        self.mpath.write_text(json.dumps(manifest(entry("../evil", b"x"))))
        for argv in (("verify", "--dir", self.dir), ("licenses", "--dir", self.dir),
                     ("pin", "--dir", self.dir), ("fetch", "--dest", self.tmp / "d")):
            with self.subTest(argv=argv[0]):
                self.assertEqual(self.run_main(*argv)[0], 1)
        self.assertFalse((self.tmp / "evil").exists())

    def test_symlinked_destination_is_refused(self):
        dest = self.tmp / "dest"
        (dest / "sub").mkdir(parents=True)
        target = self.tmp / "outside"
        target.write_text("keep")
        (dest / "a.bin").symlink_to(target)
        rc, _, err = self.run_main("fetch", "--dest", dest)
        self.assertEqual(rc, 1)
        self.assertIn("symlink", err)
        self.assertEqual(target.read_text(), "keep")


class FakeResponse(io.BytesIO):
    def __init__(self, data, headers=None):
        super().__init__(data)
        self.headers = headers or {}

    def __enter__(self):
        return self

    def __exit__(self, *a):
        self.close()


class FakeOpener:
    def __init__(self, data, headers=None):
        self.data, self.headers = data, headers

    def open(self, req, timeout=None):
        h = {"Content-Length": str(len(self.data))} if self.headers is None else self.headers
        return FakeResponse(self.data, h)


class Download(Fixture):
    def test_good_download_and_atomic_rename(self):
        dest = self.tmp / "out" / "x.bin"
        data = b"hello" * 100
        got = B.download("https://example.org/x", dest, sha(data), len(data), [10**6], opener=FakeOpener(data))
        self.assertEqual(got, (sha(data), len(data)))
        self.assertEqual(dest.read_bytes(), data)
        self.assertFalse(dest.with_name("x.bin.part").exists())

    def test_hash_mismatch_leaves_nothing(self):
        dest = self.tmp / "x.bin"
        with self.assertRaises(B.AssetError):
            B.download("https://example.org/x", dest, sha(b"other"), 5, [10**6], opener=FakeOpener(b"hello"))
        self.assertFalse(dest.exists())
        self.assertFalse(dest.with_name("x.bin.part").exists())

    def test_size_mismatch(self):
        with self.assertRaises(B.AssetError):
            B.download("https://example.org/x", self.tmp / "x", None, 4, [10**6],
                       opener=FakeOpener(b"hello", headers={}))

    def test_budget_and_limits(self):
        with self.assertRaises(B.AssetError):
            B.download("https://example.org/x", self.tmp / "x", None, None, [3], opener=FakeOpener(b"hello"))
        with self.assertRaises(B.AssetError):  # no Content-Length: still bounded while streaming
            B.download("https://example.org/x", self.tmp / "y", None, None, [3],
                       opener=FakeOpener(b"hello", headers={}))
        self.assertFalse((self.tmp / "x").exists() or (self.tmp / "y").exists())

    def test_non_https_refused(self):
        with self.assertRaises(B.AssetError):
            B.download("http://example.org/x", self.tmp / "x", None, None, [10**6], opener=FakeOpener(b"x"))

    def test_redirect_to_http_refused(self):
        h = B._HttpsOnlyRedirect()
        with self.assertRaises(B.AssetError):
            h.redirect_request(None, None, 302, "Found", {}, "http://example.org/x")

    def test_fetch_refuses_unpinned_without_flag(self):
        self.mpath.write_text(json.dumps(manifest(entry("a.bin"))))
        rc, _, err = self.run_main("fetch", "--dest", self.tmp / "d")
        self.assertEqual(rc, 1)
        self.assertIn("unpinned", err)

    def test_fetch_skips_present_verified_files(self):
        rc, out, _ = self.run_main("fetch", "--dest", self.dir)
        self.assertEqual(rc, 0)
        self.assertEqual(out.count("already present"), 3)


class Pin(Fixture):
    def test_pin_fills_nulls_and_never_overwrites(self):
        self.mpath.write_text(json.dumps(manifest(entry("a.bin"), entry("sub/b.txt", self.files["sub/b.txt"]))))
        rc, _, _ = self.run_main("pin", "--dir", self.dir)
        self.assertEqual(rc, 0)
        a = json.loads(self.mpath.read_text())["assets"]
        self.assertEqual(a[0]["sha256"], sha(self.files["a.bin"]))
        self.assertEqual(a[0]["size"], len(self.files["a.bin"]))
        # a different byte now: the existing pin must not change
        (self.dir / "a.bin").write_bytes(b"changed")
        before = self.mpath.read_text()
        rc, _, err = self.run_main("pin", "--dir", self.dir)
        self.assertEqual(rc, 1)
        self.assertIn("refusing to overwrite", err)
        self.assertEqual(self.mpath.read_text(), before)

    def test_pin_errors_for_unpinned_missing_file_and_writes_nothing(self):
        self.mpath.write_text(json.dumps(manifest(entry("a.bin"), entry("sub/b.txt"))))
        (self.dir / "sub/b.txt").unlink()
        before = self.mpath.read_text()
        self.assertEqual(self.run_main("pin", "--dir", self.dir)[0], 1)
        self.assertEqual(self.mpath.read_text(), before)


def read_joliet(iso: bytes) -> dict:
    """Independent minimal reader: walk the Joliet tree and return {path: bytes}."""
    sec = 2048
    svd = None
    for i in range(16, 40):
        d = iso[i * sec:(i + 1) * sec]
        assert d[1:6] == b"CD001"
        if d[0] == 255:
            break
        if d[0] == 2 and d[88:91] == b"%/E":
            svd = d
    assert svd is not None, "no Joliet descriptor"
    label = svd[40:72].decode("utf-16-be").strip()

    def walk(ext, ln, prefix, out):
        data = iso[ext * sec:ext * sec + ln]
        off = 0
        while off < ln:
            if data[off] == 0:
                off = (off // sec + 1) * sec
                continue
            n = data[off]
            rec = data[off:off + n]
            e, size = struct.unpack("<I", rec[2:6])[0], struct.unpack("<I", rec[10:14])[0]
            flags = rec[25]
            ident = rec[33:33 + rec[32]]
            off += n
            if ident in (b"\x00", b"\x01"):
                continue
            name = ident.decode("utf-16-be")
            if flags & 2:
                walk(e, size, prefix + name + "/", out)
            else:
                out[prefix + name] = iso[e * sec:e * sec + size]
    root = svd[156:190]
    out: dict = {}
    walk(struct.unpack("<I", root[2:6])[0], struct.unpack("<I", root[10:14])[0], "", out)
    return {"__label__": label, **out}


class IsoTests(Fixture):
    def setUp(self):
        super().setUp()
        # a name in a directory, a 0-byte-free but sector-unaligned file, and a >1 sector file
        self.files["big/long_name_with.dots.v2.bin"] = os.urandom(2048 * 3 + 17)
        d = self.dir / "big"
        d.mkdir()
        (d / "long_name_with.dots.v2.bin").write_bytes(self.files["big/long_name_with.dots.v2.bin"])
        self.m = manifest(*[entry(n, x) for n, x in self.files.items()])
        self.mpath.write_text(json.dumps(self.m))

    def build(self, engine):
        out = self.tmp / f"{engine}.iso"
        rc, o, e = self.run_main("iso", "--dir", self.dir, "--out", out, "--engine", engine)
        self.assertEqual(rc, 0, e)
        return out

    def check_contents(self, got):
        self.assertEqual(got.pop("__label__"), "KFDATA")
        expect = dict(self.files)
        expect["MANIFEST.json"] = (self.dir / "MANIFEST.json").read_bytes()
        expect["LICENSES.txt"] = (self.dir / "LICENSES.txt").read_bytes()
        self.assertEqual(got, expect)

    def test_python_engine_roundtrip(self):
        out = self.build("python")
        self.assertEqual(out.stat().st_size % 2048, 0)
        self.check_contents(read_joliet(out.read_bytes()))

    def test_python_engine_is_deterministic(self):
        a = self.build("python").read_bytes()
        out2 = self.tmp / "again.iso"
        self.assertEqual(self.run_main("iso", "--dir", self.dir, "--out", out2, "--engine", "python")[0], 0)
        self.assertEqual(a, out2.read_bytes())

    @unittest.skipUnless(shutil.which("isoinfo"), "isoinfo not installed")
    def test_python_iso_is_readable_by_isoinfo(self):
        out = self.build("python")
        listing = subprocess.run(["isoinfo", "-J", "-l", "-i", str(out)], capture_output=True, text=True,
                                 check=True).stdout
        for n in ("long_name_with.dots.v2.bin", "b.txt", "c.dat", "a.bin", "LICENSES.txt", "MANIFEST.json"):
            self.assertIn(n, listing)
        hdr = subprocess.run(["isoinfo", "-d", "-i", str(out)], capture_output=True, text=True,
                             check=True).stdout
        self.assertIn("KFDATA", hdr)
        data = subprocess.run(["isoinfo", "-J", "-i", str(out), "-x", "/sub/c.dat"], capture_output=True,
                              check=True).stdout
        self.assertEqual(data, self.files["sub/c.dat"])

    @unittest.skipUnless(B.find_iso_tool(), "no genisoimage, mkisofs or xorriso")
    def test_tool_engine_roundtrip(self):
        out = self.build("tool")
        self.check_contents(read_joliet(out.read_bytes()))

    def test_iso_refuses_unverified_directory(self):
        (self.dir / "a.bin").write_bytes(b"tampered")
        out = self.tmp / "bad.iso"
        rc, _, err = self.run_main("iso", "--dir", self.dir, "--out", out, "--engine", "python")
        self.assertEqual(rc, 1)
        self.assertFalse(out.exists())

    def test_iso_output_inside_dir_refused(self):
        rc, _, err = self.run_main("iso", "--dir", self.dir, "--out", self.dir / "x.iso", "--engine", "python")
        self.assertEqual(rc, 1)
        self.assertIn("must not be inside", err)

    def test_missing_tool_message(self):
        old = B.find_iso_tool
        B.find_iso_tool = lambda: None
        try:
            rc, _, err = self.run_main("iso", "--dir", self.dir, "--out", self.tmp / "t.iso", "--engine", "tool")
            self.assertEqual(rc, 1)
            self.assertIn("no genisoimage", err)
            rc, _, err = self.run_main("iso", "--dir", self.dir, "--out", self.tmp / "p.iso")
            self.assertEqual(rc, 0)
            self.assertIn("pure-Python", err)
        finally:
            B.find_iso_tool = old


if __name__ == "__main__":
    unittest.main()
