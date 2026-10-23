"""B1f named per-lease profile "7-visible-core deviation"; the default profile stays unchanged.

Run: KF_BW3_D=/workspace/bw3-round-b1f-run python3 -m unittest test_lease_profile
"""

import datetime as dt
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

D = Path(os.environ.get("KF_BW3_D", "/workspace/bw3-round-b1f-run"))
TRACE = Path(__file__).resolve().parents[1]
K = Path(os.environ.get("KF_BW3_K", str(TRACE / "run_kit")))
sys.path.insert(0, "/workspace/bw-native-edge-wt/scripts/bench/blackwell")
sys.path.insert(0, str(D))
import round_budget as B  # noqa: E402
import lease_profile as LP  # noqa: E402

NOW = dt.datetime(2026, 10, 23, 22, 30, tzinfo=dt.timezone.utc)
PREFLIGHT_OK = """=== kayfabe host preflight ===
  ✔ /dev/kvm present
  ✔ RAM 49G
  ✔ cores 16
=== PREFLIGHT DONE ===
VERDICT=SOUND
EXIT=0
"""


def preflight(cores, extra_bad=()):
    lines = ["=== kayfabe host preflight ===", "  ✔ /dev/kvm present", "  ✔ RAM 49G"]
    lines.append("  ✘ HOST FAULT: only %d cores" % cores)
    lines += ["  ✘ HOST FAULT: " + bad for bad in extra_bad]
    lines += ["=== PREFLIGHT DONE ===", "VERDICT=HOST_FAULT", "EXIT=2"]
    return "\n".join(lines) + "\n"


class Defaults(unittest.TestCase):
    def test_default_values_unchanged(self):
        self.assertEqual(LP.DEFAULT_OFFER_CPU_FLOOR, 16)
        self.assertEqual(LP.DEFAULT_VISIBLE_FLOOR, 8)
        self.assertEqual(LP.offer_floor(None), 16)
        self.assertEqual(LP.visible_floor(None), 8)
        self.assertEqual(LP.guest_smp(None), 8)

    def test_default_rejects_seven(self):
        self.assertFalse(LP.visible_ok(7, None))
        self.assertTrue(LP.visible_ok(8, None))

    def test_default_plan_150_minutes(self):
        plan = LP.plan(0.3, 28.0, NOW, stop_usd=2.0, cap_usd=2.5, max_seconds=None)
        self.assertEqual(plan["seconds"], 150 * 60)
        self.assertEqual(B.MAX_SECONDS, 150 * 60)


    def test_default_offer_rejects_7_core_offer(self):
        self.assertFalse(LP.offer_ok({"cpu_cores_effective": 7.0}, None))
        self.assertFalse(LP.offer_ok({"cpu_cores_effective": 15.0}, None))


class Profile(unittest.TestCase):
    def test_profile_accepts_7_rejects_6(self):
        self.assertEqual(LP.offer_floor(LP.PROFILE), 7)
        self.assertEqual(LP.visible_floor(LP.PROFILE), 7)
        self.assertTrue(LP.visible_ok(7, LP.PROFILE))
        self.assertFalse(LP.visible_ok(6, LP.PROFILE))
        self.assertTrue(LP.offer_ok({"cpu_cores_effective": 7.0}, LP.PROFILE))
        self.assertFalse(LP.offer_ok({"cpu_cores_effective": 6.0}, LP.PROFILE))
        self.assertFalse(LP.offer_ok({"cpu_cores_effective": 6.9}, LP.PROFILE))

    def test_unknown_profile_refused(self):
        for value in ("8-core deviation", "", "7", 7):
            with self.assertRaises(ValueError):
                LP.offer_floor(value)
            with self.assertRaises(ValueError):
                LP.visible_floor(value)

    def test_unknown_profile_refused_by_offer_floor(self):
        with self.assertRaises(ValueError):
            LP.offer_floor("8-core deviation")

    def test_unknown_profile_refused_by_guest_smp(self):
        with self.assertRaises(ValueError):
            LP.guest_smp("other")

    def test_scaled_worker_counts(self):
        self.assertEqual(LP.guest_smp(LP.PROFILE), 6)
        self.assertEqual(LP.build_jobs(LP.PROFILE), 7)
        self.assertLess(LP.guest_smp(LP.PROFILE), LP.visible_floor(LP.PROFILE))
        self.assertEqual(LP.FUZZ_CORES, 3)
        self.assertTrue(LP.pinned_fuzz_hostable(7))
        self.assertFalse(LP.pinned_fuzz_hostable(2))

    def test_plan_3h_and_stop_binds(self):
        plan = LP.plan(0.3, 28.0, NOW, stop_usd=2.0, cap_usd=2.5, max_seconds=LP.MAX_SECONDS)
        self.assertEqual(plan["seconds"], 3 * 3600)
        self.assertLess(plan["reservation_usd"], 2.0)
        self.assertEqual(B.MAX_SECONDS, 150 * 60)
        with self.assertRaises(ValueError):
            LP.plan(0.3, 28.0, NOW, stop_usd=2.0, cap_usd=2.5, max_seconds=4 * 3600)

    def test_prefer_machine_99805_else_all(self):
        rows = [{"machine_id": 1}, {"machine_id": 99805}]
        self.assertEqual(LP.prefer(rows), [rows[1]])
        self.assertEqual(LP.prefer([rows[0]]), [rows[0]])

    def test_prefer_keeps_only_99805_rows(self):
        rows = [{"machine_id": 5}, {"machine_id": 99805}, {"machine_id": 7}]
        self.assertEqual([r["machine_id"] for r in LP.prefer(rows)], [99805])

    def test_ceiling_restored_after_override_plan(self):
        LP.plan(0.3, 28.0, NOW, stop_usd=2.0, cap_usd=2.5, max_seconds=LP.MAX_SECONDS)
        self.assertEqual(B.MAX_SECONDS, 150 * 60)
        plan = B.plan(0.3, 28.0, NOW, stop_usd=2.0, cap_usd=2.5)
        self.assertEqual(plan["seconds"], 150 * 60)

    def test_deviation_fields(self):
        f = LP.deviation_fields(7, 8.0)
        self.assertEqual(f["cpu_visible"], 7)
        self.assertEqual(f["profile"], "7-visible-core deviation")
        self.assertEqual(f["cpu_profile_floor"], 7)
        self.assertEqual(f["guest_smp"], 6)
        self.assertEqual(f["cpu_cores_effective_offer"], 8.0)


class PreflightJudge(unittest.TestCase):
    def judge(self, text, profile, nproc):
        with tempfile.TemporaryDirectory() as tmp:
            log = Path(tmp) / "pre.log"
            log.write_text(text)
            lease = Path(tmp) / "lease.json"
            row = {"id": 1}
            if profile:
                row.update(profile=profile, cpu_profile_floor=7)
            lease.write_text(json.dumps(row))
            out = Path(tmp) / "verdict.json"
            p = subprocess.run(
                [sys.executable, str(K / "preflight_profile.py"), str(log), str(lease), str(out),
                 str(nproc)],
                capture_output=True, text=True,
            )
            return p.returncode, (json.loads(out.read_text()) if out.exists() else None)

    def test_default_requires_sound(self):
        self.assertEqual(self.judge(PREFLIGHT_OK, None, 16)[0], 0)
        self.assertNotEqual(self.judge(preflight(7), None, 7)[0], 0)

    def test_profile_accepts_only_the_7_core_fault(self):
        rc, verdict = self.judge(preflight(7), LP.PROFILE, 7)
        self.assertEqual(rc, 0)
        self.assertEqual(verdict["cpu_visible"], 7)
        self.assertEqual(verdict["profile"], LP.PROFILE)
        self.assertEqual(verdict["accepted_deviation"], ["only 7 cores"])

    def test_profile_rejects_6_cores(self):
        self.assertNotEqual(self.judge(preflight(6), LP.PROFILE, 6)[0], 0)

    def test_profile_rejects_any_other_fault(self):
        self.assertNotEqual(self.judge(preflight(7, ["only 8G RAM"]), LP.PROFILE, 7)[0], 0)
        self.assertNotEqual(self.judge(preflight(7, ["no /dev/kvm"]), LP.PROFILE, 7)[0], 0)

    def test_profile_rejects_ram_fault(self):
        self.assertNotEqual(self.judge(preflight(7, ["only 8G RAM"]), LP.PROFILE, 7)[0], 0)

    def test_profile_rejects_kvm_fault(self):
        self.assertNotEqual(self.judge(preflight(7, ["no /dev/kvm"]), LP.PROFILE, 7)[0], 0)

    def test_default_rejects_any_host_fault(self):
        text = PREFLIGHT_OK.replace("VERDICT=SOUND", "VERDICT=HOST_FAULT")
        self.assertNotEqual(self.judge(text, None, 16)[0], 0)

    def test_profile_rejects_7_core_fault_with_8_visible(self):
        self.assertNotEqual(self.judge(preflight(7), LP.PROFILE, 8)[0], 0)

    def test_profile_rejects_nproc_mismatch(self):
        self.assertNotEqual(self.judge(preflight(7), LP.PROFILE, 6)[0], 0)

    def test_profile_with_sound_preflight_passes(self):
        rc, verdict = self.judge(PREFLIGHT_OK, LP.PROFILE, 16)
        self.assertEqual(rc, 0)
        self.assertEqual(verdict["accepted_deviation"], [])


class Wiring(unittest.TestCase):
    def test_operator_uses_profile(self):
        ops = (D / "ops.py").read_text()
        self.assertIn('f"cpu_cores_effective>={floor} cpu_ram>=32', ops)
        self.assertIn("floor = LP.offer_floor(LP.PROFILE)", ops)
        self.assertIn("LP.offer_ok(x, LP.PROFILE)", ops)
        self.assertIn("eligible = LP.prefer(eligible)", ops)
        self.assertIn("**LP.deviation_fields(", ops)
        self.assertNotIn("cpu_cores_effective>=16", ops)

    def test_controller_gates_preflight_through_profile(self):
        c = (K / "controller.sh").read_text()
        self.assertIn("|| preflight_rc=$?", c)
        self.assertIn("python3 /root/sweep2/preflight_profile.py", c)
        self.assertNotIn("grep '^VERDICT=SOUND$' /root/prov/sweep_host_preflight.log", c)
        self.assertIn("CARGO_BUILD_JOBS=7", c)

    def test_controller_runs_judge_line(self):
        import re as _re

        c = (K / "controller.sh").read_text()
        self.assertTrue(_re.search(r"^python3 /root/sweep2/preflight_profile\.py ", c, _re.M))

    def test_operator_search_has_no_default_floor(self):
        self.assertNotIn("cpu_cores_effective>=16", (D / "ops.py").read_text())
        self.assertIn("LP.offer_ok(x, LP.PROFILE)", (D / "ops.py").read_text())

    def test_install_guest_smp_6(self):
        self.assertIn("export WINVM_RAM_MB=8192 WINVM_SMP=6", (K / "install.sh").read_text())

    def test_common_guest_smp_6(self):
        self.assertIn("WINVM_RAM_MB=8192 WINVM_SMP=6", (K / "common.sh").read_text())

    def test_guest_and_build_scaling_in_run_kit(self):
        for name in ("install.sh", "common.sh"):
            text = (K / name).read_text()
            self.assertIn("WINVM_SMP=6", text, name)
            self.assertNotIn("WINVM_SMP=8", text, name)
        self.assertIn("CARGO_BUILD_JOBS=7", (K / "bind.sh").read_text())
        b1 = (K / "build_b1.sh").read_text()
        self.assertIn("CARGO_BUILD_JOBS=7", b1)
        self.assertIn("cargo test -j7", b1)

    def test_bind_build_jobs_7(self):
        b = (K / "bind.sh").read_text()
        self.assertNotIn("CARGO_BUILD_JOBS=16", b)

    def test_fuzz_arm_unchanged(self):
        f = (K / "fuzz_findings.py").read_text()
        self.assertIn('"--fuzz-cores", "3"', f)


if __name__ == "__main__":
    unittest.main()
