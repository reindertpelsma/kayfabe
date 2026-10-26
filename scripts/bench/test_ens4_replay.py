"""Synthetic fixtures: no real failing first-153 trace has been captured by ENS4."""
import unittest
import ens4_replay as m


def ev(ns, op, packet=1):
    return {"ns": ns, "op": op, "packet": packet}


class ReplayTests(unittest.TestCase):
    def test_class_i(self):
        self.assertEqual(m.classify(False, False, None, True), "i_unpublished")

    def test_class_i_other_packet(self):
        self.assertEqual(m.classify(False, None, None, True), "i_unpublished")

    def test_class_ii_needs_guest_negative_evidence(self):
        self.assertEqual(m.classify(True, True, None, True), "ii_candidate")
        self.assertEqual(m.classify(True, True, False, True), "ii_undelivered")

    def test_class_iii(self):
        self.assertEqual(m.classify(True, False, None, True), "iii_host_pending")

    def test_missing_marker_is_unknown(self):
        self.assertEqual(m.classify(True, None, None, True), "unknown")

    def test_censored_negative_evidence_is_unknown(self):
        self.assertEqual(m.classify(True, False, False, False), "unknown")

    def test_delivery_deadline(self):
        trace = [ev(0, "publish"), ev(1, "host_complete"), ev(14, "guest_observe")]
        self.assertIn("COMPLETION_DELIVERY_LATE", m.replay(trace))

    def test_delivery_lost_at_complete_horizon(self):
        trace = [ev(0, "publish"), ev(1, "host_complete"), ev(14, "horizon")]
        self.assertIn("COMPLETION_DELIVERY_LATE", m.replay(trace))

    def test_apply_does_not_mask_lost_completion(self):
        trace = [ev(0, "publish"), ev(1, "apply_begin"), ev(2, "host_complete"),
                 ev(100, "apply_end"), ev(101, "horizon")]
        self.assertIn("COMPLETION_DELIVERY_LATE", m.replay(trace))

    def test_completion_before_publish_refused(self):
        self.assertIn("COMPLETION_WITHOUT_PUBLICATION", m.replay([ev(1, "host_complete")]))

    def test_duplicate_publication_refused(self):
        self.assertIn("DUPLICATE_PUBLICATION", m.replay([ev(0, "publish"), ev(1, "publish")]))

    def test_wrong_packet_does_not_discharge(self):
        trace = [ev(0, "publish"), ev(1, "host_complete"), ev(2, "guest_observe", 2),
                 ev(14, "horizon")]
        self.assertIn("COMPLETION_DELIVERY_LATE", m.replay(trace))

    def test_correct_schedule(self):
        self.assertEqual(m.replay(m.schedule(31)), [])
        self.assertTrue(m.schedule(31))

    def test_seed_replays_identically(self):
        self.assertEqual(m.schedule(42, True), m.schedule(42, True))
        self.assertIn("COMPLETION_DELIVERY_LATE", m.replay(m.schedule(42, True)))

    def test_minimization_keeps_failure(self):
        trace = m.schedule(42, True)
        short = m.minimize(trace)
        self.assertLess(len(short), len(trace))
        self.assertIn("COMPLETION_DELIVERY_LATE", m.replay(short))

    def test_minimization_idempotent(self):
        short = m.minimize(m.schedule(37, True))
        self.assertTrue(short)
        self.assertEqual(m.minimize(short), short)

    def test_convert_does_not_invent_guest_delivery_from_nsi(self):
        out = m.convert("kf3: ENS3 NSI offered=99 raised=99\n")
        self.assertEqual(out["events"], [])
        self.assertEqual(out["coverage"], "incomplete")

    def test_exact_marker_and_put_records(self):
        text = ("kf3: GATELOG REC token=0x1 t_ns=10 kind=marker_published a=1 b=1 c=0\n"
                "kf3: GATELOG REC token=0x1 t_ns=20 kind=marker_seen a=1 b=1 c=0\n")
        out = m.convert(text)
        self.assertEqual([e["op"] for e in out["events"]], ["publish", "host_complete"])
        self.assertEqual(out["coverage"], "incomplete")

    def test_malformed_canonical_trace_rejected(self):
        with self.assertRaises(ValueError):
            m.replay([ev(2, "publish"), ev(1, "host_complete")])

    def test_empty_trace_never_proves_no_bug(self):
        self.assertEqual(m.convert("")["result"], "no reproduction")


if __name__ == "__main__":
    unittest.main()
