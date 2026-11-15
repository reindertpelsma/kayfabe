#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of scripts/ci/qemu_source.py (STATUS: LIVE, 2026-11-15)."""
import functools
import hashlib
import http.server
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import unittest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import qemu_source as q  # noqa: E402

BODY = b"pretend tarball bytes" * 100
NAME = "qemu-1.2.3.tar.xz"


def pin_text(body=BODY, size=None):
    return f"# c\n{NAME} sha256={hashlib.sha256(body).hexdigest()} size={len(body) if size is None else size}\n"


class Base(unittest.TestCase):
    def setUp(self):
        t = tempfile.TemporaryDirectory()
        self.addCleanup(t.cleanup)
        self.tmp = Path(t.name)
        self.pin = self.tmp / "pin"
        self.pin.write_text(pin_text())
        self.dest = self.tmp / "dest"
        self.src = self.tmp / "local.tar.xz"
        self.src.write_bytes(BODY)

    def serve(self, body):
        d = self.tmp / "www"
        d.mkdir(exist_ok=True)
        (d / NAME).write_bytes(body)
        h = functools.partial(http.server.SimpleHTTPRequestHandler, directory=str(d))
        h.log_message = lambda *a: None
        srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), h)
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        self.addCleanup(srv.server_close)
        self.addCleanup(srv.shutdown)
        return f"http://127.0.0.1:{srv.server_address[1]}/"

    def leftovers(self):
        return sorted(p.name for p in self.dest.glob("*")) if self.dest.exists() else []


class Verify(Base):
    def test_matching_file_passes(self):
        self.assertEqual(q.main(["--pin", str(self.pin), "verify", str(self.src)]), 0)

    def test_wrong_hash_refuses(self):
        self.src.write_bytes(b"X" + BODY[1:])
        self.assertEqual(q.main(["--pin", str(self.pin), "verify", str(self.src)]), 1)

    def test_truncated_file_refuses(self):
        self.src.write_bytes(BODY[:-1])
        self.assertEqual(q.main(["--pin", str(self.pin), "verify", str(self.src)]), 1)

    def test_missing_file_refuses(self):
        self.assertEqual(q.main(["--pin", str(self.pin), "verify", str(self.tmp / "nope")]), 1)

    def test_right_size_wrong_hash_with_pin_size_matching_refuses(self):
        self.src.write_bytes(b"Y" * len(BODY))
        with self.assertRaises(q.Refused):
            q.check(self.src, hashlib.sha256(BODY).hexdigest(), len(BODY))


class PinFile(Base):
    def test_version_comes_from_the_pin(self):
        self.assertEqual(q.read_pin(self.pin)[1], "1.2.3")

    def test_malformed_pins_refuse(self):
        for bad in ["", NAME + " sha256=ab size=3\n", NAME + " size=3\n",
                    pin_text() + pin_text(), f"evil.tar sha256={'0' * 64} size=3\n",
                    f"{NAME} sha256={'0' * 64} size=0\n"]:
            self.pin.write_text(bad)
            with self.assertRaises(q.Refused, msg=bad):
                q.read_pin(self.pin)

    def test_missing_pin_refuses(self):
        with self.assertRaises(q.Refused):
            q.read_pin(self.tmp / "none")

    def test_the_committed_pin_is_well_formed_and_names_10_2(self):
        name, ver, sha, size = q.read_pin()
        self.assertTrue(ver.startswith("10.2."))
        self.assertEqual((len(sha), size > 10_000_000), (64, True))


class FromFile(Base):
    def test_override_path_good_file_is_installed(self):
        out = q.fetch(self.dest, self.pin, from_file=self.src)
        self.assertEqual(out.read_bytes(), BODY)
        self.assertEqual(self.leftovers(), [NAME])

    def test_override_path_is_hash_checked(self):
        self.src.write_bytes(b"Z" * len(BODY))
        with self.assertRaises(q.Refused):
            q.fetch(self.dest, self.pin, from_file=self.src)
        self.assertEqual(self.leftovers(), [])

    def test_override_path_truncated_refuses(self):
        self.src.write_bytes(BODY[:10])
        with self.assertRaises(q.Refused):
            q.fetch(self.dest, self.pin, from_file=self.src)
        self.assertEqual(self.leftovers(), [])

    def test_override_path_missing_refuses(self):
        with self.assertRaises(q.Refused):
            q.fetch(self.dest, self.pin, from_file=self.tmp / "nope")
        self.assertEqual(self.leftovers(), [])

    def test_bad_override_replaces_a_cached_good_copy_with_nothing(self):
        q.fetch(self.dest, self.pin, from_file=self.src)
        self.src.write_bytes(b"Z" * len(BODY))
        with self.assertRaises(q.Refused):
            q.fetch(self.dest, self.pin, from_file=self.src)
        self.assertEqual(self.leftovers(), [])


class Download(Base):
    def test_good_download_installs(self):
        out = q.fetch(self.dest, self.pin, base_url=self.serve(BODY))
        self.assertEqual(out.read_bytes(), BODY)
        self.assertEqual(self.leftovers(), [NAME])

    def test_tampered_download_refuses_and_leaves_nothing(self):
        with self.assertRaises(q.Refused):
            q.fetch(self.dest, self.pin, base_url=self.serve(b"Q" + BODY[1:]))
        self.assertEqual(self.leftovers(), [])

    def test_truncated_download_refuses(self):
        with self.assertRaises(q.Refused):
            q.fetch(self.dest, self.pin, base_url=self.serve(BODY[:-5]))
        self.assertEqual(self.leftovers(), [])

    def test_oversize_download_refuses(self):
        with self.assertRaises(q.Refused):
            q.fetch(self.dest, self.pin, base_url=self.serve(BODY + b"x" * 5000))
        self.assertEqual(self.leftovers(), [])

    def test_unreachable_server_refuses(self):
        with self.assertRaises(q.Refused):
            q.fetch(self.dest, self.pin, base_url="http://127.0.0.1:9/")
        self.assertEqual(self.leftovers(), [])

    def test_cached_good_copy_needs_no_network(self):
        q.fetch(self.dest, self.pin, from_file=self.src)
        out = q.fetch(self.dest, self.pin, base_url="http://127.0.0.1:9/")
        self.assertEqual(out.read_bytes(), BODY)

    def test_cached_corrupt_copy_is_not_used(self):
        self.dest.mkdir()
        (self.dest / NAME).write_bytes(b"corrupt")
        with self.assertRaises(q.Refused):
            q.fetch(self.dest, self.pin, base_url="http://127.0.0.1:9/")
        self.assertEqual(self.leftovers(), [])


class Cli(Base):
    def run_cli(self, *args):
        return subprocess.run([sys.executable, "-I", str(HERE / "qemu_source.py"), "--pin", str(self.pin), *args],
                              capture_output=True, text=True, env={"PATH": os.environ["PATH"]})

    def test_cli_exit_codes_and_output(self):
        r = self.run_cli("fetch", str(self.dest), "--from-file", str(self.src))
        self.assertEqual((r.returncode, r.stdout.strip()), (0, str(self.dest / NAME)))
        self.assertEqual(self.run_cli("version").stdout.strip(), "1.2.3")
        self.src.write_bytes(b"bad")
        r = self.run_cli("verify", str(self.src))
        self.assertEqual(r.returncode, 1)
        self.assertIn("QEMU_SOURCE_REFUSED", r.stderr)

    def test_no_environment_variable_disables_the_check(self):
        self.src.write_bytes(b"bad")
        env = {"PATH": os.environ["PATH"], "KF_SKIP_QEMU_CHECK": "1", "QEMU_SKIP_VERIFY": "1", "KF_INSECURE": "1"}
        r = subprocess.run([sys.executable, "-I", str(HERE / "qemu_source.py"), "--pin", str(self.pin),
                            "verify", str(self.src)], capture_output=True, env=env)
        self.assertEqual(r.returncode, 1)


class Callers(unittest.TestCase):
    """The download sites must go through the helper and must not curl download.qemu.org themselves."""

    def test_no_script_downloads_qemu_directly(self):
        root = HERE.parents[1]
        for rel in ["scripts/bench/provision_bench_tree.sh", "scripts/ci/vast_box_main.sh", "scripts/bench/build_kf3.sh"]:
            text = (root / rel).read_text()
            self.assertNotIn("download.qemu.org", text, rel)
        for rel in ["scripts/bench/provision_bench_tree.sh", "scripts/ci/vast_box_main.sh"]:
            self.assertIn("qemu_source.py", (root / rel).read_text(), rel)


if __name__ == "__main__":
    unittest.main()
