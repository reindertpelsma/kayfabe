"""B1b records real native return codes and preserves the exact fault guard."""

import io
import json
from pathlib import Path
import runpy
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import phase_xid_guard as G

SCRIPT = (
    Path(__file__).resolve().parents[3]
    / "traces/blackwell_ab_20261023/ROUND_B1b_1815/run_kit/native_wrap.py"
)
BAD = (
    "2026-10-23T16:00:00.000000+0000 host kernel: NVRM: Xid (PCI:0000:00:07): "
    "32, pid=4242, name=kayfabe-rm-ladd, channel 0x00000001 intr 00200000"
)


class NativeFindingWrapper(unittest.TestCase):
    def invoke(self, kind, rc, journal="", prior_fatal=False):
        writes = []
        child = SimpleNamespace(pid=4242, wait=lambda: rc)
        before = {"approved_native_xid_lines": [], "pci": "0000:00:07"}
        with (
            patch.object(sys, "argv", [str(SCRIPT), kind]),
            patch.object(Path, "exists", return_value=prior_fatal),
            patch.object(Path, "read_text", return_value=json.dumps(before)),
            patch.object(Path, "open", side_effect=lambda *a, **k: io.StringIO()),
            patch("os.readlink", return_value="/workspace/bench/native.log"),
            patch("subprocess.Popen", return_value=child) as start,
            patch.object(G.N, "kernel_snapshot", return_value=journal),
            patch.object(G.N, "bounded", return_value="raw native finding"),
            patch.object(G, "atomic_json", side_effect=lambda p, j: writes.append((str(p), j))),
        ):
            try:
                runpy.run_path(str(SCRIPT), run_name="__main__")
                code = 0
            except SystemExit as result:
                code = result.code
        return code, writes, start.call_count

    def test_fuzz_failure_keeps_real_rc_without_a_fatal_marker(self):
        code, writes, starts = self.invoke("--concurrent-fuzz", 1)
        self.assertEqual(code, 1)
        self.assertFalse(any(p.endswith("XID_STOP") for p, j in writes))

    def test_other_native_verdict_failure_is_also_reported_without_gating(self):
        code, writes, starts = self.invoke("--rpc-mixed-allocs", 1)
        self.assertEqual(code, 1)
        self.assertFalse(any(p.endswith("XID_STOP") for p, j in writes))

    def test_native_xid32_still_creates_a_fatal_marker(self):
        code, writes, starts = self.invoke("--concurrent-fuzz", 1, BAD)
        self.assertEqual(code, 7)
        self.assertTrue(any(p.endswith("XID_STOP") for p, j in writes))

    def test_native_unapproved_fault_is_fatal_even_with_zero_command_rc(self):
        code, writes, starts = self.invoke("--concurrent-fuzz", 0, BAD)
        self.assertEqual(code, 7)
        self.assertTrue(any(p.endswith("XID_STOP") for p, j in writes))

    def test_prior_fatal_refuses_without_starting_another_client(self):
        code, writes, starts = self.invoke("--concurrent-fuzz", 0, prior_fatal=True)
        self.assertEqual(code, 7)
        self.assertEqual(starts, 0)


if __name__ == "__main__":
    unittest.main()
