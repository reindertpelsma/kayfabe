#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""No QEMU user-network port forward listens on every interface (STATUS: LIVE, 2026-11-23; stdlib only).

Stage-2 security audit, finding S2-04 (docs/audits/2026-11-23-v3-stage2.md; stage 1 S1-28 and bug class 33): QEMU's
`hostfwd=[tcp|udp]:[hostaddr]:hostport-...` binds INADDR_ANY when `hostaddr` is empty, so a guest's ssh is reachable
from every network the host is on. Every forward in the scripts must name its host address: 127.0.0.1, or a variable
whose default is loopback (scripts/bench/windows/win_vm.sh `--ssh-bind`). An empty address, `0.0.0.0` or `*` fails.

Files under scripts/ci named test_*.py or *_mutants.py are skipped: they hold the bad form on purpose, as fixtures.
"""
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCAN_DIRS = ("scripts", "tools", "ci", "qemu", "crates")
SKIP_PARTS = {"node_modules", "__pycache__", "target", ".git"}
SUFFIXES = {".sh", ".py", ".js", ".rs", ".bash", ""}
HOSTFWD = re.compile(r"hostfwd=(?:tcp|udp)?:(?P<addr>[^:,\s\"']*):")
ANY = {"", "0.0.0.0", "*"}


def is_fixture(path):
    return path.parent.name == "ci" and path.parent.parent.name == "scripts" and (
        path.name.startswith("test_") or path.name.endswith("_mutants.py"))


def findings_in(text):
    """(line number, matched text) of every forward that listens on every interface."""
    out = []
    for n, line in enumerate(text.splitlines(), 1):
        for m in HOSTFWD.finditer(line):
            if m.group("addr") in ANY:
                out.append((n, m.group(0)))
    return out


def scan(root=ROOT):
    found = []
    for d in SCAN_DIRS:
        base = root / d
        if not base.is_dir():
            continue
        for path in sorted(base.rglob("*")):
            if not path.is_file() or SKIP_PARTS & set(path.parts) or path.suffix not in SUFFIXES or is_fixture(path):
                continue
            try:
                text = path.read_text(encoding="utf-8")
            except (UnicodeDecodeError, OSError):
                continue
            for n, what in findings_in(text):
                found.append("%s:%d %s" % (path.relative_to(root).as_posix(), n, what))
    return found


class HostfwdLoopback(unittest.TestCase):
    def test_the_matcher_catches_every_any_address_form(self):
        for bad in ("hostfwd=tcp::2222-:22", "hostfwd=tcp:0.0.0.0:2222-:22", "hostfwd=::2222-:22",
                    "hostfwd=udp::5353-:53", "user,id=n0,hostfwd=tcp::$PORT-:22", "hostfwd=tcp:*:1-:22"):
            self.assertEqual(len(findings_in(bad)), 1, bad)

    def test_the_matcher_accepts_loopback_and_a_named_variable(self):
        for good in ("hostfwd=tcp:127.0.0.1:2222-:22", "hostfwd=tcp:127.0.0.1:$PORT-:22",
                     "hostfwd=tcp:$SSH_BIND:$SSH_PORT-:22", "hostfwd=tcp:127.0.0.1:%d-10.0.2.15:22",
                     "hostfwd=tcp:127.0.0.1:{a.port}-:22"):
            self.assertEqual(findings_in(good), [], good)

    def test_fixture_files_are_the_only_skipped_ones(self):
        self.assertTrue(is_fixture(ROOT / "scripts/ci/test_propimage.py"))
        self.assertTrue(is_fixture(ROOT / "scripts/ci/propimage_mutants.py"))
        self.assertFalse(is_fixture(ROOT / "scripts/ci/install_flow.py"))
        self.assertFalse(is_fixture(ROOT / "scripts/bench/provision_bench_tree.sh"))

    def test_no_script_forwards_a_port_on_every_interface(self):
        found = scan()
        self.assertEqual(found, [], "bind the forward to 127.0.0.1:\n  " + "\n  ".join(found))


if __name__ == "__main__":
    unittest.main()
