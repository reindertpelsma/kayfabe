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
import page_subset  # noqa: E402
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

    def test_hex_lines_from_the_guest_collector_round_trip(self):
        raw = header(0x193, [1, 2, 3, 4], 134370637168629955)
        line = "HDR C:/Windows/LiveKernelReports/WATCHDOG/x.dmp 1007878144 " + raw[: dump_header.HEADER_BYTES].hex()
        d = dump_header.parse_hexline(line)
        self.assertEqual(d["file"], "C:/Windows/LiveKernelReports/WATCHDOG/x.dmp")
        self.assertEqual(d["bytes"], 1007878144)
        self.assertEqual(d["bugcheck"], "0x193")
        self.assertEqual(d["system_time_utc"], "2026-10-21T13:41:56.862995+00:00")
        with self.assertRaises(ValueError):
            dump_header.parse_hexline("HDR only-two-fields")

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
        v = xid_policy.judge([XID % 99683], arm="native_x", qemu_pid=99683, code0_seen=True)
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

    def test_on_arms_get_the_same_exception_only_after_full_init_and_only_in_the_observed_shape(self):
        pair = [XID % 99683, (XID % 99683).replace("0x0000000c", "0x0e00000b")]
        self.assertEqual(xid_policy.judge(pair, "on_b", 99683, True)["verdict"], "EXPECTED")
        self.assertEqual(xid_policy.judge(pair, "on_b", 99683, False)["verdict"], "FATAL")
        self.assertEqual(xid_policy.judge(pair, "on_c3", 99683, True)["verdict"], "EXPECTED")
        # shape: one plain chid and one 0x0e0000xx chid, never two of a kind, never the same twice
        plain2 = [XID % 99683, (XID % 99683).replace("0x0000000c", "0x00000014")]
        self.assertEqual(xid_policy.judge(plain2, "on_b", 99683, True)["verdict"], "FATAL")
        dup = [XID % 99683, XID % 99683]
        self.assertEqual(xid_policy.judge(dup, "off_c", 99683, True)["verdict"], "FATAL")
        single = [XID % 99683]
        self.assertEqual(xid_policy.judge(single, "on_b", 99683, True)["verdict"], "EXPECTED")

    def test_no_xid_is_clean_and_non_xid_lines_are_ignored(self):
        self.assertEqual(xid_policy.judge([], "on_a", 1, False)["verdict"], "CLEAN")
        noise = ["2026-10-21T13:39:14.975419+0000 ubuntu kernel: NVRM:unknown NVRM ioctl command: 0xf0"]
        self.assertEqual(xid_policy.judge(noise, "on_a", 1, False)["verdict"], "CLEAN")


class XidFailClosed(unittest.TestCase):
    def test_unparsed_mmu_fault_is_fatal_even_in_exception_arm(self):
        line = "NVRM: Xid (PCI:0000:00:07): 31, pid=99683, name=qemu-system-x86, channel 0x0000000c, MMU Fault: ENGINE CE0 HUBCLIENT_CE1 faulted @ 0x60_00010000. Fault is of type FAULT_PTE ACCESS_TYPE_VIRT_READ"
        self.assertEqual(xid_policy.judge([line], "on_c3", 99683, True)["verdict"], "FATAL")

    def test_malformed_xid_cannot_be_ignored_as_clean_noise(self):
        line = "NVRM: Xid (PCI:0000:00:07): 32, pid=99683, name=qemu-system-x86, channel 0x0000000c intr UNKNOWN"
        self.assertEqual(xid_policy.judge([line], "off_c", 99683, True)["verdict"], "FATAL")

    def test_high_channel_must_have_only_an_eight_bit_suffix(self):
        line = (XID % 99683).replace("0x0000000c", "0x0e000100")
        self.assertEqual(xid_policy.judge([line], "on_c3", 99683, True)["verdict"], "FATAL")

    def test_arbitrary_channel_under_the_high_prefix_is_fatal(self):
        line = (XID % 99683).replace("0x0000000c", "0x0e123456")
        self.assertEqual(xid_policy.judge([line], "off_c", 99683, True)["verdict"], "FATAL")


ORACLE = (
    "# version=580.65.06 differences=3\n"
    "offset\tderived\tnative_oracle\theader\tb\tn\n"
    "0x680\t0x007b0200\t0x02ff01d0\tx\ty\tz\n"      # POSTCOMP: always derived
    "0x780\t0x01df0000\t0x01df21d0\tx\ty\tz\n"      # PRECOMP window 0, type 0 (CAPA)
    "0x788\t0x00000000\t0x000e200e\tx\ty\tz\n"      # window 0, type 2 (CAPC): derived zero
    "0x7a8\t0x00000000\t0x000e200e\tx\ty\tz\n"      # window 1, type 2
)
NATIVE = "\n".join(
    ["WORD 0x004 0x000000ff", "WORD 0x680 0x02ff01d0", "WORD 0x780 0x01df21d0", "WORD 0x784 0x0000da90",
     "WORD 0x788 0x000e200e", "WORD 0x7a8 0x000e200e", "# end words=6"]
)


class PageSubset(unittest.TestCase):
    def test_no_native_types_is_the_derived_page(self):
        w = page_subset.build(ORACLE, NATIVE, set())
        self.assertEqual(w[0x680], 0x007B0200)
        self.assertEqual(w[0x780], 0x01DF0000)
        self.assertNotIn(0x788, w)  # derived zero drops the word
        self.assertEqual(w[0x784], 0x0000DA90)  # equal in both: kept
        self.assertEqual(w[0x004], 0xFF)

    def test_selected_precomp_types_take_the_native_word_everywhere_and_nothing_else_moves(self):
        w = page_subset.build(ORACLE, NATIVE, {2})
        self.assertEqual(w[0x788], 0x000E200E)
        self.assertEqual(w[0x7A8], 0x000E200E)
        self.assertEqual(w[0x780], 0x01DF0000)
        self.assertEqual(w[0x680], 0x007B0200)  # POSTCOMP stays derived whatever the subset
        w0 = page_subset.build(ORACLE, NATIVE, {0})
        self.assertEqual(w0[0x780], 0x01DF21D0)
        self.assertNotIn(0x788, w0)

    def test_all_six_types_equals_the_previous_page_b_for_the_precomp_region(self):
        w = page_subset.build(ORACLE, NATIVE, {0, 1, 2, 3, 4, 5})
        self.assertEqual(w[0x780], 0x01DF21D0)
        self.assertEqual(w[0x788], 0x000E200E)

    def test_text_form_round_trips_and_bounds_the_type_set(self):
        text = page_subset.render(page_subset.build(ORACLE, NATIVE, set()), "t")
        self.assertIn("WORD 0x680 0x007b0200", text)
        self.assertTrue(text.rstrip().endswith("# end words=%d" % text.count("WORD ")))
        with self.assertRaises(ValueError):
            page_subset.build(ORACLE, NATIVE, {6})


if __name__ == "__main__":
    unittest.main()
