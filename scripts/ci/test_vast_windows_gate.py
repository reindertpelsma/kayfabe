#!/usr/bin/env python3
"""GPU-free falsifiers for the opt-in Windows stage of the box merge bar (2026-11-01).

scripts/bench/box/windows_gate.sh installs Windows 11 LTSC once per box, boots it on kf3 with the stock
NVIDIA driver (GSP forced), and ends in exactly one verdict line. merge_check.sh runs it only for
KF_MERGE_WINDOWS=1, last. These tests need no box: they read the scripts, run the gate's own
`--selftest` (the verdict classification with the box stubbed out) and run the gate against a machine
with no /dev/kvm. The real run is evidence in traces/windows_gate_20261101/.

    KF_WINDOWS_GATE=<path>  KF_MERGE_CHECK=<path>   aim these tests at a mutated copy
"""

import os
import re
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GATE = Path(os.environ.get("KF_WINDOWS_GATE") or ROOT / "scripts/bench/box/windows_gate.sh")
MERGE_CHECK = Path(os.environ.get("KF_MERGE_CHECK") or ROOT / "scripts/bench/box/merge_check.sh")
WIN = ROOT / "scripts/bench/windows"


def code(path):
    return "\n".join(
        s for s in Path(path).read_text().splitlines() if not s.lstrip().startswith("#")
    )


def run_gate(*args, env=None):
    base = dict(os.environ)
    base.update(env or {})
    return subprocess.run(
        ["bash", str(GATE), *args], env=base, capture_output=True, text=True, timeout=120
    )


class MergeCheckWiring(unittest.TestCase):
    def test_the_windows_stage_is_opt_in_for_the_flag_value_1(self):
        text = code(MERGE_CHECK)
        block = re.search(
            r'if \[ "\$\{KF_MERGE_WINDOWS:-0\}" = 1 \]; then\n(.*?)\nfi(?:\n|$)', text, re.S
        )
        self.assertIsNotNone(block)
        script = (
            'PROV=p; T=t\nif [ "${KF_MERGE_WINDOWS:-0}" = 1 ]; then\n'
            + block[1].replace("bash scripts/bench/box/windows_gate.sh", "echo CALLED")
            + "\nfi\n"
        )
        for value, called in (("1", True), ("0", False), ("yes", False), ("", False), (None, False)):
            with self.subTest(repr(value)):
                env = {k: v for k, v in os.environ.items() if k != "KF_MERGE_WINDOWS"}
                if value is not None:
                    env["KF_MERGE_WINDOWS"] = value
                out = subprocess.run(
                    ["bash", "-c", script], env=env, capture_output=True, text=True, timeout=30
                ).stdout
                self.assertEqual("CALLED" in out, called, out)

    def test_the_windows_stage_runs_last_and_gets_the_tag_and_the_prov_dir(self):
        text = code(MERGE_CHECK)
        order = ["birth_census.sh", "BIRTH_CENSUS_OK", "windows_gate.sh"]
        positions = [text.rindex(s) for s in order]
        self.assertEqual(positions, sorted(positions))
        self.assertRegex(text, r'windows_gate\.sh "\$T" "\$PROV"')
        self.assertEqual(text.count("windows_gate.sh"), 1)

    def test_a_failing_gate_fails_the_bar(self):
        # merge_check.sh runs under `set -e` and the call is not part of a list: its rc is the bar's rc
        text = code(MERGE_CHECK)
        self.assertRegex(text, r"set -euo pipefail")
        call = [ln for ln in text.splitlines() if "windows_gate.sh" in ln][0]
        self.assertNotIn("||", call)
        self.assertNotIn("true", call)


class GateVerdicts(unittest.TestCase):
    def test_the_selftest_classifies_every_case(self):
        p = run_gate("--selftest")
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        self.assertRegex(p.stdout, r"^WINDOWS_GATE_SELFTEST_OK cases=10$")

    def test_a_guest_failure_is_kayfabes_only_with_both_references_green(self):
        text = code(GATE)
        self.assertRegex(
            text,
            r'\[ "\$HOSTREF_BEFORE" != green \] \|\| \[ "\$HOSTREF_AFTER" != green \]',
        )
        self.assertIn("class=KAYFABE", text.replace("verdict FAIL KAYFABE", "class=KAYFABE"))

    def test_the_verdict_vocabulary_is_closed(self):
        text = code(GATE)
        for line in (
            "WINDOWS_GATE_PASS",
            "WINDOWS_GATE_FAIL",
            "WINDOWS_GATE_UNATTRIBUTED",
            "WINDOWS_GATE_REFUSED",
            "WINDOWS_GATE_EXIT",
        ):
            self.assertIn(line, text)
        # the EXIT trap turns a missing verdict into UNATTRIBUTED, never into a pass
        trap = text[text.index("on_exit()") : text.index("trap on_exit EXIT")]
        self.assertIn("gate script ended without a verdict", trap)
        self.assertNotIn("WINDOWS_GATE_PASS", trap)

    def test_the_exit_codes(self):
        text = code(GATE)
        for expect in ("rc_final=0", "rc_final=1", "rc_final=3", "rc_final=2"):
            self.assertIn(expect, text)

    def test_no_kvm_is_a_refusal_with_an_exit_line(self):
        if Path("/dev/kvm").exists():
            self.skipTest("this machine has /dev/kvm")
        with tempfile.TemporaryDirectory() as d:
            p = run_gate("tagx", d, env={"WINVM_ROOT": d + "/w", "BENCH": d + "/b"})
            self.assertEqual(p.returncode, 2, p.stdout + p.stderr)
            self.assertIn("WINDOWS_GATE_REFUSED why=", p.stdout)
            lines = p.stdout.strip().splitlines()
            self.assertRegex(lines[-1], r"^WINDOWS_GATE_EXIT rc=2 ")
            self.assertNotIn("WINDOWS_GATE_PASS", p.stdout)
            self.assertTrue((Path(d) / "tagx_windows.log").exists())

    def test_it_refuses_to_overwrite_a_log(self):
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / "tagx_windows.log").write_text("old\n")
            p = run_gate("tagx", d)
            self.assertEqual(p.returncode, 2, p.stdout)
            self.assertIn("would-overwrite", p.stdout)
            self.assertEqual((Path(d) / "tagx_windows.log").read_text(), "old\n")

    def test_a_bad_tag_is_refused(self):
        p = run_gate("../x")
        self.assertEqual(p.returncode, 2)
        self.assertIn("invalid-tag", p.stdout)


class GateRules(unittest.TestCase):
    def test_the_gate_is_bounded(self):
        text = code(GATE)
        self.assertIn("KF_WINGATE_BUDGET_S", text)
        self.assertIn("WINVM_INSTALL_WAIT_S", text)
        self.assertIn("DEADLINE=", text)
        # every wait loop takes its bound from cap() or a fixed number, never an open `while true`
        self.assertNotRegex(text, r"while (true|:)\b")

    def test_the_gsp_keys_are_demanded_before_the_device_is_enabled(self):
        text = code(GATE)
        a = text.index("nv-install --no-enable")
        b = text.index("EnableGpuFirmware=1")
        c = text.index("gate_enable.ps1")
        self.assertLess(a, b)
        self.assertLess(b, c)

    def test_the_device_is_enabled_exactly_once_and_never_replayed(self):
        # the gate calls the enable script in one place; the script enables only a DISABLED device, once
        self.assertEqual(len(re.findall(r"gps \d+ 'C:\\kf\\gate_enable\.ps1'", code(GATE))), 1)
        ps = (WIN / "gate_enable.ps1").read_text()
        self.assertEqual(ps.count("pnputil /enable-device"), 1)
        self.assertIn("CM_PROB_DISABLED", ps)
        self.assertIn("REFUSED the device is not disabled", ps)

    def test_the_graded_probes_run_before_and_after_the_idle(self):
        text = code(GATE)
        self.assertEqual(text.count('probes_round ""'), 1)
        self.assertEqual(text.count('probes_round "_post"'), 1)
        self.assertLess(text.index('probes_round ""'), text.index("stage_begin idle"))
        self.assertLess(text.index("stage_begin idle"), text.index('probes_round "_post"'))
        for needle in ("CUP8 VERDICT: PASS", "bad=0 maxerr=0", "KFDX RESULT OK"):
            self.assertIn(needle, text)

    def test_every_reset_counter_must_be_zero(self):
        text = code(GATE)
        self.assertIn("total=0$", text)
        ps = (WIN / "gate_state.ps1").read_text()
        for name in (
            "display4101",
            "nvlddmkm153",
            "kernelpower41",
            "eventlog6008",
            "bugcheck1001",
            "livekernelevent",
        ):
            self.assertIn(name, ps)

    def test_the_host_reference_is_taken_before_and_after(self):
        text = code(GATE)
        self.assertIn("hostref before", text)
        self.assertIn("hostref after", text)
        self.assertIn("CUP8 VERDICT: PASS", text)
        self.assertLess(text.index("hostref before"), text.index("stage_begin install"))

    def test_the_guest_is_stopped_gracefully_never_killed(self):
        text = code(GATE)
        self.assertIn('bash "$WV" stop', text)
        self.assertNotRegex(text, r"\bkill\b|pkill|killall")

    def test_no_secret_is_copied_into_the_evidence(self):
        text = code(GATE)
        for secret in ("secrets/", "tpm/", "firmware/", "db.key", "OVMF_VARS"):
            copies = [ln for ln in text.splitlines() if re.search(r"\bcp\b", ln) and secret in ln]
            self.assertEqual(copies, [], secret)
        self.assertIn("grep -rqF", text)  # the evidence is scanned for the guest password

    def test_only_official_hosts_are_named_and_no_token_is_read(self):
        text = code(GATE)
        for banned in ("VAST", "vastai", "CLOUDFLARE", "R2_", "api_key", "http://", "https://"):
            self.assertNotIn(banned, text)


class GuestScripts(unittest.TestCase):
    def test_gate_state_is_read_only_and_ends_with_a_done_line(self):
        ps = (WIN / "gate_state.ps1").read_text()
        for bad in ("Set-ItemProperty", "New-ItemProperty", "Remove-Item", "Restart-", "Stop-Process", "pnputil"):
            self.assertNotIn(bad, ps)
        self.assertIn("STATE_DONE", ps)
        self.assertIn("-Mode desktop", ps)

    def test_gate_run_only_runs_probes_under_the_probe_directory(self):
        ps = (WIN / "gate_run.ps1").read_text()
        self.assertIn(r"^C:\\kf\\probes\\", ps)
        self.assertIn("TimeoutS", ps)
        self.assertIn("GATE_PROBE", ps)


if __name__ == "__main__":
    unittest.main()
