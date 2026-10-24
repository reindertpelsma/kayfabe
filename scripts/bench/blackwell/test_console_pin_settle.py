"""Settle-and-retry readiness pin (controller 12:10, BW5 B1q), synthetic console/scan sequences.

Lease 57498790 (B1p): one ssh-keyscan ~23 s BEFORE the console published its ED25519 pin, then a
single comparison and a refusal. The settled guard re-polls the console AND re-scans the host key
at 15 s intervals for at most 180 s after the first publication and accepts only two consecutive
checks where the scanned ed25519 equals the latest published ed25519. Everything else refuses.
"""

import base64
import hashlib
import struct
import unittest

import console_pin as C


def key(seed):
    raw = struct.pack(">I", 11) + b"ssh-ed25519" + struct.pack(">I", 32)
    raw += hashlib.sha256(seed.encode()).digest()
    return base64.b64encode(raw).decode()


def fp(k):
    return C.fingerprint(k)


def labelled(k, kind="ED25519"):
    return f"<14>Oct 24 10:03:53 cloud-init: 256 {fp(k)} root@ubuntu ({kind})"


def console(*keys, extra=()):
    lines = ["[   13.6] cloud-init[1483]: ssh-keygen: generating new host keys: RSA DSA ED25519"]
    lines += list(extra)
    for k in keys:
        lines += [labelled(k), labelled(key("rsa" + k), "RSA"), "ssh-ed25519 " + k + " root@ubuntu"]
    return "\n".join(lines) + "\n"


A, B, X = key("a"), key("b"), key("x")


class Clock:
    def __init__(self):
        self.t = 0.0

    def __call__(self):
        return self.t

    def sleep(self, seconds):
        assert seconds >= 0
        self.t += seconds


def run(consoles, scans, *, factor=4, elapsed=90):
    """consoles[i] / scans[i] answer check i (the last value repeats)."""
    clock = Clock()
    receipts = []
    state = {"c": 0, "s": 0}

    def fetch(timeout):
        assert 0 < timeout <= C.POLL_SECONDS
        i = min(state["c"], len(consoles) - 1)
        state["c"] += 1
        clock.t += 1
        return consoles[i]

    def scan():
        i = min(state["s"], len(scans) - 1)
        state["s"] += 1
        return scans[i]

    try:
        result = C.settle_pin(
            scan,
            fetch,
            elapsed_since_create=elapsed,
            factor=factor,
            clock=clock,
            sleep=clock.sleep,
            record=receipts.append,
        )
        return result, None, receipts, clock
    except (C.PinMismatch, C.PinExpired) as error:
        return None, error, receipts, clock


class Settle(unittest.TestCase):
    def test_immediate_match_needs_a_second_consecutive_check(self):
        result, error, receipts, clock = run([console(A)], [A])
        self.assertIsNone(error)
        self.assertEqual(result["fingerprint"], fp(A))
        self.assertEqual(result["key"], A)
        self.assertEqual([r["state"] for r in receipts], ["match", "match"])
        self.assertEqual([r["consecutive"] for r in receipts], [1, 2])
        self.assertGreaterEqual(clock.t, C.SETTLE_SECONDS)

    def test_the_b1p_sequence_settles_late_after_the_rescan(self):
        # absent console with an old scan, then publication; the re-scan reaches the new key.
        result, error, receipts, _ = run(["boot\n", console(A), console(A)], [X, X, A, A])
        self.assertIsNone(error)
        states = [r["state"] for r in receipts]
        self.assertEqual(states, ["absent", "mismatch", "match", "match"])
        self.assertEqual(result["key"], A)

    def test_late_match_after_n_polls(self):
        n = 5
        result, error, receipts, _ = run([console(A)], [X] * n + [A])
        self.assertIsNone(error)
        self.assertEqual([r["state"] for r in receipts], ["mismatch"] * n + ["match", "match"])

    def test_console_update_mid_window_restarts_the_count(self):
        consoles = [console(A), console(A, B), console(A, B)]
        result, error, receipts, _ = run(consoles, [A, B, B])
        self.assertIsNone(error)
        self.assertEqual([r["latest"] for r in receipts], [fp(A), fp(B), fp(B)])
        self.assertEqual([r["consecutive"] for r in receipts], [1, 1, 2])
        self.assertEqual(result["fingerprint"], fp(B))

    def test_a_match_on_an_older_generation_is_not_a_match(self):
        result, error, receipts, _ = run([console(A, B)], [A])
        self.assertIsNone(result)
        self.assertIsInstance(error, C.PinMismatch)
        self.assertNotIsInstance(error, C.PinFlap)
        self.assertTrue(all(r["state"] == "mismatch" for r in receipts))

    def test_never_matching_refuses_at_the_bounded_window(self):
        result, error, receipts, clock = run([console(A)], [X])
        self.assertIsNone(result)
        self.assertIsInstance(error, C.PinMismatch)
        first = receipts[0]["wait_seconds"]
        self.assertLessEqual(receipts[-1]["wait_seconds"] - first, C.SETTLE_WINDOW)
        self.assertEqual(len(receipts), C.SETTLE_WINDOW // C.SETTLE_SECONDS)
        gaps = [b["wait_seconds"] - a["wait_seconds"] for a, b in zip(receipts, receipts[1:])]
        self.assertTrue(all(abs(g - C.SETTLE_SECONDS) < 1e-6 for g in gaps), gaps)

    def test_a_scan_that_flips_without_matching_refuses_at_once(self):
        result, error, receipts, _ = run([console(A)], [X, B, A, A])
        self.assertIsNone(result)
        self.assertIsInstance(error, C.PinFlap)
        self.assertEqual([r["state"] for r in receipts], ["mismatch", "flap"])

    def test_a_match_then_a_different_key_is_a_flap(self):
        result, error, receipts, _ = run([console(A)], [A, X, A])
        self.assertIsInstance(error, C.PinFlap)
        self.assertEqual([r["state"] for r in receipts], ["match", "flap"])

    def test_an_unlabelled_or_unknown_type_fingerprint_refuses(self):
        for line in (
            "random " + fp(X),
            "<14>cloud-init: 256 " + fp(X) + " root@ubuntu (SK-ED448)",
        ):
            with self.subTest(line=line[:30]):
                result, error, receipts, _ = run([console(A, extra=[line])], [A])
                self.assertIsInstance(error, C.PinUnknownType)
                self.assertEqual(receipts[-1]["state"], "unknown_type")

    def test_other_labelled_types_are_ignored(self):
        extra = [labelled(X, "ECDSA"), labelled(B, "DSA"), labelled(key("r"), "RSA")]
        result, error, _, _ = run([console(A, extra=extra)], [A])
        self.assertIsNone(error)
        self.assertEqual(result["key"], A)

    def test_absent_console_until_the_stage_deadline_expires(self):
        result, error, receipts, clock = run(["boot\n"], [A])
        self.assertIsInstance(error, C.PinExpired)
        self.assertTrue(all(r["state"] == "absent" for r in receipts))
        self.assertLessEqual(clock.t, 90 + C.stage_seconds(4))

    def test_a_publication_that_disappears_does_not_count_and_refuses_at_window_end(self):
        result, error, receipts, _ = run([console(A), "truncated\n"], [A])
        self.assertIsInstance(error, C.PinMismatch)
        self.assertEqual(receipts[1]["state"], "absent")
        self.assertTrue(all(r["consecutive"] == 0 for r in receipts[1:]))

    def test_a_failed_scan_is_a_mismatch_not_a_flap(self):
        result, error, receipts, _ = run([console(A)], [None, None, A, A])
        self.assertIsNone(error)
        self.assertEqual([r["state"] for r in receipts], ["mismatch", "mismatch", "match", "match"])

    def test_a_malformed_scanned_key_refuses(self):
        result, error, _, _ = run([console(A)], ["AAAAnotakey"])
        self.assertIsInstance(error, C.PinMismatch)

    def test_receipts_store_the_scanned_fingerprint_and_the_host_key_lines(self):
        line = "ci-info: | ssh-ed25519 | 69:66:ac | - | user@laptop"
        _, _, receipts, _ = run([console(A, extra=[line])], [X, A, A])
        r = receipts[0]
        self.assertEqual(r["scanned_fingerprint"], fp(X))
        self.assertEqual(r["latest"], fp(A))
        self.assertEqual(r["published"], [fp(A), fp(A)])  # labelled line + full key line
        self.assertIn(labelled(A), r["host_key_lines"])
        self.assertIn("ssh-ed25519 " + A + " root@ubuntu", r["host_key_lines"])
        self.assertFalse(any("ci-info" in s for s in r["host_key_lines"]))
        self.assertEqual(r["ci_info_lines_omitted"], 1)
        self.assertTrue(any("generating new host keys" in s for s in r["host_key_lines"]))

    def test_the_stage_deadline_bounds_the_settle_window(self):
        # Publication late in the console stage: the window ends at the stage deadline.
        result, error, receipts, clock = run(
            ["boot\n"] * 11 + [console(A)], [X], factor=4, elapsed=90
        )
        self.assertIsInstance(error, C.PinMismatch)
        self.assertLessEqual(clock.t, 90 + C.stage_seconds(4) - 90 + 1e-6)


class Pinned(unittest.TestCase):
    def test_the_controller_numbers_are_pinned(self):
        self.assertEqual((C.SETTLE_SECONDS, C.SETTLE_WINDOW, C.SETTLE_CONSECUTIVE), (15, 180, 2))

    def test_a_never_matching_window_is_twelve_checks_fifteen_seconds_apart(self):
        _, error, receipts, _ = run([console(A)], [X])
        self.assertIsInstance(error, C.PinMismatch)
        self.assertEqual(len(receipts), 12)
        self.assertEqual(round(receipts[1]["wait_seconds"] - receipts[0]["wait_seconds"]), 15)
        self.assertEqual(round(receipts[-1]["wait_seconds"] - receipts[0]["wait_seconds"]), 165)

    def test_a_new_publication_needs_its_own_two_checks(self):
        result, error, receipts, _ = run([console(A), console(A, B)], [A, B, X])
        self.assertIsNone(result)
        self.assertIsInstance(error, C.PinFlap)
        self.assertEqual([r["consecutive"] for r in receipts], [1, 1, 0])

    def test_host_key_lines_omit_the_ci_info_table(self):
        text = console(A, extra=["ci-info: | ssh-ed25519 | 69:66 | - | u@h", "ci-info: +---+"])
        lines, omitted = C.host_key_lines(text)
        self.assertEqual(omitted, 1)
        self.assertFalse(any("ci-info" in line for line in lines))
        self.assertIn(labelled(A), lines)

    def test_an_absent_pin_is_an_expiry_not_a_mismatch(self):
        _, error, _, _ = run(["no keys yet\n"], [None], factor=10)
        self.assertIsInstance(error, C.PinExpired)
        self.assertNotIsInstance(error, C.PinMismatch)

    def test_a_malformed_scan_refuses_at_once_by_name(self):
        _, error, receipts, _ = run([console(A)], [A, "AAAAnotakey"])
        self.assertIn("malformed scanned", str(error))
        self.assertEqual([r["state"] for r in receipts], ["match", "malformed_scan"])
        _, error, receipts, _ = run([console(A)], ["AAAAnotakey"])
        self.assertEqual(len(receipts), 1)

    def test_a_malformed_scan_is_never_followed_by_an_acceptance(self):
        result, error, _, _ = run([console(A)], ["AAAAnotakey", A, A])
        self.assertIsNone(result)
        self.assertIsInstance(error, C.PinMismatch)

    def test_a_late_publication_gets_only_the_checks_before_the_stage_deadline(self):
        # factor 5: stage 300 s from 90 s; publication at the 13th check (t=240) => checks at
        # 240, 255, 270, 285 only, then the stage deadline (300) ends the window.
        _, error, receipts, clock = run(["boot\n"] * 12 + [console(A)], [X], factor=5)
        self.assertIsInstance(error, C.PinMismatch)
        self.assertEqual(sum(r["latest"] is not None for r in receipts), 4)
        self.assertLessEqual(clock.t, 300 + 1e-6)


if __name__ == "__main__":
    unittest.main()
