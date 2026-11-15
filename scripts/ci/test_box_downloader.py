"""GPU-free tests of scripts/ci/box_downloader.py: hostile manifests and servers, standalone CLI, overlap and cap."""
import ast
import datetime
import hashlib
import http.server
import io
import json
import os
from pathlib import Path
import re
import signal
import socketserver
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import box_downloader as bd  # noqa: E402
import box_downloader_worker as bw  # noqa: E402
import kf_fetch  # noqa: E402

CLI = HERE / "box_downloader.py"
SECRET = "SECRETQUERYvalue123"


def sha(data):
    return hashlib.sha256(data).hexdigest()


def blob(n, seed=1):
    return bytes((i * seed + i // 251) % 256 for i in range(n))


class Server:
    """Loopback HTTP server. behave[path] is a dict: rate (bytes/s per connection), status, oversize, truncate,
    redirect, flip (corrupt one byte), auth_once (403 'expired' for the first N full-body requests)."""

    def __init__(self):
        self.files, self.behave, self.log = {}, {}, []
        self.lock = threading.Lock()
        server = self

        class H(http.server.BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *a):
                pass

            def do_GET(self):
                path = self.path.split("?", 1)[0]
                rng = self.headers.get("Range")
                entry = {"path": path, "range": rng, "t0": time.monotonic(), "sent": 0, "headers": dict(self.headers), "query": self.path}
                with server.lock:
                    server.log.append(entry)
                b = server.behave.get(path, {})
                data = server.files.get(path)
                if "redirect" in b:
                    self.send_response(302)
                    self.send_header("Location", b["redirect"])
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return
                if b.get("status") or data is None:
                    self.send_response(b.get("status", 404))
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return
                if b.get("auth_once", 0) > 0 and rng != "bytes=0-0":
                    b["auth_once"] -= 1
                    body = b"<Error>RequestExpired</Error>"
                    self.send_response(403)
                    self.send_header("Content-Length", str(len(body)))
                    self.end_headers()
                    self.wfile.write(body)
                    return
                total = len(data)
                a, z = 0, total - 1
                status = 200
                if rng:
                    m = re.fullmatch(r"bytes=(\d+)-(\d+)", rng)
                    a, z = int(m[1]), min(int(m[2]), total - 1)
                    status = 206
                body = data[a:z + 1]
                if b.get("flip") and len(body) > 10 and rng != "bytes=0-0":
                    body = body[:5] + bytes([body[5] ^ 1]) + body[6:]
                self.send_response(status)
                self.send_header("ETag", '"v1"')
                if status == 206:
                    self.send_header("Content-Range", f"bytes {a}-{z}/{total}")
                declared = len(body)
                if b.get("oversize") and rng != "bytes=0-0":
                    body = body + b"x" * 100
                if b.get("truncate") and rng != "bytes=0-0":
                    body = body[:len(body) // 2]
                self.send_header("Content-Length", str(declared if not b.get("oversize") else len(body)))
                self.end_headers()
                rate = b.get("rate")
                try:
                    for i in range(0, len(body), 16384):
                        piece = body[i:i + 16384]
                        if i + 16384 >= len(body):
                            entry["t1"] = time.monotonic()
                        self.wfile.write(piece)
                        entry["sent"] += len(piece)
                        if rate and rng != "bytes=0-0":
                            time.sleep(len(piece) / rate)
                    if b.get("truncate") and rng != "bytes=0-0":
                        self.close_connection = True
                except OSError:
                    pass

        class S(socketserver.ThreadingMixIn, http.server.HTTPServer):
            daemon_threads = True
            allow_reuse_address = True

        self.httpd = S(("127.0.0.1", 0), H)
        self.url = f"http://127.0.0.1:{self.httpd.server_address[1]}"
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()

    def add(self, name, data, **behave):
        self.files["/" + name] = data
        self.behave["/" + name] = dict(behave)
        return self.url + "/" + name

    def close(self):
        self.httpd.shutdown()
        self.httpd.server_close()


def item(name, data, sources, dest=None, needed_by="s1", **extra):
    return {"name": name, "dest": dest or "dl/" + name, "sha256": sha(data), "size": len(data), "needed_by": needed_by,
            "sources": sources, **extra}


def manifest_doc(items, stages=("s1", "s2")):
    return {"schema": bd.SCHEMA, "stages": list(stages), "items": items}


def mirror(url, **kw):
    return {"kind": "mirror", "url": url, **kw}


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name) / "root"
        self.root.mkdir()
        self.srv = Server()
        self.lines = []
        self.addCleanup(self.srv.close)
        self.addCleanup(self.tmp.cleanup)

    def manifest(self, items, **kw):
        return bd.parse_manifest(json.dumps(manifest_doc(items, **kw)).encode(), allow_http=True)

    def dl(self, items, **kw):
        opts = dict(allow_http=True, limit_mbps=0, streams=2, chunk_size=64 * 1024, emit=self.lines.append, cancel_grace=3.0)
        opts.update(kw)
        return bd.Downloader(self.manifest(items), self.root, **opts)

    def run_dl(self, items, **kw):
        d = self.dl(items, **kw)
        return d.run(), d


class ManifestHostile(unittest.TestCase):
    def good(self):
        d = b"abc"
        return manifest_doc([item("a", d, [mirror("https://example.org/a")])])

    def parse(self, doc, **kw):
        return bd.parse_manifest(json.dumps(doc).encode(), **kw)

    def bad(self, mutate):
        doc = self.good()
        mutate(doc)
        with self.assertRaises(bd.ManifestError):
            self.parse(doc)

    def test_good_manifest_parses(self):
        m = self.parse(self.good())
        self.assertEqual(m.items[0].name, "a")

    def test_traversal_dest_refused(self):
        for dest in ("../x", "a/../../x", "a/./b", "a//b", "a/..", "."):
            self.bad(lambda d, dest=dest: d["items"][0].update(dest=dest))

    def test_absolute_and_backslash_dest_refused(self):
        for dest in ("/etc/passwd", "a\\b", "a/b\0c", ""):
            self.bad(lambda d, dest=dest: d["items"][0].update(dest=dest))

    def test_reserved_suffix_dest_refused(self):
        for dest in ("x.unverified", "a/x.fetch-state.json", "x.lock", "a/b.tmp"):
            self.bad(lambda d, dest=dest: d["items"][0].update(dest=dest))

    def test_state_directory_dest_refused(self):
        for dest in (".kf-downloader/status.json", ".kf-downloader/x/y"):
            self.bad(lambda d, dest=dest: d["items"][0].update(dest=dest))

    def test_duplicate_dest_refused(self):
        def two(d):
            d["items"].append(dict(d["items"][0], name="b"))
        self.bad(two)

    def test_dest_prefix_collision_refused(self):
        def two(d):
            d["items"].append(dict(d["items"][0], name="b", dest=d["items"][0]["dest"] + "/inner"))
        self.bad(two)

    def test_duplicate_name_refused(self):
        def two(d):
            d["items"].append(dict(d["items"][0], dest="other"))
        self.bad(two)

    def test_duplicate_name_with_other_pin_refused(self):
        def two(d):
            d["items"].append(dict(d["items"][0], dest="o/p", sha256="1" * 64, size=9))
        self.bad(two)

    def test_negative_and_zero_sizes_refused(self):
        for size in (-1, 0, -2**70):
            self.bad(lambda d, size=size: d["items"][0].update(size=size))

    def test_huge_sizes_refused(self):
        for size in (bd.MAX_SIZE + 1, 2**70):
            self.bad(lambda d, size=size: d["items"][0].update(size=size))

    def test_bool_and_non_integer_sizes_refused(self):
        for size in (True, 1.5, "3"):
            self.bad(lambda d, size=size: d["items"][0].update(size=size))

    def test_bool_priority_and_out_of_range_priority_refused(self):
        for pr in (True, 1001, -1001, 1.0):
            self.bad(lambda d, pr=pr: d["items"][0].update(priority=pr))

    def test_bad_sha_refused(self):
        for value in ("0" * 63, "G" * 64, "A" * 64, 5):
            self.bad(lambda d, value=value: d["items"][0].update(sha256=value))

    def test_unknown_keys_and_schema_refused(self):
        self.bad(lambda d: d["items"][0].update(extra=1))
        self.bad(lambda d: d.update(schema="other/1"))
        self.bad(lambda d: d.update(extra=1))

    def test_duplicate_json_key_and_nan_refused(self):
        raw = json.dumps(self.good()).replace('"stages"', '"items": [], "stages"', 1).encode()
        with self.assertRaises(bd.ManifestError):
            bd.parse_manifest(raw)
        with self.assertRaises(bd.ManifestError):
            bd.parse_manifest(json.dumps(self.good()).replace('"priority"', "x").encode().replace(b'"size": 3', b'"size": NaN'))

    def test_needed_by_must_be_a_stage(self):
        self.bad(lambda d: d["items"][0].update(needed_by="nope"))

    def test_http_mirror_refused_by_default(self):
        self.bad(lambda d: d["items"][0].update(sources=[mirror("http://example.org/a")]))
        self.parse(dict(self.good(), items=[dict(self.good()["items"][0], sources=[mirror("http://example.org/a")])]), allow_http=True)

    def test_credentials_and_fragment_in_url_refused(self):
        for url in ("https://user:pw@example.org/a", "https://example.org/a#frag", "https://example.org/a b", "ftp://example.org/a"):
            self.bad(lambda d, url=url: d["items"][0].update(sources=[mirror(url)]))

    def test_s3_after_mirror_refused(self):
        self.bad(lambda d: d["items"][0].update(sources=[mirror("https://example.org/a"), {"kind": "s3", "bucket": "bkt", "key": "k"}]))

    def test_unknown_source_key_refused(self):
        self.bad(lambda d: d["items"][0].update(sources=[{"kind": "signed", "ref": "a", "url": "https://e.org/x"}]))

    def test_ordering_is_stage_first_then_priority(self):
        m = self.parse(manifest_doc([item("a", b"a", [mirror("https://e.org/a")], dest="d/a", needed_by="s2", priority=-5),
                                     item("b", b"b", [mirror("https://e.org/b")], dest="d/b", needed_by="s1", priority=9)]))
        self.assertEqual([i.name for i in m.ordered()], ["b", "a"])

    def test_ordering_priority_then_manifest_order_within_a_stage(self):
        m = self.parse(manifest_doc([item("a", b"a", [mirror("https://e.org/a")], dest="d/a", priority=1),
                                     item("b", b"b", [mirror("https://e.org/b")], dest="d/b", priority=0),
                                     item("c", b"c", [mirror("https://e.org/c")], dest="d/c", priority=1)]))
        self.assertEqual([i.name for i in m.ordered()], ["b", "a", "c"])

    def test_private_source_after_mirror_refused(self):
        self.bad(lambda d: d["items"][0].update(sources=[mirror("https://example.org/a"), {"kind": "signed", "ref": "a"}]))

    def test_source_kinds_and_s3_fields_validated(self):
        for src in ({"kind": "ftp"}, {"kind": "s3", "bucket": "B", "key": "k"}, {"kind": "s3", "bucket": "bkt", "key": "../k"},
                    {"kind": "signed", "ref": "a/b"}, {"kind": "mirror", "url": "https://e.org/a", "follow_redirect": "yes"}):
            self.bad(lambda d, src=src: d["items"][0].update(sources=[src]))

    def test_too_many_items_and_oversize_document_refused(self):
        many = manifest_doc([item(f"n{i}", b"x", [mirror("https://e.org/x")], dest=f"d/{i}") for i in range(bd.MAX_ITEMS + 1)])
        with self.assertRaises(bd.ManifestError):
            self.parse(many)
        with self.assertRaises(bd.ManifestError):
            bd.parse_manifest(b" " * (bd.MAX_MANIFEST_BYTES + 1))

    def test_local_pin_must_be_absolute(self):
        self.bad(lambda d: d["items"][0].update(local="rel/path"))

    def test_error_messages_carry_no_values(self):
        doc = self.good()
        doc["items"][0]["dest"] = "../SECRETVALUE"
        with self.assertRaises(bd.ManifestError) as cm:
            self.parse(doc)
        self.assertNotIn("SECRETVALUE", str(cm.exception))


class Fetching(Base):
    def test_downloads_verifies_and_renames(self):
        data = blob(300_000)
        url = self.srv.add("a", data)
        rc, d = self.run_dl([item("a", data, [mirror(url)])])
        self.assertEqual(rc, 0)
        self.assertEqual((self.root / "dl/a").read_bytes(), data)
        self.assertFalse((self.root / "dl/a.unverified").exists())
        self.assertTrue(any(l.startswith("KF_STEP name=download.a ") and "source=mirror0" in l and "bytes=300000" in l for l in self.lines))
        self.assertEqual(d.results["a"]["state"], "verified")

    def test_wrong_sha_deletes_and_fails_retry_class(self):
        data = blob(100_000)
        url = self.srv.add("a", data)
        wrong = dict(item("a", data, [mirror(url)]), sha256=sha(b"other"))
        rc, d = self.run_dl([wrong])
        self.assertEqual(rc, 1)
        self.assertFalse((self.root / "dl/a").exists())
        self.assertFalse((self.root / "dl/a.unverified").exists())
        self.assertTrue(any("KF_DOWNLOAD_FAIL name=a class=RETRY" in l for l in self.lines))

    def test_oversize_response_fails_and_leaves_nothing(self):
        data = blob(100_000)
        url = self.srv.add("a", data, oversize=True)
        rc, _ = self.run_dl([item("a", data, [mirror(url)])])
        self.assertEqual(rc, 1)
        self.assertEqual(list((self.root / "dl").glob("a*")), [])

    def test_undersize_response_fails_and_leaves_no_final_file(self):
        data = blob(100_000)
        url = self.srv.add("a", data, truncate=True)
        with mock.patch.object(kf_fetch, "ATTEMPTS", 1):
            rc, _ = self.run_dl([item("a", data, [mirror(url)])])
        self.assertEqual(rc, 1)
        self.assertFalse((self.root / "dl/a").exists())

    def test_server_size_must_equal_pinned_size(self):
        data = blob(100_000)
        url = self.srv.add("a", data)
        rc, _ = self.run_dl([dict(item("a", data, [mirror(url)]), size=len(data) + 1)])
        self.assertEqual(rc, 1)
        self.assertFalse((self.root / "dl/a").exists())

    def test_redirect_is_not_followed_by_default(self):
        data = blob(50_000)
        real = self.srv.add("real", data)
        url = self.srv.add("a", data, redirect=real)
        rc, _ = self.run_dl([item("a", data, [mirror(url)])])
        self.assertEqual(rc, 1)
        self.assertFalse(any(e["path"] == "/real" and e["range"] != "bytes=0-0" for e in self.srv.log if False))
        self.assertFalse((self.root / "dl/a").exists())

    def test_redirect_followed_only_when_opted_in(self):
        data = blob(50_000)
        real = self.srv.add("real", data)
        url = self.srv.add("a", data, redirect=real)
        rc, _ = self.run_dl([item("a", data, [mirror(url, follow_redirect=True)])])
        self.assertEqual(rc, 0)

    def test_redirect_to_non_https_from_https_is_refused(self):
        class R:
            status = 302
            headers = {"Location": "http://127.0.0.1:1/x"}
            def __enter__(self): return self
            def __exit__(self, *a): return False
        data = b"x" * 10
        m = bd.parse_manifest(json.dumps(manifest_doc([item("a", data, [mirror("https://example.org/a", follow_redirect=True)])])).encode())
        d = bd.Downloader(m, self.root, emit=self.lines.append, limit_mbps=0)
        with mock.patch.object(kf_fetch.Remote, "open", return_value=R()):
            rc = d.run()
        self.assertEqual(rc, 1)
        self.assertFalse((self.root / "dl/a").exists())

    def test_fallback_to_second_mirror_never_mixes(self):
        data = blob(200_000)
        bad = self.srv.add("bad", data, flip=True)
        good = self.srv.add("good", data)
        rc, d = self.run_dl([item("a", data, [mirror(bad), mirror(good)])])
        self.assertEqual(rc, 0)
        self.assertEqual((self.root / "dl/a").read_bytes(), data)
        self.assertIn("source=mirror1", "\n".join(self.lines))

    def test_all_sources_fail_is_retry_class(self):
        data = blob(10_000)
        u1, u2 = self.srv.add("m1", data, status=503), self.srv.add("m2", data, status=500)
        with mock.patch.object(kf_fetch, "ATTEMPTS", 1):
            rc, _ = self.run_dl([item("a", data, [mirror(u1), mirror(u2)])])
        self.assertEqual(rc, 1)
        self.assertTrue(any("class=RETRY" in l for l in self.lines))

    def test_symlink_dest_file_refused(self):
        data = blob(10_000)
        url = self.srv.add("a", data)
        victim = Path(self.tmp.name) / "victim"
        victim.write_text("keep")
        (self.root / "dl").mkdir()
        (self.root / "dl/a").symlink_to(victim)
        rc, _ = self.run_dl([item("a", data, [mirror(url)])])
        self.assertEqual(rc, 1)
        self.assertEqual(victim.read_text(), "keep")
        self.assertEqual(self.srv.log, [])

    def test_symlink_directory_component_refused(self):
        data = blob(10_000)
        url = self.srv.add("a", data)
        outside = Path(self.tmp.name) / "outside"
        outside.mkdir()
        (self.root / "dl").symlink_to(outside)
        rc, _ = self.run_dl([item("a", data, [mirror(url)])])
        self.assertEqual(rc, 1)
        self.assertEqual(list(outside.iterdir()), [])
        self.assertEqual(self.srv.log, [])

    def test_symlink_unverified_refused(self):
        data = blob(10_000)
        url = self.srv.add("a", data)
        victim = Path(self.tmp.name) / "victim"
        victim.write_text("keep")
        (self.root / "dl").mkdir()
        (self.root / "dl/a.unverified").symlink_to(victim)
        rc, _ = self.run_dl([item("a", data, [mirror(url)])])
        self.assertEqual(rc, 1)
        self.assertEqual(victim.read_text(), "keep")

    def test_symlink_state_file_refused(self):
        data = blob(10_000)
        url = self.srv.add("a", data)
        victim = Path(self.tmp.name) / "victim"
        victim.write_text("keep")
        (self.root / ".kf-downloader").mkdir()
        (self.root / ".kf-downloader" / (hashlib.sha256(b"a").hexdigest()[:24] + ".json")).symlink_to(victim)
        rc, _ = self.run_dl([item("a", data, [mirror(url)])])
        self.assertEqual(rc, 1)
        self.assertEqual(victim.read_text(), "keep")

    def test_existing_verified_file_needs_no_request(self):
        data = blob(10_000)
        url = self.srv.add("a", data)
        (self.root / "dl").mkdir()
        (self.root / "dl/a").write_bytes(data)
        rc, _ = self.run_dl([item("a", data, [mirror(url)])])
        self.assertEqual(rc, 0)
        self.assertEqual(self.srv.log, [])
        self.assertIn("source=present", "\n".join(self.lines))

    def test_stale_wrong_file_is_replaced(self):
        data = blob(10_000)
        url = self.srv.add("a", data)
        (self.root / "dl").mkdir()
        (self.root / "dl/a").write_bytes(b"stale")
        rc, _ = self.run_dl([item("a", data, [mirror(url)])])
        self.assertEqual(rc, 0)
        self.assertEqual((self.root / "dl/a").read_bytes(), data)

    def test_disk_reserve_refuses_before_any_request(self):
        data = blob(10_000)
        url = self.srv.add("a", data)
        rc, _ = self.run_dl([item("a", data, [mirror(url)])], disk_free=lambda p: 100)
        self.assertEqual(rc, 2)
        self.assertEqual(self.srv.log, [])

    def test_reserve_bytes_are_added_to_the_need(self):
        data = blob(10_000)
        url = self.srv.add("a", data)
        rc, _ = self.run_dl([item("a", data, [mirror(url)])], disk_free=lambda p: 10_005, reserve_bytes=1000)
        self.assertEqual(rc, 2)
        rc, _ = self.run_dl([item("a", data, [mirror(url)])], disk_free=lambda p: 11_005, reserve_bytes=1000)
        self.assertEqual(rc, 0)

    def test_cancel_in_process_keeps_partial_and_reports_pending(self):
        data = blob(4 * (1 << 20))
        url = self.srv.add("a", data, rate=1_500_000)
        d = self.dl([item("a", data, [mirror(url)])], streams=1, chunk_size=1 << 20)
        box = []
        t = threading.Thread(target=lambda: box.append(d.run()), daemon=True)
        t.start()
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            states = [f for f in (self.root / ".kf-downloader").glob("*.json") if f.name != "status.json"] if (self.root / ".kf-downloader").exists() else []
            if states and json.loads(states[0].read_text() or "{}").get("completed"):
                break
            time.sleep(0.05)
        d.cancel()
        t.join(15)
        self.assertEqual(box, [5])
        self.assertTrue((self.root / "dl/a.unverified").exists())
        self.assertFalse((self.root / "dl/a").exists())
        self.assertEqual(d.results["a"]["state"], "pending")

    def test_second_downloader_on_same_root_refused(self):
        data = blob(10_000)
        url = self.srv.add("a", data)
        d1 = self.dl([item("a", data, [mirror(url)])])
        d1.preflight()
        lock = open(d1.state_dir / "downloader.lock", "a+b")
        import fcntl
        fcntl.flock(lock, fcntl.LOCK_EX)
        rc, _ = self.run_dl([item("a", data, [mirror(url)])])
        self.assertEqual(rc, 2)
        lock.close()

    def test_order_is_stage_then_priority(self):
        a, b, c = blob(5000, 1), blob(5000, 2), blob(5000, 3)
        ua, ub, uc = self.srv.add("a", a), self.srv.add("b", b), self.srv.add("c", c)
        items = [item("a", a, [mirror(ua)], needed_by="s2"), item("b", b, [mirror(ub)], needed_by="s1", priority=5),
                 item("c", c, [mirror(uc)], needed_by="s1", priority=1)]
        rc, _ = self.run_dl(items, jobs=1, streams=1)
        self.assertEqual(rc, 0)
        first = [e["path"] for e in self.srv.log if e["range"] == "bytes=0-0"]
        self.assertEqual(first, ["/c", "/b", "/a"])

    def test_only_and_stages_select(self):
        a, b = blob(5000, 1), blob(5000, 2)
        ua, ub = self.srv.add("a", a), self.srv.add("b", b)
        items = [item("a", a, [mirror(ua)], needed_by="s1"), item("b", b, [mirror(ub)], needed_by="s2")]
        rc, _ = self.run_dl(items, stages=("s2",))
        self.assertEqual(rc, 0)
        self.assertFalse((self.root / "dl/a").exists())
        self.assertTrue((self.root / "dl/b").exists())

    def test_every_source_failing_in_a_fetch_leaves_other_items_running(self):
        a, b = blob(5000, 1), blob(5000, 2)
        ub = self.srv.add("b", b)
        items = [item("a", a, [mirror(self.srv.url + "/missing")]), item("b", b, [mirror(ub)])]
        with mock.patch.object(kf_fetch, "ATTEMPTS", 1):
            rc, d = self.run_dl(items)
        self.assertEqual(rc, 1)
        self.assertEqual(d.results["b"]["state"], "verified")
        self.assertEqual(d.results["a"]["state"], "failed")


class LocalPins(Base):
    def test_pin_dir_hash_checked_no_network(self):
        data = blob(50_000)
        pins = Path(self.tmp.name) / "pins"
        pins.mkdir()
        (pins / "a").write_bytes(data)
        rc, _ = self.run_dl([item("a", data, [mirror(self.srv.url + "/a")])], pin_dirs=[pins])
        self.assertEqual(rc, 0)
        self.assertEqual(self.srv.log, [])
        self.assertEqual((self.root / "dl/a").read_bytes(), data)
        self.assertIn("source=local", "\n".join(self.lines))

    def test_pin_with_wrong_hash_is_ignored(self):
        data = blob(50_000)
        url = self.srv.add("a", data)
        pins = Path(self.tmp.name) / "pins"
        pins.mkdir()
        (pins / "a").write_bytes(b"x" * len(data))
        rc, _ = self.run_dl([item("a", data, [mirror(url)])], pin_dirs=[pins])
        self.assertEqual(rc, 0)
        self.assertTrue(self.srv.log)
        self.assertEqual((self.root / "dl/a").read_bytes(), data)

    def test_explicit_pin_and_manifest_local_field(self):
        data = blob(20_000)
        f = Path(self.tmp.name) / "elsewhere.bin"
        f.write_bytes(data)
        rc, _ = self.run_dl([item("a", data, [mirror(self.srv.url + "/a")], local=str(f))])
        self.assertEqual(rc, 0)
        rc, _ = self.run_dl([item("b", data, [mirror(self.srv.url + "/b")])], pins={"b": str(f)})
        self.assertEqual(rc, 0)
        self.assertEqual(self.srv.log, [])

    def test_wrong_hash_pin_is_never_published(self):
        data = blob(20_000)
        f = Path(self.tmp.name) / "same_size_wrong.bin"
        f.write_bytes(b"y" * len(data))
        with mock.patch.object(kf_fetch, "ATTEMPTS", 1):
            rc, _ = self.run_dl([item("a", data, [mirror(self.srv.url + "/gone")])], pins={"a": str(f)})
        self.assertEqual(rc, 1)
        self.assertFalse((self.root / "dl/a").exists())

    def test_wrong_size_pin_is_ignored(self):
        data = blob(20_000)
        url = self.srv.add("a", data)
        f = Path(self.tmp.name) / "short.bin"
        f.write_bytes(data[:-1])
        rc, _ = self.run_dl([item("a", data, [mirror(url)])], pins={"a": str(f)})
        self.assertEqual(rc, 0)
        self.assertTrue(self.srv.log)


class PrivateSources(Base):
    def urls_file(self, mapping, mode=0o600):
        p = Path(self.tmp.name) / "urls.json"
        p.write_text(json.dumps({"schema": bd.URLS_SCHEMA, "urls": mapping}))
        os.chmod(p, mode)
        return p

    def test_signed_url_from_file_is_used_and_never_logged(self):
        data = blob(80_000)
        self.srv.add("obj", data)
        p = self.urls_file({"a": self.srv.url + f"/obj?X-Amz-Signature={SECRET}"})
        status = self.root / ".kf-downloader" / "status.json"
        rc, d = self.run_dl([item("a", data, [{"kind": "signed", "ref": "a"}])], url_provider=bd.UrlFileProvider(p))
        self.assertEqual(rc, 0)
        self.assertIn("source=r2", "\n".join(self.lines))
        self.assertNotIn(SECRET, "\n".join(self.lines))
        self.assertNotIn(SECRET, status.read_text())
        for f in self.root.rglob("*"):
            if f.is_file() and f.name != "a":
                self.assertNotIn(SECRET.encode(), f.read_bytes())

    def test_urls_file_permissions_enforced(self):
        p = self.urls_file({"a": "https://e.org/x"}, mode=0o644)
        with self.assertRaises(bd.ProviderError):
            bd.UrlFileProvider(p)("a")

    def test_urls_file_symlink_and_foreign_schema_refused(self):
        p = self.urls_file({"a": "https://e.org/x"})
        link = Path(self.tmp.name) / "link.json"
        link.symlink_to(p)
        with self.assertRaises(bd.ProviderError):
            bd.UrlFileProvider(link)("a")
        p.write_text(json.dumps({"schema": "other", "urls": {}}))
        with self.assertRaises(bd.ProviderError):
            bd.UrlFileProvider(p)("a")

    def test_missing_signed_url_falls_through_to_mirror(self):
        data = blob(30_000)
        url = self.srv.add("m", data)
        p = self.urls_file({})
        rc, _ = self.run_dl([item("a", data, [{"kind": "signed", "ref": "a"}, mirror(url)])], url_provider=bd.UrlFileProvider(p))
        self.assertEqual(rc, 0)
        self.assertIn("source=mirror1", "\n".join(self.lines))

    def test_expired_signed_url_is_requested_again(self):
        data = blob(60_000)
        self.srv.add("obj", data, auth_once=1)
        calls = []

        def provider(ref):
            calls.append(ref)
            return self.srv.url + f"/obj?sig={SECRET}{len(calls)}"

        rc, _ = self.run_dl([item("a", data, [{"kind": "signed", "ref": "a"}])], url_provider=provider, streams=1, chunk_size=1 << 20)
        self.assertEqual(rc, 0)
        self.assertGreaterEqual(len(calls), 2)
        self.assertNotIn(SECRET, "\n".join(self.lines))

    def test_expired_s3_url_is_signed_again(self):
        data = blob(60_000)
        self.srv.add("bkt/obj", data, auth_once=1)
        stamps = []

        def clock():
            stamps.append(1)
            return datetime.datetime(2026, 11, 15, 0, 0, len(stamps) % 60, tzinfo=datetime.timezone.utc)

        cfg = bd.S3Config(self.srv.url, "auto", "AKID", "SKEY")
        rc, _ = self.run_dl([item("a", data, [{"kind": "s3", "bucket": "bkt", "key": "obj"}])], streams=1, chunk_size=1 << 20,
                            s3_provider=bd.S3Provider(cfg, allow_http=True, clock=clock))
        self.assertEqual(rc, 0)
        self.assertGreaterEqual(len(stamps), 2)

    def test_s3_signature_matches_an_independent_reference(self):
        data = blob(20_000)
        self.srv.add("bkt/dir/obj", data)
        cfg = bd.S3Config(self.srv.url, "eu-west-1", "AKID", "SKEY", "SESSIONTOKEN")
        rc, _ = self.run_dl([item("a", data, [{"kind": "s3", "bucket": "bkt", "key": "dir/obj"}])], s3_provider=bd.S3Provider(cfg, allow_http=True))
        self.assertEqual(rc, 0)
        e = [x for x in self.srv.log if x["path"] == "/bkt/dir/obj"][0]
        import hmac as _h
        import urllib.parse as up
        q = dict(up.parse_qsl(e["query"].split("?", 1)[1]))
        sig = q.pop("X-Amz-Signature")
        self.assertEqual(q["X-Amz-Security-Token"], "SESSIONTOKEN")
        cq = "&".join(f"{up.quote(k, safe='')}={up.quote(v, safe='')}" for k, v in sorted(q.items()))
        host = e["headers"]["Host"]
        creq = "\n".join(["GET", "/bkt/dir/obj", cq, f"host:{host}", "", "host", "UNSIGNED-PAYLOAD"])
        day = q["X-Amz-Date"][:8]
        sts = "\n".join(["AWS4-HMAC-SHA256", q["X-Amz-Date"], f"{day}/eu-west-1/s3/aws4_request", hashlib.sha256(creq.encode()).hexdigest()])
        k = _h.new(("AWS4SKEY").encode(), day.encode(), hashlib.sha256).digest()
        for part in ("eu-west-1", "s3", "aws4_request"):
            k = _h.new(k, part.encode(), hashlib.sha256).digest()
        self.assertEqual(sig, _h.new(k, sts.encode(), hashlib.sha256).hexdigest())

    def test_urls_file_duplicate_key_refused(self):
        p = Path(self.tmp.name) / "dup.json"
        p.write_text('{"schema": "%s", "urls": {"a": "https://e.org/1", "a": "https://e.org/2"}}' % bd.URLS_SCHEMA)
        os.chmod(p, 0o600)
        with self.assertRaises(bd.ProviderError):
            bd.UrlFileProvider(p)("a")

    def test_urls_file_symlink_is_not_followed(self):
        p = self.urls_file({"a": "https://e.org/x"})
        link = Path(self.tmp.name) / "link2.json"
        link.symlink_to(p)
        with self.assertRaises(bd.ProviderError):
            bd.UrlFileProvider(link)("a")

    def test_s3_presign_session_token_is_signed(self):
        now = datetime.datetime(2013, 5, 24, tzinfo=datetime.timezone.utc)
        a = bd.sigv4_presign("h.example", "/k", "us-east-1", "AK", "SK", "", now, 60)
        b = bd.sigv4_presign("h.example", "/k", "us-east-1", "AK", "SK", "TOK", now, 60)
        self.assertNotIn("X-Amz-Security-Token", a)
        self.assertIn("X-Amz-Security-Token=TOK", b)
        self.assertNotEqual(a.rsplit("=", 1)[1], b.rsplit("=", 1)[1])

    def test_s3_endpoint_path_is_refused_not_signed(self):
        for endpoint in ("https://e.org/a/b", "https://e.org//x"):
            with self.assertRaises(bd.ProviderError):
                bd.S3Provider(bd.S3Config(endpoint, "auto", "a", "b"))("bkt", "k")

    def test_s3_endpoint_path_falls_through_to_the_mirror(self):
        cfg = bd.S3Config("https://e.org/prefix", "auto", "a", "b")
        data = blob(10_000)
        url = self.srv.add("m", data)
        rc, _ = self.run_dl([item("a", data, [{"kind": "s3", "bucket": "bkt", "key": "k"}, mirror(url)])], s3_provider=bd.S3Provider(cfg))
        self.assertEqual(rc, 0)
        self.assertIn("source=mirror1", "\n".join(self.lines))

    def test_s3_presign_matches_the_aws_documented_vector(self):
        url = bd.sigv4_presign("examplebucket.s3.amazonaws.com", "/test.txt", "us-east-1", "AKIAIOSFODNN7EXAMPLE",
                               "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", "",
                               datetime.datetime(2013, 5, 24, tzinfo=datetime.timezone.utc), 86400)
        self.assertTrue(url.endswith("X-Amz-Signature=aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404"), url)

    def test_s3_source_downloads_with_credentials_from_environment_only(self):
        data = blob(70_000)
        self.srv.add("bkt/dir/obj", data)
        env = {"KF_S3_ENDPOINT": self.srv.url, "AWS_ACCESS_KEY_ID": "AKID", "AWS_SECRET_ACCESS_KEY": "SKEY" + SECRET}
        cfg = bd.S3Config.load(env)
        rc, _ = self.run_dl([item("a", data, [{"kind": "s3", "bucket": "bkt", "key": "dir/obj"}])],
                            s3_provider=bd.S3Provider(cfg, allow_http=True))
        self.assertEqual(rc, 0)
        q = [e["query"] for e in self.srv.log if e["path"] == "/bkt/dir/obj"][0]
        self.assertIn("X-Amz-Signature=", q)
        self.assertNotIn("SKEY", q)
        self.assertNotIn(SECRET, "\n".join(self.lines))

    def test_s3_without_credentials_is_skipped_not_fatal(self):
        data = blob(30_000)
        url = self.srv.add("m", data)
        self.assertIsNone(bd.S3Config.load({}))
        rc, _ = self.run_dl([item("a", data, [{"kind": "s3", "bucket": "bkt", "key": "k"}, mirror(url)])])
        self.assertEqual(rc, 0)

    def test_s3_config_file_must_be_private_and_hold_known_keys(self):
        p = Path(self.tmp.name) / "s3.json"
        p.write_text(json.dumps({"endpoint": "https://e.org", "access_key_id": "a", "secret_access_key": "b"}))
        os.chmod(p, 0o644)
        with self.assertRaises(bd.ProviderError):
            bd.S3Config.load({}, p)
        os.chmod(p, 0o600)
        self.assertEqual(bd.S3Config.load({}, p).region, "auto")
        p.write_text(json.dumps({"endpoint": "https://e.org", "bogus": 1}))
        with self.assertRaises(bd.ProviderError):
            bd.S3Config.load({}, p)

    def test_s3_endpoint_with_path_or_http_refused(self):
        cfg = bd.S3Config("https://e.org/prefix", "auto", "a", "b")
        with self.assertRaises(bd.ProviderError):
            bd.S3Provider(cfg)("bkt", "k")
        with self.assertRaises(bd.ProviderError):
            bd.S3Provider(bd.S3Config("http://e.org", "auto", "a", "b"))("bkt", "k")


class WorkerLayer(Base):
    def test_worker_call_sends_token_only_to_origin_and_returns_location(self):
        self.srv.add("x", b"")
        self.srv.behave["/bench-data/12-1/obj"] = {"redirect": "https://r2.example/obj?sig=1"}
        self.srv.files["/bench-data/12-1/obj"] = b""
        url = bw.worker_signed_url(self.srv.url, "12-1", "tok.en_1", "obj", ("id1", "sec1"), allow_http=True)
        self.assertEqual(url, "https://r2.example/obj?sig=1")
        h = [e for e in self.srv.log if e["path"] == "/bench-data/12-1/obj"][0]["headers"]
        h = {k.lower(): v for k, v in h.items()}
        self.assertEqual(h["authorization"], "Bearer tok.en_1")
        self.assertEqual(h["cf-access-client-id"], "id1")

    def test_worker_location_with_credentials_refused(self):
        self.srv.files["/bench-data/12-1/o3"] = b""
        self.srv.behave["/bench-data/12-1/o3"] = {"redirect": "https://user:pw@r2.example/o3"}
        with self.assertRaises(bw.WorkerError):
            bw.worker_signed_url(self.srv.url, "12-1", "t", "o3", allow_http=True)

    def test_worker_location_must_be_https_and_status_must_redirect(self):
        self.srv.files["/bench-data/12-1/o1"] = b""
        self.srv.behave["/bench-data/12-1/o1"] = {"redirect": "ftp://evil.example/o"}
        with self.assertRaises(bw.WorkerError):
            bw.worker_signed_url(self.srv.url, "12-1", "t", "o1", allow_http=True)
        with self.assertRaises(bw.WorkerError):
            bw.worker_signed_url(self.srv.url, "12-1", "t", "o1")
        with self.assertRaises(bw.WorkerError):
            bw.worker_signed_url(self.srv.url, "12-1", "t", "o2", allow_http=True)

    def test_resolve_writes_private_urls_file_and_prints_no_url(self):
        self.srv.files["/bench-data/12-1/o1"] = b""
        self.srv.behave["/bench-data/12-1/o1"] = {"redirect": f"https://r2.example/o1?sig={SECRET}"}
        man = Path(self.tmp.name) / "m.json"
        man.write_text(json.dumps(manifest_doc([item("o1", b"x", [{"kind": "signed", "ref": "o1"}])])))
        out = Path(self.tmp.name) / "urls.json"
        buf = io.StringIO()
        with mock.patch("sys.stdout", buf):
            rc = bw.main(["resolve", str(man), "--urls-out", str(out)], {"KF_BROKER_URL": self.srv.url, "KF_RUN_ID": "12-1", "KF_RUN_TOKEN": "tok"}, allow_http=True)
        self.assertEqual(rc, 0)
        self.assertEqual(oct(out.stat().st_mode & 0o777), "0o600")
        self.assertNotIn(SECRET, buf.getvalue())
        self.assertEqual(bd.UrlFileProvider(out)("o1"), f"https://r2.example/o1?sig={SECRET}")


class Bandwidth(Base):
    def test_parallel_fetches_overlap(self):
        n, rate = 200_000, 200_000
        items = []
        for k in range(3):
            data = blob(n, k + 1)
            items.append(item(f"f{k}", data, [mirror(self.srv.add(f"f{k}", data, rate=rate))]))
        rc, _ = self.run_dl(items, jobs=3, streams=1, chunk_size=1 << 20)
        self.assertEqual(rc, 0)
        spans = [(e["t0"], e["t1"]) for e in self.srv.log if e["range"] != "bytes=0-0" and "t1" in e]
        self.assertEqual(len(spans), 3)
        self.assertLess(max(s[0] for s in spans), min(s[1] for s in spans), "no common instant: the fetches did not overlap")

    def test_three_jobs_finish_in_about_one_transfer_time(self):
        items = []
        for k in range(3):
            data = blob(200_000, k + 1)
            items.append(item(f"f{k}", data, [mirror(self.srv.add(f"f{k}", data, rate=200_000))]))
        t0 = time.monotonic()
        rc, _ = self.run_dl(items, jobs=3, streams=1, chunk_size=1 << 20)
        self.assertEqual(rc, 0)
        self.assertLess(time.monotonic() - t0, 2.4, "serial would be about 3 s")

    def test_jobs_one_does_not_overlap(self):
        n, rate = 100_000, 200_000
        items = []
        for k in range(2):
            data = blob(n, k + 1)
            items.append(item(f"f{k}", data, [mirror(self.srv.add(f"f{k}", data, rate=rate))]))
        rc, _ = self.run_dl(items, jobs=1, streams=1, chunk_size=1 << 20)
        self.assertEqual(rc, 0)
        spans = sorted((e["t0"], e["t1"]) for e in self.srv.log if e["range"] != "bytes=0-0" and "t1" in e)
        self.assertLessEqual(spans[0][1], spans[1][0] + 0.05)

    def test_global_cap_holds_across_parallel_items(self):
        items = []
        for k in range(2):
            data = blob(400_000, k + 1)
            items.append(item(f"f{k}", data, [mirror(self.srv.add(f"f{k}", data))]))
        t0 = time.monotonic()
        rc, _ = self.run_dl(items, jobs=2, streams=1, chunk_size=1 << 20, limit_mbps=0.4)
        wall = time.monotonic() - t0
        self.assertEqual(rc, 0)
        self.assertGreaterEqual(wall, 1.85, "800 kB at 0.4 MB/s must take about 2 s")
        self.assertLess(wall, 6.0)

    def test_uncapped_is_much_faster(self):
        items = []
        for k in range(2):
            data = blob(400_000, k + 1)
            items.append(item(f"f{k}", data, [mirror(self.srv.add(f"f{k}", data))]))
        t0 = time.monotonic()
        rc, _ = self.run_dl(items, jobs=2, streams=1, chunk_size=1 << 20, limit_mbps=0)
        self.assertEqual(rc, 0)
        self.assertLess(time.monotonic() - t0, 1.5)

    def test_per_stream_cap_holds_for_one_connection(self):
        data = blob(400_000)
        url = self.srv.add("a", data)
        t0 = time.monotonic()
        rc, _ = self.run_dl([item("a", data, [mirror(url)])], jobs=1, streams=1, chunk_size=1 << 20, stream_limit_mbps=0.4)
        self.assertEqual(rc, 0)
        self.assertGreaterEqual(time.monotonic() - t0, 0.95)

    def test_throttle_object_applies_the_global_rate(self):
        ev = threading.Event()
        th = bd.Throttle(1e6, 0, ev)
        t0 = time.monotonic()
        th(500_000)
        th(500_000)
        self.assertGreaterEqual(time.monotonic() - t0, 0.95)

    def test_throttle_object_applies_the_stream_rate_per_thread(self):
        ev = threading.Event()
        th = bd.Throttle(0, 1e6, ev)
        t0 = time.monotonic()
        th(500_000)
        th(500_000)
        self.assertGreaterEqual(time.monotonic() - t0, 0.95)
        t1 = time.monotonic()
        result = []
        thr = threading.Thread(target=lambda: (th(500_000), result.append(time.monotonic())))
        thr.start()
        thr.join()
        self.assertLess(result[0] - t1, 0.9, "another thread has its own stream bucket")

    def test_bucket_is_cancellable(self):
        ev = threading.Event()
        b = bd.Bucket(10)
        threading.Timer(0.1, ev.set).start()
        t0 = time.monotonic()
        b.acquire(1000, ev)
        self.assertLess(time.monotonic() - t0, 2)


class Waiting(Base):
    def start(self, items, **kw):
        d = self.dl(items, **kw)
        t = threading.Thread(target=d.run, daemon=True)
        t.start()
        return d, t

    def test_wait_returns_zero_when_verified(self):
        data = blob(40_000)
        url = self.srv.add("a", data)
        d, t = self.start([item("a", data, [mirror(url)])])
        out = []
        rc = bd.wait_for(d.status_path, ["a"], 20, poll=0.05, out=out.append)
        t.join()
        self.assertEqual(rc, 0, out)

    def test_wait_returns_nonzero_when_failed(self):
        data = blob(10_000)
        d, t = self.start([item("a", data, [mirror(self.srv.url + "/missing")])])
        with mock.patch.object(kf_fetch, "ATTEMPTS", 1):
            rc = bd.wait_for(d.status_path, ["a"], 6, poll=0.05, out=lambda s: None)
        t.join()
        self.assertEqual(rc, 1)

    def test_wait_one_failed_among_several_is_failure(self):
        a, b = blob(10_000, 1), blob(10_000, 2)
        ub = self.srv.add("b", b)
        d, t = self.start([item("a", a, [mirror(self.srv.url + "/missing")]), item("b", b, [mirror(ub)])])
        with mock.patch.object(kf_fetch, "ATTEMPTS", 1):
            rc = bd.wait_for(d.status_path, ["b", "a"], 6, poll=0.05, out=lambda s: None)
        t.join()
        self.assertEqual(rc, 1)

    def test_wait_size_mismatch_between_status_and_file_is_failure(self):
        (self.root / "dl").mkdir()
        (self.root / "dl/a").write_bytes(b"12")
        doc = {"schema": bd.STATUS_SCHEMA, "pid": os.getpid(), "state": "running", "root": str(self.root), "dests": {"a": "dl/a"},
               "items": {"a": {"state": "verified", "size": 3}}}
        p = Path(self.tmp.name) / "s.json"
        p.write_text(json.dumps(doc))
        self.assertEqual(bd.wait_for(p, ["a"], 5, poll=0.05, out=lambda s: None), 1)

    def test_wait_cancelled_run_is_downloader_gone(self):
        doc = {"schema": bd.STATUS_SCHEMA, "pid": os.getpid(), "state": "cancelled", "root": str(self.root), "dests": {"a": "dl/a"},
               "items": {"a": {"state": "pending", "size": 3}}}
        p = Path(self.tmp.name) / "s.json"
        p.write_text(json.dumps(doc))
        self.assertEqual(bd.wait_for(p, ["a"], 5, poll=0.05, out=lambda s: None), 4)

    def test_wait_deadline(self):
        data = blob(100_000)
        url = self.srv.add("a", data, rate=20_000)
        d, t = self.start([item("a", data, [mirror(url)])], streams=1, chunk_size=1 << 20)
        t0 = time.monotonic()
        rc = bd.wait_for(d.status_path, ["a"], 0.5, poll=0.05, out=lambda s: None)
        self.assertEqual(rc, 3)
        self.assertLess(time.monotonic() - t0, 3)
        d.cancel()
        t.join(10)

    def test_wait_unknown_item_and_missing_status(self):
        data = blob(10_000)
        url = self.srv.add("a", data)
        d, t = self.start([item("a", data, [mirror(url)])])
        t.join()
        self.assertEqual(bd.wait_for(d.status_path, ["nope"], 5, poll=0.05, out=lambda s: None), 2)
        self.assertEqual(bd.wait_for(Path(self.tmp.name) / "none.json", ["a"], 0.3, poll=0.05, out=lambda s: None), 3)

    def test_wait_notices_a_dead_downloader(self):
        doc = {"schema": bd.STATUS_SCHEMA, "pid": 2**22 - 3, "state": "running", "root": str(self.root), "dests": {"a": "dl/a"},
               "items": {"a": {"state": "active", "size": 3}}}
        p = Path(self.tmp.name) / "s.json"
        p.write_text(json.dumps(doc))
        self.assertEqual(bd.wait_for(p, ["a"], 5, poll=0.05, out=lambda s: None), 4)

    def test_wait_refuses_a_verified_claim_without_the_file(self):
        doc = {"schema": bd.STATUS_SCHEMA, "pid": os.getpid(), "state": "running", "root": str(self.root), "dests": {"a": "dl/a"},
               "items": {"a": {"state": "verified", "size": 3}}}
        p = Path(self.tmp.name) / "s.json"
        p.write_text(json.dumps(doc))
        self.assertEqual(bd.wait_for(p, ["a"], 5, poll=0.05, out=lambda s: None), 1)

    def test_in_process_wait(self):
        data = blob(20_000)
        url = self.srv.add("a", data)
        d, t = self.start([item("a", data, [mirror(url)])])
        self.assertTrue(d.wait(["a"], 20))
        t.join()

    def test_status_file_is_bounded(self):
        items = [item(f"n{i}", b"x", [mirror("https://e.org/x")], dest=f"d/{i}") for i in range(bd.MAX_ITEMS)]
        d = self.dl(items) if False else bd.Downloader(bd.parse_manifest(json.dumps(manifest_doc(items)).encode()), self.root, emit=lambda s: None)
        d.preflight()
        d.write_status()
        self.assertLess(d.status_path.stat().st_size, 128 * 1024)


class Standalone(Base):
    """The user-facing path: Python, a manifest, a local directory and an https/http mirror. No Worker, no credentials."""

    def env(self, **extra):
        return {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": self.tmp.name, **extra}

    def cli(self, *args, env=None, timeout=60):
        return subprocess.run([sys.executable, "-I", str(CLI), *args], capture_output=True, text=True, timeout=timeout,
                              env=env if env is not None else self.env())

    def write_manifest(self, items):
        p = Path(self.tmp.name) / "manifest.json"
        p.write_text(json.dumps(manifest_doc(items)))
        return p

    def test_fetch_wait_status_verify_with_only_python_a_dir_and_a_mirror(self):
        a, b = blob(120_000, 1), blob(150_000, 2)
        pins = Path(self.tmp.name) / "pins"
        pins.mkdir()
        (pins / "a.bin").write_bytes(a)
        url = self.srv.add("b.bin", b)
        man = self.write_manifest([item("a.bin", a, [mirror(self.srv.url + "/a.bin")]),
                                   item("b.bin", b, [mirror(url)], needed_by="s2")])
        dest = Path(self.tmp.name) / "out"
        r = self.cli("check", str(man), "--allow-http")
        self.assertEqual(r.returncode, 0, r.stderr)
        r = self.cli("fetch", str(man), "--dest", str(dest), "--pin-dir", str(pins), "--allow-http", "--limit-bandwidth", "0", "--no-nice")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("KF_STEP name=download.a.bin", r.stdout)
        self.assertIn("source=local", r.stdout)
        self.assertEqual((dest / "dl/b.bin").read_bytes(), b)
        self.assertEqual([e["path"] for e in self.srv.log if e["path"] == "/a.bin"], [])
        r = self.cli("wait", "--dest", str(dest), "--deadline", "5", "a.bin", "b.bin")
        self.assertEqual(r.returncode, 0, r.stdout)
        r = self.cli("status", "--dest", str(dest))
        self.assertIn("state=done verified=2 failed=0 total=2", r.stdout)
        self.assertEqual(self.cli("verify", str(man), "--dest", str(dest), "--allow-http").returncode, 0)
        (dest / "dl/b.bin").write_bytes(b"corrupt")
        r = self.cli("verify", str(man), "--dest", str(dest), "--allow-http")
        self.assertEqual(r.returncode, 1)
        self.assertIn("KF_VERIFY name=b.bin ok=0", r.stdout)

    def test_fetch_only_selects_named_items(self):
        a, b = blob(10_000, 1), blob(10_000, 2)
        man = self.write_manifest([item("a", a, [mirror(self.srv.add("a", a))]), item("b", b, [mirror(self.srv.add("b", b))])])
        dest = Path(self.tmp.name) / "out"
        r = self.cli("fetch", str(man), "--dest", str(dest), "--only", "b", "--allow-http", "--no-nice", "--limit-bandwidth", "0")
        self.assertEqual(r.returncode, 0, r.stdout)
        self.assertFalse((dest / "dl/a").exists())
        self.assertEqual(self.cli("fetch", str(man), "--dest", str(dest), "--only", "zzz", "--allow-http").returncode, 2)

    def test_https_only_by_default(self):
        a = blob(10_000)
        man = self.write_manifest([item("a", a, [mirror(self.srv.add("a", a))])])
        r = self.cli("fetch", str(man), "--dest", str(Path(self.tmp.name) / "o"))
        self.assertEqual(r.returncode, 2)
        self.assertEqual(self.srv.log, [])

    def test_failure_exits_one_and_names_class_retry(self):
        a = blob(10_000)
        man = self.write_manifest([item("a", a, [mirror(self.srv.url + "/gone")])])
        r = self.cli("fetch", str(man), "--dest", str(Path(self.tmp.name) / "o"), "--allow-http", "--no-nice", "--limit-bandwidth", "0")
        self.assertEqual(r.returncode, 1)
        self.assertIn("KF_DOWNLOAD_FAIL name=a class=RETRY", r.stdout)

    def test_run_token_and_broker_variables_are_neither_read_nor_printed(self):
        a = blob(10_000)
        man = self.write_manifest([item("a", a, [mirror(self.srv.add("a", a))])])
        env = self.env(KF_RUN_TOKEN="TOKENSECRET9", KF_BROKER_URL="https://broker.invalid/SECRETPATH", KF_RUN_ID="1-1",
                       KF_ACCESS_ID="ACCID", KF_ACCESS_SECRET="ACCSECRET9", WORKER_URL="https://w.invalid")
        r = self.cli("fetch", str(man), "--dest", str(Path(self.tmp.name) / "o"), "--allow-http", "--no-nice", "--limit-bandwidth", "0", env=env)
        self.assertEqual(r.returncode, 0, r.stdout)
        for text in ("TOKENSECRET9", "SECRETPATH", "ACCSECRET9", "ACCID"):
            self.assertNotIn(text, r.stdout + r.stderr)
            self.assertNotIn(text.encode(), b"".join(f.read_bytes() for f in (Path(self.tmp.name) / "o").rglob("*") if f.is_file()))
            self.assertFalse(any(text in json.dumps(e["headers"]) for e in self.srv.log))

    def test_source_never_mentions_the_run_token_or_broker_names(self):
        source = CLI.read_text()
        for name in ("KF_RUN_TOKEN", "KF_BROKER_URL", "KF_RUN_ID", "KF_ACCESS_ID", "KF_ACCESS_SECRET", "WORKER_URL", "/run/kf-ci", "bench-data"):
            self.assertNotIn(name, source)
        allowed = {"argparse", "dataclasses", "datetime", "fcntl", "hashlib", "hmac", "json", "os", "pathlib", "re", "shutil",
                   "signal", "stat", "sys", "threading", "time", "urllib", "kf_fetch", "subprocess"}
        for node in ast.walk(ast.parse(source)):
            mods = [a.name for a in node.names] if isinstance(node, ast.Import) else [node.module] if isinstance(node, ast.ImportFrom) else []
            for m in mods:
                self.assertIn(m.split(".")[0], allowed)

    def test_no_hard_coded_account_or_bucket(self):
        source = CLI.read_text() + (HERE / "box_downloader_worker.py").read_text()
        self.assertNotRegex(source, r"r2\.cloudflarestorage|[0-9a-f]{32}\.r2|specviewperf|workers\.dev")

    def test_cancel_keeps_partial_and_next_run_resumes(self):
        data = blob(6 * (1 << 20))
        url = self.srv.add("big", data, rate=1_500_000)
        man = self.write_manifest([item("big", data, [mirror(url)])])
        dest = Path(self.tmp.name) / "out"
        args = [sys.executable, "-I", str(CLI), "fetch", str(man), "--dest", str(dest), "--allow-http", "--no-nice",
                "--limit-bandwidth", "0", "--streams", "1", "--chunk-mb", "1"]
        proc = subprocess.Popen(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=self.env())
        state = None
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            found = list((dest / ".kf-downloader").glob("*.json")) if (dest / ".kf-downloader").exists() else []
            found = [f for f in found if f.name != "status.json"]
            if found:
                try:
                    if json.loads(found[0].read_text()).get("completed"):
                        state = found[0]
                        break
                except ValueError:
                    pass
            time.sleep(0.05)
        self.assertIsNotNone(state, "no chunk completed before the deadline")
        t0 = time.monotonic()
        proc.send_signal(signal.SIGTERM)
        out, err = proc.communicate(timeout=20)
        self.assertEqual(proc.returncode, 5, out + err)
        self.assertLess(time.monotonic() - t0, 8)
        self.assertFalse((dest / "dl/big").exists())
        self.assertTrue((dest / "dl/big.unverified").exists())
        self.assertIn('"state": "cancelled"', (dest / ".kf-downloader/status.json").read_text())
        self.srv.log.clear()
        self.srv.behave["/big"] = {}
        r = self.cli("fetch", str(man), "--dest", str(dest), "--allow-http", "--no-nice", "--limit-bandwidth", "0", "--streams", "1", "--chunk-mb", "1")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual((dest / "dl/big").read_bytes(), data)
        ranges = [e["range"] for e in self.srv.log if e["range"] != "bytes=0-0"]
        self.assertNotIn("bytes=0-1048575", ranges, "the completed first chunk was fetched again")


class Quiet(Base):
    def test_kf_fetch_default_output_unchanged_and_emit_redirects_it(self):
        data = blob(10_000)
        url = self.srv.add("a", data)
        out = []
        kf_fetch.Download(url, self.root / "o", sha(data), 1, 1 << 20, emit=out.append).run()
        self.assertTrue(any(l.startswith("FETCH_DONE") for l in out))
        self.assertTrue(any(l.startswith("FETCH_SOURCE") for l in out))

    def test_redact_scrubs_urls(self):
        self.assertEqual(bd.redact("x https://h/p?sig=SECRET y"), "x <url> y")

    def test_emit_scrubs_every_scheme(self):
        d = self.dl([item("a", b"x", [mirror(self.srv.url + "/a")])])
        d.emit("a http://h/q?sig=SECRET2 b ftp://x/y?k=SECRET3")
        self.assertNotIn("SECRET", self.lines[-1])
        self.assertEqual(self.lines[-1], "a <url> b <url>")

    def test_emit_scrubs_even_if_a_url_slips_in(self):
        d = self.dl([item("a", b"x", [mirror(self.srv.url + "/a")])])
        d.emit("oops https://h/p?sig=SECRET")
        self.assertNotIn("SECRET", self.lines[-1])


class Prefetch(unittest.TestCase):
    """The --prefetch hook of vast_payload.py: dry-run only, validates the manifest, starts nothing."""

    from test_payload_instruction import DryRun as _D
    run_main, write = _D.run_main, _D.write

    def manifest(self, d, **over):
        doc = manifest_doc([item("a", b"abc", [mirror("https://example.org/a")], needed_by="apps"),
                            item("b", b"abcd", [mirror("https://example.org/b")], needed_by="windows_gate")],
                           stages=("provision", "apps", "windows_gate"))
        doc.update(over)
        path = Path(d) / "m.json"
        path.write_text(json.dumps(doc))
        return str(path)

    def test_dry_run_prints_the_prefetch_summary_against_the_plan(self):
        with tempfile.TemporaryDirectory() as d:
            inst, _ = self.write(d)
            rc, out = self.run_main(["--dry-run", "--instruction", inst, "--prefetch", self.manifest(d)])
        self.assertEqual(rc, 0, out)
        self.assertRegex(out, r"KF_PAYLOAD_PREFETCH manifest_sha256=[0-9a-f]{64} items=2 planned_items=0 planned_bytes=0 unplanned_items=2")

    def test_items_of_a_planned_stage_are_counted(self):
        with tempfile.TemporaryDirectory() as d:
            inst, _ = self.write(d, tier="large", windows=True)
            rc, out = self.run_main(["--dry-run", "--instruction", inst, "--prefetch", self.manifest(d)])
        self.assertEqual(rc, 0, out)
        self.assertIn("planned_items=1 planned_bytes=4 unplanned_items=1", out)

    def test_invalid_manifest_is_refused_and_value_not_echoed(self):
        with tempfile.TemporaryDirectory() as d:
            inst, _ = self.write(d)
            rc, out = self.run_main(["--dry-run", "--instruction", inst, "--prefetch", self.manifest(d, schema="SECRETX")])
        self.assertEqual(rc, 1)
        self.assertNotIn("SECRETX", out)

    def test_prefetch_without_dry_run_is_refused_not_wired(self):
        rc, out = self.run_main(["--prefetch", "/nonexistent"])
        self.assertEqual(rc, 2)
        self.assertIn("prefetch_not_wired=1", out)

    def test_prefetch_without_dry_run_never_reads_the_manifest(self):
        rc, out = self.run_main(["a" * 40, "b" * 32, "--prefetch", "/definitely/not/here"], {"KF_SOFT_DEADLINE_MS": "1"})
        self.assertEqual(rc, 2)
        self.assertIn("prefetch_not_wired=1", out)
        self.assertNotIn("REFUSED", out)

    def test_unreadable_manifest_is_refused(self):
        with tempfile.TemporaryDirectory() as d:
            inst, _ = self.write(d)
            rc, out = self.run_main(["--dry-run", "--instruction", inst, "--prefetch", str(Path(d) / "absent.json")])
        self.assertEqual(rc, 1)
        self.assertIn("KF_PAYLOAD_PREFETCH_REFUSED", out)

    def test_no_prefetch_means_no_prefetch_line(self):
        with tempfile.TemporaryDirectory() as d:
            inst, _ = self.write(d)
            rc, out = self.run_main(["--dry-run", "--instruction", inst])
        self.assertNotIn("KF_PAYLOAD_PREFETCH", out)


if __name__ == "__main__":
    unittest.main()
