"""Exercise the gate runner's verdict/exit/census logic without a GPU."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class GateRunnerTests(unittest.TestCase):
    def run_fixture(self, mode):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            commands = root / "bin"
            commands.mkdir()
            for name, body in (("cargo", "exit 0"), ("nvidia-smi", "echo fixture")):
                path = commands / name
                path.write_text("#!/bin/sh\n" + body + "\n")
                path.chmod(0o700)
            target = root / "custom-target" / "release"
            target.mkdir(parents=True)
            for n in range(1, 10):
                if mode == "missing" and n == 9:
                    continue
                path = target / f"kf-gate{n}"
                verdict = "FAIL" if mode == "fail" and n == 9 else "PASS"
                code = 7 if mode == "bad-exit" and n == 9 else 0
                # THE_CONSTRAINTS §30: gates 2 and 5 birth one channel each, the way the real
                # ones log it through kf-host.
                birth = ""
                if n in (2, 5) and mode != "no-births":
                    flag = "1" if mode == "admin-birth" and n == 5 else "0"
                    birth = (f"echo 'kf-host: channel birth h=0xcafe000b engine=0x9 "
                             f"reply_flags=0x00000080 PRIVILEGED_CHANNEL={flag} privilege=USER "
                             f"cap_sys_admin=cleared-for-call' >&2\n")
                if mode == "refused-birth" and n == 5:
                    birth += "echo 'kf-host: ⊘ PRIVILEGED CHANNEL REFUSED h=0xcafe000c' >&2\n"
                path.write_text(f"#!/bin/sh\n{birth}echo GATE{n}_VERDICT={verdict}\nexit {code}\n")
                path.chmod(0o700)
            # Gate 10 (2026-10-10, D3): the micro-reservation probe. PASS, FALLBACK (host RM refused
            # the reservation: not a failure), FAIL (inconsistency), a probe that dies without a
            # verdict, or no binary at all.
            if mode != "g10-missing":
                verdict, code = {"g10-fail": ("FAIL", 1), "g10-fallback": ("FALLBACK", 0),
                                 "g10-noverdict": (None, 0), "g10-bad-exit": ("PASS", 3),
                                 "g10-fallback-unproven": ("FALLBACK", 0)
                                 }.get(mode, ("PASS", 0))
                line = f"echo MICRO_RESERVE_VERDICT arm=reserve {verdict}\n" if verdict else ""
                if mode == "g10-fallback":
                    # The probe's proof that the flat FB alias places without reservations.
                    line = "echo 'CHECK fallback_flat_fb_alias_placeable PASS a 2 MiB leaf'\n" + line
                if mode in ("g10-admin-birth", "g10-user-birth"):
                    flag = "1" if mode == "g10-admin-birth" else "0"
                    line = ("echo 'kf-host: channel birth h=0xcafe000d engine=0x9 reply_flags=0x00000080 "
                            f"PRIVILEGED_CHANNEL={flag} privilege=USER cap_sys_admin=cleared-for-call' >&2\n") + line
                probe = target / "kf-micro-reserve-probe"
                probe.write_text(f"#!/bin/sh\n{line}exit {code}\n")
                probe.chmod(0o700)
            env = dict(os.environ, PATH=f"{commands}:/usr/bin:/bin",
                       CARGO_TARGET_DIR=str(target.parent))
            return subprocess.run(["bash", str(ROOT / "scripts/bench/v3_gates.sh"),
                                   str(root / "result.log")], env=env,
                                  capture_output=True, text=True, timeout=20)

    def test_all_nine_pass_in_custom_target(self):
        result = self.run_fixture("pass")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("V3_GATES_SUMMARY pass=9 fail=0 gate10=PASS", result.stdout)
        self.assertIn("GATE10_VERDICT=PASS", result.stdout)

    def test_gate_10_fails_only_on_an_inconsistency(self):
        """The summary line carries gate 10, so a consumer that reads only it cannot see 9/9 green
        while gate 10 failed; FALLBACK (reservation refused) passes, loudly."""
        for mode in ("g10-fail", "g10-missing", "g10-noverdict", "g10-bad-exit",
                     "g10-fallback-unproven"):
            with self.subTest(mode=mode):
                result = self.run_fixture(mode)
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertIn("V3_GATES_SUMMARY pass=9 fail=0 gate10=FAIL", result.stdout)
        result = self.run_fixture("g10-fallback")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("V3_GATES_SUMMARY pass=9 fail=0 gate10=FALLBACK", result.stdout)
        self.assertIn("GATE10_FALLBACK_ACTIVE", result.stdout)
        # ... but a FALLBACK the probe did not PROVE (the flat FB alias, the 6fafcc6e failure class)
        # is a FAIL: see the "g10-fallback-unproven" case above.
        unproven = self.run_fixture("g10-fallback-unproven")
        self.assertIn("GATE10_FALLBACK_UNPROVEN", unproven.stdout)

    def test_failed_missing_and_nonzero_exit_cannot_report_success(self):
        for mode in ("fail", "missing", "bad-exit"):
            with self.subTest(mode=mode):
                result = self.run_fixture(mode)
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertIn("V3_GATES_SUMMARY pass=8 fail=1", result.stdout)

    def test_a_non_user_birth_or_no_birth_at_all_fails_the_run(self):
        """THE_CONSTRAINTS §30: all nine gates pass, and the run still fails."""
        for mode in ("admin-birth", "refused-birth", "no-births"):
            with self.subTest(mode=mode):
                result = self.run_fixture(mode)
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertIn("V3_GATES_SUMMARY pass=9 fail=0", result.stdout)
                self.assertIn("ok=0", result.stdout)

    def test_the_birth_census_counts_every_gate(self):
        result = self.run_fixture("pass")
        self.assertIn("V3_GATES_BIRTHS births=2 user=2 refused=0 ok=1", result.stdout)

    def test_the_birth_census_reads_gate_10_too(self):
        """A non-USER channel the probe births fails the run like any gate's; a USER one counts."""
        result = self.run_fixture("g10-admin-birth")
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn("gate10=PASS", result.stdout)
        self.assertIn("V3_GATES_BIRTHS births=3 user=2 refused=0 ok=0", result.stdout)
        result = self.run_fixture("g10-user-birth")
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("V3_GATES_BIRTHS births=3 user=3 refused=0 ok=1", result.stdout)
