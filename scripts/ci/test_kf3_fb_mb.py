"""Tests for the `fb-mb` launcher gate (scripts/ci/kf3_fb_mb.py).

Owner decision 2026-10-24: the kayfabe store size `fb-mb` is REQUIRED on the QEMU command line,
exactly like the RAM size (`qemu/hw/misc/kf3/kf3.c` refuses an unset size at realize). The gate is
a static scan of every launcher: a `-device kf3-gpu,...` string that does not contain `fb-mb=` is
a launcher that would no longer start. The last test runs it over this repository.
"""
import tempfile
import unittest
from pathlib import Path

import kf3_fb_mb as gate

ROOT = Path(__file__).resolve().parents[2]


def scan(text, name="scripts/x.sh"):
    return gate.scan_text(name, text)


class Emitters(unittest.TestCase):
    def test_a_device_string_without_fb_mb_is_a_violation(self):
        v = scan('qemu -device "kf3-gpu,id=kf0,bar1-size=1,display=on" -m 4G\n')
        self.assertEqual([x.line for x in v], [1])

    def test_a_device_string_with_fb_mb_passes(self):
        self.assertEqual(scan('qemu -device "kf3-gpu,id=kf0,fb-mb=4096,bar1-size=1"\n'), [])

    def test_a_script_variable_default_is_fine_it_is_still_on_the_command_line(self):
        self.assertEqual(scan('DEV="kf3-gpu,fb-mb=${WIN_FB_MB:-4096},id=kf0"\n'), [])

    def test_a_bare_device_name_is_a_violation(self):
        for line in (
            "qemu -device kf3-gpu -m 4G\n",
            'qemu -device "kf3-gpu"\n',
            "qemu -device=kf3-gpu\n",
            "args = ['-device', 'kf3-gpu']\n",
            "qemu -device kf3-gpu \\\n",
        ):
            self.assertEqual(len(scan(line)), 1, line)

    def test_python_argv_form(self):
        self.assertEqual(len(scan("c = ['-device','kf3-gpu,id=kf0,display=on']\n")), 1)
        self.assertEqual(scan("c = ['-device','kf3-gpu,fb-mb=4096,id=kf0']\n"), [])

    def test_a_name_that_is_not_a_device_definition_is_ignored(self):
        for line in (
            '-global "kf3-gpu.x-gsp-observer=$RUN/gsp.jsonl"\n',
            "if not cmd[-1].startswith('kf3-gpu,'):\n",
            'Identifier "kf3-gpu"\n',
            'grep -q "kf3-gpu" /proc/$QP/cmdline\n',
            "# -device kf3-gpu,display=on  (a comment showing the old form)\n",
        ):
            self.assertEqual(scan(line), [], line)

    def test_a_global_fb_mb_covers_the_file(self):
        text = '-global kf3-gpu.fb-mb=4096 -device "kf3-gpu,id=kf0"\n'
        self.assertEqual(scan(text), [])

    def test_the_fb_mb_must_be_a_property_not_a_substring(self):
        self.assertEqual(len(scan('-device "kf3-gpu,id=kf0,x-fb-mb=1"\n')), 1)
        self.assertEqual(len(scan('-device "kf3-gpu,id=kf0,nofb-mb=1"\n')), 1)

    def test_every_device_on_a_line_is_checked(self):
        v = scan('-device "kf3-gpu,fb-mb=1,id=a" -device "kf3-gpu,id=b"\n')
        self.assertEqual(len(v), 1)


class Repo(unittest.TestCase):
    def test_scan_root_walks_scripts_and_tools_without_git(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "scripts/bench").mkdir(parents=True)
            (root / "scripts/bench/bad.sh").write_text('qemu -device "kf3-gpu,id=kf0"\n')
            (root / "scripts/bench/good.sh").write_text('qemu -device "kf3-gpu,fb-mb=1,id=kf0"\n')
            (root / "tools").mkdir()
            (root / "tools/bad.py").write_text("c=['-device','kf3-gpu,id=kf0']\n")
            v = gate.scan_root(root)
            self.assertEqual(sorted(x.path for x in v), ["scripts/bench/bad.sh", "tools/bad.py"])

    def test_the_gate_itself_and_its_tests_are_not_scanned(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "scripts/ci").mkdir(parents=True)
            (root / "scripts/ci/test_kf3_fb_mb.py").write_text('x = "-device kf3-gpu"\n')
            self.assertEqual(gate.scan_root(root), [])

    def test_this_repository_has_no_launcher_without_fb_mb(self):
        v = gate.scan_root(ROOT)
        self.assertEqual(v, [], "launchers that would not start: " + "; ".join(map(str, v)))

    def test_the_scan_sees_the_known_launchers(self):
        # A scanner that finds nothing would pass the test above on any tree.
        n = gate.count_emitters(ROOT)
        self.assertGreaterEqual(n, 10, n)


if __name__ == "__main__":
    unittest.main()
