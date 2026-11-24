"""gate_reuse.py replays any complete PASS of the gate, whatever stages the gate planned, GPU-free.

FALSIFIER (written before the change): the replay accepts only a four-stage PASS, so the pull-request gate to master,
which plans more stages, is never reused and the push to master rents a second box. The stage names are not kept in
a second hand-written list for the gate: the payload's own plan (vast_payload.plan_stage_names) is fed through the
replay for every tier, and the base the replay requires first must be the base the payload plans first.
"""

import importlib.util
from pathlib import Path
import unittest

import payload_instruction as pi

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
PARENT = "a" * 40


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


PAYLOAD = load("vast_payload_reuse", HERE / "vast_payload.py")
REUSE = load("gate_reuse_stages", ROOT / "ci/vast-broker/gate_reuse.py")


def instruction(**over):
    doc = dict(tier="small", windows=False, windows_runs=1, stages=(), spec=None, winapps=None, linux_apps=None, rehearsal=False)
    doc.update(over)
    return pi.Instruction(**doc)


# What the Worker asks of each tier (ci/vast-broker/src/tier.js TIER_CONTENTS and middleAppsRequest).
TIERS = {
    "small": instruction(),
    "small with windows": instruction(windows=True),
    "middle": instruction(tier="middle", windows=True, winapps="mid", linux_apps="middle"),
    "large": instruction(tier="large", windows=True, winapps="mid", linux_apps="middle"),
}


def original_log(plan):
    lines = [
        "RENTAL instance=44 host=43 machine=42 datacenter=unknown country=UNKNOWN ip=unknown gpu=RTX_3060 price=0.200000",
        "[untrusted] [Worker] selection profile=check die=GA106 reason=DEFAULT_ORDER ignored=0",
    ]
    for mode in ("host", "guest"):
        lines.append(
            "[untrusted] [box] CUDA_LADDER_STARTED=2026-11-23T01:02:03+00:00 mode=" + mode
            + " reps=1 rungs=cup2,cup3 rev=" + PARENT[:8]
        )
    lines.append("[untrusted] [box] KF_FINAL_SUMMARY_BEGIN verdict=PASS stages=" + str(len(plan)))
    lines += ["[untrusted] [box] stage name=" + n + " rc=0 duration_ms=1 result=PASS" for n in plan]
    return "\n".join(lines)


class StagesFollowThePayload(unittest.TestCase):
    def test_every_tier_plans_the_base_first(self):
        for name, i in TIERS.items():
            with self.subTest(tier=name):
                self.assertEqual(tuple(PAYLOAD.plan_stage_names(i)[: len(REUSE.BASE_STAGES)]), REUSE.BASE_STAGES)

    def test_the_replay_accepts_the_plan_of_every_tier(self):
        for name, i in TIERS.items():
            plan = PAYLOAD.plan_stage_names(i)
            with self.subTest(tier=name, plan=plan):
                lines = REUSE.summary_lines(original_log(plan), PARENT)
                self.assertEqual(sum(1 for line in lines if " stage name=" in line), len(plan))

    def test_the_middle_plan_is_longer_than_the_old_four(self):
        # the case this change is for: the tier of a pull request to master
        self.assertGreater(len(PAYLOAD.plan_stage_names(TIERS["middle"])), 4)
        self.assertIn("windows_apps", PAYLOAD.plan_stage_names(TIERS["middle"]))

    def test_a_plan_missing_its_last_stage_is_refused(self):
        plan = PAYLOAD.plan_stage_names(TIERS["middle"])
        text = original_log(plan).replace("stages=" + str(len(plan)), "stages=" + str(len(plan) + 1))
        with self.assertRaises(ValueError):
            REUSE.summary_lines(text, PARENT)

    def test_a_failed_stage_is_refused(self):
        plan = PAYLOAD.plan_stage_names(TIERS["middle"])
        text = original_log(plan).replace("stage name=" + plan[-1] + " rc=0 duration_ms=1 result=PASS", "stage name=" + plan[-1] + " rc=1 duration_ms=1 result=FAIL")
        with self.assertRaises(ValueError):
            REUSE.summary_lines(text, PARENT)


if __name__ == "__main__":
    unittest.main()
