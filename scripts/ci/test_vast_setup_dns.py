"""DNS fallback (2026-11-11): getent, resolvectl and ip are PATH stubs; nothing real is touched."""

import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parent / "kf_dns_fallback.sh"
# getent succeeds when the file `ok` exists (or `always`), and resolvectl dns creates `ok` unless `stuck`.
GETENT = '#!/bin/sh\necho "getent $*" >>"$KF_STUB/calls"\n[ -e "$KF_STUB/always" ] && exit 0\n' \
    '[ -e "$KF_STUB/ok" ] || exit 2\nexit 0\n'
RESOLVECTL = '#!/bin/sh\necho "resolvectl $*" >>"$KF_STUB/calls"\ncase "$1" in\n' \
    '  status) exit 0 ;;\n  dns) [ -e "$KF_STUB/stuck" ] || : >"$KF_STUB/ok"; exit 0 ;;\nesac\nexit 0\n'
IP = '#!/bin/sh\necho "default via 10.0.0.1 dev eth7 proto dhcp"\n'


class DnsFallback(unittest.TestCase):
    def run_script(self, *, always=False, stuck=False, resolvectl=True, resolv_ok_after_write=True):
        with tempfile.TemporaryDirectory() as d:
            stub = Path(d) / "stub"
            bin_dir = Path(d) / "bin"
            stub.mkdir()
            bin_dir.mkdir()
            tools = {"getent": GETENT, "ip": IP}
            if resolvectl:
                tools["resolvectl"] = RESOLVECTL
            for name, body in tools.items():
                (bin_dir / name).write_text(body)
                (bin_dir / name).chmod(0o755)
            if always:
                (stub / "always").touch()
            if stuck:
                (stub / "stuck").touch()
            conf = Path(d) / "resolv.conf"
            conf.write_text("nameserver 168.63.129.16\n")
            env = {
                "PATH": str(bin_dir) + os.pathsep + os.environ["PATH"],
                "KF_STUB": str(stub),
                "KF_DNS_RESOLV_CONF": str(conf),
            }
            if resolv_ok_after_write and not resolvectl:
                # Without resolvectl the only way to fix resolution is the file: getent reads it.
                (bin_dir / "getent").write_text(
                    '#!/bin/sh\necho "getent $*" >>"$KF_STUB/calls"\n'
                    'grep -q 1.1.1.1 "$KF_DNS_RESOLV_CONF" || exit 2\nexit 0\n'
                )
            p = subprocess.run(
                ["bash", str(SCRIPT)], env=env, capture_output=True, text=True, timeout=60
            )
            calls = (stub / "calls").read_text() if (stub / "calls").exists() else ""
            return p, calls, conf.read_text(), (Path(str(conf) + ".kf-orig").exists())

    def test_resolving_host_is_left_alone(self):
        p, calls, conf, orig = self.run_script(always=True)
        self.assertEqual(p.returncode, 0)
        self.assertEqual(p.stdout.strip(), "KF_DNS_FALLBACK used=0")
        self.assertNotIn("resolvectl", calls)
        self.assertEqual(conf, "nameserver 168.63.129.16\n")
        self.assertFalse(orig)

    def test_resolvectl_fixes_it_on_the_default_route_interface(self):
        p, calls, conf, _ = self.run_script()
        self.assertEqual(p.returncode, 0)
        self.assertEqual(p.stdout.strip(), "KF_DNS_FALLBACK used=1 ok=1")
        self.assertIn("resolvectl dns eth7 1.1.1.1 9.9.9.9", calls)
        self.assertEqual(conf, "nameserver 168.63.129.16\n")

    def test_resolv_conf_fallback_without_resolved(self):
        p, calls, conf, orig = self.run_script(resolvectl=False)
        self.assertEqual(p.returncode, 0)
        self.assertEqual(p.stdout.strip(), "KF_DNS_FALLBACK used=1 ok=1")
        self.assertEqual(conf, "nameserver 1.1.1.1\nnameserver 9.9.9.9\n")
        self.assertTrue(orig)

    def test_still_failing_reports_ok0_and_exits_zero(self):
        p, _, _, _ = self.run_script(stuck=True)
        self.assertEqual(p.returncode, 0)
        self.assertEqual(p.stdout.strip(), "KF_DNS_FALLBACK used=1 ok=0")

    def test_resolv_conf_path_still_failing_reports_ok0_and_exits_zero(self):
        p, _, conf, _ = self.run_script(resolvectl=False, resolv_ok_after_write=False)
        self.assertEqual(p.returncode, 0)
        self.assertEqual(p.stdout.strip(), "KF_DNS_FALLBACK used=1 ok=0")
        self.assertEqual(conf, "nameserver 1.1.1.1\nnameserver 9.9.9.9\n")

    def test_rerun_after_fix_is_idempotent(self):
        p, _, conf, _ = self.run_script(resolvectl=False)
        self.assertEqual(conf.count("nameserver 1.1.1.1"), 1)
        self.assertEqual(p.stdout.count("KF_DNS_FALLBACK"), 1)


if __name__ == "__main__":
    unittest.main()
