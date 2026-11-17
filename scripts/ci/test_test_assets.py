#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free, network-free contracts for scripts/bench/assets/build_test_assets.py.

Falsifier: a manifest with a duplicate name, a non-https URL, a disallowed licence, a missing or
unknown source_kind, a private entry without a value, a public-mirror without a mirror_path, a
Wikimedia URL claimed as origin, or a share-alike licence without the flag validates; a private
entry reaches the ISO, MANIFEST.json or LICENSES.txt; the mirror is not tried after an origin
failure, or a mirror file that differs from the pin is accepted; a synthetic generator gives
different bytes on a second run, or a changed byte passes its pin; verify accepts a changed byte, an extra
file, or a missing file; a name that escapes the destination is accepted; the pure-Python
ISO loses a file name or a byte.

Run: python3 -I scripts/ci/test_test_assets.py
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
import urllib.error
from unittest import mock
import wave
import zlib

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts" / "bench" / "assets"))
import build_test_assets as B  # noqa: E402


def sha(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def entry(name, data=None, **kw):
    e = {
        "name": name,
        "url": "https://example.org/" + name.replace("/", "_"),
        "source_kind": "origin",
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
        for lic in ("CC-BY-NC-4.0", "CC-BY-ND-4.0", "CC-BY-NC-SA-4.0", "Proprietary", "", "GPL-2.0-only",
                    "GPL-3.0-or-later", "LGPL-2.1-only", "AGPL-3.0-only", "MPL-2.0", "NOASSERTION"):
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

    def test_shipped_project_entries(self):
        """project/ files: permissive licence, commit-pinned source, project-type extension."""
        _, assets = B.load_manifest(B.DEFAULT_MANIFEST)
        project = [a for a in assets if a["name"].startswith("project/")]
        self.assertGreaterEqual(len(project), 30)
        permissive = {"CC0-1.0", "Public-Domain", "CC-BY-2.0", "CC-BY-2.5", "CC-BY-3.0", "CC-BY-4.0",
                      "MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib"}
        exts = {"psd", "psb", "ai", "indd", "aep", "fla", "xfl", "prproj", "dn", "lrcat", "lrtemplate"}
        total = 0
        for a in project:
            with self.subTest(name=a["name"]):
                self.assertIn(a["license"], permissive)
                self.assertFalse(a["share_alike"])
                self.assertIn(a["name"].rsplit(".", 1)[-1].lower(), exts)
                # every source is a raw or LFS media URL pinned to a 40-hex commit, never a branch
                self.assertRegex(a["url"], r"^https://(raw\.githubusercontent\.com|media\.githubusercontent\.com/media)/"
                                           r"[^/]+/[^/]+/[0-9a-f]{40}/")
                self.assertRegex(a["license_url"], r"^https://github\.com/[^/]+/[^/]+/blob/[0-9a-f]{40}/LICENSE$")
                self.assertIsNotNone(a["sha256"])
                self.assertLess(a["size"], 25 * 1024 * 1024)
                total += a["size"]
        self.assertLess(total, 120 * 1024 * 1024)

    def test_project_names_carry_no_vendor_word(self):
        _, assets = B.load_manifest(B.DEFAULT_MANIFEST)
        for a in assets:
            if a["name"].startswith("project/"):
                self.assertNotRegex(a["name"].lower(), r"adobe|photoshop|illustrator|indesign|premiere\b")


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


def private_entry(name, data=None, **kw):
    e = entry(name, data, url=None, source_kind="private", license="Proprietary", license_url=None,
              source_page=None, attribution="Rights holder, all rights reserved",
              fallback_url="https://example.org/official/" + name.replace("/", "_"),
              value="Real application project file; no open-licence file exercises this reader path.")
    e.update(kw)
    return e


def mirror_entry(name, data=None, **kw):
    e = entry(name, data, source_kind="public-mirror", url="https://commons.example/" + name.replace("/", "_"),
              mirror_path="test-assets/20261116/" + name)
    e.update(kw)
    return e


def synth_entry(name, generator, output, **kw):
    e = {"name": name, "generator": generator, "output": output, "description": "unit test file",
         "requires": [], "sha256": None, "size": None, "sha256_prefix": None, "prefix_bytes": None}
    e.update(kw)
    return e


def with_synth(m, *synth):
    m = dict(m)
    m["synthetic"] = list(synth)
    return m


class _Bad(unittest.TestCase):
    def bad(self, m, text):
        with self.assertRaises(B.AssetError) as cm:
            B.validate_manifest(m)
        self.assertIn(text, str(cm.exception))


class SourceKind(_Bad):
    def test_source_kind_is_required_and_closed(self):
        e = entry("x")
        del e["source_kind"]
        self.bad(manifest(e), "source_kind is required")
        for v in ("mirror", "ORIGIN", "", None, 3):
            with self.subTest(v=v):
                self.bad(manifest(entry("x", source_kind=v)), "source_kind must be one of")

    def test_origin_takes_no_kind_specific_keys(self):
        for k, v in (("mirror_path", "test-assets/20261116/x"), ("fallback_url", "https://example.org/y"),
                     ("value", "a reason that is long enough to count")):
            with self.subTest(k=k):
                self.bad(manifest(entry("x", **{k: v})), "keys must be exactly")

    def test_origin_url_must_be_stable(self):
        for u in ("https://commons.wikimedia.org/wiki/Special:FilePath/X.jpg",
                  "https://upload.wikimedia.org/wikipedia/commons/a/a1/X.jpg", "https://en.wikipedia.org/x"):
            with self.subTest(u=u):
                self.bad(manifest(entry("x", url=u)), "public-mirror")
        for u in ("https://raw.githubusercontent.com/o/r/main/x.psd",
                  "https://media.githubusercontent.com/media/o/r/refs/heads/main/x.psd"):
            with self.subTest(u=u):
                self.bad(manifest(entry("x", url=u)), "40-hex commit")
        c = "0123456789abcdef0123456789abcdef01234567"
        B.validate_manifest(manifest(entry("x", url=f"https://raw.githubusercontent.com/o/r/{c}/x.psd")))
        B.validate_manifest(manifest(entry("x", url="https://download.blender.org/a/b.mp4")))

    def test_origin_needs_an_https_url(self):
        self.bad(manifest(entry("x", url=None)), "https URL")

    def test_public_mirror_rules(self):
        B.validate_manifest(manifest(mirror_entry("photo/x.jpg", b"1")))
        self.bad(manifest(mirror_entry("x.jpg", mirror_path=None)), "mirror_path must be")
        for mp in ("x.jpg", "test-assets/x.jpg", "test-assets/2026-11-16/x.jpg", "test-assets/20261116/y.jpg",
                   "/test-assets/20261116/x.jpg", "test-assets/20261116/../x.jpg", 5):
            with self.subTest(mp=mp):
                self.bad(manifest(mirror_entry("x.jpg", mirror_path=mp)), "mirror_path must be")
        e = mirror_entry("x.jpg")
        del e["mirror_path"]
        self.bad(manifest(e), "mirror_path must be")
        self.bad(manifest(mirror_entry("x.jpg", value="long enough text to be a value")), "keys must be exactly")
        # a public mirror may name any https origin, including an unstable one
        B.validate_manifest(manifest(mirror_entry("x.jpg",
                                                  url="https://commons.wikimedia.org/wiki/Special:FilePath/X.jpg")))

    def test_private_minimal_and_url_null(self):
        a = B.validate_manifest(manifest(private_entry("p/doc.psd", b"secret")))
        self.assertIsNone(a[0]["url"])
        B.validate_manifest(manifest(private_entry("p/doc.psd", url="https://example.org/private/doc.psd")))
        e = private_entry("p/doc.psd")
        del e["fallback_url"]
        B.validate_manifest(manifest(e))

    def test_private_needs_value_licence_and_https_fallback(self):
        e = private_entry("p/doc.psd")
        del e["value"]
        self.bad(manifest(e), "needs a 'value' text")
        for v in ("", "short", "   ", None, 5):
            with self.subTest(v=v):
                self.bad(manifest(private_entry("p/doc.psd", value=v)), "needs a 'value' text")
        self.bad(manifest(private_entry("p/doc.psd", license="CC0-1.0")), "licence 'Proprietary'")
        self.bad(manifest(private_entry("p/doc.psd", share_alike=True)), "share_alike false")
        self.bad(manifest(private_entry("p/doc.psd", fallback_url="http://example.org/x")), "fallback_url")
        self.bad(manifest(private_entry("p/doc.psd", url="http://example.org/x")), "https URL")
        self.bad(manifest(private_entry("p/doc.psd", mirror_path="test-assets/20261116/p/doc.psd")),
                 "keys must be exactly")

    def test_proprietary_is_not_allowed_for_public_entries(self):
        self.bad(manifest(entry("x", license="Proprietary")), "not allowed")

    def test_private_sizes_do_not_count_towards_the_bundle(self):
        big = [entry(f"f{i}", sha256=sha(bytes([i])), size=B.MAX_FILE) for i in range(2)]
        priv = private_entry("p/big.psd", sha256=sha(b"p"), size=B.MAX_FILE * 3)
        B.validate_manifest(manifest(*big, priv))
        self.bad(manifest(private_entry("p/big.psd", sha256=sha(b"p"), size=B.PRIVATE_MAX_FILE + 1)),
                 "per-file limit")

    def test_shipped_manifest_classification(self):
        m, assets = B.load_manifest(B.DEFAULT_MANIFEST)
        kinds = {k: [a for a in assets if a["source_kind"] == k] for k in B.SOURCE_KINDS}
        self.assertEqual(sum(len(v) for v in kinds.values()), len(assets))
        self.assertGreaterEqual(len(kinds["public-mirror"]), 7)
        for a in assets:
            host = B._host(a["url"]) if a["url"] else ""
            if host.endswith("wikimedia.org"):
                self.assertEqual(a["source_kind"], "public-mirror", a["name"])
        for a in kinds["public-mirror"]:
            self.assertEqual(a["mirror_path"], "test-assets/20261116/" + a["name"])
            self.assertIsNotNone(a["sha256"])
        for a in kinds["private"]:
            self.assertTrue(a["value"].strip())
        self.assertTrue(all(a["sha256"] for a in assets))
        self.assertEqual(m["schema"], B.SCHEMA)


class ShareAlike(Fixture):
    def test_sa_licences_are_allowed_with_flag_and_attribution(self):
        for lic in ("CC-BY-SA-3.0", "CC-BY-SA-4.0"):
            self.assertIn(lic, B.ALLOWED_LICENSES)
            B.validate_manifest(manifest(entry("x", license=lic, share_alike=True)))
            with self.assertRaises(B.AssetError):
                B.validate_manifest(manifest(entry("x", license=lic, share_alike=True, attribution=" ")))
            with self.assertRaises(B.AssetError):
                B.validate_manifest(manifest(entry("x", license=lic, share_alike=False)))
        for lic in ("CC-BY-NC-SA-4.0", "CC-BY-NC-4.0", "CC-BY-ND-4.0", "CC-BY-SA-2.0", "unknown"):
            with self.assertRaises(B.AssetError):
                B.validate_manifest(manifest(entry("x", license=lic, share_alike="-SA-" in lic)))

    def test_licenses_file_marks_share_alike_and_forbids_publishing_renders(self):
        m = manifest(
            entry("a.bin", self.files["a.bin"], license="CC-BY-SA-4.0", share_alike=True,
                  attribution="Ann (CC BY-SA 4.0)"),
            entry("sub/b.txt", self.files["sub/b.txt"]),
            entry("sub/c.dat", self.files["sub/c.dat"], license="CC-BY-SA-3.0", share_alike=True))
        self.mpath.write_text(json.dumps(m))
        self.assertEqual(self.run_main("licenses", "--dir", self.dir)[0], 0)
        text = (self.dir / "LICENSES.txt").read_text()
        self.assertEqual(text.count("[SHARE-ALIKE]"), 2)
        self.assertIn("File:         a.bin   [SHARE-ALIKE]", text)
        self.assertIn("File:         sub/b.txt\n", text)
        self.assertIn("derived outputs or evidence", text)
        self.assertIn("must\nnot be published", text)
        self.assertIn("Share-alike:  YES (SHARE-ALIKE)", text)


class MirrorFetch(Fixture):
    ORIGIN = "https://commons.example/photo_x.jpg"
    MIRROR = "https://bundles.example/test-assets/20261116/photo/x.jpg"

    class Net:
        """Fake opener: maps URL -> bytes, or an int HTTP status; records every request."""

        def __init__(self, table):
            self.table, self.seen = table, []

        def open(self, req, timeout=None):
            url = req.full_url
            self.seen.append(url)
            v = self.table.get(url, 404)
            if isinstance(v, int):
                raise urllib.error.HTTPError(url, v, "err", {}, io.BytesIO(b""))
            return FakeResponse(v, {"Content-Length": str(len(v))})

    def setUp(self):
        super().setUp()
        self.data = b"mirror payload " * 50
        self.mpath.write_text(json.dumps(manifest(mirror_entry("photo/x.jpg", self.data))))
        self.dest = self.tmp / "dest"

    def fetch(self, net, mirror_base="https://bundles.example"):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            B.cmd_fetch(self.mpath, self.dest, opener=net, mirror_base=mirror_base)
        return out.getvalue()

    def test_origin_first_and_mirror_untouched(self):
        net = self.Net({self.ORIGIN: self.data, self.MIRROR: b"x"})
        self.fetch(net)
        self.assertEqual(net.seen, [self.ORIGIN])
        self.assertEqual((self.dest / "photo/x.jpg").read_bytes(), self.data)

    def test_mirror_used_when_origin_fails(self):
        for origin in (404, 503, b"wrong bytes of another length", self.data[:-1] + b"X"):
            with self.subTest(origin=origin if isinstance(origin, int) else len(origin)):
                shutil.rmtree(self.dest, ignore_errors=True)
                net = self.Net({self.ORIGIN: origin, self.MIRROR: self.data})
                with mock.patch.object(B.time, "sleep"):
                    out = self.fetch(net)
                self.assertEqual(net.seen[-1], self.MIRROR)
                self.assertIn("trying the public mirror", out)
                self.assertEqual((self.dest / "photo/x.jpg").read_bytes(), self.data)

    def test_mirror_bytes_are_checked_against_the_pin(self):
        net = self.Net({self.ORIGIN: 404, self.MIRROR: self.data[:-1] + b"X"})
        with self.assertRaises(B.AssetError) as cm:
            self.fetch(net)
        self.assertIn("origin and mirror both failed", str(cm.exception))
        self.assertFalse((self.dest / "photo/x.jpg").exists())
        self.assertFalse((self.dest / "photo/x.jpg.part").exists())

    def test_no_mirror_base_means_origin_only(self):
        net = self.Net({self.ORIGIN: 404, self.MIRROR: self.data})
        with self.assertRaises(B.AssetError) as cm:
            self.fetch(net, mirror_base=None)
        self.assertIn("no --mirror-base", str(cm.exception))
        self.assertEqual(net.seen, [self.ORIGIN])

    def test_origin_entries_never_use_the_mirror(self):
        self.mpath.write_text(json.dumps(manifest(entry("photo/x.jpg", self.data, url=self.ORIGIN))))
        net = self.Net({self.ORIGIN: 404, self.MIRROR: self.data})
        with self.assertRaises(B.AssetError):
            self.fetch(net)
        self.assertEqual(net.seen, [self.ORIGIN])

    def test_mirror_base_must_be_https_and_path_is_quoted(self):
        with self.assertRaises(B.AssetError):
            B.mirror_url("http://bundles.example", "test-assets/20261116/x")
        self.assertEqual(B.mirror_url("https://bundles.example/", "test-assets/20261116/a b.jpg"),
                         "https://bundles.example/test-assets/20261116/a%20b.jpg")


class PrivateEntries(Fixture):
    def setUp(self):
        super().setUp()
        self.secret = b"PRIVATE-BYTES-" * 100
        self.m = manifest(*[entry(n, d) for n, d in self.files.items()],
                          private_entry("priv/doc.psd", self.secret))
        self.mpath.write_text(json.dumps(self.m))
        self.pdir = self.tmp / "private"
        (self.pdir / "priv").mkdir(parents=True)
        (self.pdir / "priv/doc.psd").write_bytes(self.secret)

    def test_fetch_skips_private_by_default(self):
        rc, out, _ = self.run_main("fetch", "--dest", self.dir)
        self.assertEqual(rc, 0)
        self.assertIn("skip  1 private entry", out)
        self.assertFalse((self.dir / "priv").exists())

    def test_include_private_verifies_in_place_and_never_copies(self):
        rc, out, _ = self.run_main("fetch", "--dest", self.dir, "--include-private", "--private-dir", self.pdir)
        self.assertEqual(rc, 0, out)
        self.assertIn("private, present and pin verified", out)
        self.assertFalse((self.dir / "priv").exists())

    def test_include_private_failures(self):
        rc, _, err = self.run_main("fetch", "--dest", self.dir, "--include-private")
        self.assertEqual((rc, "--private-dir" in err), (1, True))
        rc, _, err = self.run_main("fetch", "--dest", self.dir, "--private-dir", self.pdir)
        self.assertEqual(rc, 1)
        (self.pdir / "priv/doc.psd").write_bytes(self.secret + b"x")
        rc, _, err = self.run_main("fetch", "--dest", self.dir, "--include-private", "--private-dir", self.pdir)
        self.assertEqual(rc, 1)
        self.assertIn("differs from the pin", err)
        (self.pdir / "priv/doc.psd").unlink()
        rc, out, err = self.run_main("fetch", "--dest", self.dir, "--include-private", "--private-dir", self.pdir)
        self.assertEqual(rc, 1)
        self.assertIn("MISSING private priv/doc.psd", out)
        rc, _, err = self.run_main("fetch", "--dest", self.dir, "--include-private",
                                   "--private-dir", self.dir / "inner")
        self.assertEqual(rc, 1)
        self.assertIn("separate directory", err)

    def test_private_url_is_never_requested(self):
        net = MirrorFetch.Net({})
        with contextlib.redirect_stdout(io.StringIO()):
            B.cmd_fetch(self.mpath, self.dir, opener=net)
        self.assertEqual(net.seen, [])

    def test_verify_handles_every_kind(self):
        self.assertEqual(self.run_main("verify", "--dir", self.dir)[0], 0)           # absent: fine
        rc, _, err = self.run_main("verify", "--dir", self.dir, "--require-private")
        self.assertEqual(rc, 1)
        self.assertIn("priv/doc.psd: private file missing", err)
        (self.dir / "priv").mkdir()
        (self.dir / "priv/doc.psd").write_bytes(self.secret)                          # present: checked
        self.assertEqual(self.run_main("verify", "--dir", self.dir, "--require-private")[0], 0)
        (self.dir / "priv/doc.psd").write_bytes(b"tampered")
        rc, _, err = self.run_main("verify", "--dir", self.dir)
        self.assertEqual(rc, 1)
        self.assertIn("priv/doc.psd", err)

    def test_private_never_enters_licenses_manifest_or_iso(self):
        (self.dir / "priv").mkdir()
        (self.dir / "priv/doc.psd").write_bytes(self.secret)       # even when the file sits in the directory
        self.assertEqual(self.run_main("licenses", "--dir", self.dir)[0], 0)
        for f in ("LICENSES.txt", "MANIFEST.json"):
            text = (self.dir / f).read_text()
            for needle in ("priv/doc.psd", "Rights holder", "Proprietary", "no open-licence file", "official/"):
                self.assertNotIn(needle, text, f)
        self.assertEqual([a["name"] for a in json.loads((self.dir / "MANIFEST.json").read_text())["assets"]],
                         list(self.files))
        out = self.tmp / "p.iso"
        self.assertEqual(self.run_main("iso", "--dir", self.dir, "--out", out, "--engine", "python")[0], 0)
        got = read_joliet(out.read_bytes())
        self.assertNotIn("priv/doc.psd", got)
        self.assertNotIn(b"PRIVATE-BYTES", out.read_bytes())
        self.assertEqual(set(got) - {"__label__"}, set(self.files) | {"MANIFEST.json", "LICENSES.txt"})

    def test_pin_fills_a_private_entry_from_its_directory(self):
        self.mpath.write_text(json.dumps(manifest(private_entry("priv/doc.psd"))))
        self.assertEqual(self.run_main("pin", "--dir", self.pdir)[0], 0)
        a = json.loads(self.mpath.read_text())["assets"][0]
        self.assertEqual((a["sha256"], a["size"]), (sha(self.secret), len(self.secret)))


class SyntheticSchema(_Bad):
    def test_valid_and_pin_shapes(self):
        base = manifest(entry("x"))
        B.validate_manifest(with_synth(base, synth_entry("t", "wav_sine_sweep", "synthetic/t.wav")))
        B.validate_manifest(with_synth(base, synth_entry("t", "wav_sine_sweep", "synthetic/t.wav",
                                                         sha256=sha(b"1"), size=3)))
        B.validate_manifest(with_synth(base, synth_entry("t", ["ffmpeg", "-i", "x", "{output}"], "t.mp4",
                                                         requires=["ffmpeg"], sha256_prefix=sha(b"1"),
                                                         prefix_bytes=10)))
        for kw in ({"sha256": sha(b"1")}, {"size": 3}, {"sha256_prefix": sha(b"1")}, {"prefix_bytes": 3},
                   {"sha256": sha(b"1"), "size": 3, "sha256_prefix": sha(b"1"), "prefix_bytes": 3},
                   {"sha256": "zz", "size": 3}, {"sha256": sha(b"1"), "size": 0}):
            with self.subTest(kw=kw):
                self.bad(with_synth(base, synth_entry("t", "wav_sine_sweep", "t.wav", **kw)), "synthetic[0]")

    def test_generator_and_requires_rules(self):
        base = manifest(entry("x"))
        for gen, req in (("nope", []), ([], []), (["ffmpeg", "x"], ["ffmpeg"]),
                         (["ffmpeg", "{output}", "{output}"], ["ffmpeg"]), (["ffmpeg", "{output}"], []),
                         ([1, "{output}"], []), (5, [])):
            with self.subTest(gen=gen):
                self.bad(with_synth(base, synth_entry("t", gen, "t.bin", requires=req)), "synthetic[0]")

    def test_names_outputs_and_keys(self):
        base = manifest(entry("x"))
        s1, s2 = synth_entry("a", "wav_sine_sweep", "a.wav"), synth_entry("a", "wav_sine_sweep", "b.wav")
        self.bad(with_synth(base, s1, s2), "duplicate synthetic name")
        self.bad(with_synth(base, s1, synth_entry("b", "wav_sine_sweep", "A.WAV")), "duplicate synthetic output")
        self.bad(with_synth(base, synth_entry("a", "wav_sine_sweep", "x")), "collides with an asset name")
        self.bad(with_synth(base, synth_entry("a", "wav_sine_sweep", "../a.wav")), "unsafe asset name")
        e = synth_entry("a", "wav_sine_sweep", "a.wav")
        del e["description"]
        self.bad(with_synth(base, e), "keys must be exactly")
        self.bad(with_synth(base, "x"), "not an object")
        m = dict(base)
        m["synthetic"] = "x"
        self.bad(m, "must be a list")

    def test_shipped_synthetic_section(self):
        m, _ = B.load_manifest(B.DEFAULT_MANIFEST)
        syn = {s["name"]: s for s in m["synthetic"]}
        self.assertEqual(set(syn), {"png_gradient_8k", "tiff_16bit_big", "wav_sine_sweep", "uhd_testsrc2_10s"})
        for n in ("png_gradient_8k", "tiff_16bit_big", "wav_sine_sweep"):
            self.assertIsInstance(syn[n]["generator"], str)
            self.assertEqual(syn[n]["requires"], [])
            self.assertIsNotNone(syn[n]["sha256"], n)
        v = syn["uhd_testsrc2_10s"]
        self.assertEqual(v["requires"], ["ffmpeg"])
        self.assertEqual(
            " ".join(v["generator"][:-1]),
            "ffmpeg -f lavfi -i testsrc2=size=3840x2160:rate=25:duration=10 -c:v libx264 -preset veryfast "
            "-crf 18 -pix_fmt yuv420p -bitexact -fflags +bitexact -flags:v +bitexact")
        self.assertEqual(v["generator"][-1], "{output}")
        self.assertTrue(v["output"].endswith("uhd_testsrc2_10s.mp4"))
        self.assertIsNone(v["sha256"])  # prefix pin only; may still be unpinned until generated with ffmpeg
        self.assertEqual((v["sha256_prefix"] is None), (v["prefix_bytes"] is None))


def png_decode(data: bytes):
    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    pos, idat, tags, ihdr = 8, b"", [], None
    while pos < len(data):
        n = struct.unpack(">I", data[pos:pos + 4])[0]
        tag, body = data[pos + 4:pos + 8], data[pos + 8:pos + 8 + n]
        assert zlib.crc32(tag + body) & 0xFFFFFFFF == struct.unpack(">I", data[pos + 8 + n:pos + 12 + n])[0]
        if tag == b"IHDR":
            ihdr = struct.unpack(">IIBBBBB", body)
        if tag == b"IDAT":
            idat += body
        tags.append(tag)
        pos += 12 + n
    return ihdr, tags, zlib.decompress(idat)


class Generators(unittest.TestCase):
    def setUp(self):
        self._td = tempfile.TemporaryDirectory()
        self.tmp = Path(self._td.name)
        self.addCleanup(self._td.cleanup)

    def twice(self, fn, name, *a, **kw):
        p1, p2 = self.tmp / ("1" + name), self.tmp / ("2" + name)
        fn(p1, *a, **kw)
        fn(p2, *a, **kw)
        self.assertEqual(p1.read_bytes(), p2.read_bytes())
        return p1.read_bytes()

    def test_png_is_valid_and_deterministic(self):
        data = self.twice(B.gen_png_gradient, "g.png", 300, 200)
        ihdr, tags, raw = png_decode(data)
        self.assertEqual(ihdr, (300, 200, 8, 2, 0, 0, 0))
        self.assertEqual((tags[0], tags[-1]), (b"IHDR", b"IEND"))
        self.assertEqual(len(raw), 200 * (1 + 900))
        row = lambda y: raw[y * 901:(y + 1) * 901]  # noqa: E731
        self.assertEqual(row(0)[0], 0)
        self.assertEqual(row(0)[1:4][:2], b"\x00\x00")                    # red 0, green 0 at the corner
        self.assertEqual(row(199)[1 + 3 * 299:][:2], b"\xff\xff")          # red 255, green 255 at the far corner
        self.assertNotEqual(row(0)[3::3], row(16)[3::3])                  # blue pattern changes by 16-row block

    def test_png_does_not_depend_on_idat_or_block_sizes_of_the_sink(self):
        a = self.twice(B.gen_png_gradient, "a.png", 300, 200)
        with mock.patch.object(B, "IDAT_SIZE", 4096):
            b = self.twice(B.gen_png_gradient, "b.png", 300, 200)
        self.assertGreater(png_decode(b)[1].count(b"IDAT"), png_decode(a)[1].count(b"IDAT"))
        self.assertEqual(png_decode(a)[2], png_decode(b)[2])

    def test_png_uses_no_compressor(self):
        with mock.patch.object(zlib, "compressobj", side_effect=AssertionError), \
                mock.patch.object(zlib, "compress", side_effect=AssertionError):
            self.twice(B.gen_png_gradient, "g.png", 64, 48)

    def test_tiff_structure(self):
        w, h = 70, 40
        data = self.twice(B.gen_tiff_16bit, "g.tif", w, h)
        self.assertEqual(data[:4], b"II*\x00")
        ifd = struct.unpack("<I", data[4:8])[0]
        n = struct.unpack("<H", data[ifd:ifd + 2])[0]
        tags = {}
        for i in range(n):
            tag, typ, cnt, val = struct.unpack("<HHII", data[ifd + 2 + 12 * i:ifd + 14 + 12 * i])
            tags[tag] = (typ, cnt, val)
        self.assertEqual((tags[256][2], tags[257][2], tags[259][2], tags[262][2], tags[277][2]), (w, h, 1, 2, 3))
        self.assertEqual(struct.unpack("<HHH", data[tags[258][2]:tags[258][2] + 6]), (16, 16, 16))
        off, cnt = tags[273][2], tags[279][2]
        self.assertEqual(cnt, w * h * 6)
        self.assertEqual(len(data), off + cnt)
        px = struct.unpack("<3H", data[off:off + 6])
        self.assertEqual(px[:2], (0, 0))
        last = struct.unpack("<3H", data[len(data) - 6:])
        self.assertEqual(last[:2], (65535, 65535))

    def test_sine_table_matches_libm_without_using_it(self):
        import math
        t = B._sine_table()
        self.assertEqual((len(t), t[0], t[256], t[512], t[768], t[1024]), (1025, 0, 1 << 24, 0, -(1 << 24), 0))
        for k in range(1025):
            self.assertLessEqual(abs(t[k] - round(math.sin(2 * math.pi * k / 1024) * (1 << 24))), 1)

    def test_wav_sweep(self):
        data = self.twice(B.gen_wav_sine_sweep, "s.wav", 2, 8000, 100, 1500)
        p = self.tmp / "1s.wav"
        with wave.open(str(p)) as w:
            self.assertEqual((w.getnchannels(), w.getsampwidth(), w.getframerate(), w.getnframes()),
                             (2, 2, 8000, 16000))
            frames = w.readframes(w.getnframes())
        self.assertEqual(len(data), 44 + len(frames))
        import array
        a = array.array("h")
        a.frombytes(frames)
        self.assertEqual(min(a), -16384)
        self.assertGreaterEqual(max(a), 16380)
        self.assertLessEqual(max(a), 16384)
        left, right = a[0::2], a[1::2]

        def crossings(x):
            return sum(1 for i in range(1, len(x)) if (x[i - 1] < 0) != (x[i] < 0))
        self.assertLess(crossings(left[:2000]), crossings(left[-2000:]))      # left rises
        self.assertGreater(crossings(right[:2000]), crossings(right[-2000:]))  # right falls

    def test_builtin_names_are_the_manifest_names(self):
        self.assertEqual(set(B.BUILTIN_GENERATORS), {"png_gradient_8k", "tiff_16bit_big", "wav_sine_sweep"})


class SyntheticCommands(Fixture):
    """synth / pin-synth / verify / iso with small registered generators and a fake ffmpeg."""

    def setUp(self):
        super().setUp()
        patcher = mock.patch.dict(B.BUILTIN_GENERATORS, {"tiny": self.tiny})
        patcher.start()
        self.addCleanup(patcher.stop)
        self.calls = 0
        self.fake_bin = self.tmp / "bin"
        self.fake_bin.mkdir()
        tool = self.fake_bin / "ffmpeg"
        tool.write_text('#!/bin/sh\nfor a; do last="$a"; done\nprintf "FAKE-MP4-HEADER-0123456789" > "$last"\n'
                        'printf "%s" "$*" >> "$last"\n')
        tool.chmod(0o755)
        self.ff = ["ffmpeg", "-f", "lavfi", "-bitexact", "{output}"]

    def tiny(self, path):
        self.calls += 1
        Path(path).write_bytes(b"tiny-synthetic-" * 40)

    def write(self, *synth):
        self.mpath.write_text(json.dumps(with_synth(self.m, *synth)))

    def test_synth_pins_must_match_and_unpinned_needs_the_flag(self):
        good = synth_entry("t", "tiny", "synthetic/t.bin", sha256=sha(b"tiny-synthetic-" * 40), size=600)
        self.write(good)
        d = self.tmp / "out"
        rc, out, _ = self.run_main("synth", "--dest", d)
        self.assertEqual(rc, 0, out)
        self.assertEqual((d / "synthetic/t.bin").read_bytes(), b"tiny-synthetic-" * 40)
        self.assertEqual(list(d.iterdir()), [d / "synthetic"])               # no temp directory is left
        self.assertEqual(list((d / "synthetic").iterdir()), [d / "synthetic/t.bin"])
        n = self.calls
        rc, out, _ = self.run_main("synth", "--dest", d)                     # present and verified: not rebuilt
        self.assertEqual((rc, self.calls, "already present" in out), (0, n, True))
        (d / "synthetic/t.bin").write_bytes(b"tampered")                      # wrong file: regenerated
        self.assertEqual(self.run_main("synth", "--dest", d)[0], 0)
        self.assertEqual(self.calls, n + 1)
        self.write(dict(good, sha256=sha(b"other")))                          # wrong pin: fail and remove
        rc, _, err = self.run_main("synth", "--dest", d)
        self.assertEqual(rc, 1)
        self.assertIn("sha256", err)
        self.assertFalse((d / "synthetic/t.bin").exists())
        self.write(dict(good, size=601))
        self.assertEqual(self.run_main("synth", "--dest", d)[0], 1)
        self.write(synth_entry("t", "tiny", "synthetic/t.bin"))               # unpinned
        self.assertEqual(self.run_main("synth", "--dest", d)[0], 1)
        rc, out, _ = self.run_main("synth", "--dest", d, "--allow-unpinned")
        self.assertEqual(rc, 0)
        self.assertIn("unpinned; generated", out)

    def test_only_and_unknown_name(self):
        self.write(synth_entry("t", "tiny", "synthetic/t.bin"), synth_entry("u", "tiny", "synthetic/u.bin"))
        d = self.tmp / "out"
        self.assertEqual(self.run_main("synth", "--dest", d, "--only", "u", "--allow-unpinned")[0], 0)
        self.assertEqual(sorted(p.name for p in (d / "synthetic").iterdir()), ["u.bin"])
        rc, _, err = self.run_main("synth", "--dest", d, "--only", "zz")
        self.assertEqual((rc, "unknown synthetic" in err), (1, True))

    def test_pin_synth_fills_full_and_prefix_pins_and_never_overwrites(self):
        self.write(synth_entry("t", "tiny", "synthetic/t.bin"),
                   synth_entry("v", self.ff, "synthetic/v.mp4", requires=["ffmpeg"]))
        d = self.tmp / "out"
        with mock.patch.dict(os.environ, {"PATH": str(self.fake_bin) + os.pathsep + os.environ["PATH"]}):
            self.assertEqual(self.run_main("synth", "--dest", d, "--allow-unpinned")[0], 0)
            self.assertEqual(self.run_main("pin-synth", "--dir", d)[0], 0)
            syn = {s["name"]: s for s in json.loads(self.mpath.read_text())["synthetic"]}
            self.assertEqual((syn["t"]["sha256"], syn["t"]["size"]), (sha(b"tiny-synthetic-" * 40), 600))
            v = (d / "synthetic/v.mp4").read_bytes()
            self.assertIsNone(syn["v"]["sha256"])
            self.assertEqual(syn["v"]["prefix_bytes"], min(B.PREFIX_BYTES_DEFAULT, len(v)))
            self.assertEqual(syn["v"]["sha256_prefix"], sha(v[:syn["v"]["prefix_bytes"]]))
            self.assertEqual(self.run_main("synth", "--dest", d)[0], 0)       # now verified by the pins
            (d / "synthetic/t.bin").write_bytes(b"changed")
            before = self.mpath.read_text()
            rc, _, err = self.run_main("pin-synth", "--dir", d)
            self.assertEqual((rc, "refusing to overwrite" in err, self.mpath.read_text()), (1, True, before))

    def test_ffmpeg_entry_skipped_when_the_tool_is_absent(self):
        self.write(synth_entry("v", self.ff, "synthetic/v.mp4", requires=["ffmpeg"]))
        d = self.tmp / "out"
        with mock.patch.dict(os.environ, {"PATH": str(self.tmp / "empty")}):
            rc, out, _ = self.run_main("synth", "--dest", d)
            self.assertEqual(rc, 0)
            self.assertIn("needs ffmpeg", out)
            self.assertIn("not a failure", out)
            self.assertFalse((d / "synthetic").exists())
            self.assertEqual(self.run_main("pin-synth", "--dir", d)[0], 0)
            self.assertIn("still unpinned", self.run_main("pin-synth", "--dir", d)[1])

    def test_ffmpeg_prefix_pin_checks_only_the_prefix(self):
        pre = b"FAKE-MP4-HEADER-0123456789"
        self.write(synth_entry("v", self.ff, "synthetic/v.mp4", requires=["ffmpeg"],
                               sha256_prefix=sha(pre), prefix_bytes=len(pre)))
        d = self.tmp / "out"
        with mock.patch.dict(os.environ, {"PATH": str(self.fake_bin) + os.pathsep + os.environ["PATH"]}):
            self.assertEqual(self.run_main("synth", "--dest", d)[0], 0)
            self.write(synth_entry("v", self.ff, "synthetic/v.mp4", requires=["ffmpeg"],
                                   sha256_prefix=sha(b"FAKE-MP4-HEADER-9999999999"), prefix_bytes=len(pre)))
            _rm = (d / "synthetic/v.mp4")
            rc, _, err = self.run_main("synth", "--dest", d)
            self.assertEqual((rc, "first 26 bytes" in err, _rm.exists()), (1, True, False))
            self.write(synth_entry("v", self.ff, "synthetic/v.mp4", requires=["ffmpeg"],
                                   sha256_prefix=sha(pre), prefix_bytes=10 ** 6))
            rc, _, err = self.run_main("synth", "--dest", d)
            self.assertEqual((rc, "shorter than the pinned prefix" in err), (1, True))

    def test_failing_tool_is_reported(self):
        bad = self.fake_bin / "ffmpeg"
        bad.write_text("#!/bin/sh\necho boom >&2\nexit 3\n")
        self.write(synth_entry("v", self.ff, "synthetic/v.mp4", requires=["ffmpeg"]))
        with mock.patch.dict(os.environ, {"PATH": str(self.fake_bin) + os.pathsep + os.environ["PATH"]}):
            rc, _, err = self.run_main("synth", "--dest", self.tmp / "o", "--allow-unpinned")
        self.assertEqual(rc, 1)
        self.assertIn("boom", err)

    def test_verify_with_and_without_synthetic(self):
        self.write(synth_entry("t", "tiny", "synthetic/t.bin", sha256=sha(b"tiny-synthetic-" * 40), size=600))
        self.assertEqual(self.run_main("verify", "--dir", self.dir)[0], 0)                  # absent: fine
        rc, _, err = self.run_main("verify", "--dir", self.dir, "--with-synthetic")
        self.assertEqual((rc, "synthetic file missing" in err), (1, True))
        (self.dir / "synthetic").mkdir()
        (self.dir / "synthetic/t.bin").write_bytes(b"tiny-synthetic-" * 40)
        self.assertEqual(self.run_main("verify", "--dir", self.dir, "--with-synthetic")[0], 0)
        (self.dir / "synthetic/t.bin").write_bytes(b"tiny-synthetic-" * 39 + b"x" * 15)
        self.assertEqual(self.run_main("verify", "--dir", self.dir)[0], 1)                  # checked when present

    def test_iso_and_licenses_include_synthetic_only_on_request(self):
        self.write(synth_entry("t", "tiny", "synthetic/t.bin"))
        self.assertEqual(self.run_main("synth", "--dest", self.dir, "--allow-unpinned")[0], 0)
        self.assertEqual(self.run_main("pin-synth", "--dir", self.dir)[0], 0)
        plain, withs = self.tmp / "plain.iso", self.tmp / "with.iso"
        self.assertEqual(self.run_main("iso", "--dir", self.dir, "--out", plain, "--engine", "python")[0], 0)
        self.assertNotIn("synthetic/t.bin", read_joliet(plain.read_bytes()))
        self.assertNotIn("synthetic", (self.dir / "MANIFEST.json").read_text())
        self.assertEqual(self.run_main("iso", "--dir", self.dir, "--out", withs, "--engine", "python",
                                       "--with-synthetic")[0], 0)
        self.assertEqual(read_joliet(withs.read_bytes())["synthetic/t.bin"], b"tiny-synthetic-" * 40)
        self.assertIn("synthetic/t.bin", (self.dir / "LICENSES.txt").read_text())
        self.assertIn("synthetic", json.loads((self.dir / "MANIFEST.json").read_text()))

    def test_unpinned_synthetic_file_in_the_directory_fails_verify(self):
        self.write(synth_entry("t", "tiny", "synthetic/t.bin"))
        (self.dir / "synthetic").mkdir()
        (self.dir / "synthetic/t.bin").write_bytes(b"x")
        self.assertIn("not pinned", self.run_main("verify", "--dir", self.dir)[2])


class ShippedSyntheticPins(unittest.TestCase):
    """The real generators, run twice into separate directories; the pins in the manifest must hold."""

    def test_builtin_pins_hold_on_two_runs(self):
        m, _ = B.load_manifest(B.DEFAULT_MANIFEST)
        builtin = [s for s in m["synthetic"] if isinstance(s["generator"], str)]
        self.assertEqual(len(builtin), 3)
        hashes = []
        for run in range(2):
            with tempfile.TemporaryDirectory() as td:
                small = {"schema": B.SCHEMA, "assets": [entry("a.txt", b"a")], "synthetic": builtin}
                mp = Path(td) / "m.json"
                mp.write_text(json.dumps(small))
                out, err = io.StringIO(), io.StringIO()
                with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                    rc = B.cmd_synth(mp, Path(td) / "d")
                self.assertEqual(rc, 0, err.getvalue())
                hashes.append({s["output"]: B.sha256_file(Path(td) / "d" / s["output"]) for s in builtin})
        self.assertEqual(hashes[0], hashes[1])
        for s in builtin:
            self.assertEqual(hashes[0][s["output"]], (s["sha256"], s["size"]), s["name"])



if __name__ == "__main__":
    unittest.main()
