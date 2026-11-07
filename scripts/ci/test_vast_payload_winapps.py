#!/usr/bin/env python3
"""The Windows apps stage in the Vast payload (docs/design/V3_WINDOWS_APPS_GATE.md), GPU-free.

FALSIFIER (written before the code): any of these starts the stage or accepts it without a guest pass:
  - the stage runs without KF_WINAPPS (today's Worker must see today's run), or without the Windows stage;
  - a malformed KF_WINAPPS value is accepted;
  - a RETRY (host, harness, input, rental fault), a REFUSED, a missing or doubled verdict, a PASS of another
    revision or profile, or a PASS with a non-zero exit becomes a passing row;
  - a guest FAIL becomes a RETRY (an excuse) or a RETRY becomes a FAIL row without its class;
  - the stage runs before the Windows gate, or after a red gate (a first red cancels later work);
  - the stage is not given too little time as a RETRY (not a verdict on the guest).
"""

import unittest

from test_vast_payload import SHA
import test_vast_payload_v7 as v7

WINDOWS_PASS = v7.WINDOWS_PASS


def apps_log(kind, profile="small", rev=SHA[:8], exit_rc=None, extra=""):
    rc = {"PASS": 0, "FAIL": 1, "RETRY": 3, "REFUSED": 2}[kind] if exit_rc is None else exit_rc
    line = {
        "PASS": f'WINAPPS_PASS rev={rev} gpu="RTX 4090" profile={profile} apps=8 hostref=green/green secs=1 {extra}',
        "FAIL": f'WINAPPS_FAIL diag=KAYFABE profile={profile} stage=done why="x" rev={rev} gpu="RTX 4090" hostref=green/green secs=1',
        "RETRY": f'WINAPPS_RETRY action=rerun-on-a-fresh-box class={extra or "OPERATOR"} profile={profile} stage=grade why="x" rev={rev} gpu="RTX 4090" hostref=green/green secs=1',
        "REFUSED": "WINAPPS_REFUSED why=no-WINDOWS_GATE_PASS",
    }[kind]
    details = ""
    if kind == "PASS":
        details = "".join(f"WINAPPS_APP app=a{i} class=pass verdict=PASS batched=PASS alone=- secs=1 tdr=0 wer=0 gsp=0 note=-\n" for i in range(8))
        details += "WINAPPS_GRADE kind=PASS expected=8 pass=8 guest_fail=0 boot_fail=0 harness=0 not_run=0 extra=0\n"
    return f"APPS_NOTE x\n{details}{line}\nWINAPPS_EXIT rc={rc} 2026-11-07T00:00:00+00:00\n", rc


class GradeTests(unittest.TestCase):
    setUp = v7.V7Tests.setUp

    def grade(self, text, rc, profile="small"):
        try:
            return self.m.winapps_rows(text, rc, SHA, profile), None
        except self.m.EvidenceError as error:
            return None, error.name

    def test_only_a_guest_pass_of_this_revision_and_profile_is_a_row(self):
        text, rc = apps_log("PASS")
        rows, err = self.grade(text, rc)
        self.assertEqual(err, None)
        self.assertEqual([r["name"] for r in rows], ["windows_apps"])
        self.assertEqual(rows[0]["pass"], 8)

    def test_pass_requires_nonempty_complete_per_app_rows(self):
        text, rc = apps_log("PASS")
        for bad in (text.replace("WINAPPS_APP app=a0", "DISCARDED app=a0"),
                    text.replace("app=a1", "app=a0"), text.replace("class=pass verdict=PASS", "class=harness verdict=NOTRUN", 1),
                    text.replace("expected=8 pass=8", "expected=0 pass=0")):
            self.assertEqual(self.grade(bad, rc)[1], "windows_apps_retry_operator")

    def test_pass_requires_matching_summary_and_stage_counts(self):
        text, rc = apps_log("PASS")
        for bad in (text.replace(" apps=8 ", " apps=0 "), text.replace(" apps=8 ", " apps=7 "),
                    text.replace("WINAPPS_GRADE", "DISCARDED"), text.replace("harness=0", "harness=1")):
            self.assertEqual(self.grade(bad, rc)[1], "windows_apps_retry_operator")

    def test_a_pass_of_another_revision_or_profile_is_not_a_pass(self):
        for text, rc in (apps_log("PASS", rev="deadbeef"), apps_log("PASS", profile="mid")):
            self.assertEqual(self.grade(text, rc)[1], "windows_apps_retry_operator")

    def test_a_pass_with_a_nonzero_exit_is_not_a_pass(self):
        text, _ = apps_log("PASS", exit_rc=0)
        self.assertEqual(self.grade(text, 1)[1], "windows_apps_retry_operator")
        text, _ = apps_log("PASS", exit_rc=3)
        self.assertEqual(self.grade(text, 0)[1], "windows_apps_retry_operator")

    def test_a_guest_failure_stays_a_failure_and_is_not_an_excuse(self):
        text, rc = apps_log("FAIL")
        self.assertEqual(self.grade(text, rc)[1], "windows_apps")

    def test_every_retry_class_is_named_and_never_a_pass(self):
        for cls in ("PROVIDER", "MEDIA", "OPERATOR", "HOSTREF", "BAR1_EXCEPTION"):
            text, rc = apps_log("RETRY", extra=cls)
            self.assertEqual(self.grade(text, rc)[1], "windows_apps_retry_" + cls.lower())
        text, rc = apps_log("RETRY", extra="MADEUP")
        self.assertEqual(self.grade(text, rc)[1], "windows_apps_retry_operator")

    def test_refused_missing_and_doubled_verdicts_are_retries(self):
        text, rc = apps_log("REFUSED")
        self.assertEqual(self.grade(text, rc)[1], "windows_apps_retry_refused")
        self.assertEqual(self.grade("", 0)[1], "windows_apps_retry_operator")
        self.assertEqual(self.grade("APPS_NOTE only\n", 1)[1], "windows_apps_retry_operator")
        a, _ = apps_log("PASS")
        self.assertEqual(self.grade(a + a, 0)[1], "windows_apps_retry_operator")
        b, _ = apps_log("FAIL")
        self.assertEqual(self.grade(a + b, 1)[1], "windows_apps")      # a FAIL anywhere in the log is never hidden by a PASS


class RequestTests(unittest.TestCase):
    setUp = v7.V7Tests.setUp

    def test_absent_is_no_stage_and_the_value_is_popped(self):
        env = {}
        self.assertIsNone(self.m.winapps_requested(env, True))
        env = {"KF_WINAPPS": "mid"}
        self.assertEqual(self.m.winapps_requested(env, True), "mid")
        self.assertNotIn("KF_WINAPPS", env)

    def test_malformed_or_without_windows_is_refused(self):
        for value in ("", "huge", "SMALL", "small:mid", "1"):
            with self.assertRaises(self.m.EvidenceError):
                self.m.winapps_requested({"KF_WINAPPS": value}, True)
        with self.assertRaises(self.m.EvidenceError):
            self.m.winapps_requested({"KF_WINAPPS": "small"}, False)

    def test_the_time_a_profile_needs_grows_with_the_profile(self):
        w = self.m.WINAPPS_MIN_REMAINING_MS
        self.assertEqual(set(w), set(self.m.WINAPPS_PROFILES))
        self.assertLess(w["small"], w["mid"])
        self.assertLess(w["mid"], w["full"])


class StageTests(unittest.TestCase):
    setUp = v7.V7Tests.setUp
    _main_fixture = v7.V7Tests._main_fixture

    def run_stage(self, profile="small", apps=None, gate=None, soft_ms=300 * 60000, windows="1"):
        text, rc = apps or apps_log("PASS", profile=profile)
        logs = {"windows_apps": (text, rc)}
        extra = {"KF_WINAPPS": profile} if profile else {}
        return self._main_fixture(
            windows=windows, soft_ms=soft_ms, extra_env=extra, stage_logs=logs, windows_log=gate
        )

    def test_no_env_no_stage_the_run_is_todays(self):
        rc, result, calls = self._main_fixture(windows="1", soft_ms=300 * 60000, stage_logs={})
        self.assertEqual(rc, 0, result)
        self.assertNotIn("windows_apps", calls)

    def test_the_stage_runs_right_after_the_gate_and_is_its_own_row(self):
        rc, result, calls = self.run_stage("small")
        self.assertEqual(rc, 0, result)
        self.assertEqual(calls[-2:], ["windows_gate", "windows_apps"])
        names = [r["name"] for r in result["suites"]]
        self.assertIn("windows_gate", names)
        self.assertIn("windows_apps", names)
        self.assertEqual(result["verdict"], "PASS")

    def test_a_guest_failure_fails_the_run_with_the_apps_row(self):
        rc, result, calls = self.run_stage("mid", apps=apps_log("FAIL", profile="mid"))
        self.assertEqual(rc, 1)
        self.assertEqual([r["name"] for r in result["suites"] if r["fail"]], ["windows_apps"])
        self.assertEqual(result["verdict"], "FAIL")

    def test_a_harness_retry_fails_the_run_with_a_named_row(self):
        rc, result, _ = self.run_stage("full", apps=apps_log("RETRY", profile="full", extra="MEDIA"))
        self.assertEqual(rc, 1)
        self.assertEqual(
            [r["name"] for r in result["suites"] if r["fail"]], ["windows_apps_retry_media"]
        )
        self.assertEqual(result["verdict"], "FAIL")

    def test_a_red_gate_cancels_the_apps(self):
        red = WINDOWS_PASS.replace("WINDOWS_GATE_PASS", "WINDOWS_GATE_FAIL diag=KAYFABE stage=idle")
        rc, result, calls = self.run_stage("small", gate=red)
        self.assertEqual(rc, 1)
        self.assertNotIn("windows_apps", calls)

    def test_too_little_time_is_a_retry_not_a_verdict_on_the_guest(self):
        rc, result, calls = self.run_stage("full", soft_ms=100 * 60000)
        self.assertEqual(rc, 1)
        self.assertNotIn("windows_apps", calls)
        self.assertEqual(
            [r["name"] for r in result["suites"] if r["fail"]], ["windows_apps_retry_time"]
        )

    def test_the_stage_without_the_windows_stage_is_refused_before_any_stage(self):
        rc, result, calls = self.run_stage("small", windows="0")
        self.assertEqual(rc, 1)
        self.assertEqual(calls, [])
        self.assertEqual([r["name"] for r in result["suites"] if r["fail"]], ["preflight"])


if __name__ == "__main__":
    unittest.main()
