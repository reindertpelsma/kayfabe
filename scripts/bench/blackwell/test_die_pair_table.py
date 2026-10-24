"""Owner 2026-10-24 (BW4 decision 1): per-die exact Xid32 pairs from a pinned table.

The admitted pair is chosen by the host-probed die (controller PROBE_DIE line, carried in the
permit as probed_die) and must equal the lease authority's candidate die and exact channels.
A pair of another die, an unknown die, a missing probe or a third channel still stops the arm.
Also: one named DIAGNOSTIC arm may carry a second enable (pnputil disable/enable), no other.
"""

import copy
import importlib.util
from pathlib import Path
import unittest

import test_saved_xid32_observation as saved
import test_saved_xid32_watch_loop as loop
import test_windows_observation as base

SPEC = importlib.util.spec_from_file_location(
    "saved_xid32_observation_die", Path(__file__).with_name("saved_xid32_observation.py")
)
O = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(O)

GB205 = (0x14, 0x1000000B)
GB202 = (0x14, 0x0E00000B)
AUTH205 = copy.deepcopy(saved.AUTHORITY)
AUTH202 = {**copy.deepcopy(saved.AUTHORITY), "candidate_die": "GB202"}
AUTH202["exact_channels"] = ["0x14", "0x0e00000b"]
PERMIT205 = dict(saved.PERMIT)
PERMIT202 = {**saved.PERMIT, "probed_die": "GB202"}
PNP_ARM = "on_diag_pnp"
AUTH_PNP = {
    **copy.deepcopy(AUTH205),
    "arms": ["on_observe_b", PNP_ARM],
    "diagnostic_enable_arms": {PNP_ARM: 2},
}
PHASE_PNP = {**base.PHASE, "arm": PNP_ARM}
PERMIT_PNP = {**PERMIT205, "arm": PNP_ARM, "enable_count": 2, "label": "DIAGNOSTIC"}


def pair(channels, count=2):
    return [base.xid(channels[n % 2], second=n + 1) for n in range(count)]


def judge(lines, *, authority, permit, phase=base.PHASE):
    return O.judge_observation(lines, phase=phase, permit=permit, authority=authority, now=base.NOW)


class PinnedTable(unittest.TestCase):
    def test_table_is_exactly_the_owner_pinned_pairs(self):
        self.assertEqual(
            O.DIE_PAIRS,
            {"GB205": ("0x14", "0x1000000b"), "GB202": ("0x14", "0x0e00000b")},
        )

    def test_gb202_pair_is_recorded_as_observation_only(self):
        records = judge(pair(GB202), authority=AUTH202, permit=PERMIT202)
        self.assertEqual([r["channel"] for r in records], ["0x14", "0xe00000b"])
        self.assertTrue(all(r["verdict"] == "OBSERVATION" for r in records))
        self.assertTrue(all(r["pass"] is False and r["attestation"] is False for r in records))
        self.assertTrue(all(r["die"] == "GB202" for r in records))

    def test_gb205_pair_is_still_recorded(self):
        records = judge(pair(GB205), authority=AUTH205, permit=PERMIT205)
        self.assertEqual([r["channel"] for r in records], ["0x14", "0x1000000b"])
        self.assertTrue(all(r["die"] == "GB205" for r in records))

    def test_eight_gb202_records_are_counted(self):
        self.assertEqual(len(judge(pair(GB202, 8), authority=AUTH202, permit=PERMIT202)), 8)

    def test_ninth_gb202_record_refuses(self):
        with self.assertRaises(ValueError):
            judge(pair(GB202, 9), authority=AUTH202, permit=PERMIT202)

    def test_gb205_pair_on_gb202_die_refuses(self):
        with self.assertRaises(ValueError):
            judge([base.xid(0x1000000B)], authority=AUTH202, permit=PERMIT202)

    def test_gb202_pair_on_gb205_die_refuses(self):
        with self.assertRaises(ValueError):
            judge([base.xid(0x0E00000B)], authority=AUTH205, permit=PERMIT205)

    def test_third_channel_on_gb202_refuses(self):
        for channel in (0x15, 0x13, 0x0E00000C, 0x1000000B):
            with self.subTest(channel=hex(channel)), self.assertRaises(ValueError):
                judge(
                    pair(GB202) + [base.xid(channel, second=9)], authority=AUTH202, permit=PERMIT202
                )

    def test_third_channel_on_gb205_refuses(self):
        for channel in (0x15, 0x13, 0x1000000C, 0x0E00000B):
            with self.subTest(channel=hex(channel)), self.assertRaises(ValueError):
                judge(
                    pair(GB205) + [base.xid(channel, second=9)], authority=AUTH205, permit=PERMIT205
                )

    def test_unknown_die_refuses_even_with_a_known_pair(self):
        for die in ("GB206", "GB203", "GB207", "AD104", ""):
            authority = {**copy.deepcopy(AUTH205), "candidate_die": die}
            permit = {**PERMIT205, "probed_die": die}
            with self.subTest(die=die), self.assertRaises(ValueError):
                judge(pair(GB205), authority=authority, permit=permit)

    def test_missing_probed_die_refuses(self):
        permit = {k: v for k, v in PERMIT205.items() if k != "probed_die"}
        with self.assertRaises(ValueError):
            judge(pair(GB205), authority=AUTH205, permit=permit)

    def test_probed_die_differing_from_authority_die_refuses(self):
        with self.assertRaises(ValueError):
            judge(pair(GB205), authority=AUTH205, permit=PERMIT202)
        with self.assertRaises(ValueError):
            judge(pair(GB202), authority=AUTH202, permit=PERMIT205)

    def test_authority_channels_of_another_die_refuse(self):
        authority = {**copy.deepcopy(AUTH202), "exact_channels": ["0x14", "0x1000000b"]}
        with self.assertRaises(ValueError):
            judge(pair(GB205), authority=authority, permit=PERMIT202)
        authority = {**copy.deepcopy(AUTH205), "exact_channels": ["0x14", "0x0e00000b"]}
        with self.assertRaises(ValueError):
            judge(pair(GB202), authority=authority, permit=PERMIT205)

    def test_authority_die_must_match_even_when_channels_match_the_probe(self):
        authority = {**copy.deepcopy(AUTH205), "candidate_die": "GB202"}
        with self.assertRaises(ValueError):
            judge(pair(GB205), authority=authority, permit=PERMIT205)

    def test_authority_channels_must_match_even_when_the_xids_match_the_table(self):
        authority = {**copy.deepcopy(AUTH205), "exact_channels": ["0x14", "0x0e00000b"]}
        with self.assertRaises(ValueError):
            judge(pair(GB205), authority=authority, permit=PERMIT205)
        authority = {**copy.deepcopy(AUTH205), "exact_channels": ["0x14"]}
        with self.assertRaises(ValueError):
            judge([base.xid(0x14)], authority=authority, permit=PERMIT205)

    def test_scope_without_any_xid_still_checks_the_die(self):
        permit = {**PERMIT205, "probed_die": "GB206"}
        authority = {**copy.deepcopy(AUTH205), "candidate_die": "GB206"}
        with self.assertRaises(ValueError):
            O.scope_enabled(base.PHASE, permit, authority, base.NOW)


class ScopeChecksDie(unittest.TestCase):
    """scope_enabled alone (no Xid yet) already refuses a die/authority mismatch."""

    def scope(self, permit, authority):
        return O.scope_enabled(base.PHASE, permit, authority, base.NOW)

    def test_scope_accepts_the_matching_gb202_lease(self):
        self.assertTrue(self.scope(PERMIT202, AUTH202))

    def test_scope_refuses_a_missing_probe(self):
        permit = {k: v for k, v in PERMIT202.items() if k != "probed_die"}
        with self.assertRaises(ValueError):
            self.scope(permit, AUTH202)

    def test_scope_refuses_an_authority_die_mismatch_with_matching_channels(self):
        authority = {**copy.deepcopy(AUTH202), "candidate_die": "GB205"}
        with self.assertRaises(ValueError):
            self.scope(PERMIT202, authority)

    def test_judge_refuses_an_authority_die_mismatch_with_matching_channels(self):
        authority = {**copy.deepcopy(AUTH202), "candidate_die": "GB205"}
        with self.assertRaises(ValueError):
            judge(pair(GB202), authority=authority, permit=PERMIT202)

    def test_scope_refuses_authority_channels_of_another_die(self):
        authority = {**copy.deepcopy(AUTH202), "exact_channels": ["0x14", "0x1000000b"]}
        with self.assertRaises(ValueError):
            self.scope(PERMIT202, authority)

    def test_scope_refuses_a_boolean_single_enable(self):
        with self.assertRaises(ValueError):
            self.scope({**PERMIT205, "enable_count": True}, AUTH205)


class DiagnosticSecondEnable(unittest.TestCase):
    def test_named_diagnostic_arm_may_carry_a_second_enable(self):
        records = judge(pair(GB205), authority=AUTH_PNP, permit=PERMIT_PNP, phase=PHASE_PNP)
        self.assertEqual(len(records), 2)
        self.assertTrue(all(r["verdict"] == "OBSERVATION" and not r["pass"] for r in records))

    def test_diagnostic_arm_with_one_enable_is_also_valid(self):
        permit = {**PERMIT_PNP, "enable_count": 1}
        self.assertEqual(
            len(judge(pair(GB205), authority=AUTH_PNP, permit=permit, phase=PHASE_PNP)), 2
        )

    def test_third_enable_on_the_diagnostic_arm_refuses(self):
        permit = {**PERMIT_PNP, "enable_count": 3}
        with self.assertRaises(ValueError):
            judge(pair(GB205), authority=AUTH_PNP, permit=permit, phase=PHASE_PNP)

    def test_second_enable_on_another_arm_refuses(self):
        permit = {**PERMIT205, "enable_count": 2}
        with self.assertRaises(ValueError):
            judge(pair(GB205), authority=AUTH_PNP, permit=permit)

    def test_authority_cannot_raise_the_diagnostic_ceiling(self):
        authority = {**copy.deepcopy(AUTH_PNP), "diagnostic_enable_arms": {PNP_ARM: 3}}
        permit = {**PERMIT_PNP, "enable_count": 3}
        with self.assertRaises(ValueError):
            judge(pair(GB205), authority=authority, permit=permit, phase=PHASE_PNP)

    def test_second_enable_needs_the_diagnostic_label(self):
        permit = {k: v for k, v in PERMIT_PNP.items() if k != "label"}
        with self.assertRaises(ValueError):
            judge(pair(GB205), authority=AUTH_PNP, permit=permit, phase=PHASE_PNP)

    def test_other_labels_do_not_authorize_a_second_enable(self):
        for label in ("OBSERVATION", "diagnostic", None):
            permit = {**PERMIT_PNP, "label": label}
            with self.subTest(label=label), self.assertRaises(ValueError):
                judge(pair(GB205), authority=AUTH_PNP, permit=permit, phase=PHASE_PNP)

    def test_authority_ceiling_must_be_exactly_two(self):
        for ceiling in (1, 4, True, "2"):
            authority = {**copy.deepcopy(AUTH_PNP), "diagnostic_enable_arms": {PNP_ARM: ceiling}}
            with self.subTest(ceiling=ceiling), self.assertRaises(ValueError):
                judge(pair(GB205), authority=authority, permit=PERMIT_PNP, phase=PHASE_PNP)

    def test_far_above_ceiling_refuses(self):
        for count in (5, 100):
            permit = {**PERMIT_PNP, "enable_count": count}
            with self.subTest(count=count), self.assertRaises(ValueError):
                judge(pair(GB205), authority=AUTH_PNP, permit=permit, phase=PHASE_PNP)

    def test_diagnostic_arm_must_be_an_authorized_arm(self):
        authority = {**copy.deepcopy(AUTH_PNP), "arms": ["on_observe_b"]}
        with self.assertRaises(ValueError):
            judge(pair(GB205), authority=authority, permit=PERMIT_PNP, phase=PHASE_PNP)

    def test_boolean_enable_count_refuses(self):
        permit = {**PERMIT_PNP, "enable_count": True}
        with self.assertRaises(ValueError):
            judge(pair(GB205), authority=AUTH_PNP, permit=permit, phase=PHASE_PNP)


class DieWatchLoop(unittest.TestCase):
    """The real loop body with a GB202 authority/permit (one iteration, synthetic input)."""

    def run_gb202(self, lines):
        saved_auth, saved_permit = loop.AUTHORITY, loop.PERMIT
        loop.AUTHORITY, loop.PERMIT = AUTH202, PERMIT202
        try:
            return loop.SavedWatchLoop("test_owned_phase_without_new_xid_keeps_running").run_loop(
                loop.rows(lines)
            )
        finally:
            loop.AUTHORITY, loop.PERMIT = saved_auth, saved_permit

    def test_gb202_pair_runs_one_iteration_and_is_recorded(self):
        with unittest.mock.patch.object(loop, "O", O):
            r = self.run_gb202(pair(GB202))
        self.assertEqual(r["outcome"], "one_iteration", r["fatal"])
        records = r["accepted"]["observations"]["on_observe_b"]
        self.assertEqual([x["channel"] for x in records], ["0x14", "0xe00000b"])

    def test_gb205_high_channel_on_gb202_stops_fail_closed(self):
        with unittest.mock.patch.object(loop, "O", O):
            r = self.run_gb202(pair(GB205))
        self.assertIsInstance(r["outcome"], O.Refused)
        self.assertEqual((r["abort"], r["terminate"]), (1, 1))


import unittest.mock  # noqa: E402  (used above through the module attribute)

if __name__ == "__main__":
    unittest.main()
