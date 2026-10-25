"""HSCE is a copy-client spelling; every existing source proof remains required."""

import copy
import re
import unittest

import test_guest_xid_guard as guest_fixture
import test_native_xid_guard as native_fixture

G = native_fixture.G

# CODE READ: ogkm-595.91.07,
# src/nvidia/src/kernel/gpu/fifo/arch/turing/kernel_fifo_tu102.c:378-401
# emits HUBCLIENT_HSCE0..9 for NV_PFAULT_CLIENT_HUB_HSCE0..9. The public
# src/common/inc/swref/published/turing/tu102/dev_fault.h:134-143 defines those IDs.
# These are alias-spelling parser fixtures; no captured value enters product code.


def hsce(text):
    return re.sub(r"HUBCLIENT_CE([0-9]+)", r"HUBCLIENT_HSCE\1", text)


class HsceAdmission(unittest.TestCase):
    def setUp(self):
        n = native_fixture.NativeXidGuardTests()
        n.setUp()
        self.native = n
        self.native_journal = hsce(n.journal)
        q = guest_fixture.GuestGuardTests()
        q.setUp()
        self.guest = q
        self.guest_lines = q.lines[:5] + [hsce(line) for line in q.lines[5:]]

    def native_approve(self, journal=None):
        return G.approve_native(
            self.native_journal if journal is None else journal,
            self.native.defer,
            self.native.missing,
        )

    def guest_approve(self, lines=None, context=None, **kwargs):
        return G.approve_guest(
            "\n".join(self.guest_lines if lines is None else lines) + "\n",
            self.guest.base,
            kwargs.get("serial", self.guest.serial),
            kwargs.get("qlog", self.guest.qlog),
            self.guest.context if context is None else context,
        )

    def test_hsce_full_native_control_set_accepts_exact_five(self):
        self.assertEqual(self.native_approve(), G.xid_lines(self.native_journal))

    def test_hsce_native_defer_arm_accepts_exact_four(self):
        lines = G.xid_lines(self.native_journal)[:4]
        self.assertEqual(
            G.approve_native_arm("\n".join(lines), "--defer-liveness", self.native.defer),
            lines,
        )

    def test_hsce_guest_defer_accepts_same_owned_proof(self):
        self.assertEqual(self.guest_approve(), tuple(self.guest_lines))

    def test_hsce_guest_missing_accepts_existing_source_fault_class(self):
        case = self.guest.missing()
        case["lines"][-1] = hsce(case["lines"][-1])
        self.assertEqual(
            G.approve_guest(
                "\n".join(case["lines"]),
                self.guest.base,
                case["serial"],
                case["qlog"],
                case["context"],
            ),
            tuple(case["lines"]),
        )

    def test_hsce_zero_has_copy_client_spelling(self):
        self.assertTrue(G.copy_engine_client("HUBCLIENT_HSCE0"))

    def test_hsce_multi_digit_has_copy_client_spelling(self):
        self.assertTrue(G.copy_engine_client("HUBCLIENT_HSCE12"))

    def test_original_ce_zero_still_has_copy_client_spelling(self):
        self.assertTrue(G.copy_engine_client("HUBCLIENT_CE0"))

    def test_original_ce_multi_digit_still_has_copy_client_spelling(self):
        self.assertTrue(G.copy_engine_client("HUBCLIENT_CE12"))

    def test_hsce_requires_numeric_suffix(self):
        for value in ("HUBCLIENT_HSCE", "HUBCLIENT_HSCE-1", "HUBCLIENT_HSCEx"):
            with self.subTest(value=value):
                self.assertFalse(G.copy_engine_client(value))

    def test_hsce_rejects_prefix_suffix_and_other_clients(self):
        for value in ("xHUBCLIENT_HSCE0", "HUBCLIENT_HSCE0x", "HUBCLIENT_ESC"):
            with self.subTest(value=value):
                self.assertFalse(G.copy_engine_client(value))

    def test_hsce_native_wrong_channel_refuses(self):
        with self.assertRaises(G.GuardRefused):
            self.native_approve(self.native_journal.replace("channel 0x00000005", "channel 0x9"))

    def test_hsce_guest_wrong_channel_refuses(self):
        with self.assertRaises(G.GuardRefused):
            self.guest_approve(qlog=self.guest.qlog.replace("twin 0x1b", "twin 0x2b"))

    def test_hsce_native_wrong_va_refuses(self):
        with self.assertRaises(G.GuardRefused):
            self.native_approve(self.native_journal.replace("0x60_00010000", "0x60_00020000"))

    def test_hsce_guest_wrong_va_refuses(self):
        lines = [line.replace("0x60_00010000", "0x60_00020000") for line in self.guest_lines]
        with self.assertRaises(G.GuardRefused):
            self.guest_approve(lines)

    def test_hsce_native_wrong_fault_class_refuses(self):
        with self.assertRaises(G.GuardRefused):
            self.native_approve(self.native_journal.replace("FAULT_PTE", "FAULT_PDE", 1))

    def test_hsce_guest_wrong_fault_class_refuses(self):
        lines = self.guest_lines[:5] + [
            line.replace("FAULT_PTE", "FAULT_PDE") for line in self.guest_lines[5:]
        ]
        with self.assertRaises(G.GuardRefused):
            self.guest_approve(lines)

    def test_hsce_native_extra_record_refuses(self):
        with self.assertRaises(G.GuardRefused):
            self.native_approve(self.native_journal + G.xid_lines(self.native_journal)[0] + "\n")

    def test_hsce_guest_extra_record_refuses(self):
        with self.assertRaises(G.GuardRefused):
            self.guest_approve(self.guest_lines + [self.guest_lines[-1]])

    def test_hsce_guest_wrong_defer_pid_refuses(self):
        with self.assertRaises(G.GuardRefused):
            self.guest_approve(context=dict(self.guest.context, pid=91948))

    def test_hsce_guest_wrong_missing_pid_refuses(self):
        case = copy.deepcopy(self.guest.missing())
        case["lines"][-1] = hsce(case["lines"][-1])
        case["context"]["pid"] += 1
        with self.assertRaises(G.GuardRefused):
            G.approve_guest(
                "\n".join(case["lines"]),
                self.guest.base,
                case["serial"],
                case["qlog"],
                case["context"],
            )

    def test_hsce_native_non_ce_client_refuses(self):
        with self.assertRaises(G.GuardRefused):
            self.native_approve(self.native_journal.replace("HUBCLIENT_HSCE0", "HUBCLIENT_ESC"))

    def test_hsce_guest_non_ce_client_refuses(self):
        lines = [line.replace("HUBCLIENT_HSCE", "HUBCLIENT_ESC") for line in self.guest_lines]
        with self.assertRaises(G.GuardRefused):
            self.guest_approve(lines)


if __name__ == "__main__":
    unittest.main()
