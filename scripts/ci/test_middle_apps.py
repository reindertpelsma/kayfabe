"""Falsifier: PR inputs alter app policy, requested evidence disappears, or a short budget passes.
All controllers and stages use fake HTTP/results. No provider, VM or GPU is contacted.
"""

import importlib.util
import io
import json
import os
from pathlib import Path
import sys
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import test_vast_payload_apps as apps
import test_vast_payload_winapps as winapps
import test_vast_payload_v7 as v7

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location(
    "middle_runner", ROOT / "ci/vast-broker/runner.py"
)
r = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = r
spec.loader.exec_module(r)
fixture = {}
exec((ROOT / "ci/vast-broker/test/v5_fixture_data.py").read_text(), fixture)


class TrustedRunner(unittest.TestCase):
    def execute(
        self,
        tier="middle",
        enabled=True,
        rows=None,
        extra=None,
        verdict="PASS",
        policy_update=None,
        omit_policy=False,
    ):
        ctx = dict(
            run="123-1",
            sha="a" * 40,
            nonce="f" * 64,
            profile="check",
            apps_policy_required=True,
        )
        ctx.update(extra or {})
        manifest = dict(protocol_version=6, worker_version="v5", bundle_sha256="b" * 64)
        availability = fixture["availability"]()
        availability["apps_policy"] = dict(run=ctx["run"], tier=tier, enabled=enabled)
        availability["apps_policy"].update(policy_update or {})
        if omit_policy:
            availability.pop("apps_policy")
        status = dict(
            state="GRACE",
            verdict="PASS",
            run=ctx["run"],
            id="900",
            sha=ctx["sha"],
            cleanup_pending=True,
            result_at=2000,
            ended_at=None,
            started_at=1000,
            deadline_at=302000,
            gpu_model="RTX_4070",
            cost_estimate_usd=0.01,
            log_cursor=0,
            log_tail_sanitised="",
            bytes_dropped=0,
            tail_bytes_dropped=0,
            runner_nonce=ctx["nonce"],
            log_sha256="0" * 64,
            **manifest,
            **fixture["extra_status"](verdict)
        )
        status["verdict"] = verdict
        status["suites"] += rows if rows is not None else []
        if rows is None and enabled:
            status["suites"] += [
                dict(name=n, **{"pass": c, "fail": 0, "skip": 0})
                for n, c in [("apps", 63), ("windows_apps", 10)]
            ]
        calls = []

        def client(route, method="GET", body=None):
            calls.append((route, body))
            data = (
                availability
                if route.startswith("/availability")
                else status if route == "/rent" else {}
            )
            return r.Reply(
                200, {"content-type": "application/json"}, json.dumps(data).encode()
            )

        controller = r.Runner(
            ctx, manifest, client, emit=lambda _: None, summary=io.StringIO()
        )
        result = controller.execute()
        return result, controller, calls

    def test_middle_requests_both_real_app_stages(self):
        result, _, calls = self.execute()
        self.assertEqual(result["verdict"], "PASS")
        body = next(body for route, body in calls if route == "/rent")
        self.assertIs(body.get("linuxApps"), True)
        self.assertEqual(body.get("winApps"), "mid")

    def test_pr_request_fields_cannot_disable_middle_apps(self):
        _, _, calls = self.execute(
            extra=dict(
                linuxApps=False, winApps=None, tier="small", MIDDLE_APPS_ENABLED=False
            )
        )
        body = next(body for route, body in calls if route == "/rent")
        self.assertIs(body.get("linuxApps"), True)
        self.assertEqual(body.get("winApps"), "mid")

    def test_push_small_never_requests_apps(self):
        result, _, calls = self.execute(
            tier="small",
            enabled=False,
            extra=dict(linuxApps=True, winApps="mid", tier="middle"),
        )
        self.assertEqual(result["verdict"], "PASS")
        body = next(body for route, body in calls if route == "/rent")
        self.assertNotIn("linuxApps", body)
        self.assertNotIn("winApps", body)

    def test_owner_switch_disables_both_requests(self):
        _, _, calls = self.execute(enabled=False)
        body = next(body for route, body in calls if route == "/rent")
        self.assertNotIn("linuxApps", body)
        self.assertNotIn("winApps", body)

    def test_requested_linux_stage_cannot_disappear_from_a_pass(self):
        result, controller, _ = self.execute(
            rows=[dict(name="windows_apps", **{"pass": 10, "fail": 0, "skip": 0})]
        )
        self.assertEqual(result["verdict"], "FAIL")
        self.assertEqual(controller.retry, 1)

    def test_requested_windows_stage_cannot_disappear_from_a_pass(self):
        result, controller, _ = self.execute(
            rows=[dict(name="apps", **{"pass": 63, "fail": 0, "skip": 0})]
        )
        self.assertEqual(result["verdict"], "FAIL")
        self.assertEqual(controller.retry, 1)

    def test_linux_count_cannot_shrink(self):
        rows = [
            dict(name=n, **{"pass": c, "fail": 0, "skip": 0})
            for n, c in [("apps", 62), ("windows_apps", 10)]
        ]
        self.assertEqual(self.execute(rows=rows)[0]["verdict"], "FAIL")

    def test_windows_mid_count_cannot_shrink(self):
        rows = [
            dict(name=n, **{"pass": c, "fail": 0, "skip": 0})
            for n, c in [("apps", 63), ("windows_apps", 9)]
        ]
        self.assertEqual(self.execute(rows=rows)[0]["verdict"], "FAIL")

    def test_wrong_run_policy_is_rejected_before_rent(self):
        result, _, calls = self.execute(policy_update={"run": "999-1"})
        self.assertEqual(result["verified"], 0)
        self.assertFalse(any(route == "/rent" for route, _ in calls))

    def test_nonboolean_policy_is_rejected_before_rent(self):
        result, _, calls = self.execute(policy_update={"enabled": "true"})
        self.assertEqual(result["verified"], 0)
        self.assertFalse(any(route == "/rent" for route, _ in calls))

    def test_missing_policy_cannot_silently_omit_requested_apps(self):
        result, _, calls = self.execute(omit_policy=True)
        self.assertEqual(result["verified"], 0)
        self.assertFalse(any(route == "/rent" for route, _ in calls))

    def test_small_policy_cannot_enable_apps(self):
        result, _, calls = self.execute(tier="small", enabled=True)
        self.assertEqual(result["verified"], 0)
        self.assertFalse(any(route == "/rent" for route, _ in calls))

    def test_linux_media_failure_sets_retry_output(self):
        result, c, _ = self.execute(
            verdict="FAIL",
            rows=[dict(name="apps_retry_media", **{"pass": 0, "fail": 1, "skip": 0})],
        )
        self.assertEqual(result["verdict"], "FAIL")
        self.assertEqual(c.retry, 1)

    def test_windows_apps_media_failure_sets_retry_output(self):
        result, c, _ = self.execute(
            verdict="FAIL",
            rows=[
                dict(
                    name="windows_apps_retry_media", **{"pass": 0, "fail": 1, "skip": 0}
                )
            ],
        )
        self.assertEqual(result["verdict"], "FAIL")
        self.assertEqual(c.retry, 1)

    def test_linux_guest_failure_does_not_become_retry(self):
        result, c, _ = self.execute(
            verdict="FAIL",
            rows=[dict(name="apps", **{"pass": 0, "fail": 1, "skip": 0})],
        )
        self.assertEqual(result["verdict"], "FAIL")
        self.assertEqual(c.retry, 0)

    def test_windows_guest_failure_does_not_become_retry(self):
        result, c, _ = self.execute(
            verdict="FAIL",
            rows=[dict(name="windows_apps", **{"pass": 0, "fail": 1, "skip": 0})],
        )
        self.assertEqual(result["verdict"], "FAIL")
        self.assertEqual(c.retry, 0)

    def test_python_windows_count_matches_current_mid_profile(self):
        profile = json.loads(
            (ROOT / "scripts/bench/windows/appmatrix/gate_profiles.json").read_text()
        )["profiles"]["mid"]
        self.assertEqual(r.MIDDLE_APP_COUNTS["windows_apps"], len(profile["apps"]))
        self.assertEqual(profile["budget_s"], 3600)

    def test_python_linux_count_matches_supported_middle_gate(self):
        names = [
            line
            for line in (ROOT / "scripts/apps/gate_list_middle.txt")
            .read_text()
            .splitlines()
            if line and not line.startswith("#")
        ]
        self.assertEqual(r.MIDDLE_APP_COUNTS["apps"], len(names))


class TrustedMain(unittest.TestCase):
    def capture(self, event_name, inputs):
        seen = []

        class Controller:
            retry = 0

            def __init__(self, ctx, *args, **kw):
                seen.append(ctx)

            def execute(self):
                return dict(state="NONE", verdict="PASS", verified=1)

        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            event = {
                "inputs": inputs,
                "pull_request": {
                    "head": {"sha": "a" * 40, "repo": {"full_name": "o/r"}},
                    "base": {"ref": "master"},
                },
            }
            (path / "event").write_text(json.dumps(event))
            (path / "manifest").write_text(
                json.dumps(
                    dict(
                        protocol_version=6, worker_version="v5", bundle_sha256="b" * 64
                    )
                )
            )
            env = dict(
                GITHUB_EVENT_PATH=str(path / "event"),
                GITHUB_EVENT_NAME=event_name,
                GITHUB_SHA="a" * 40,
                GITHUB_REPOSITORY="o/r",
                GITHUB_REF="refs/heads/master",
                GITHUB_RUN_ID="123",
                GITHUB_RUN_ATTEMPT="1",
                GITHUB_OUTPUT=str(path / "output"),
                GITHUB_STEP_SUMMARY=str(path / "summary"),
                WORKER_URL="https://broker.example",
            )
            with patch.dict(os.environ, env, clear=True), patch.object(
                r, "Runner", Controller
            ), patch.object(sys, "argv", ["runner", str(path / "manifest")]), patch(
                "sys.stdout", io.StringIO()
            ):
                self.assertEqual(r.main(), 0)
        return seen[0]

    def test_pr_inputs_cannot_disable_authenticated_policy_read(self):
        ctx = self.capture(
            "pull_request_target",
            {
                "linuxApps": False,
                "winApps": None,
                "MIDDLE_APPS_ENABLED": "false",
                "apps_policy_required": False,
            },
        )
        self.assertIs(ctx.get("apps_policy_required"), True)

    def test_push_inputs_cannot_supply_the_apps_policy(self):
        ctx = self.capture(
            "push",
            {
                "linuxApps": True,
                "winApps": "mid",
                "tier": "middle",
                "apps_policy_required": False,
            },
        )
        self.assertIs(ctx.get("apps_policy_required"), True)


class PayloadMiddle(unittest.TestCase):
    setUp = v7.V7Tests.setUp
    _main_fixture = v7.V7Tests._main_fixture

    def go(self, soft=300, logs=None, env=None, probe=None):
        extra = {"KF_TIER": "middle", "KF_LINUX_APPS": "1", "KF_WINAPPS": "mid"}
        extra.update(env or {})
        stage_logs = {
            "apps": (apps.log(), 0),
            "windows_apps": winapps.apps_log("PASS", profile="mid"),
        }
        stage_logs.update(logs or {})
        with patch.object(self.m, "apps_gate_list", return_value=apps.NAMES):
            return self._main_fixture(
                windows="1",
                soft_ms=soft * 60000,
                extra_env=extra,
                stage_logs=stage_logs,
                probe=probe,
            )

    def test_explicit_middle_request_runs_both_after_the_linux_bar(self):
        rc, result, calls = self.go()
        self.assertEqual(rc, 0, result)
        self.assertEqual(calls[-3:], ["apps", "windows_gate", "windows_apps"])

    def test_linux_missing_output_is_retry_never_pass(self):
        rc, result, _ = self.go(logs={"apps": ("", 0)})
        self.assertEqual(rc, 1)
        self.assertIn("apps_retry_operator", [x["name"] for x in result["suites"]])

    def test_windows_missing_output_is_retry_never_pass(self):
        rc, result, _ = self.go(logs={"windows_apps": ("", 0)})
        self.assertEqual(rc, 1)
        self.assertIn(
            "windows_apps_retry_operator", [x["name"] for x in result["suites"]]
        )

    def test_short_suffix_does_not_start_linux_apps(self):
        rc, result, calls = self.go(soft=204)
        self.assertEqual(rc, 1)
        self.assertNotIn("apps", calls)
        self.assertIn("apps_retry_time", [x["name"] for x in result["suites"]])

    def test_another_short_suffix_cannot_consume_the_windows_apps_budget(self):
        rc, result, calls = self.go(soft=144)
        self.assertEqual(rc, 1)
        self.assertNotIn("apps", calls)
        self.assertNotIn("windows_gate", calls)

    def test_linux_bundle_retry_is_classified_as_media(self):
        rc, result, _ = self.go(
            logs={
                "apps": ("APPS_STAGE_RETRY class=NOT_BUILT\nAPPS_STAGE_EXIT rc=3\n", 3)
            }
        )
        self.assertEqual(rc, 1)
        self.assertIn("apps_retry_media", [x["name"] for x in result["suites"]])

    def test_windows_bundle_retry_remains_media(self):
        rc, result, _ = self.go(
            logs={
                "windows_apps": winapps.apps_log("RETRY", profile="mid", extra="MEDIA")
            }
        )
        self.assertEqual(rc, 1)
        self.assertIn("windows_apps_retry_media", [x["name"] for x in result["suites"]])

    def test_guest_failure_remains_fail(self):
        rc, result, calls = self.go(
            logs={
                "apps": (
                    apps.log(
                        results={"vectorAdd": "FAIL"}, verdict="APPS_STAGE_FAIL", rc=1
                    ),
                    1,
                )
            }
        )
        self.assertEqual(rc, 1)
        self.assertIn("apps", [x["name"] for x in result["suites"] if x["fail"]])
        self.assertNotIn("windows_gate", calls)

    def after_stage(self, stage_number, elapsed):
        now = [r.time.time()]
        start = now[0]
        calls = [0]

        def probe(_):
            calls[0] += 1
            if calls[0] == stage_number:
                now[0] = start + elapsed * 60

        with patch.object(self.m.time, "time", side_effect=lambda: now[0]):
            return self.go(probe=probe)

    def test_gate_does_not_start_without_time_for_mid_apps(self):
        rc, result, calls = self.after_stage(5, 156)
        self.assertEqual(rc, 1)
        self.assertNotIn("windows_gate", calls)
        self.assertIn("windows_retry_time", [x["name"] for x in result["suites"]])

    def test_gate_reserves_the_apps_suffix_after_another_time_jump(self):
        rc, result, calls = self.after_stage(5, 200)
        self.assertEqual(rc, 1)
        self.assertNotIn("windows_gate", calls)

    def test_mid_apps_need_their_cleanup_margin_too(self):
        rc, result, calls = self.after_stage(6, 235)
        self.assertEqual(rc, 1)
        self.assertNotIn("windows_apps", calls)
        self.assertIn("windows_apps_retry_time", [x["name"] for x in result["suites"]])

    def test_mid_apps_do_not_start_with_only_the_raw_matrix_budget(self):
        rc, result, calls = self.after_stage(6, 240)
        self.assertEqual(rc, 1)
        self.assertNotIn("windows_apps", calls)

    def test_linux_legacy_bundle_class_is_normalized_to_media(self):
        with self.assertRaises(self.m.EvidenceError) as caught:
            self.m.apps_rows(
                "APPS_STAGE_RETRY class=NOT_BUILT\nAPPS_STAGE_EXIT rc=3\n",
                3,
                apps.NAMES,
            )
        self.assertEqual(caught.exception.name, "apps_retry_media")

    def test_linux_explicit_media_class_is_not_operator(self):
        with self.assertRaises(self.m.EvidenceError) as caught:
            self.m.apps_rows(
                "APPS_STAGE_RETRY class=MEDIA\nAPPS_STAGE_EXIT rc=3\n", 3, apps.NAMES
            )
        self.assertEqual(caught.exception.name, "apps_retry_media")

    def test_explicit_linux_opt_in_cannot_enable_small(self):
        with self.assertRaises(self.m.EvidenceError):
            self.m.linuxapps_requested({"KF_LINUX_APPS": "1"}, "small")

    def test_explicit_linux_opt_in_cannot_enable_adhoc(self):
        with self.assertRaises(self.m.EvidenceError):
            self.m.linuxapps_requested({"KF_LINUX_APPS": "1"}, "adhoc")

    def test_invalid_linux_opt_in_is_not_silently_ignored(self):
        with self.assertRaises(self.m.EvidenceError):
            self.m.linuxapps_requested({"KF_LINUX_APPS": "0"}, "middle")

    def test_another_invalid_linux_opt_in_is_refused(self):
        with self.assertRaises(self.m.EvidenceError):
            self.m.linuxapps_requested({"KF_LINUX_APPS": "yes"}, "middle")

    def test_opt_in_is_consumed_before_child_helpers_run(self):
        snapshots = []
        rc, _, _ = self.go(probe=lambda env: snapshots.append(env))
        self.assertEqual(rc, 0)
        self.assertTrue(snapshots)
        self.assertTrue(all("KF_LINUX_APPS" not in env for env in snapshots))

    def test_direct_linux_opt_in_consumes_its_transport_flag(self):
        env = {"KF_LINUX_APPS": "1"}
        self.assertIs(self.m.linuxapps_requested(env, "middle"), True)
        self.assertNotIn("KF_LINUX_APPS", env)


class TierDefinitionTests(unittest.TestCase):
    setUp = v7.V7Tests.setUp
    _main_fixture = v7.V7Tests._main_fixture

    def test_llm_slot_is_explicitly_design_only(self):
        self.assertEqual(self.m.STAGE_SLOTS.get("llm_parity"), "DESIGN_ONLY")

    def test_llm_cannot_be_requested_as_a_large_stage(self):
        with self.assertRaises(self.m.EvidenceError):
            self.m.tier_requested({"KF_TIER": "large", "KF_STAGES": "llm_parity"}, True)

    def test_llm_only_cannot_silently_run_an_incomplete_large_tier(self):
        rc, _, calls = self._main_fixture(
            windows="1",
            soft_ms=300 * 60000,
            extra_env={"KF_TIER": "large", "KF_STAGES": "llm_parity"},
        )
        self.assertEqual(rc, 1)
        self.assertEqual(calls, [])

    def test_llm_plus_spec_cannot_start_any_work(self):
        rc, _, calls = self._main_fixture(
            windows="1",
            soft_ms=300 * 60000,
            extra_env={
                "KF_TIER": "large",
                "KF_STAGES": "specviewperf:llm_parity",
                "KF_SPEC": "lanea",
            },
        )
        self.assertEqual(rc, 1)
        self.assertEqual(calls, [])


class MissingMediaStage(unittest.TestCase):
    def check_profile(self, profile, stale_marker=False, empty_bundle=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "apps").mkdir()
            if stale_marker:
                (root / "apps/PROVISIONED").touch()
            if empty_bundle:
                (root / "apps/bundle.tgz").touch()
            (root / "list").write_text("vectorAdd\ndeviceQuery\n")
            matrix = root / "fake_matrix.sh"
            matrix.write_text("#!/bin/sh\nexit 3\n")
            env = {
                "PATH": os.environ["PATH"],
                "KF_APPS_ROOT": str(root / "apps"),
                "KF_APPS_MATRIX": str(matrix),
                "BENCH_DIR": str(root / "bench"),
                "KF_APPS_LIST": str(root / "list"),
            }
            ran = subprocess.run(
                [
                    "bash",
                    str(ROOT / "scripts/bench/box/apps_gate.sh"),
                    "fixture",
                    str(root / "prov"),
                    profile,
                ],
                env=env,
                capture_output=True,
                text=True,
                timeout=15,
            )
            self.assertEqual(ran.returncode, 3)
            self.assertIn("APPS_STAGE_RETRY class=MEDIA", ran.stdout)
            self.assertNotIn("APPS_STAGE_PASS", ran.stdout)
            self.assertNotIn("APPS_STAGE_START", ran.stdout)

    def test_middle_missing_bundle_is_media_without_launching_apps(self):
        self.check_profile("middle")

    def test_full_missing_bundle_also_uses_media(self):
        self.check_profile("full")

    def test_middle_stale_marker_without_bundle_is_media(self):
        self.check_profile("middle", stale_marker=True)

    def test_full_stale_marker_without_bundle_is_media(self):
        self.check_profile("full", stale_marker=True)

    def test_middle_stale_marker_with_empty_bundle_is_media(self):
        self.check_profile("middle", stale_marker=True, empty_bundle=True)

    def test_full_stale_marker_with_empty_bundle_is_media(self):
        self.check_profile("full", stale_marker=True, empty_bundle=True)


if __name__ == "__main__":
    unittest.main()
