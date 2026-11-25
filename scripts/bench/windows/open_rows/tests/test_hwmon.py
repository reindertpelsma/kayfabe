# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The LibreHardwareMonitor row (hwmon_lhm_nvapi): the judge, the runner, the release-zip anchor and the matrix row, all GPU-free, with fakes (no Windows, no network).

The dump fixtures are written to the format lhm_dump.ps1 prints, with the sensor names of LibreHardwareMonitor's NVIDIA group; they are constructed, not captured: no
Windows guest or native Windows run of this row has happened (docs/design/V3_OPEN_LANES_NVBW_LLAMAVK_LHM.md section 6).
"""
import contextlib
import copy
import hashlib
import io
import json
import os
import re
import tempfile
import unittest

import helpers as H
import hwmon_row as W
import kfopen as K
import kfopen_build as B
import kfopen_pins as P
import winapps
from kfopen_common import OpenRowError, Result, Trust

PINS = P.load_pins()
ROW = PINS["rows"]["hwmon_lhm_nvapi"]
APPS = {a["id"]: a for a in json.load(open(os.path.join(H.AM, "apps.json")))["apps"]}
GOOD = H.fixture_json("lhm_dump_nvidia.json")
SMI = W.parse_smi(H.fixture("smi_hwmon.txt"))


def failed(doc, smi=None, row=ROW):
    return [c[0] for c in W.judge(row, doc, smi)[0] if not c[1]]


def with_sensors(mutate):
    doc = copy.deepcopy(GOOD)
    mutate(doc["hardware"][0]["sensors"])
    return doc


class Judge(unittest.TestCase):
    def test_a_good_dump_passes_every_check(self):
        checks, facts = W.judge(ROW, GOOD, SMI)
        self.assertEqual([], [c for c in checks if not c[1]])
        self.assertEqual("NVIDIA GeForce RTX 4090", facts["gpu"])
        self.assertEqual(["Clock", "Fan", "Load", "Power", "SmallData", "Temperature", "Throughput"], facts["types"])
        self.assertIn("lhm_agrees_with_smi_temperature", [c[0] for c in checks])
        self.assertIn("lhm_agrees_with_smi_memory", [c[0] for c in checks])

    def test_no_nvidia_hardware_fails(self):
        self.assertIn("lhm_nvidia_hardware", failed(H.fixture_json("lhm_dump_gpu_disabled.json")))

    def test_hardware_of_another_type_is_not_the_nvapi_group(self):
        doc = copy.deepcopy(GOOD)
        doc["hardware"][0]["type"] = "GpuAmd"
        self.assertIn("lhm_nvidia_hardware", failed(doc))

    def test_a_gpu_that_is_not_named_like_an_nvidia_one_is_not_the_nvapi_group(self):
        doc = copy.deepcopy(GOOD)
        doc["hardware"][0]["name"] = "Microsoft Basic Render Driver"
        self.assertIn("lhm_nvidia_hardware", failed(doc))

    def test_every_required_sensor_type_must_have_a_value(self):
        for t in ROW["required_sensor_types"]:
            doc = with_sensors(lambda ss, t=t: [s.update(value=None) for s in ss if s["type"] == t])
            self.assertIn("lhm_sensor_" + t.lower(), failed(doc), t)
            doc = with_sensors(lambda ss, t=t: ss.__delitem__(slice(None), ) or ss.extend(s for s in GOOD["hardware"][0]["sensors"] if s["type"] != t))
            self.assertIn("lhm_sensor_" + t.lower(), failed(doc), t)

    def test_the_missing_value_and_the_missing_sensor_are_told_apart(self):
        doc = with_sensors(lambda ss: [s.update(value=None) for s in ss if s["type"] == "Temperature"])
        ok, why = [(c[1], c[2]) for c in W.judge(ROW, doc)[0] if c[0] == "lhm_sensor_temperature"][0]
        self.assertFalse(ok)
        self.assertIn("sensor_value_missing", why)

    def test_a_temperature_outside_the_window_fails(self):
        lo, hi = ROW["temperature_c"]
        for bad in (lo - 1, hi + 1, -273.0):
            doc = with_sensors(lambda ss, bad=bad: [s.update(value=bad) for s in ss if s["type"] == "Temperature"][:1])
            self.assertIn("lhm_temperature_plausible", failed(doc), bad)
        for ok in (lo, hi, 55.5):
            doc = with_sensors(lambda ss, ok=ok: [s.update(value=ok) for s in ss if s["type"] == "Temperature"])
            self.assertNotIn("lhm_temperature_plausible", failed(doc), ok)

    def test_a_load_outside_zero_to_one_hundred_fails(self):
        for bad in (-1.0, 100.5, 4294967.0):
            doc = with_sensors(lambda ss, bad=bad: [s.update(value=bad) for s in ss if s["type"] == "Load"][:1])
            self.assertIn("lhm_load_plausible", failed(doc), bad)

    def test_a_zero_clock_or_a_zero_memory_total_fails(self):
        doc = with_sensors(lambda ss: [s.update(value=0.0) for s in ss if s["type"] == "Clock"][:1])
        self.assertIn("lhm_clock_positive", failed(doc))
        doc = with_sensors(lambda ss: [s.update(value=0.0) for s in ss if s["name"] == "GPU Memory Total"])
        self.assertIn("lhm_memory_total_positive", failed(doc))

    def test_a_reader_error_fails_even_with_good_sensors(self):
        doc = copy.deepcopy(GOOD)
        doc["errors"] = ["System.IO.FileLoadException: Could not load file or assembly 'System.Memory'"]
        self.assertEqual(["lhm_no_reader_errors"], failed(doc))

    def test_nvidia_smi_disagreement_fails_and_a_missing_answer_skips_the_comparison(self):
        hot = dict(SMI, temperature_c=90.0)
        self.assertEqual(["lhm_agrees_with_smi_temperature"], failed(GOOD, hot))
        big = dict(SMI, memory_mib=SMI["memory_mib"] * 2)
        self.assertEqual(["lhm_agrees_with_smi_memory"], failed(GOOD, big))
        self.assertEqual([], failed(GOOD, dict(SMI, temperature_c=None, memory_mib=None)))
        self.assertEqual([], failed(GOOD, None))
        edge = dict(SMI, temperature_c=41.0 + ROW["smi_tolerance"]["temperature_c"])
        self.assertEqual([], failed(GOOD, edge))

    def test_powershell_writes_a_single_element_array_as_the_element(self):
        doc = copy.deepcopy(GOOD)
        gpu = doc["hardware"][0]
        doc["hardware"] = gpu                     # ConvertTo-Json: one hardware -> an object, not a one-element array
        gpu["sensors"] = gpu["sensors"]
        self.assertEqual([], failed(W.parse_dump(json.dumps({"hardware": [gpu], "errors": None, "lib_version": "x"}))))
        self.assertEqual([gpu], W.as_list(gpu) and [gpu])
        self.assertEqual([], W.as_list(None))
        sub = {"type": "GpuNvidia", "name": "NVIDIA GeForce RTX 4090", "sensors": GOOD["hardware"][0]["sensors"], "sub": []}
        nested = {"lib_version": "x", "errors": [], "hardware": [{"type": "Motherboard", "name": "board", "sensors": [], "sub": [sub]}]}
        self.assertEqual([], failed(nested))

    def test_the_parse_of_the_dump_refuses_garbage(self):
        for bad in ("", "no json at all", "[1, 2]", json.dumps({"hardware": "x"}), json.dumps({"nothing": 1})):
            with self.assertRaises(OpenRowError, msg=bad):
                W.parse_dump(bad)
        self.assertEqual("1", W.parse_dump("some banner line\n" + json.dumps({"hardware": [], "lib_version": "1"}))["lib_version"])

    def test_parse_smi(self):
        self.assertEqual({"name": "NVIDIA X", "temperature_c": 40.0, "memory_mib": 8192.0}, W.parse_smi("NVIDIA X, 40, 8192\n"))
        self.assertEqual({"name": "NVIDIA X", "temperature_c": None, "memory_mib": 8192.0}, W.parse_smi("NVIDIA X, [N/A], 8192"))
        self.assertIsNone(W.parse_smi(""))
        self.assertIsNone(W.parse_smi("garbage"))


class FakeLhmRunner(H.FakeRunner):
    def __init__(self, dump_text, rc=0, smi_text="NVIDIA GeForce RTX 4090, 40, 24564\n", smi_rc=0):
        super().__init__([(lambda a: a[0] == "powershell.exe", Result(rc, dump_text, 1.0)), (lambda a: a[0] == "nvidia-smi.exe", Result(smi_rc, smi_text))])


def run_row(runner, negctl=False):
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        rc = W.run_hwmon("hwmon_lhm_nvapi", ROW, runner, r"C:\kfopen\tools\lhm", negctl=negctl)
    return rc, buf.getvalue()


class Runner(unittest.TestCase):
    def test_a_good_run_passes_and_prints_the_line_the_matrix_row_expects(self):
        rc, out = run_row(FakeLhmRunner(H.fixture("lhm_dump_nvidia.json")))
        self.assertEqual(0, rc)
        self.assertIsNotNone(re.search(ROW["matrix"]["row"]["rx"], out, re.M), out)
        self.assertIn("KFOPEN HWMON gpu=NVIDIA_GeForce_RTX_4090 lib=0.9.6.0", out)
        self.assertNotIn("FAIL", out)

    def test_the_reader_is_asked_for_the_gpu_group_and_the_control_for_none(self):
        r = FakeLhmRunner(H.fixture("lhm_dump_nvidia.json"))
        run_row(r)
        argv = r.calls[0]["argv"]
        self.assertEqual(["-GpuEnabled", "1"], argv[-2:])
        self.assertTrue(argv[argv.index("-File") + 1].endswith("lhm_dump.ps1"))
        r = FakeLhmRunner(H.fixture("lhm_dump_gpu_disabled.json"))
        run_row(r, negctl=True)
        self.assertEqual(["-GpuEnabled", "0"], r.calls[0]["argv"][-2:])

    def test_a_missing_sensor_type_fails_the_row_and_names_it(self):
        doc = copy.deepcopy(GOOD)
        doc["hardware"][0]["sensors"] = [s for s in doc["hardware"][0]["sensors"] if s["type"] != "Temperature"]
        rc, out = run_row(FakeLhmRunner(json.dumps(doc)))
        self.assertEqual(1, rc)
        self.assertIn("CHECK lhm_sensor_temperature FAIL", out)
        self.assertIn("KFOPEN VERDICT FAIL row=hwmon_lhm_nvapi failed=lhm_sensor_temperature", out)
        self.assertIsNone(re.search(ROW["matrix"]["row"]["rx"], out, re.M))

    def test_a_reader_that_prints_nothing_or_not_json_is_a_fail_never_a_pass(self):
        for text in ("", "not json"):
            rc, out = run_row(FakeLhmRunner(text))
            self.assertEqual(1, rc)
            self.assertIn("CHECK lhm_dump FAIL", out)
            self.assertNotIn("VERDICT PASS", out)

    def run_with(self, mutate, **kw):
        doc = copy.deepcopy(GOOD)
        mutate(doc["hardware"][0]["sensors"])
        return run_row(FakeLhmRunner(json.dumps(doc), **kw))

    def test_sensors_that_have_no_value_fail_the_row_and_name_the_type(self):
        rc, out = self.run_with(lambda ss: [s.update(value=None) for s in ss if s["type"] == "Temperature"])
        self.assertEqual(1, rc)
        self.assertIn("failed=lhm_sensor_temperature", out)
        self.assertIn("sensor_value_missing", out)

    def test_hardware_of_the_wrong_type_fails_the_row(self):
        doc = copy.deepcopy(GOOD)
        doc["hardware"][0]["type"] = "GpuAmd"
        rc, out = run_row(FakeLhmRunner(json.dumps(doc)))
        self.assertEqual(1, rc)
        self.assertIn("failed=lhm_nvidia_hardware", out)

    def test_hardware_with_a_name_that_is_not_an_nvidia_one_fails_the_row(self):
        doc = copy.deepcopy(GOOD)
        doc["hardware"][0]["name"] = "Radeon RX 7900"
        rc, out = run_row(FakeLhmRunner(json.dumps(doc)))
        self.assertEqual(1, rc)
        self.assertIn("failed=lhm_nvidia_hardware", out)

    def test_an_implausible_temperature_fails_the_row(self):
        rc, out = self.run_with(lambda ss: [s.update(value=900.0) for s in ss if s["type"] == "Temperature"])
        self.assertEqual(1, rc)
        self.assertIn("failed=lhm_temperature_plausible", out)

    def test_an_implausible_load_fails_the_row(self):
        rc, out = self.run_with(lambda ss: [s.update(value=250.0) for s in ss if s["type"] == "Load"][:1])
        self.assertEqual(1, rc)
        self.assertIn("failed=lhm_load_plausible", out)

    def test_a_zero_clock_fails_the_row(self):
        rc, out = self.run_with(lambda ss: [s.update(value=0.0) for s in ss if s["type"] == "Clock"][:1])
        self.assertEqual(1, rc)
        self.assertIn("failed=lhm_clock_positive", out)

    def test_a_zero_memory_total_fails_the_row(self):
        rc, out = self.run_with(lambda ss: [s.update(value=0.0) for s in ss if s["name"] == "GPU Memory Total"])
        self.assertEqual(1, rc)
        self.assertIn("failed=lhm_memory_total_positive", out)

    def test_a_temperature_that_disagrees_with_nvidia_smi_fails_the_row(self):
        rc, out = run_row(FakeLhmRunner(H.fixture("lhm_dump_nvidia.json"), smi_text="NVIDIA GeForce RTX 4090, 95, 24564\n"))
        self.assertEqual(1, rc)
        self.assertIn("failed=lhm_agrees_with_smi_temperature", out)

    def test_a_memory_size_that_disagrees_with_nvidia_smi_fails_the_row(self):
        rc, out = run_row(FakeLhmRunner(H.fixture("lhm_dump_nvidia.json"), smi_text="NVIDIA GeForce RTX 4090, 40, 8192\n"))
        self.assertEqual(1, rc)
        self.assertIn("failed=lhm_agrees_with_smi_memory", out)

    def test_a_reader_error_fails_the_row(self):
        doc = copy.deepcopy(GOOD)
        doc["errors"] = ["System.IO.FileLoadException: x"]
        rc, out = run_row(FakeLhmRunner(json.dumps(doc)))
        self.assertEqual(1, rc)
        self.assertIn("failed=lhm_no_reader_errors", out)

    def test_a_reader_that_exits_nonzero_fails_the_row_even_with_a_good_dump(self):
        rc, out = run_row(FakeLhmRunner(H.fixture("lhm_dump_nvidia.json"), rc=1))
        self.assertEqual(1, rc)
        self.assertIn("DUMP_FAILED", out)

    def test_a_negative_control_with_a_failing_reader_is_broken_even_with_a_gpu_less_dump(self):
        rc, out = run_row(FakeLhmRunner(H.fixture("lhm_dump_gpu_disabled.json"), rc=1), negctl=True)
        self.assertEqual(1, rc)
        self.assertIn("CONTROL_BROKEN", out)
        self.assertNotIn("CONTROL_OK", out)

    def test_the_negative_control_is_ok_only_when_the_judge_fails_the_disabled_reader(self):
        rc, out = run_row(FakeLhmRunner(H.fixture("lhm_dump_gpu_disabled.json")), negctl=True)
        self.assertEqual(0, rc)
        self.assertIn("KFOPEN NEGCTL CONTROL_OK row=hwmon_lhm_nvapi judged=FAIL", out)
        self.assertNotIn("VERDICT PASS", out)

    def test_a_negative_control_that_the_judge_passes_is_broken(self):
        rc, out = run_row(FakeLhmRunner(H.fixture("lhm_dump_nvidia.json")), negctl=True)
        self.assertEqual(1, rc)
        self.assertIn("CONTROL_BROKEN", out)
        self.assertNotIn("VERDICT PASS", out)

    def test_a_negative_control_whose_reader_did_not_run_is_broken_not_ok(self):
        rc, out = run_row(FakeLhmRunner("", rc=1), negctl=True)
        self.assertEqual(1, rc)
        self.assertIn("CONTROL_BROKEN row=hwmon_lhm_nvapi judged=ERROR", out)

    def test_nvidia_smi_trouble_never_fails_the_row_by_itself(self):
        rc, out = run_row(FakeLhmRunner(H.fixture("lhm_dump_nvidia.json"), smi_rc=1))
        self.assertEqual(0, rc)
        self.assertIn("smi=n/a", out)

    def test_every_command_is_an_argument_list(self):
        r = FakeLhmRunner(H.fixture("lhm_dump_nvidia.json"))
        run_row(r)
        for call in r.calls:
            self.assertTrue(all(isinstance(a, str) for a in call["argv"]))
        self.assertEqual(["-NoProfile", "-ExecutionPolicy", "Bypass"], r.calls[0]["argv"][1:4])


class ReleaseZipAnchor(unittest.TestCase):
    """The LibreHardwareMonitor release zip: its sha256 is the release's own, read at run time; nothing about the binary is pinned in git."""

    def setUp(self):
        self._t = tempfile.TemporaryDirectory(prefix="kfopen-lhm-")
        self.tmp = self._t.name
        self.addCleanup(self._t.cleanup)
        self.spec = PINS["tools"]["lhm"]

    def zip_bytes(self, with_dll=True):
        import zipfile
        buf = io.BytesIO()
        with zipfile.ZipFile(buf, "w") as z:
            z.writestr("HidSharp.dll", b"h")
            if with_dll:
                z.writestr("LibreHardwareMonitorLib.dll", b"lib")
        return buf.getvalue()

    def release(self, data, digest=None):
        rel = H.fixture_json("github_release_lhm.json")
        rel["assets"][1]["digest"] = "sha256:" + (digest or hashlib.sha256(data).hexdigest())
        return json.dumps(rel)

    def ctx(self, data, rel_json, trust=None):
        url = self.spec["download_prefix"] + self.spec["asset"]
        fetcher = H.FakeFetcher({self.spec["release_api"]: rel_json}, {url: data})
        return B.Ctx(PINS, root=self.tmp, runner=H.FakeRunner(), fetcher=fetcher, trust=trust or Trust(), log=lambda *a: None, jobs=2), fetcher

    def test_the_zip_is_unpacked_when_its_sha256_equals_the_releases(self):
        data = self.zip_bytes()
        ctx, fetcher = self.ctx(data, self.release(data))
        d = B.ensure_lhm(ctx)
        self.assertTrue(os.path.exists(os.path.join(d, "LibreHardwareMonitorLib.dll")))
        self.assertEqual("verified", ctx.receipt["tools"]["lhm"]["verified"])
        self.assertEqual(hashlib.sha256(data).hexdigest(), ctx.receipt["tools"]["lhm"]["sha256"])

    def test_a_zip_that_is_not_what_the_release_says_is_refused_and_not_unpacked(self):
        data = self.zip_bytes()
        ctx, _ = self.ctx(data, self.release(data, digest="f" * 64))
        with self.assertRaises(OpenRowError) as e:
            B.ensure_lhm(ctx)
        self.assertIn("HASH_MISMATCH", str(e.exception))
        self.assertFalse(os.path.exists(os.path.join(self.tmp, "tools", "lhm", "LibreHardwareMonitorLib.dll")))

    def test_an_asset_url_outside_the_pinned_release_is_refused(self):
        data = self.zip_bytes()
        rel = json.loads(self.release(data))
        rel["assets"][1]["browser_download_url"] = "https://evil.example/LibreHardwareMonitor.zip"
        ctx, _ = self.ctx(data, json.dumps(rel))
        with self.assertRaises(OpenRowError) as e:
            B.ensure_lhm(ctx)
        self.assertIn("ANCHOR_ORIGIN", str(e.exception))

    def test_a_release_with_no_digest_is_refused(self):
        data = self.zip_bytes()
        rel = json.loads(self.release(data))
        del rel["assets"][1]["digest"]
        ctx, _ = self.ctx(data, json.dumps(rel))
        with self.assertRaises(OpenRowError) as e:
            B.ensure_lhm(ctx)
        self.assertIn("ANCHOR_MISSING", str(e.exception))

    def test_lifting_the_anchor_is_explicit_and_recorded(self):
        data = self.zip_bytes()
        ctx, _ = self.ctx(data, self.release(data, digest="f" * 64), trust=Trust(accept_unanchored="my own mirror"))
        B.ensure_lhm(ctx)
        self.assertEqual("lifted: my own mirror", ctx.receipt["tools"]["lhm"]["verified"])

    def test_a_zip_without_the_library_is_an_install_failure(self):
        data = self.zip_bytes(with_dll=False)
        ctx, _ = self.ctx(data, self.release(data))
        with self.assertRaises(OpenRowError) as e:
            B.ensure_lhm(ctx)
        self.assertIn("INSTALL_FAILED", str(e.exception))

    def test_a_failed_install_records_nothing_in_the_receipt(self):
        data = self.zip_bytes(with_dll=False)
        ctx, _ = self.ctx(data, self.release(data))
        with self.assertRaises(OpenRowError):
            B.ensure_lhm(ctx)
        self.assertNotIn("lhm", ctx.receipt["tools"])

    def test_a_tool_the_user_passes_is_trusted_and_hash_recorded(self):
        d = os.path.join(self.tmp, "mine")
        os.makedirs(d)
        ctx, fetcher = self.ctx(b"", "{}", trust=Trust(user_tools={"lhm": d}))
        self.assertEqual(d, B.ensure_lhm(ctx))
        self.assertEqual("trusted-by-user", ctx.receipt["tools"]["lhm"]["verified"])
        self.assertEqual([], fetcher.downloads)

    def test_an_installed_tool_is_not_downloaded_again(self):
        data = self.zip_bytes()
        ctx, fetcher = self.ctx(data, self.release(data))
        B.ensure_lhm(ctx)
        n = len(fetcher.downloads)
        ctx2, fetcher2 = self.ctx(data, self.release(data))
        B.ensure_lhm(ctx2)
        self.assertEqual(1, n)
        self.assertEqual([], fetcher2.downloads)

    def test_the_expected_hash_is_read_from_the_pinned_release_under_the_pinned_prefix(self):
        from unittest import mock
        data = self.zip_bytes()
        ctx, _ = self.ctx(data, self.release(data))
        real = P.expected_from_github_release
        with mock.patch.object(P, "expected_from_github_release", side_effect=real) as spy:
            B.ensure_lhm(ctx)
        self.assertEqual((self.spec["asset"], self.spec["download_prefix"]), spy.call_args[0][1:])

    def test_the_row_needs_only_the_lhm_tool(self):
        self.assertEqual(("lhm",), B.ROW_NEEDS["hwmon"])
        self.assertEqual({"lhm"}, set(B.ensure_toolchain(self.ctx(b"", "{}", Trust(user_tools={"lhm": self.tmp}))[0], ("lhm",))))


class PinsPolicy(unittest.TestCase):
    def test_the_committed_tool_entry_is_an_upstream_release_with_no_digest(self):
        spec = PINS["tools"]["lhm"]
        self.assertEqual("upstream-sha256", spec["anchor"])
        self.assertEqual("release-zip", spec["kind"])
        self.assertTrue(spec["download_prefix"].startswith("https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/releases/download/" + spec["release_tag"]))
        self.assertEqual("MPL-2.0", spec["licence"])
        self.assertNotRegex(json.dumps(spec), r"(?i)\b[0-9a-f]{64}\b")

    def test_a_release_zip_without_its_release_fields_or_with_another_anchor_is_refused(self):
        for mutate in (lambda d: d["tools"]["lhm"].pop("release_api"), lambda d: d["tools"]["lhm"].pop("asset"), lambda d: d["tools"]["lhm"].update(anchor="authenticode", signer="x"),
                       lambda d: d["tools"]["lhm"].update(sha256="a" * 64), lambda d: d["tools"]["lhm"].update(release_api="http://x")):
            doc = copy.deepcopy(PINS)
            mutate(doc)
            with self.assertRaises(OpenRowError):
                P.validate_pins(doc)

    def test_a_hwmon_row_must_name_a_tool_that_exists(self):
        doc = copy.deepcopy(PINS)
        doc["rows"]["hwmon_lhm_nvapi"]["tool"] = "nope"
        with self.assertRaises(OpenRowError):
            P.validate_pins(doc)

    def test_the_row_states_its_thresholds_and_the_negative_control(self):
        self.assertEqual("GpuNvidia", ROW["gpu_hardware_type"])
        self.assertEqual("gpu-disabled", ROW["negctl"])
        self.assertTrue({"Temperature", "Clock", "Load", "SmallData"} <= set(ROW["required_sensor_types"]))

    def test_plan_names_the_anchor(self):
        steps = B.plan(PINS, "hwmon_lhm_nvapi")
        self.assertEqual(["lhm"], [s["tool"] for s in steps])
        self.assertEqual("upstream-sha256", steps[0]["anchor"])


class MatrixRow(unittest.TestCase):
    """The row is deferred as a matrix row (apps.json is hash-covered by bundle.pin: a new row there needs the coordinator's republish); its definition is in
    open_rows.json, and these tests hold that definition against what the code prints, so that wiring it later is one call of gen_apps.open_row."""
    spec = ROW["matrix"]["row"]

    def test_the_row_is_deferred_with_a_reason_and_is_in_no_matrix_file_yet(self):
        self.assertEqual("deferred", ROW["matrix"]["state"])
        self.assertIn("bundle.pin", ROW["matrix"]["reason"])
        self.assertNotIn("hwmon_lhm_nvapi", APPS)
        gen = open(os.path.join(H.AM, "gen_apps.py")).read()
        self.assertNotIn("hwmon_lhm_nvapi", gen)
        prof = json.load(open(os.path.join(H.AM, "gate_profiles.json")))
        self.assertNotIn("hwmon_lhm_nvapi", json.dumps(prof))

    def test_a_matrix_block_must_be_deferred_with_a_reason_and_the_definition(self):
        for mutate in (lambda r: r["matrix"].update(state="wired"), lambda r: r["matrix"].update(reason="short"), lambda r: r["matrix"]["row"].pop("rx")):
            doc = copy.deepcopy(PINS)
            mutate(doc["rows"]["hwmon_lhm_nvapi"])
            with self.assertRaises(OpenRowError):
                P.validate_pins(doc)

    def test_the_deferred_rx_matches_what_a_passing_run_prints_and_nothing_else(self):
        rc, out = run_row(FakeLhmRunner(H.fixture("lhm_dump_nvidia.json")))
        self.assertEqual(0, rc)
        self.assertIsNotNone(re.search(self.spec["rx"], out, re.M), out)
        for text in ("KFNOTRUN hwmon_lhm_nvapi reason=x\n", "KFOPEN NEGCTL CONTROL_OK row=hwmon_lhm_nvapi judged=FAIL\n", "KFOPEN VERDICT FAIL row=hwmon_lhm_nvapi failed=x\n"):
            self.assertIsNone(re.search(self.spec["rx"], text, re.M))

    def test_the_gpu_use_proof_is_the_guest_self_report_of_the_nvidia_hardware(self):
        self.assertEqual(["out:KFOPEN HWMON gpu=NVIDIA"], self.spec["proof"])
        regex = self.spec["proof"][0][len("out:"):]
        rc, out = run_row(FakeLhmRunner(H.fixture("lhm_dump_nvidia.json")))
        self.assertIsNotNone(re.search(regex, out))
        rc, out = run_row(FakeLhmRunner(H.fixture("lhm_dump_gpu_disabled.json")))
        self.assertIsNone(re.search(regex, out), "the GPU-less reader must not satisfy the proof")

    def test_the_deferred_definition_has_the_shape_of_the_other_open_rows(self):
        self.assertEqual({"id", "category", "session", "tier", "timeout_s", "expect_s", "rx", "proof", "smi"}, set(self.spec))
        self.assertEqual("service", self.spec["session"])
        self.assertEqual(3, self.spec["tier"])
        self.assertGreaterEqual(self.spec["timeout_s"], self.spec["expect_s"])
        self.assertGreaterEqual(self.spec["timeout_s"], ROW["timeout_s"])
        self.assertFalse(self.spec["smi"])

    def test_the_reader_and_the_judge_are_pushed_to_the_guest(self):
        files = set(winapps.open_rows_files())
        self.assertTrue({"hwmon_row.py", "lhm_dump.ps1"} <= files)
        for mod in re.findall(r"^import (\w+)", open(os.path.join(H.OR, "kfopen.py")).read(), re.M):
            if os.path.exists(os.path.join(H.OR, mod + ".py")):
                self.assertIn(mod + ".py", files)

    def test_the_ps_reader_is_glue(self):
        text = open(os.path.join(H.OR, "lhm_dump.ps1"), encoding="utf-8").read()
        self.assertLess(len(text.splitlines()), 60)
        text.encode("ascii")
        for bad in ("Invoke-Expression", "iex ", "DownloadFile", "Invoke-WebRequest", "Start-Process"):
            self.assertNotIn(bad, text)
        self.assertIn("-GpuEnabled", text)
        self.assertIn("IsGpuEnabled = [bool]$GpuEnabled", text)

    def test_offline_without_the_tool_is_notrun_and_with_it_is_not(self):
        from unittest import mock
        with tempfile.TemporaryDirectory() as t:
            ctx = B.Ctx(PINS, root=t, runner=H.FakeRunner(), fetcher=H.FakeFetcher(), log=lambda *a: None)
            with mock.patch.object(K, "can_reach", lambda *a, **k: False):
                self.assertIn("offline-guest-without-lhm", K.preflight(ctx, "hwmon_lhm_nvapi", dry_run=True))
            os.makedirs(os.path.join(t, "tools", "lhm"))
            with mock.patch.object(K, "can_reach", lambda *a, **k: False):
                self.assertIsNone(K.preflight(ctx, "hwmon_lhm_nvapi", dry_run=True))

    def test_run_row_prints_kfnotrun_offline_without_the_tool_and_returns_3(self):
        import argparse
        import sys
        from unittest import mock
        with tempfile.TemporaryDirectory() as t:
            runner = FakeLhmRunner(H.fixture("lhm_dump_nvidia.json"))
            ctx = B.Ctx(PINS, root=t, runner=runner, fetcher=H.FakeFetcher(), log=lambda *a: None)
            a = argparse.Namespace(out=t, control=None, role="guest", allow_different_gpu=False, smoke=False, share=False, negctl=False)
            buf = io.StringIO()
            with mock.patch.object(sys, "platform", "win32"), mock.patch.object(K, "can_reach", lambda *x, **k: False), contextlib.redirect_stdout(buf):
                rc = K.run_row(ctx, "hwmon_lhm_nvapi", a)
            self.assertEqual(3, rc)
            self.assertIn("KFNOTRUN hwmon_lhm_nvapi reason=offline-guest-without-lhm", buf.getvalue())
            self.assertEqual([], runner.calls)

    def test_a_machine_that_is_not_windows_is_notrun_with_a_named_reason(self):
        with tempfile.TemporaryDirectory() as t:
            ctx = B.Ctx(PINS, root=t, runner=H.FakeRunner(), fetcher=H.FakeFetcher(), log=lambda *a: None)
            if os.name != "nt":
                self.assertIn("not-windows", K.preflight(ctx, "hwmon_lhm_nvapi"))

    def test_run_row_dispatches_a_hwmon_row_to_the_judge_with_the_control_flag(self):
        import argparse
        from unittest import mock
        for negctl, dump in ((False, "lhm_dump_nvidia.json"), (True, "lhm_dump_gpu_disabled.json")):
            with tempfile.TemporaryDirectory() as t:
                runner = FakeLhmRunner(H.fixture(dump))
                ctx = B.Ctx(PINS, root=t, runner=runner, fetcher=H.FakeFetcher(), log=lambda *a: None)
                a = argparse.Namespace(out=t, control=None, role="guest", allow_different_gpu=False, smoke=False, share=False, negctl=negctl)
                buf = io.StringIO()
                with mock.patch.object(K, "preflight", lambda *x, **k: None), mock.patch.object(K, "prepare", lambda c, r: {"lib_dir": t}), contextlib.redirect_stdout(buf):
                    rc = K.run_row(ctx, "hwmon_lhm_nvapi", a)
                self.assertEqual(0, rc, buf.getvalue())
                self.assertEqual(["-GpuEnabled", "0" if negctl else "1"], runner.calls[0]["argv"][-2:])

    def test_the_list_and_check_commands_know_the_row(self):
        self.assertEqual([], K.check_all())
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            K.main(["list"])
        self.assertIn("hwmon_lhm_nvapi", buf.getvalue())


if __name__ == "__main__":
    unittest.main()
