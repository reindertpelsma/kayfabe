"""Tests for the Blackwell resume capture helpers (dump header, key-line times, Xid policy).

Run: cd scripts/bench/blackwell && python3 -m unittest test_blackwell_capture
"""
import json
import struct
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

import dump_header  # noqa: E402
import keyline_times  # noqa: E402
import xid_policy  # noqa: E402


def header(code, params, filetime):
    """A DUMP_HEADER64-shaped buffer (public layout: code @56, P1..P4 @64..88, SystemTime @4008)."""
    b = bytearray(8192)
    b[0:8] = b"PAGEDU64"
    struct.pack_into("<I", b, 56, code)
    for i, p in enumerate(params):
        struct.pack_into("<Q", b, 64 + 8 * i, p)
    struct.pack_into("<Q", b, 4008, filetime)
    return bytes(b)


class DumpHeader(unittest.TestCase):
    def test_reads_code_all_four_parameters_and_time(self):
        raw = header(0x50, [0xFFFFDB047809FCC7, 2, 0xFFFFF80735EFA1B0, 2], 134370637657093532)
        d = dump_header.parse(raw)
        self.assertEqual(d["bugcheck"], "0x50")
        self.assertEqual(d["parameters"], ["0xffffdb047809fcc7", "0x2", "0xfffff80735efa1b0", "0x2"])
        self.assertEqual(d["system_time_utc"], "2026-10-21T13:42:45.709353+00:00")

    def test_refuses_a_wrong_signature_and_a_short_buffer(self):
        with self.assertRaises(ValueError):
            dump_header.parse(b"X" * 8192)
        with self.assertRaises(ValueError):
            dump_header.parse(b"PAGEDU64" + b"\0" * 100)


class KeylineTimes(unittest.TestCase):
    def test_maps_a_key_line_to_the_bracketing_rpc_times(self):
        log = b"aaa\nkf-rm: rpc-trace one\nkf3: PROBE-DUMP x\nkf-rm: rpc-trace two\nzzz\n"
        off_two = log.index(b"kf-rm: rpc-trace two") + len(b"kf-rm: rpc-trace two\n")
        off_one = log.index(b"kf3: PROBE-DUMP")
        rows = [
            {"ordinal": 0, "observed_utc": "2026-10-21T13:42:27.000000+00:00", "file_offset_after": off_one},
            {"ordinal": 1, "observed_utc": "2026-10-21T13:42:28.000000+00:00", "file_offset_after": off_two},
        ]
        with tempfile.TemporaryDirectory() as d:
            lp = Path(d) / "q.log"
            lp.write_bytes(log)
            out = keyline_times.key_lines(lp, rows, [b"PROBE-DUMP"])
        self.assertEqual(len(out), 1)
        r = out[0]
        self.assertEqual(r["line"], 3)
        self.assertEqual(r["offset"], off_one)
        self.assertEqual(r["after_ordinal"], 0)
        self.assertEqual(r["not_before_utc"], "2026-10-21T13:42:27.000000+00:00")
        self.assertEqual(r["not_after_utc"], "2026-10-21T13:42:28.000000+00:00")

    def test_a_line_before_the_first_row_has_no_lower_bound(self):
        rows = [{"ordinal": 0, "observed_utc": "2026-10-21T13:42:27.000000+00:00", "file_offset_after": 100}]
        with tempfile.TemporaryDirectory() as d:
            lp = Path(d) / "q.log"
            lp.write_bytes(b"x PROBE-DUMP y\n")
            out = keyline_times.key_lines(lp, rows, [b"PROBE-DUMP"])
        self.assertIsNone(out[0]["not_before_utc"])
        self.assertEqual(out[0]["not_after_utc"], "2026-10-21T13:42:27.000000+00:00")


XID = "2026-10-21T13:42:27.205488+0000 ubuntu kernel: NVRM: Xid (PCI:0000:00:07): 32, pid=%d, name=qemu-system-x86, channel 0x0000000c intr 00200000"


class XidPolicy(unittest.TestCase):
    def test_any_xid_in_a_non_exception_arm_is_fatal(self):
        v = xid_policy.judge([XID % 99683], arm="on_a", qemu_pid=99683, code0_seen=True)
        self.assertEqual(v["verdict"], "FATAL")

    def test_the_exact_exception_allows_two_matching_xids_after_code0_only(self):
        lines = [XID % 99683, (XID % 99683).replace("0x0000000c", "0x0e000007")]
        ok = xid_policy.judge(lines, arm="off_c", qemu_pid=99683, code0_seen=True)
        self.assertEqual(ok["verdict"], "EXPECTED")
        early = xid_policy.judge(lines, arm="off_c", qemu_pid=99683, code0_seen=False)
        self.assertEqual(early["verdict"], "FATAL")

    def test_third_other_code_other_pid_or_other_intr_is_fatal(self):
        base = XID % 99683
        three = [base, base, base]
        self.assertEqual(xid_policy.judge(three, "off_c", 99683, True)["verdict"], "FATAL")
        self.assertEqual(xid_policy.judge([base.replace(": 32,", ": 31,")], "off_c", 99683, True)["verdict"], "FATAL")
        self.assertEqual(xid_policy.judge([XID % 1], "off_c", 99683, True)["verdict"], "FATAL")
        self.assertEqual(xid_policy.judge([base.replace("00200000", "00800000")], "off_c", 99683, True)["verdict"], "FATAL")

    def test_no_xid_is_clean_and_non_xid_lines_are_ignored(self):
        self.assertEqual(xid_policy.judge([], "on_a", 1, False)["verdict"], "CLEAN")
        noise = ["2026-10-21T13:39:14.975419+0000 ubuntu kernel: NVRM:unknown NVRM ioctl command: 0xf0"]
        self.assertEqual(xid_policy.judge(noise, "on_a", 1, False)["verdict"], "CLEAN")


if __name__ == "__main__":
    unittest.main()
