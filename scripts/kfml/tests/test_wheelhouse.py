"""The offline wheelhouse (2026-11-17): built on the host, verified and installed in an offline guest. GPU-free, no network."""
import hashlib
import io
import json
import os
import tarfile
import tempfile
import unittest
from pathlib import Path

from kfml import cli, datasets, setup_venv, wheelhouse as wh

PINS = datasets.load_pins()


class FakePip:
    """subprocess.run stand-in for `pip download`: writes wheels named after the lock into -d."""

    def __init__(self, rc=0, names=("torch-2.7.1-cp312-cp312-manylinux_2_28_x86_64.whl", "numpy-2.2.6-cp312-cp312-manylinux_2_28_x86_64.whl")):
        self.calls, self.rc, self.names = [], rc, names

    def __call__(self, cmd, **kw):
        self.calls.append(cmd)
        if self.rc == 0 and "download" in cmd:
            d = Path(cmd[cmd.index("-d") + 1])
            for n in self.names:
                (d / n).write_bytes(n.encode() * 100)

        class P:
            pass
        p = P()
        p.returncode, p.stdout, p.stderr = self.rc, "", ""
        return p


def fake_fetch(url, dest):
    Path(dest).write_bytes(b"x")


class Build(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.t = Path(self.tmp.name)

    def tearDown(self):
        self.tmp.cleanup()

    def pins_with_redist_files(self):
        """Redist archives whose pinned sha256 is that of the bytes fake_fetch writes."""
        import copy
        pins = copy.deepcopy(PINS)
        sha = hashlib.sha256(b"x").hexdigest()
        for fl in pins["cuda_redist"].values():
            for c in fl["components"].values():
                c["sha256"] = sha
        return pins

    def test_command_is_hash_checked_binary_only_and_names_the_guest_interpreter(self):
        host = wh.download_cmd("py", Path("lock.txt"), Path("d"))
        self.assertIn("--require-hashes", host)
        self.assertIn("--only-binary=:all:", host)
        self.assertNotIn("--platform", host)
        guest = wh.download_cmd("py", Path("lock.txt"), Path("d"), "3.12.3")
        self.assertEqual(guest[guest.index("--python-version") + 1], "3.12")
        self.assertEqual(guest[guest.index("--abi") + 1], "cp312")
        self.assertIn("--platform", guest)
        self.assertIn("--only-binary=:all:", guest)
        with self.assertRaises(wh.WheelhouseError):
            wh.py_tag("three")

    def test_every_manylinux_minor_up_to_the_guests_is_listed(self):
        """Measured 2026-11-17: pip does not expand --platform manylinux_2_28 to 2_27, and nvidia-*-cu12 12.8 wheels are tagged 2_27."""
        pl = wh.platforms("2.39")
        for n in (17, 24, 27, 28, 34, 39):
            self.assertIn("manylinux_2_%d_x86_64" % n, pl)
        self.assertNotIn("manylinux_2_40_x86_64", pl)
        self.assertIn("manylinux2014_x86_64", pl)
        self.assertNotIn("manylinux_2_36_x86_64", wh.platforms("2.35"))
        with self.assertRaises(wh.WheelhouseError):
            wh.platforms("3.1")
        cmd = wh.download_cmd("py", Path("l"), Path("d"), "3.12", "2.35")
        self.assertIn("manylinux_2_27_x86_64", cmd)
        self.assertNotIn("manylinux_2_36_x86_64", cmd)

    def test_build_writes_a_manifest_with_sizes_and_hashes(self):
        run = FakePip()
        m = wh.build(self.t / "wh", ["cu126", "cu128"], self.pins_with_redist_files(), guest_python="3.12", run=run, fetch_fn=fake_fetch)
        self.assertEqual(sorted(m["flavors"]), ["cu126", "cu128"])
        self.assertEqual(len(run.calls), 2)
        self.assertTrue(any(k.startswith("cu126/wheels/torch") for k in m["files"]))
        self.assertTrue(any(k.startswith("cu128/redist/") for k in m["files"]))
        f = m["files"]["cu126/wheels/numpy-2.2.6-cp312-cp312-manylinux_2_28_x86_64.whl"]
        self.assertEqual(f["sha256"], wh.sha256_file(self.t / "wh" / "cu126" / "wheels" / "numpy-2.2.6-cp312-cp312-manylinux_2_28_x86_64.whl"))
        self.assertEqual(m["total_bytes"], sum(x["size"] for x in m["files"].values()))
        self.assertEqual(m["flavors"]["cu126"]["lock_sha256"], PINS["flavors"]["cu126"]["lock_sha256"])
        self.assertEqual(wh.verify(self.t / "wh")["python"], "3.12")

    def test_pip_failure_is_an_error_not_a_partial_wheelhouse(self):
        with self.assertRaises(wh.WheelhouseError):
            wh.build(self.t / "wh", ["cu126"], PINS, run=FakePip(rc=1), with_redist=False)
        self.assertFalse((self.t / "wh" / "MANIFEST.json").exists())

    def test_an_unhashed_lock_is_refused_before_pip_runs(self):
        lock = self.t / "l.txt"
        lock.write_text("torch==1\n")
        run = FakePip()
        with self.assertRaises(wh.WheelhouseError):
            wh.build(self.t / "wh", ["cu126"], PINS, run=run, lock_for=lambda fl: lock, with_redist=False)
        self.assertEqual(run.calls, [])

    def test_a_redist_archive_with_the_wrong_hash_is_refused(self):
        with self.assertRaises(setup_venv.SetupError):
            wh.build(self.t / "wh", ["cu126"], PINS, run=FakePip(), fetch_fn=fake_fetch)  # real pins, fake bytes

    def test_datasets_are_fetched_into_data_and_listed(self):
        pins = {"datasets": {"d": {"url": "http://x", "filename": "d.bin", "sha256": hashlib.sha256(b"x").hexdigest()}},
                "flavors": PINS["flavors"], "cuda_redist": PINS["cuda_redist"]}
        import kfml.datasets as ds
        real = ds.fetch
        ds.fetch = lambda name, data_dir, pins=None, **k: (Path(data_dir).mkdir(parents=True, exist_ok=True), (Path(data_dir) / "d.bin").write_bytes(b"x"))
        try:
            m = wh.build(self.t / "wh", ["cu126"], pins, run=FakePip(), with_redist=False, datasets=True)
        finally:
            ds.fetch = real
        self.assertIn("data/d.bin", m["files"])


class Verify(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name) / "wh"
        wh.build(self.root, ["cu126"], PINS, guest_python="3.12", run=FakePip(), with_redist=False)
        self.wheel = next((self.root / "cu126" / "wheels").glob("torch*"))

    def tearDown(self):
        self.tmp.cleanup()

    def test_clean_copy_verifies(self):
        self.assertEqual(wh.verify(self.root, "cu126")["schema"], wh.SCHEMA)

    def test_altered_wheel_is_refused(self):
        self.wheel.write_bytes(self.wheel.read_bytes()[:-1] + b"Z")
        with self.assertRaises(wh.WheelhouseError):
            wh.verify(self.root)

    def test_truncated_copy_is_refused(self):
        self.wheel.write_bytes(self.wheel.read_bytes()[:10])
        with self.assertRaises(wh.WheelhouseError):
            wh.verify(self.root, "cu126")

    def test_missing_wheel_is_refused(self):
        self.wheel.unlink()
        with self.assertRaises(wh.WheelhouseError):
            wh.verify(self.root)

    def test_a_file_the_manifest_does_not_list_is_refused(self):
        (self.root / "cu126" / "wheels" / "evil-1.0-py3-none-any.whl").write_bytes(b"x")
        with self.assertRaises(wh.WheelhouseError) as cm:
            wh.verify(self.root)
        self.assertIn("unlisted", str(cm.exception))

    def test_unknown_flavour_and_missing_manifest(self):
        with self.assertRaises(wh.WheelhouseError):
            wh.verify(self.root, "cu128")
        with self.assertRaises(wh.WheelhouseError):
            wh.verify(self.root / "nope")

    def test_cli_verify_exit_codes(self):
        out = io.StringIO()
        self.assertEqual(cli.main(["wheelhouse", "--out", str(self.root), "--verify"], out), 0)
        self.assertEqual(json.loads(out.getvalue())["flavors"], ["cu126"])
        self.wheel.write_bytes(b"bad")
        self.assertEqual(cli.main(["wheelhouse", "--out", str(self.root), "--verify"], io.StringIO()), 1)


class OfflineSetup(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.t = Path(self.tmp.name)
        self.root = self.t / "wh"
        wh.build(self.root, ["cu126"], PINS, guest_python="3.12", run=FakePip(), with_redist=False)

    def tearDown(self):
        self.tmp.cleanup()

    def run_setup(self, **kw):
        from tests.test_setup_cli import FakeRun
        fr = FakeRun()
        r = setup_venv.setup(self.t / "v", flavor="cu126", run=fr, wheelhouse=str(self.root), **kw)
        return r, fr

    def test_install_is_offline_hashed_and_reads_only_the_wheelhouse(self):
        _, fr = self.run_setup()
        cmd = fr.calls[-1][0]
        self.assertIn("--no-index", cmd)
        self.assertEqual(cmd[cmd.index("--find-links") + 1], str(self.root / "cu126" / "wheels"))
        self.assertIn("--require-hashes", cmd)

    def test_the_offline_lock_has_no_index_option_and_every_other_line_unchanged(self):
        """pip honours --index-url inside a requirements file even with --no-index on the command line (measured)."""
        _, fr = self.run_setup()
        used = Path(fr.calls[-1][0][-1])
        self.assertNotEqual(used.name, "requirements.cu126.lock.txt")
        text = used.read_text()
        self.assertNotRegex(text, r"(?m)^\s*(--index-url|--extra-index-url|-i|--find-links|-f)\b")
        orig = (setup_venv.ROOT / "requirements.cu126.lock.txt").read_text().splitlines(True)
        self.assertEqual(text.splitlines(True), [l for l in orig if not l.startswith("--index-url") and not l.startswith("--extra-index-url")])
        self.assertIn("--hash=sha256:", text)
        self.assertTrue(setup_venv.has_hashes(text))

    def test_the_recorded_lock_hash_is_the_repository_locks_not_the_offline_copys(self):
        r, _ = self.run_setup()
        self.assertEqual(r["lock_sha256"], PINS["flavors"]["cu126"]["lock_sha256"])
        self.assertTrue((self.t / "v" / "kfml-lock.sha256").read_text().startswith(PINS["flavors"]["cu126"]["lock_sha256"]))

    def test_the_online_install_still_reads_the_lock_itself(self):
        from tests.test_setup_cli import FakeRun
        fr = FakeRun()
        setup_venv.setup(self.t / "v-on", flavor="cu126", run=fr)
        self.assertTrue(fr.calls[-1][0][-1].endswith("requirements.cu126.lock.txt"))

    def test_a_tampered_wheelhouse_is_refused_before_pip_runs(self):
        from tests.test_setup_cli import FakeRun
        next((self.root / "cu126" / "wheels").glob("numpy*")).write_bytes(b"bad")
        fr = FakeRun()
        with self.assertRaises(wh.WheelhouseError):
            setup_venv.setup(self.t / "v", flavor="cu126", run=fr, wheelhouse=str(self.root))
        self.assertFalse(any("pip" in c[0] for c in fr.calls if "install" in c[0]))

    def test_an_unhashed_lock_cannot_install_offline(self):
        from unittest.mock import patch
        from tests.test_setup_cli import FakeRun
        r = self.t / "locks"
        r.mkdir()
        for fl, fn in setup_venv.FLAVORS.items():
            (r / fn).write_text("torch==2.7.1+%s\n" % fl)
        with patch.object(setup_venv, "ROOT", r):
            with self.assertRaises(setup_venv.SetupError):
                setup_venv.setup(self.t / "v2", flavor="cu126", run=FakeRun(), allow_unhashed=True, wheelhouse=str(self.root))

    def test_nvcc_comes_from_the_wheelhouse_redist_never_the_network(self):
        arc_src = self.t / "a.tar.xz"
        inner = self.t / "pkg" / "bin"
        inner.mkdir(parents=True)
        (inner / "nvcc").write_text("#!/bin/sh\n")
        with tarfile.open(arc_src, "w:xz") as tf:
            tf.add(self.t / "pkg", arcname="cuda_x-archive")
        sha = hashlib.sha256(arc_src.read_bytes()).hexdigest()
        pins = {"cuda_redist": {"cu126": {"base_url": "https://never.invalid/", "components": {"cuda_x": {"path": "x/a.tar.xz", "sha256": sha}}}}}
        redist = self.t / "redist"
        redist.mkdir()
        (redist / "a.tar.xz").write_bytes(arc_src.read_bytes())
        (self.t / "v3").mkdir()
        home = setup_venv.install_nvcc(self.t / "v3", "cu126", pins, archive_dir=redist)
        self.assertTrue((home / "bin" / "nvcc").exists())
        with self.assertRaises(setup_venv.SetupError):  # an archive that is not in the wheelhouse is not fetched
            (self.t / "v4").mkdir()
            setup_venv.install_nvcc(self.t / "v4", "cu126", pins, archive_dir=self.t / "empty")


class OfflineData(unittest.TestCase):
    def test_dataset_from_a_local_copy_is_still_checked_against_the_pin(self):
        with tempfile.TemporaryDirectory() as d:
            d = Path(d)
            good = b"hello"
            pins = {"datasets": {"x": {"url": "https://never.invalid/x", "filename": "x.bin", "sha256": hashlib.sha256(good).hexdigest()}}}
            (d / "src").mkdir()
            (d / "src" / "x.bin").write_bytes(good)
            r = datasets.fetch("x", d / "data", pins, source_dir=d / "src")
            self.assertEqual(Path(r["path"]).read_bytes(), good)
            (d / "src" / "x.bin").write_bytes(b"tampered")
            with self.assertRaises(datasets.HashMismatch):
                datasets.fetch("x", d / "data2", pins, source_dir=d / "src")
            with self.assertRaises(FileNotFoundError):
                datasets.fetch("x", d / "data3", pins, source_dir=d / "nowhere")


if __name__ == "__main__":
    unittest.main()


class ParallelDownload(unittest.TestCase):
    """Ranged parallel download against a local HTTP server that honours Range (stdlib), including resets and a server without ranges."""

    @classmethod
    def serve(cls, payload, ranges=True, fail_first=0):
        import http.server
        import threading

        state = {"fail": fail_first, "hits": 0}

        class H(http.server.BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def head(self):
                self.send_response(200)
                self.send_header("Content-Length", str(len(payload)))
                if ranges:
                    self.send_header("Accept-Ranges", "bytes")
                self.end_headers()

            def do_HEAD(self):
                self.head()

            def do_GET(self):
                state["hits"] += 1
                rng = self.headers.get("Range")
                if rng and ranges:
                    if state["fail"] > 0:
                        state["fail"] -= 1
                        self.connection.close()  # a reset in the middle of the transfer
                        return
                    a, b = [int(x) for x in rng.split("=")[1].split("-")]
                    body = payload[a:b + 1]
                    self.send_response(206)
                    self.send_header("Content-Range", "bytes %d-%d/%d" % (a, b, len(payload)))
                else:
                    body = payload
                    self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        return srv, state

    def test_parts_are_reassembled_in_order(self):
        payload = bytes(range(256)) * 4001
        srv, st = self.serve(payload)
        try:
            with tempfile.TemporaryDirectory() as d:
                n = datasets.download_parallel("http://127.0.0.1:%d/x" % srv.server_address[1], Path(d) / "f", 6, sleep=lambda s: None)
                self.assertEqual(n, len(payload))
                self.assertEqual((Path(d) / "f").read_bytes(), payload)
            self.assertGreaterEqual(st["hits"], 6)
        finally:
            srv.shutdown()

    def test_a_reset_is_retried_and_the_result_is_still_correct(self):
        payload = os.urandom(300000)
        srv, st = self.serve(payload, fail_first=3)
        try:
            with tempfile.TemporaryDirectory() as d:
                datasets.download_parallel("http://127.0.0.1:%d/x" % srv.server_address[1], Path(d) / "f", 4, sleep=lambda s: None)
                self.assertEqual((Path(d) / "f").read_bytes(), payload)
        finally:
            srv.shutdown()

    def test_a_server_that_keeps_failing_is_an_error_not_a_short_file(self):
        srv, _ = self.serve(os.urandom(1000), fail_first=10 ** 6)
        try:
            with tempfile.TemporaryDirectory() as d:
                with self.assertRaises(OSError):
                    datasets.download_parallel("http://127.0.0.1:%d/x" % srv.server_address[1], Path(d) / "f", 3, retries=2, sleep=lambda s: None)
        finally:
            srv.shutdown()

    def test_no_range_support_means_none_so_the_caller_downloads_sequentially(self):
        srv, _ = self.serve(b"abc" * 100, ranges=False)
        try:
            with tempfile.TemporaryDirectory() as d:
                self.assertIsNone(datasets.download_parallel("http://127.0.0.1:%d/x" % srv.server_address[1], Path(d) / "f", 4))
        finally:
            srv.shutdown()

    def test_fetch_uses_parallel_for_a_pin_that_asks_and_still_checks_the_sha(self):
        payload = os.urandom(120000)
        srv, _ = self.serve(payload)
        try:
            with tempfile.TemporaryDirectory() as d:
                url = "http://127.0.0.1:%d/x" % srv.server_address[1]
                pins = {"datasets": {"x": {"url": url, "filename": "x.bin", "sha256": hashlib.sha256(payload).hexdigest(), "parallel": 4}}}
                r = datasets.fetch("x", Path(d) / "data", pins)
                self.assertEqual(Path(r["path"]).read_bytes(), payload)
                bad = {"datasets": {"x": dict(pins["datasets"]["x"], sha256="0" * 64)}}
                with self.assertRaises(datasets.HashMismatch):
                    datasets.fetch("x", Path(d) / "data2", bad)
                self.assertFalse(list((Path(d) / "data2").glob("x.bin*")))  # nothing unverified is left behind as the dataset
        finally:
            srv.shutdown()

    def test_cifar_pin_asks_for_six_connections_not_more(self):
        self.assertEqual(PINS["datasets"]["cifar10_binary"]["parallel"], 6)
        self.assertNotIn("parallel", PINS["datasets"]["tinyshakespeare"])


class Licenses(unittest.TestCase):
    """wheelhouse-licenses.json: information from METADATA, proprietary flagged, never a build failure, never an upload."""

    def make(self, d):
        from tests.test_licenses_bundle import make_wheel
        w = d / "cu126" / "wheels"
        w.mkdir(parents=True)
        make_wheel(w, "torch", "2.7.1+cu126", ["License: BSD-3-Clause"], tags="cp310-cp310-manylinux_2_28_x86_64")
        make_wheel(w, "numpy", "2.2.6", ["License-Expression: BSD-3-Clause"], tags="cp310-cp310-manylinux_2_28_x86_64")
        make_wheel(w, "nvidia_cublas_cu12", "12.6.4.1", ["License: NVIDIA Proprietary Software"], {"nvidia/cublas/lib/libcublas.so.12": b"x"})
        make_wheel(w, "triton", "3.3.1", ["License: MIT"], {"triton/backends/nvidia/bin/ptxas": b"x", "triton/x.py": b"y"})
        make_wheel(w, "mystery", "1.0", ["Summary: no licence anywhere"])
        return d

    def test_every_wheel_is_listed_with_its_licence_and_the_proprietary_ones_are_flagged(self):
        with tempfile.TemporaryDirectory() as d:
            doc = wh.licenses_manifest(self.make(Path(d)), ["cu126"])
        by = {w["name"]: w for w in doc["wheels"]}
        self.assertEqual(by["torch"]["license_spdx"], "BSD-3-Clause")
        self.assertFalse(by["torch"]["proprietary"])  # a +cu local tag is a build, not a vendor binary
        self.assertFalse(by["numpy"]["proprietary"])
        self.assertTrue(by["nvidia_cublas_cu12"]["proprietary"])
        self.assertTrue(by["triton"]["proprietary"])
        self.assertIn("ptxas", by["triton"]["proprietary_reason"])
        self.assertEqual(by["triton"]["license_spdx"], "MIT")  # MIT itself, proprietary only in what it bundles
        self.assertEqual(doc["proprietary_count"], 2)
        self.assertFalse(doc["stored_by_us"])

    def test_an_unclassified_wheel_is_listed_not_a_failure(self):
        with tempfile.TemporaryDirectory() as d:
            doc = wh.licenses_manifest(self.make(Path(d)), ["cu126"])
        self.assertIn("mystery", doc["unclassified"])

    def test_build_writes_the_file_next_to_the_manifest_and_verify_ignores_it(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d) / "wh"
            m = wh.build(root, ["cu126"], PINS, run=FakePip(), with_redist=False)
            self.assertTrue((root / "wheelhouse-licenses.json").is_file())
            self.assertEqual(m["licenses"]["file"], "wheelhouse-licenses.json")
            wh.verify(root)
            (root / "wheelhouse-licenses.json").write_text("{}")  # information only: not part of the integrity check
            wh.verify(root)
