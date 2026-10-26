#!/usr/bin/env python3
"""Synthetic scorer contracts, no hardware or saved workload result asserted."""
import copy
import unittest
import ens4_score


def fixture():
    client = """ENS4_CHANNEL client=0x1 channel=0xa
ENS4_CHURN client=0x1 vaspace=0x3
ENS4_CALIB n=10000 p999_ns=100 k=4 bound_ns=400 frozen=true
ENS4_LINUX packets=2 rounds=1 maps_per_round=4174
ENS4_CLIENT_VERDICT=PASS
ENS4_CLIENT_DONE rc=0
"""
    qemu = """Windows USER ring chan 0x1:0xa BORN USER private-read-only-ring token 0x7 host 0x10067;
kf3: ENS4 APPLY space=0x100000003 walk=1 runs=4174 wall_ns=77000000 injected_ms=0
kf3: GATELOG HOLDTOP token=0x10067 vas=(0x1,0x2) holds=1 hold_ms=77
kf3: GATELOG END #1 lines=6
"""
    rows = [{"phase": "calibration", "index": str(i), "latency_ns": "100",
             "engine_receipt": "true"} for i in range(10000)]
    rows += [{"phase": "churn", "index": str(i), "latency_ns": "200",
              "engine_receipt": "true"} for i in range(10000, 10002)]
    return client, qemu, rows


class Score(unittest.TestCase):
    def test_proven_same_route_large_apply_and_clean_latencies_pass(self):
        c, q, rows = fixture()
        self.assertEqual(ens4_score.score(c, q, rows)["verdict"], "PASS")

    def test_exact_bound_is_still_pass(self):
        c, q, rows = fixture()
        rows[-1]["latency_ns"] = "400"
        self.assertEqual(ens4_score.score(c, q, rows)["verdict"], "PASS")

    def test_one_over_bound_packet_fails_even_when_the_next_completes(self):
        c, q, rows = fixture()
        rows[-2]["latency_ns"] = "401"
        c = c.replace("VERDICT=PASS", "VERDICT=FAIL").replace("rc=0", "rc=1")
        self.assertEqual(ens4_score.score(c, q, rows)["verdict"], "FAIL")

    def test_irq_or_get_progress_does_not_substitute_for_engine_receipt(self):
        c, q, rows = fixture()
        rows[-1]["latency_ns"] = ""
        rows[-1]["engine_receipt"] = "false"
        c = c.replace("VERDICT=PASS", "VERDICT=FAIL").replace("rc=0", "rc=1")
        self.assertEqual(ens4_score.score(c, q, rows)["verdict"], "FAIL")

    def test_missing_route_is_invalid_even_if_client_passed(self):
        c, q, rows = fixture()
        self.assertEqual(ens4_score.score(c, q.replace("BORN USER", "BORN Passthrough"), rows)
                         ["verdict"], "INVALID")

    def test_another_clients_identical_channel_cannot_supply_route_proof(self):
        c, q, rows = fixture()
        q = q.replace("chan 0x1:0xa", "chan 0x2:0xa")
        self.assertEqual(ens4_score.score(c, q, rows)["verdict"], "INVALID")

    def test_small_or_unrelated_applies_cannot_supply_exposure(self):
        c, q, rows = fixture()
        for other in [q.replace("runs=4174", "runs=999"),
                      q.replace("space=0x100000003", "space=0x200000003")]:
            self.assertEqual(ens4_score.score(c, other, rows)["verdict"], "INVALID")

    def test_another_channel_hold_is_not_the_failed_channels_hold(self):
        c, q, rows = fixture()
        q = q.replace("HOLDTOP token=0x10067", "HOLDTOP token=0x10068")
        self.assertEqual(ens4_score.score(c, q, rows)["verdict"], "INVALID")

    def test_truncated_dump_or_client_never_becomes_pass(self):
        c, q, rows = fixture()
        for cc, qq in [(c.replace("ENS4_CLIENT_DONE rc=0", ""), q),
                       (c, q.replace("GATELOG END", "clipped"))]:
            self.assertEqual(ens4_score.score(cc, qq, rows)["verdict"], "INVALID")

    def test_baseline_or_bound_disagreement_is_invalid(self):
        c, q, rows = fixture()
        for cc in [c.replace("bound_ns=400", "bound_ns=800"),
                   c.replace("n=10000", "n=9999")]:
            self.assertEqual(ens4_score.score(cc, q, rows)["verdict"], "INVALID")

    def test_dropped_samples_or_missing_calibration_receipts_are_invalid(self):
        c, q, rows = fixture()
        truncated = copy.deepcopy(rows)
        truncated.pop(12)
        missing = copy.deepcopy(rows)
        missing[12]["engine_receipt"] = "false"
        for rr in [truncated, missing]:
            self.assertEqual(ens4_score.score(c, q, rr)["verdict"], "INVALID")

    def test_false_client_pass_does_not_mask_an_over_bound_csv_sample(self):
        c, q, rows = fixture()
        rows[-1]["latency_ns"] = "401"
        self.assertEqual(ens4_score.score(c, q, rows)["verdict"], "INVALID")


if __name__ == "__main__":
    unittest.main()
