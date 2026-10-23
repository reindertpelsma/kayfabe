"""Console publication may lag; attested mismatches and absolute bounds still refuse."""

import base64
import hashlib
from pathlib import Path
import struct
import sys
import unittest

sys.path.insert(0, str(Path(__file__).parent))
try:
    import console_pin as C
except ImportError:
    C = None


def key(byte):
    raw = struct.pack(">I", 11) + b"ssh-ed25519" + struct.pack(">I", 32) + bytes([byte]) * 32
    return base64.b64encode(raw).decode()


KEY = key(1)
OTHER = key(2)
FP = "SHA256:" + base64.b64encode(hashlib.sha256(base64.b64decode(KEY)).digest()).decode().rstrip(
    "="
)


class ConsolePin(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(C, "console publication readiness helper missing")

    def wait(self, snapshots, elapsed=600, factor=4, costs=None, stage_started=None):
        clock = [0.0]
        calls, sleeps, receipts = [], [], []

        def fetch(timeout):
            index = len(calls)
            calls.append((clock[0], timeout))
            if costs:
                clock[0] += costs[min(index, len(costs) - 1)]
            return snapshots[min(index, len(snapshots) - 1)]

        def sleep(seconds):
            sleeps.append(seconds)
            clock[0] += seconds

        result = C.wait_for_pin(
            KEY,
            fetch,
            elapsed_since_create=elapsed,
            factor=factor,
            stage_started_elapsed=stage_started,
            clock=lambda: clock[0],
            sleep=sleep,
            record=receipts.append,
        )
        return result, calls, sleeps, receipts

    def test_absent_then_present_within_bound_passes(self):
        result, calls, sleeps, receipts = self.wait(["booting", "ssh-ed25519 " + KEY])
        self.assertEqual(result["fingerprint"], FP)
        self.assertEqual([at for at, _ in calls], [0, 20])
        self.assertEqual([r["state"] for r in receipts], ["absent", "match"])

    def test_several_missing_console_reads_do_not_refuse_early(self):
        result, calls, sleeps, receipts = self.wait(["", "", "", FP])
        self.assertEqual(result["fingerprint"], FP)
        self.assertEqual([at for at, _ in calls], [0, 20, 40, 60])

    def test_present_different_ed25519_pin_refuses_immediately(self):
        with self.assertRaises(C.PinMismatch):
            self.wait(["ssh-ed25519 " + OTHER, FP])

    def test_different_fingerprint_refuses_immediately(self):
        with self.assertRaises(C.PinMismatch):
            self.wait(["256 SHA256:BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB host (ED25519)", FP])

    def test_conflicting_published_ed25519_keys_refuse(self):
        with self.assertRaises(C.PinMismatch):
            self.wait(["ssh-ed25519 " + KEY + "\nssh-ed25519 " + OTHER])

    def test_absent_past_adaptive_stage_bound_refuses(self):
        with self.assertRaises(C.PinExpired):
            self.wait(["booting"], elapsed=600, factor=4)

    def test_absent_past_create_ceiling_refuses(self):
        with self.assertRaises(C.PinExpired):
            self.wait(["booting"], elapsed=1190, factor=10)

    def test_present_after_create_ceiling_is_not_readiness(self):
        with self.assertRaises(C.PinExpired):
            self.wait([FP], elapsed=1199, factor=4, costs=[2])

    def test_present_after_adaptive_stage_bound_is_not_readiness(self):
        with self.assertRaises(C.PinExpired):
            self.wait([FP], elapsed=0, factor=4, costs=[241])

    def test_slow_factor_can_observe_later_console_publication(self):
        snapshots = [""] * 20 + [FP]
        result, calls, sleeps, receipts = self.wait(snapshots, elapsed=300, factor=10)
        self.assertEqual(result["fingerprint"], FP)
        self.assertEqual(calls[-1][0], 400)

    def test_poll_schedule_accounts_for_console_read_time(self):
        result, calls, sleeps, receipts = self.wait(["", FP], costs=[7, 2])
        self.assertEqual([at for at, _ in calls], [0, 20])
        self.assertEqual(sleeps, [13])

    def test_other_algorithm_fingerprint_does_not_fake_ed25519_mismatch(self):
        result, calls, sleeps, receipts = self.wait(["256 SHA256:OTHER host (ECDSA)", FP])
        self.assertEqual(result["fingerprint"], FP)
        self.assertEqual(len(calls), 2)

    def test_factor_is_clamped_to_four_through_ten(self):
        self.assertEqual(C.stage_seconds(1), 240)
        self.assertEqual(C.stage_seconds(100), 600)

    def test_factor_floor_does_not_cut_off_publication_at_two_minutes(self):
        result, calls, sleeps, receipts = self.wait([""] * 6 + [FP], factor=1)
        self.assertEqual(result["fingerprint"], FP)
        self.assertEqual(calls[-1][0], 120)

    def test_slow_factor_still_cannot_extend_past_twenty_minutes_since_create(self):
        with self.assertRaises(C.PinExpired):
            self.wait([""] * 10 + [FP], elapsed=1100, factor=100)

    def test_receipts_keep_public_fingerprints_and_console_hash_not_raw_console(self):
        result, calls, sleeps, receipts = self.wait(["PRIVATE_BODY " + FP])
        self.assertEqual(receipts[0]["fingerprints"], [FP])
        self.assertEqual(len(receipts[0]["console_sha256"]), 64)
        self.assertNotIn("PRIVATE_BODY", str(receipts))

    def test_existing_stage_is_not_restarted_after_a_console_attempt(self):
        with self.assertRaises(C.PinExpired):
            self.wait([FP], elapsed=839, stage_started=600, costs=[2])

    def test_outer_retry_keeps_the_original_stage_deadline(self):
        with self.assertRaises(C.PinExpired):
            self.wait(["", FP], elapsed=839, stage_started=600)


if __name__ == "__main__":
    unittest.main()
