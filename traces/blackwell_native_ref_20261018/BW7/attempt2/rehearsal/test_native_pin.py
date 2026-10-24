"""Controller BW7 attempt 2: staged native identity must never authorize an unpinned login."""

import hashlib
import importlib
from pathlib import Path
import sys
import unittest

K = Path(__file__).resolve().parents[1] / "kit"
sys.path[:0] = [str(K), str(K / "guards")]
import console_pin as C
import native_pin as N
from test_console_pin_settle import A, B, X, console


def native(*keys):
    return console(*keys) + "Save this WINDOWS SSH HOST KEY before reboot:\nssh-ed25519 " + keys[-1]


class Clock:
    def __init__(self, start=0):
        self.t = float(start)

    def __call__(self):
        return self.t

    def sleep(self, seconds):
        if not 0 < seconds <= 30:
            raise AssertionError("poll must be positive and bounded")
        self.t += seconds
        if self.t > 1300:
            raise AssertionError("unbounded wait crossed the independent test clock limit")


def run(sequence, *, start=0, first=0, fetch_cost=0, scan_cost=0):
    clock = Clock(start)
    receipts = []
    index = [0]
    def fetch(timeout):
        if not 0 < timeout <= 30:
            raise AssertionError("unbounded console timeout")
        clock.t += fetch_cost
        return sequence[min(index[0], len(sequence) - 1)][0]
    def scan(timeout=None):
        clock.t += scan_cost
        value = sequence[min(index[0], len(sequence) - 1)][1]
        index[0] += 1
        return value
    result = error = None
    try:
        result = N.settle_pin(scan, fetch, first_reachable_at=first, clock=clock,
                              sleep=clock.sleep, record=receipts.append)
    except (C.PinMismatch, C.PinExpired) as exc:
        error = exc
    return result, error, receipts, clock.t


class NativeStability(unittest.TestCase):
    def test_constants_are_owner_bounds(self):
        self.assertEqual(getattr(N, "STABILITY_SECONDS", None), 120)
        self.assertEqual(getattr(N, "ABSOLUTE_SECONDS", None), 1200)

    def test_two_matches_alone_do_not_authorize(self):
        result, error, rows, end = run([(native(A), A)])
        self.assertIsNone(error)
        self.assertIsNotNone(result)
        self.assertGreaterEqual(end, 120)
        self.assertGreaterEqual(result["consecutive"], 2)

    def test_stability_applies_when_first_contact_was_long_ago(self):
        result, error, rows, end = run([(native(A), A)], start=700)
        self.assertIsNone(error)
        self.assertGreaterEqual(end, 820)

    def test_mismatch_restarts_stability(self):
        seq = [(native(A), A)] * 6 + [(native(A), X)] + [(native(A), A)] * 10
        result, error, rows, end = run(seq)
        self.assertIsNone(error)
        mismatch = next(r for r in rows if r["state"] == "mismatch")
        self.assertGreaterEqual(end - mismatch["elapsed_since_first_reachable"], 120)

    def test_console_change_restarts_stability(self):
        result, error, rows, end = run([(native(A), A)] * 6 + [(native(A, B), B)] * 10)
        self.assertIsNone(error)
        first_b = next(r for r in rows if r["latest"] == C.fingerprint(B))
        self.assertGreaterEqual(end - first_b["elapsed_since_first_reachable"], 120)
        self.assertEqual(result["key"], B)

    def test_recorded_bw7_transition_waits_without_trusting_the_third_key(self):
        # Lease 57562949: Linux scan A, latest Windows B; three missing scans; third key X.
        seq = [(native(A, B), A)] + [(native(A, B), None)] * 3
        seq += [(native(A, B), X)] * 14 + [(native(A, B), B)] * 10
        result, error, rows, end = run(seq)
        self.assertIsNone(error)
        self.assertEqual(result["key"], B)
        self.assertGreaterEqual(end, 390)

    def test_console_absent_never_authorizes_scanned_key(self):
        result, error, rows, end = run([("boot noise", A)])
        self.assertIsNone(result)
        self.assertIsInstance(error, C.PinFlap)
        self.assertLessEqual(end, 1200)

    def test_console_transport_loss_never_becomes_attestation(self):
        result, error, rows, end = run([(native(A), A)] * 2 + [("", A)] * 100)
        self.assertIsNone(result)
        self.assertIsInstance(error, C.PinFlap)

    def test_an_older_console_pin_is_not_accepted(self):
        result, error, rows, end = run([(native(A, B), A)])
        self.assertIsNone(result)
        self.assertIsInstance(error, C.PinFlap)

    def test_latest_pin_update_cannot_leave_old_match_authorized(self):
        result, error, rows, end = run([(native(A), A)] * 2 + [(native(A, B), A)] * 100)
        self.assertIsNone(result)
        self.assertIsInstance(error, C.PinFlap)

    def test_changing_pins_cannot_extend_deadline(self):
        result, error, rows, end = run([(native(A), X), (native(B), X)] * 100)
        self.assertIsNone(result)
        self.assertIsInstance(error, C.PinFlap)
        self.assertLessEqual(end, 1200)

    def test_late_invocation_uses_first_reachable_deadline(self):
        result, error, rows, end = run([(native(A), X)], start=1190)
        self.assertIsNone(result)
        self.assertIsInstance(error, C.PinFlap)
        self.assertLessEqual(end, 1200)

    def test_match_arriving_at_deadline_is_refused(self):
        result, error, rows, end = run([(native(A), A)], start=1180, fetch_cost=20)
        self.assertIsNone(result)
        self.assertIsInstance(error, C.PinFlap)

    def test_expired_entry_does_not_poll(self):
        result, error, rows, end = run([(native(A), A)], start=1200)
        self.assertIsNone(result)
        self.assertIsInstance(error, C.PinFlap)
        self.assertEqual(rows, [])

    def test_linux_stage_cannot_authorize_native_windows_login(self):
        result, error, rows, end = run([(console(A), A)])
        self.assertIsNone(result)
        self.assertIsInstance(error, C.PinFlap)

    def test_missing_windows_marker_after_transition_resets_window(self):
        result, error, rows, end = run([(native(A), A)] * 2 + [(console(A), A)] * 100)
        self.assertIsNone(result)
        self.assertIsInstance(error, C.PinFlap)


class KayfabeRegression(unittest.TestCase):
    def test_legacy_source_bytes_unchanged(self):
        self.assertEqual(hashlib.sha256((K / "guards/console_pin.py").read_bytes()).hexdigest(),
                         "169f99eef6216e2ff0b69185877af20f3cc74794a875b73158c7495812b8208f")

    def test_legacy_still_accepts_two_matches_without_native_window(self):
        from test_console_pin_settle import run as legacy_run
        result, error, rows, clock = legacy_run([console(A)], [A])
        self.assertIsNone(error)
        self.assertEqual(len(rows), 2)
        self.assertLess(clock.t, 120)

    def test_legacy_still_refuses_a_scan_flap_immediately(self):
        from test_console_pin_settle import run as legacy_run
        result, error, rows, clock = legacy_run([console(A)], [B, X])
        self.assertIsNone(result)
        self.assertIsInstance(error, C.PinFlap)
        self.assertEqual(len(rows), 2)
