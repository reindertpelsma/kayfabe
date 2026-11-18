#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of the GPU-PV gap probes and the differential runner core (scripts/apps/gap_probes). Stdlib only.

STATUS: RESEARCH, 2026-11-18 (docs/design/V3_GPUPV_GAP_MATRIX.md). Mutation checks: scripts/ci/gap_probes_mutants.py (two named killers per mutant).
"""
import atexit
import io
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]
GP = ROOT / "scripts/apps/gap_probes"
sys.path.insert(0, str(GP))
sys.path.insert(0, str(ROOT / "scripts/apps/open_rows"))
import diffrun as D  # noqa: E402
import gap_lib as L  # noqa: E402
import cts_groups as CG  # noqa: E402
import cts_report as CR  # noqa: E402
import gap_probes as GPR  # noqa: E402
import rowio  # noqa: E402

TABLE = (GP / "gap_cases.tsv").read_text()
HDR = "id|group|kind|prog|timeout_s|gpu|tags|env|falsifier\n"
FALS = "the guest differs from the host in a measurable way"


def case_line(id="a", kind="c", prog="a", timeout="30", gpu="yes", tags="-", env="-", fal=FALS):
    return "|".join((id, "g", kind, prog, timeout, gpu, tags, env, fal)) + "\n"


class CaseTable(unittest.TestCase):
    def test_the_real_table_loads_and_every_case_has_a_falsifier(self):
        cases = L.load_cases(TABLE)
        self.assertGreaterEqual(len(cases), 30)
        for c in cases:
            self.assertGreaterEqual(len(c["falsifier"]), 20, c["id"])

    def test_ids_are_unique_and_a_duplicate_is_refused(self):
        with self.assertRaises(ValueError):
            L.load_cases(HDR + case_line("a") + case_line("a"))

    def test_a_case_without_a_falsifier_is_refused(self):
        with self.assertRaises(ValueError):
            L.load_cases(HDR + case_line(fal="short"))

    def test_unknown_kind_tag_or_a_tiny_timeout_is_refused(self):
        for bad in (case_line(kind="sh"), case_line(tags="root_only"), case_line(timeout="3"), case_line(gpu="maybe")):
            with self.assertRaises(ValueError):
                L.load_cases(HDR + bad)

    def test_env_is_parsed_as_data(self):
        c = L.load_cases(HDR + case_line(env="A=1,B=x=y"))[0]
        self.assertEqual(c["env"], {"A": "1", "B": "x=y"})

    def test_the_table_names_no_image_and_no_vendor_app(self):
        """The standalone rule: images are configuration, not code. A probe row never names one."""
        for f in GP.iterdir():
            if f.suffix in (".tsv", ".py", ".c"):
                text = f.read_text().lower()
                for word in ("propimage", "image a", "image b", "photoshop", "premiere", "davinci", "adobe", "matlab"):
                    self.assertNotIn(word, text, "%s names %s" % (f.name, word))

    def test_every_c_case_exists_in_the_binary_and_the_binary_has_no_unlisted_case(self):
        exe = build_c(self)
        names = set(subprocess.run([exe, "list"], capture_output=True, text=True).stdout.split())
        wanted = {c["prog"] for c in L.load_cases(TABLE) if c["kind"] == "c"}
        self.assertEqual(wanted - names, set())
        self.assertEqual(names - wanted, {"kill_target"})   # the one case that is a fixture of the lifecycle cases, not a probe

    def test_every_py_case_exists_in_gap_scripts(self):
        import gap_scripts
        for c in L.load_cases(TABLE):
            if c["kind"] == "py":
                self.assertIn(c["prog"], gap_scripts.CASES)

    def test_the_disruptive_cases_are_guest_only_and_opt_in(self):
        by = {c["id"]: c for c in L.load_cases(TABLE)}
        for name in ("life_module_cycle", "life_reset", "life_busy_poweroff"):
            self.assertIn("guest_only", by[name]["tags"])
        self.assertIn("destructive", by["life_reset"]["tags"])
        self.assertIn("terminal", by["life_busy_poweroff"]["tags"])


_C = {}


def build_c(tc, flags=()):
    key = tuple(flags)
    if key not in _C:
        d = tempfile.mkdtemp(prefix="gapc")
        atexit.register(shutil.rmtree, d, True)
        exe = str(Path(d) / "gap_probe")
        p = subprocess.run(["gcc", "-O2", "-Wall", "-Werror=implicit", "-Wno-unused-function", "-Wno-unused-const-variable", *flags, "-o", exe, str(GP / "gap_probe.c"), "-ldl"],
                           capture_output=True, text=True)
        if p.returncode:
            tc.fail(p.stderr[-800:])
        _C[key] = exe
    return _C[key]


class Selection(unittest.TestCase):
    cases = L.load_cases(TABLE)

    def ids(self, side, **kw):
        return [c["id"] for c in L.select(self.cases, side, **kw)[0]]

    def test_a_guest_only_case_never_runs_on_the_host_side(self):
        self.assertNotIn("life_module_cycle", self.ids("host", destructive=True, terminal=True))
        self.assertIn("life_module_cycle", self.ids("guest"))

    def test_destructive_and_terminal_cases_need_their_flag(self):
        self.assertNotIn("life_reset", self.ids("guest"))
        self.assertIn("life_reset", self.ids("guest", destructive=True))
        self.assertNotIn("life_busy_poweroff", self.ids("guest", destructive=True))
        self.assertIn("life_busy_poweroff", self.ids("guest", terminal=True))

    def test_the_terminal_case_is_last_and_destructive_ones_come_after_the_safe_ones(self):
        ids = self.ids("guest", destructive=True, terminal=True)
        self.assertEqual(ids[-1], "life_busy_poweroff")
        self.assertGreater(ids.index("life_reset"), ids.index("spin_30s"))

    def test_only_selects_by_glob_and_skipped_carry_a_reason(self):
        run, skipped = L.select(self.cases, "host", only="pool_*,canary")
        self.assertEqual(sorted(c["id"] for c in run), ["canary", "pool_alloc_free", "pool_ipc_export", "pool_threshold_trim"])
        run, skipped = L.select(self.cases, "host")
        self.assertIn(("life_reset", "guest_only"), [(c["id"], r) for c, r in skipped])


class SelectionOrder(unittest.TestCase):
    TABLE = HDR + case_line("t", prog="t", tags="terminal") + case_line("d", prog="d", tags="destructive") + case_line("s", prog="s")

    def test_a_terminal_case_listed_first_still_runs_last(self):
        run, _ = L.select(L.load_cases(self.TABLE), "guest", destructive=True, terminal=True)
        self.assertEqual([c["id"] for c in run], ["s", "d", "t"])

    def test_a_destructive_case_listed_before_a_safe_one_runs_after_it(self):
        run, _ = L.select(L.load_cases(self.TABLE), "guest", destructive=True, terminal=True)
        self.assertLess([c["id"] for c in run].index("s"), [c["id"] for c in run].index("d"))

    def test_the_default_selection_has_neither_destructive_nor_terminal_cases(self):
        run, skipped = L.select(L.load_cases(self.TABLE), "guest")
        self.assertEqual([c["id"] for c in run], ["s"])
        self.assertEqual(sorted(r for _, r in skipped), ["destructive_not_requested", "terminal_not_requested"])


class ProbeOutput(unittest.TestCase):
    OUT = "GAPKV x attr=41:1\nGAPKV x attr=83:0\nGAPKV x mem_total_mib=4096\nGAPKV other k=v\nGAPPROBE case=x status=PASS rc=0 secs=1.5 reason=-\n"

    def test_attribute_lines_become_per_attribute_keys(self):
        p = L.parse_probe_output(self.OUT, "x")
        self.assertEqual(p["kv"]["attr.41"], "1")
        self.assertEqual(p["kv"]["attr.83"], "0")
        self.assertEqual(p["kv"]["mem_total_mib"], "4096")

    def test_lines_of_another_case_are_ignored(self):
        self.assertNotIn("k", L.parse_probe_output(self.OUT, "x")["kv"])
        self.assertIsNone(L.parse_probe_output(self.OUT, "other"))

    def test_an_unknown_status_or_no_result_line_is_none(self):
        self.assertIsNone(L.parse_probe_output("GAPPROBE case=x status=GREAT rc=0 secs=1 reason=-\n", "x"))
        self.assertIsNone(L.parse_probe_output("GAPKV x a=1\n", "x"))

    def test_values_cannot_inject_a_tab_or_a_second_line(self):
        self.assertEqual(L.clean("a\tb|c\nd"), "a b c d")

    def test_a_timeout_a_crash_and_a_missing_line_are_failures_with_a_named_reason(self):
        self.assertEqual(L.classify_process(124, True, None), ("FAIL", "timeout"))
        self.assertEqual(L.classify_process(-11, False, None), ("FAIL", "crash_signal_11"))
        self.assertEqual(L.classify_process(3, False, None), ("FAIL", "no_result_line_exit_3"))
        self.assertEqual(L.classify_process(127, False, None), ("NOT_RUN", "cannot_execute"))

    def test_a_parsed_result_wins_over_the_exit_code_but_not_over_a_timeout(self):
        parsed = {"status": "NOT_SUPPORTED", "reason": "attr_0"}
        self.assertEqual(L.classify_process(0, False, parsed), ("NOT_SUPPORTED", "attr_0"))
        self.assertEqual(L.classify_process(124, True, parsed), ("FAIL", "timeout"))

    def test_a_failure_asks_for_a_canary_and_three_dead_canaries_stop_the_run(self):
        self.assertEqual(L.canary_policy("FAIL", "timeout", 0), (True, False))
        self.assertEqual(L.canary_policy("PASS", "-", 0), (False, False))
        self.assertEqual(L.canary_policy("NOT_SUPPORTED", "attr_0", 0), (False, False))
        self.assertEqual(L.canary_policy("FAIL", "timeout", 2), (True, True))


class CJudge(unittest.TestCase):
    """The C judge table, compiled with -DGAP_SELFTEST (no libcuda): the rule every case relies on."""

    def table(self):
        exe = build_c(self, ("-DGAP_SELFTEST",))
        out = subprocess.run([exe], capture_output=True, text=True).stdout
        return {tuple(map(int, re.findall(r"=(\d+)", l.split("->")[0]))): l.split("-> ")[1] for l in out.splitlines()}

    def test_worked_and_the_attribute_says_yes_is_pass_and_worked_against_it_is_a_mismatch(self):
        t = self.table()
        self.assertEqual(t[(1, 0, 1)], "PASS")
        self.assertEqual(t[(0, 0, 1)], "MISMATCH")

    def test_refused_and_the_attribute_says_no_is_not_supported_and_refused_against_it_is_fail(self):
        t = self.table()
        self.assertEqual(t[(0, 801, 0)], "NOT_SUPPORTED")
        self.assertEqual(t[(1, 801, 0)], "FAIL")

    def test_wrong_data_is_a_fail_whatever_the_attribute_says(self):
        t = self.table()
        self.assertEqual(t[(1, 0, 0)], "FAIL")
        self.assertEqual(t[(0, 0, 0)], "FAIL")

    def test_without_libcuda_every_case_is_not_run_with_its_reason_never_a_pass(self):
        if any(Path(p).exists() for p in ("/usr/lib/x86_64-linux-gnu/libcuda.so.1", "/usr/lib64/libcuda.so.1", "/lib/x86_64-linux-gnu/libcuda.so.1")):
            self.skipTest("libcuda present")
        exe = build_c(self)
        for case in ("canary", "vmm_basic", "ipc_legacy"):
            out = subprocess.run([exe, case], capture_output=True, text=True).stdout
            p = L.parse_probe_output(out, case)
            self.assertEqual((p["status"], p["reason"]), ("NOT_RUN", "no_libcuda"))


class Setters(unittest.TestCase):
    def test_classes_of_the_answers_a_geforce_gives(self):
        c = L.classify_setter
        self.assertEqual(c(0, "Compute mode is set to DEFAULT for GPU 0"), "OK")
        self.assertEqual(c(1, "Setting ECC mode is not supported for GPU"), "NOT_SUPPORTED")
        self.assertEqual(c(1, "Insufficient Permissions"), "NO_PERMISSION")
        self.assertEqual(c(1, "Unable to set power limit: Not Supported"), "NOT_SUPPORTED")
        self.assertEqual(c(4, "NVML: Unknown Error"), "ERROR")
        self.assertEqual(c(-11, ""), "CRASH")
        self.assertEqual(c(124, "", True), "TIMEOUT")

    def test_a_timeout_or_a_crash_or_an_unknown_error_fails_the_probe(self):
        for bad in ("TIMEOUT", "CRASH", "ERROR"):
            st, why = L.setters_verdict({"pm_on": "OK", "ecc_off": bad}, True)
            self.assertEqual(st, "FAIL")
            self.assertIn("ecc_off", why)

    def test_a_dead_device_after_the_setters_fails_the_probe(self):
        self.assertEqual(L.setters_verdict({"a": "OK"}, False), ("FAIL", "device_unusable_after_setters"))
        self.assertEqual(L.setters_verdict({"a": "OK", "b": "NOT_SUPPORTED", "c": "NO_PERMISSION"}, True), ("PASS", "-"))

    def test_a_host_state_that_changes_while_the_guest_runs_setters_is_a_leak(self):
        same = [("Default", "Disabled", "350.00", "-", "210")] * 5
        self.assertEqual(L.host_state_changed(same)[0], False)
        self.assertEqual(L.host_state_changed(same + [("Exclusive_Process", "Disabled", "350.00", "-", "210")])[0], True)

    def test_the_host_state_watcher_reads_control_state_only_not_a_live_clock(self):
        import gap_scripts
        self.assertNotIn("current", gap_scripts.STATE_FIELDS)
        self.assertEqual(gap_scripts.STATE_FIELDS.split(","), ["compute_mode", "persistence_mode", "power.limit", "clocks.applications.graphics"])
        self.assertIn("clocks.current.graphics", gap_scripts.SMI_FIELDS)   # the setters probe still records it, as information

    def test_the_hoststate_verdict_command_reads_the_csv(self):
        with tempfile.TemporaryDirectory() as d:
            f = Path(d) / "h.csv"
            f.write_text("1.0;Default;Disabled;350\n1.5;Default;Disabled;350\n")
            p = subprocess.run([sys.executable, str(GP / "gap_probes.py"), "hoststate", "verdict", "--csv", str(f)], capture_output=True, text=True)
            self.assertEqual((p.returncode, p.stdout.strip()), (0, "HOST_STATE_LEAK=0 samples=2 distinct=1"))
            f.write_text("1.0;Default;Disabled;350\n1.5;Exclusive_Process;Disabled;350\n")
            p = subprocess.run([sys.executable, str(GP / "gap_probes.py"), "hoststate", "verdict", "--csv", str(f)], capture_output=True, text=True)
            self.assertEqual((p.returncode, p.stdout.strip()), (1, "HOST_STATE_LEAK=1 samples=2 distinct=2"))


class VulkanTokens(unittest.TestCase):
    def test_an_extension_is_the_same_token_in_both_json_layouts(self):
        a = L.vulkan_tokens(["ArrayOfVkExtensionProperties[3].extensionName=VK_KHR_swapchain", "ArrayOfVkExtensionProperties[3].specVersion=70"])
        b = L.vulkan_tokens(["capabilities.device.extensions.VK_KHR_swapchain=70"])
        self.assertIn("ext:VK_KHR_swapchain", a)
        self.assertEqual([t for t in a if t.startswith("ext:")], b)

    def test_limits_and_features_become_name_equals_value_with_their_indexes(self):
        t = L.vulkan_tokens(["capabilities.device.properties.VkPhysicalDeviceProperties.limits.maxComputeWorkGroupCount[1]=65535",
                             "VkPhysicalDeviceFeatures.sparseBinding=True", "$schema=https://x"])
        self.assertEqual(t, ["maxComputeWorkGroupCount[1]=65535", "sparseBinding=True"])

    def test_a_lost_feature_shows_in_the_token_diff_whatever_the_layout(self):
        host = L.vulkan_tokens(["a.b.sparseBinding=True", "ArrayOfVkExtensionProperties[0].extensionName=VK_EXT_x"])
        guest = L.vulkan_tokens(["capabilities.features.sparseBinding=False", "capabilities.device.extensions.VK_EXT_x=1"])
        d = L.vulkan_diff(host, guest)
        self.assertEqual((d["only_host"], d["only_guest"]), (["sparseBinding=True"], ["sparseBinding=False"]))

    def test_a_value_with_a_space_survives_the_leaves_file(self):
        t = L.vulkan_tokens(["x.deviceName=NVIDIA GeForce RTX 3090"])
        self.assertEqual(t, ["deviceName=NVIDIA GeForce RTX 3090"])
        d = Path(tempfile.mkdtemp(prefix="gaptok"))
        self.addCleanup(shutil.rmtree, d, True)
        (d / "h").mkdir()
        (d / "g").mkdir()
        res = "id\tgroup\tstatus\trc\tsecs\tt0\tt1\treason\tcanary\nvk_dump\tg\tPASS\t0\t1\t1\t2\t-\t-\n"
        for side, name in (("h", "NVIDIA GeForce RTX 3090"), ("g", "NVIDIA GeForce RTX 4090")):
            (d / side / "results.tsv").write_text(res)
            (d / side / "vk_leaves.tsv").write_text("deviceName=%s\n" % name)
        p = subprocess.run([sys.executable, str(GP / "gap_probes.py"), "compare", str(d / "h"), str(d / "g")], capture_output=True, text=True)
        self.assertIn("VKDIFF only_host deviceName=NVIDIA GeForce RTX 3090", p.stdout)
        self.assertIn("VKDIFF_SUMMARY only_host=1 only_guest=1", p.stdout)


class Vulkan(unittest.TestCase):
    DOC = {"capabilities": {"device": {"properties": {"VkPhysicalDeviceProperties": {"deviceName": "X", "deviceUUID": [1, 2], "limits": {"maxImageDimension2D": 32768}},
                                                      "VkPhysicalDeviceIDProperties": {"deviceLUID": [1]}},
                                       "features": {"VkPhysicalDeviceFeatures": {"sparseBinding": True}}}},
           "extensions": [{"extensionName": "VK_KHR_swapchain"}]}

    def test_uuids_and_luids_are_dropped_and_the_rest_is_kept_sorted(self):
        leaves = L.flatten_vulkan(self.DOC)
        self.assertEqual(leaves, sorted(leaves))
        self.assertFalse([l for l in leaves if "UUID" in l or "LUID" in l])
        self.assertIn("capabilities.device.features.VkPhysicalDeviceFeatures.sparseBinding=True", leaves)
        self.assertIn("extensions[0].extensionName=VK_KHR_swapchain", leaves)

    def test_groups_name_a_difference_by_its_first_two_path_components(self):
        g = L.vulkan_groups(L.flatten_vulkan(self.DOC))
        self.assertIn("capabilities.device", g)
        self.assertIn("extensions.0", g)

    def test_a_feature_the_guest_loses_is_found_in_only_host(self):
        h = L.flatten_vulkan(self.DOC)
        doc = json.loads(json.dumps(self.DOC))
        doc["capabilities"]["device"]["features"]["VkPhysicalDeviceFeatures"]["sparseBinding"] = False
        d = L.vulkan_diff(h, L.flatten_vulkan(doc))
        self.assertEqual(d["n_only_host"], 1)
        self.assertTrue(d["only_host"][0].endswith("sparseBinding=True"))
        self.assertTrue(d["only_guest"][0].endswith("sparseBinding=False"))


def R(case, status, area="a", reason="-", kv=None, t0=None, t1=None):
    return D.row(case, status, area, reason, "1", t0, t1, kv)


class Categories(unittest.TestCase):
    def test_the_comparison_table(self):
        c = D.categorize
        self.assertEqual(c("pass", "pass"), "SAME_OK")
        self.assertEqual(c("not_supported", "not_supported"), "SAME_NOT_SUPPORTED")
        self.assertEqual(c("pass", "fail"), "GUEST_ONLY_FAIL")
        self.assertEqual(c("pass", "timeout"), "GUEST_ONLY_FAIL")
        self.assertEqual(c("pass", "not_supported"), "GUEST_LOSES_SUPPORT")
        self.assertEqual(c("not_supported", "pass"), "GUEST_GAINS_SUPPORT")
        self.assertEqual(c("not_supported", "crash"), "GUEST_FAIL_HOST_NS")
        self.assertEqual(c("fail", "fail"), "BOTH_FAIL")
        self.assertEqual(c("fail", "timeout"), "BOTH_FAIL_DIFFERENT")
        self.assertEqual(c("fail", "pass"), "HOST_ONLY_FAIL")
        self.assertEqual(c("pass", "not_run"), "INCOMPLETE")

    def test_an_unknown_raw_status_is_a_failure_never_a_pass(self):
        self.assertEqual(D.normalise("Excellent"), "fail")
        rows = D.parse_gap_results("id\tgroup\tstatus\trc\tsecs\tt0\tt1\treason\tcanary\nx\tg\tSUPERB\t0\t1\t1\t2\t-\t-\n")
        self.assertEqual(rows["x"]["status"], "fail")

    def test_statuses_are_normalised_and_timeouts_and_crashes_stay_apart_from_fail(self):
        self.assertEqual(D.normalise("Pass"), "pass")
        self.assertEqual(D.normalise("QualityWarning"), "pass")
        self.assertEqual(D.normalise("NotSupported"), "not_supported")
        self.assertEqual(D.normalise("FAIL", "timeout"), "timeout")
        self.assertEqual(D.normalise("FAIL", "crash_signal_11"), "crash")
        self.assertEqual(D.normalise("FAIL", "wrong_value"), "fail")
        self.assertEqual(D.normalise("MISMATCH"), "fail")
        self.assertEqual(D.normalise("never_heard_of_it"), "fail")   # an unknown status is never a pass


class Compare(unittest.TestCase):
    def test_only_differences_are_reported(self):
        diffs = D.compare({"a": R("a", "pass"), "b": R("b", "pass")}, {"a": R("a", "pass"), "b": R("b", "fail")})
        self.assertEqual([d["case"] for d in diffs], ["b"])
        self.assertEqual(diffs[0]["category"], "GUEST_ONLY_FAIL")

    def test_an_unclassified_difference_is_a_kayfabe_gap_candidate(self):
        d = D.compare({"b": R("b", "pass")}, {"b": R("b", "timeout", reason="timeout")})[0]
        self.assertEqual(d["bucket"], "kayfabe_gap_candidate")

    def test_a_case_that_fails_on_the_host_control_too_is_not_kayfabes(self):
        d = D.compare({"b": R("b", "fail")}, {"b": R("b", "fail")})[0]
        self.assertEqual((d["category"], d["bucket"]), ("BOTH_FAIL", "app_or_harness_bug"))

    def test_a_value_difference_between_two_passes_is_reported(self):
        d = D.compare({"a": R("a", "pass", kv={"gran": "2097152"})}, {"a": R("a", "pass", kv={"gran": "65536"})})[0]
        self.assertEqual(d["category"], "VALUE_DIFF")
        self.assertIn("gran:2097152->65536", d["kvdiff"])

    def test_a_listed_limit_explains_exactly_the_difference_it_names(self):
        limits = D.parse_limits("attr_dump\tmem_total_mib\tpass\tpass\tkayfabe_limitation\tthe guest reports its own frame buffer\n")
        h = {"attr_dump": R("attr_dump", "pass", kv={"mem_total_mib": "24000", "attr.75": "8"})}
        g = {"attr_dump": R("attr_dump", "pass", kv={"mem_total_mib": "4096", "attr.75": "8"})}
        d = D.compare(h, g, limits)[0]
        self.assertEqual((d["bucket"], d["why"]), ("kayfabe_limitation", "the guest reports its own frame buffer"))
        g["attr_dump"]["kv"]["attr.75"] = "7"   # an unrelated key now differs too: the limit must not hide it
        d = D.compare(h, g, limits)[0]
        self.assertEqual(d["bucket"], "kayfabe_gap_candidate")

    def test_a_limit_for_one_key_does_not_hide_a_difference_in_another(self):
        limits = D.parse_limits("attr_dump\tmem_total_mib\t*\t*\tkayfabe_limitation\tthe guest reports its own frame buffer size\n")
        d = D.compare({"attr_dump": R("attr_dump", "pass", kv={"attr.1": "1"})}, {"attr_dump": R("attr_dump", "pass", kv={"attr.1": "2"})}, limits)[0]
        self.assertEqual(d["bucket"], "kayfabe_gap_candidate")

    def test_a_limit_with_a_dash_or_a_blank_reason_is_refused(self):
        for reason in ("-", "   "):
            with self.assertRaises(ValueError):
                D.parse_limits("a\t*\t*\t*\tkayfabe_limitation\t%s\n" % reason)

    def test_the_cli_refuses_a_limits_file_with_an_unknown_bucket(self):
        d = Path(tempfile.mkdtemp(prefix="gaplim"))
        self.addCleanup(shutil.rmtree, d, True)
        res = "id\tgroup\tstatus\trc\tsecs\tt0\tt1\treason\tcanary\nx\tg\tPASS\t0\t1\t1\t2\t-\t-\n"
        (d / "h.tsv").write_text(res)
        (d / "g.tsv").write_text(res)
        (d / "lim.tsv").write_text("x\t*\t*\t*\twhatever\tbecause it is documented somewhere\n")
        p = subprocess.run([sys.executable, str(GP / "diffrun.py"), "diff", str(d / "h.tsv"), str(d / "g.tsv"), "--limits", str(d / "lim.tsv")], capture_output=True, text=True)
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("bucket", p.stderr)

    def test_perf_keys_are_measurements_and_never_a_difference(self):
        h = {"a": R("a", "pass", kv={"perf.secs": "5", "gran": "1"})}
        g = {"a": R("a", "pass", kv={"perf.secs": "9", "gran": "1"})}
        self.assertEqual(D.compare(h, g), [])
        g["a"]["kv"]["gran"] = "2"
        self.assertEqual(D.compare(h, g)[0]["kvdiff"], "gran:1->2")

    def test_info_keys_are_text_for_the_evidence_and_never_a_difference(self):
        h = {"a": R("a", "pass", kv={"info.msg": "one version"})}
        g = {"a": R("a", "pass", kv={"info.msg": "another version"})}
        self.assertEqual(D.compare(h, g), [])

    def test_the_cli_ignores_perf_keys_in_the_kv_files_of_two_runs(self):
        d = Path(tempfile.mkdtemp(prefix="gapperf"))
        self.addCleanup(shutil.rmtree, d, True)
        res = "id\tgroup\tstatus\trc\tsecs\tt0\tt1\treason\tcanary\nx\tg\tPASS\t0\t1\t1\t2\t-\t-\n"
        for side, val in (("h", "100"), ("g", "7")):
            (d / side).mkdir()
            (d / side / "results.tsv").write_text(res)
            (d / side / "kv.tsv").write_text("x\tperf.launches_per_s\t%s\nx\tattr.1\t1\n" % val)
        p = subprocess.run([sys.executable, str(GP / "gap_probes.py"), "compare", str(d / "h"), str(d / "g")], capture_output=True, text=True)
        self.assertEqual(p.returncode, 0, p.stdout)
        self.assertIn("differing=0", p.stdout)

    def test_the_c_probe_names_every_timing_value_perf(self):
        src = (GP / "gap_probe.c").read_text()
        for m in re.finditer(r'kv\("([a-z_.]*(?:secs|per_s)[a-z_.]*)"', src):
            self.assertTrue(m.group(1).startswith("perf."), m.group(1))

    def test_a_limit_needs_a_known_bucket_and_a_reason(self):
        with self.assertRaises(ValueError):
            D.parse_limits("a\t*\t*\t*\tfine\tbecause\n")
        with self.assertRaises(ValueError):
            D.parse_limits("a\t*\t*\t*\tkayfabe_limitation\t-\n")
        with self.assertRaises(ValueError):
            D.parse_limits("a\t*\t*\tkayfabe_limitation\tbecause\n")

    def test_the_shipped_limits_parse_and_every_one_has_a_reason(self):
        limits = D.parse_limits((GP / "gap_known_limits.tsv").read_text())
        self.assertGreaterEqual(len(limits), 1)
        for l in limits:
            self.assertGreater(len(l["reason"]), 20)

    def test_a_host_xid_inside_a_case_window_is_attached_and_one_outside_is_not(self):
        xids = D.parse_xids("1731900010.5 host kernel: NVRM: Xid (PCI:0000:01:00): 109, pid=1, name=x\n1731901000.0 host kernel: NVRM: Xid (PCI:0000:01:00): 13, pid=2\nnoise line\n")
        self.assertEqual([x for _, x, _ in xids], [109, 13])
        g = {"a": R("a", "timeout", reason="timeout", t0=1731900000.0, t1=1731900013.0), "b": R("b", "fail", t0=1731900500.0, t1=1731900510.0)}
        h = {"a": R("a", "pass"), "b": R("b", "pass")}
        d = {x["case"]: x for x in D.compare(h, g, (), xids)}
        self.assertEqual(d["a"]["xid"], "109")
        self.assertEqual(d["b"]["xid"], "-")

    def test_an_xid_before_the_window_is_not_attached_and_one_just_after_it_is(self):
        xids = D.parse_xids("1731899000.0 k: NVRM: Xid (PCI:0000:01:00): 31, pid=1\n1731900016.0 k: NVRM: Xid (PCI:0000:01:00): 109, pid=2\n")
        g = {"a": R("a", "timeout", reason="timeout", t0=1731900000.0, t1=1731900013.0)}
        d = D.compare({"a": R("a", "pass")}, g, (), xids)[0]
        self.assertEqual(d["xid"], "109")   # 16 s is inside t1 + 5 s slack only for the second; the first is 1000 s earlier

    def test_the_summary_counts_candidates_and_buckets(self):
        h = {"a": R("a", "pass"), "b": R("b", "fail"), "c": R("c", "pass")}
        g = {"a": R("a", "timeout"), "b": R("b", "fail"), "c": R("c", "pass")}
        diffs = D.compare(h, g)
        line = D.summary_line(h, g, diffs)
        self.assertIn("cases=3 differing=2 candidates=1", line)
        self.assertIn("harness=1", line)


class Adapters(unittest.TestCase):
    pass


class AdaptersMore(unittest.TestCase):
    def test_vkcts_rows_and_nvcodec_rows_read_into_the_same_shape(self):
        vk = "VKCTS_RESULTS_BEGIN v1 side=host\n#case\tstatus\tclass\tarea\tdetail\ndEQP-VK.a\tPass\tpass\tinfo\tPass\ndEQP-VK.b\tNotSupported\tnot_supported\tsparse\tno\ndEQP-VK.c\tTimeout\ttimeout\tsparse\tTimeout\n"
        rows = D.parse_vkcts_results(vk)
        self.assertEqual({k: v["status"] for k, v in rows.items()}, {"dEQP-VK.a": "pass", "dEQP-VK.b": "not_supported", "dEQP-VK.c": "timeout"})
        nv = "case_id\tgroup\texpect\tresult\treason\trc\tsecs\tpsnr\tframes\tn_ok\tsess_peak\tdetail\nenc_h264\tencode\tPASS\tPASS\tok\t0\t3.1\t30\t30\t-\t-\t-\n"
        self.assertEqual(D.parse_nvcodec_results(nv)["enc_h264"]["status"], "pass")

    def test_the_gap_results_and_kv_files_round_trip(self):
        res = "id\tgroup\tstatus\trc\tsecs\tt0\tt1\treason\tcanary\nx\tg\tFAIL\t0\t3.0\t10\t13\ttimeout\tFAIL\n"
        rows = D.parse_gap_results(res)
        D.parse_gap_kv("x\tk\tv\ny\tz\tw\n", rows)
        self.assertEqual((rows["x"]["status"], rows["x"]["kv"]["k"], rows["x"]["t1"]), ("timeout", "v", 13.0))


FAKE = """#!/usr/bin/env python3
import sys, time, os, signal
case = sys.argv[1]
if case == "list":
    sys.exit(0)
if case == "canary":
    if os.path.exists(os.environ.get("FAKE_DEAD", "/nonexistent")):
        print("GAPPROBE case=canary status=FAIL rc=700 secs=0.1 reason=cuCtxSynchronize")
    else:
        print("GAPPROBE case=canary status=PASS rc=0 secs=0.1 reason=-")
elif case == "good":
    print("GAPKV good attr=41:1"); print("GAPPROBE case=good status=PASS rc=0 secs=0.2 reason=-")
elif case == "refused":
    print("GAPPROBE case=refused status=NOT_SUPPORTED rc=801 secs=0.2 reason=attr_0")
elif case == "bad":
    print("GAPPROBE case=bad status=FAIL rc=700 secs=0.2 reason=cuMemMap")
elif case == "crash":
    os.kill(os.getpid(), signal.SIGSEGV)
elif case == "hang":
    time.sleep(600)
else:
    sys.exit(3)
"""


class RunnerEndToEnd(unittest.TestCase):
    def run_table(self, rows, env=None, side="host", extra=()):
        d = tempfile.mkdtemp(prefix="gaprun")
        self.addCleanup(shutil.rmtree, d, True)
        d = Path(d)
        fake = d / "fake_probe"
        fake.write_text(FAKE)
        fake.chmod(fake.stat().st_mode | stat.S_IEXEC)
        table = d / "cases.tsv"
        table.write_text(HDR + "".join(rows))
        saved = GPR.CASES
        GPR.CASES = table
        self.addCleanup(setattr, GPR, "CASES", saved)
        old = dict(os.environ)
        os.environ.update(env or {})
        self.addCleanup(lambda: (os.environ.clear(), os.environ.update(old)))
        out = d / "out"
        buf = io.StringIO()
        real, sys.stdout = sys.stdout, buf
        try:
            rc = GPR.main(["run", "--side", side, "--out", str(out), "--probe", str(fake), *extra])
        finally:
            sys.stdout = real
        rows = {l.split("\t")[0]: l.split("\t") for l in (out / "results.tsv").read_text().splitlines()[1:]}
        return rc, rows, out, buf.getvalue()

    def test_pass_refused_fail_and_crash_are_each_told_apart(self):
        rc, rows, out, text = self.run_table([case_line("canary", prog="canary"), case_line("good", prog="good"), case_line("refused", prog="refused"),
                                              case_line("bad", prog="bad"), case_line("crash", prog="crash")])
        self.assertEqual({k: v[2] for k, v in rows.items()}, {"canary": "PASS", "good": "PASS", "refused": "NOT_SUPPORTED", "bad": "FAIL", "crash": "FAIL"})
        self.assertEqual(rows["crash"][7], "crash_signal_11")
        self.assertEqual(rows["bad"][7], "cuMemMap")
        self.assertEqual(rc, 1)
        self.assertIn("GAP_PROBES_SUMMARY side=host pass=2 not_supported=1 fail=2", text)
        self.assertIn("good\tattr.41\t1", (out / "kv.tsv").read_text())

    def test_a_failure_runs_a_canary_and_records_it(self):
        rc, rows, out, _ = self.run_table([case_line("bad", prog="bad"), case_line("good", prog="good")])
        self.assertEqual(rows["bad"][8], "PASS")
        self.assertEqual(rows["good"][8], "-")

    def test_a_dead_device_stops_the_run_and_the_rest_are_not_run_with_the_reason(self):
        dead = tempfile.NamedTemporaryFile(delete=False)
        self.addCleanup(os.unlink, dead.name)
        rows = [case_line("bad%d" % i, prog="bad") for i in range(4)] + [case_line("good", prog="good")]
        rc, res, out, _ = self.run_table(rows, env={"FAKE_DEAD": dead.name})
        self.assertEqual(res["bad2"][8], "FAIL")
        self.assertEqual((res["bad3"][2], res["bad3"][7]), ("NOT_RUN", "device_dead_after_bad2"))
        self.assertEqual((res["good"][2], res["good"][7]), ("NOT_RUN", "device_dead_after_bad2"))

    def test_a_hang_is_a_timeout_failure_not_a_pass_and_the_run_goes_on(self):
        rc, rows, out, _ = self.run_table([case_line("hang", prog="hang", timeout="10"), case_line("good", prog="good")])
        self.assertEqual((rows["hang"][2], rows["hang"][7]), ("FAIL", "timeout"))
        self.assertEqual(rows["good"][2], "PASS")
        self.assertEqual(rc, 1)

    def test_a_guest_only_case_is_not_run_on_the_host_and_says_why(self):
        rc, rows, out, _ = self.run_table([case_line("good", prog="good"), case_line("g", prog="good", tags="guest_only")])
        self.assertEqual((rows["g"][2], rows["g"][7]), ("NOT_RUN", "guest_only"))
        self.assertEqual(rc, 0)

    def test_without_a_probe_binary_and_without_gcc_everything_is_not_run_never_a_pass(self):
        d = Path(tempfile.mkdtemp(prefix="gapnogcc"))
        self.addCleanup(shutil.rmtree, d, True)
        saved = GPR.build_probe
        GPR.build_probe = lambda out: None
        self.addCleanup(setattr, GPR, "build_probe", saved)
        table = d / "c.tsv"
        table.write_text(HDR + case_line("good", prog="good"))
        savedc = GPR.CASES
        GPR.CASES = table
        self.addCleanup(setattr, GPR, "CASES", savedc)
        real, sys.stdout = sys.stdout, io.StringIO()
        try:
            rc = GPR.main(["run", "--side", "host", "--out", str(d / "o")])
        finally:
            sys.stdout = real
        row = (d / "o/results.tsv").read_text().splitlines()[1].split("\t")
        self.assertEqual((row[2], row[7], rc), ("NOT_RUN", "no_gap_probe_binary_and_no_gcc", 0))

    def test_the_vulkan_leaves_travel_in_the_emit_blocks_when_a_case_wrote_them(self):
        d = Path(tempfile.mkdtemp(prefix="gapleaf"))
        self.addCleanup(shutil.rmtree, d, True)
        (d / "out").mkdir()
        (d / "out/vk_leaves.tsv").write_text("a=1\nb=2\n")
        fake = d / "fake_probe"
        fake.write_text(FAKE)
        fake.chmod(0o755)
        table = d / "cases.tsv"
        table.write_text(HDR + case_line("good", prog="good"))
        saved = GPR.CASES
        GPR.CASES = table
        self.addCleanup(setattr, GPR, "CASES", saved)
        buf = io.StringIO()
        real, sys.stdout = sys.stdout, buf
        try:
            GPR.main(["run", "--side", "host", "--out", str(d / "out"), "--probe", str(fake), "--emit"])
        finally:
            sys.stdout = real
        self.assertEqual(rowio.extract(buf.getvalue()).get("vk_leaves.tsv"), "a=1\nb=2\n")

    def test_emit_blocks_carry_the_files_and_compare_reads_a_log(self):
        rc, rows, out, text = self.run_table([case_line("good", prog="good"), case_line("bad", prog="bad")], extra=("--emit",))
        blocks = rowio.extract(text)
        self.assertEqual(set(blocks), {"results.tsv", "kv.tsv", "summary.txt"})
        rc, rows2, out2, text2 = self.run_table([case_line("good", prog="good"), case_line("bad", prog="refused")], extra=("--emit",))
        d = Path(tempfile.mkdtemp(prefix="gapcmp"))
        self.addCleanup(shutil.rmtree, d, True)
        (d / "host.log").write_text(text)
        (d / "guest.log").write_text(text2)
        p = subprocess.run([sys.executable, str(GP / "gap_probes.py"), "compare", str(d / "host.log"), str(d / "guest.log")], capture_output=True, text=True)
        self.assertEqual(p.returncode, 0, p.stdout)   # host FAIL vs guest NOT_SUPPORTED is HOST_FAIL_GUEST_NOT_SUPPORTED: not a guest candidate
        self.assertIn("bad", p.stdout)


FAKE_SMI = """#!/usr/bin/env python3
import sys
a = sys.argv[1:]
if any(x.startswith("--query-gpu") for x in a):
    print("Default, Disabled, 350.00, 1500, 210"); sys.exit(0)
if "-e" in a:
    print("Setting ECC mode is not supported for GPU 0"); sys.exit(1)
if "-c" in a and "PROHIBITED" in a:
    print("Insufficient Permissions"); sys.exit(1)
print("ok"); sys.exit(0)
"""
FAKE_VK = """#!/usr/bin/env python3
import sys, json
out = sys.argv[sys.argv.index("-o") + 1] if "-o" in sys.argv else "vulkaninfo.json"
doc = {"capabilities": {"device": {"properties": {"VkPhysicalDeviceProperties": {"deviceName": "X", "deviceUUID": [1, 2, 3]}, "VkPhysicalDeviceIDProperties": {"deviceLUID": [9]}},
       "features": {"VkPhysicalDeviceFeatures": {"f%d" % i: True for i in range(150)}}}}}
open(out, "w").write(json.dumps(doc))
"""


class ScriptCases(unittest.TestCase):
    """The py cases run against fake nvidia-smi / vulkaninfo / probe programs on PATH: the decisions of gap_lib are tested above, this checks they are wired."""

    def setUp(self):
        import gap_scripts
        self.S = gap_scripts
        self.d = Path(tempfile.mkdtemp(prefix="gapscr"))
        self.addCleanup(shutil.rmtree, self.d, True)
        for name, body in (("nvidia-smi", FAKE_SMI), ("vulkaninfo", FAKE_VK)):
            f = self.d / name
            f.write_text(body)
            f.chmod(0o755)
        old = os.environ["PATH"]
        os.environ["PATH"] = str(self.d) + os.pathsep + old
        self.addCleanup(os.environ.__setitem__, "PATH", old)

    def probe(self, canary_ok):
        f = self.d / "probe"
        f.write_text("#!/bin/sh\necho 'GAPPROBE case=canary status=%s rc=0 secs=0.1 reason=-'\n" % ("PASS" if canary_ok else "FAIL"))
        f.chmod(0o755)
        return str(f)

    def test_smi_setters_records_one_class_per_setter_and_passes_on_native_answers(self):
        r = self.S.smi_setters({"probe": self.probe(True), "out": self.d})
        self.assertEqual(r["status"], "PASS", r)
        self.assertEqual(r["kv"]["setter.ecc_off"], "NOT_SUPPORTED")
        self.assertEqual(r["kv"]["setter.compute_prohibited"], "NO_PERMISSION")
        self.assertEqual(r["kv"]["setter.pm_on"], "OK")

    def test_smi_setters_fails_when_the_device_is_dead_afterwards(self):
        r = self.S.smi_setters({"probe": self.probe(False), "out": self.d})
        self.assertEqual((r["status"], r["reason"]), ("FAIL", "device_unusable_after_setters"))

    def test_persistenced_is_found_by_name_after_it_daemonises_and_stopped_with_sigterm(self):
        """A fake nvidia-persistenced that forks a sleeper named like the daemon and returns 0 at once, as the real one does."""
        d = self.d
        daemon = d / "nvidia-persistenced"
        sleeper = d / "sleeper"
        sleeper.mkdir()
        shutil.copy(shutil.which("bash"), sleeper / "nvidia-persistenced")   # a process whose kernel name is the daemon's
        daemon.write_text("#!/bin/sh\n%s/nvidia-persistenced -c 'sleep 300; true' >/dev/null 2>&1 &\nexit 0\n" % sleeper)
        daemon.chmod(0o755)
        r = self.S.life_persistenced({"probe": self.probe(True), "out": d})
        self.assertEqual(r["status"], "PASS", r)
        self.assertEqual(r["kv"]["stopped_on_sigterm"], "1")
        self.assertEqual(r["kv"]["daemon_found"], "1")

    def test_a_daemon_the_machine_already_runs_is_stopped_first_and_started_again_afterwards(self):
        d = self.d
        sleeper = d / "sleeper"
        sleeper.mkdir()
        shutil.copy(shutil.which("bash"), sleeper / "nvidia-persistenced")
        daemon = d / "nvidia-persistenced"
        daemon.write_text("#!/bin/sh\n%s/nvidia-persistenced -c 'sleep 300; true' >/dev/null 2>&1 &\nexit 0\n" % sleeper)
        daemon.chmod(0o755)
        pre = subprocess.Popen([str(sleeper / "nvidia-persistenced"), "-c", "sleep 300; true"], start_new_session=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

        def reap():
            for p in self.S.pids_of("nvidia-persistenced"):
                try:
                    os.killpg(os.getpgid(p), 9)
                except OSError:
                    pass
        self.addCleanup(reap)
        time.sleep(0.5)
        self.assertEqual(self.S.pids_of("nvidia-persistenced"), [pre.pid])
        r = self.S.life_persistenced({"probe": self.probe(True), "out": d})
        pre.wait(timeout=5)
        self.assertEqual(r["status"], "PASS", r)
        self.assertEqual(r["kv"]["info.preexisting"], "1")
        time.sleep(1)
        self.assertEqual(len(self.S.pids_of("nvidia-persistenced")), 1)   # the machine's own daemon is back, started from its own command line

    def test_the_tool_cases_keep_the_tools_last_words_as_an_info_key(self):
        d = self.d
        fake = d / "ncu"
        fake.write_text("#!/bin/sh\necho '==ERROR== Profiling is not supported on this device'\nexit 1\n")
        fake.chmod(0o755)
        saved = self.S.find_tool
        self.S.find_tool = lambda name: str(fake)
        self.addCleanup(setattr, self.S, "find_tool", saved)
        r = self.S.tools_ncu({"probe": "/bin/true", "out": d})
        self.assertEqual(r["status"], "NOT_SUPPORTED")
        self.assertIn("not supported on this device", r["kv"]["info.metric_msg"])

    def test_a_persistenced_that_never_starts_a_daemon_fails_the_case(self):
        d = self.d
        daemon = d / "nvidia-persistenced"
        daemon.write_text("#!/bin/sh\nexit 0\n")
        daemon.chmod(0o755)
        r = self.S.life_persistenced({"probe": self.probe(True), "out": d})
        self.assertEqual((r["status"], r["reason"]), ("FAIL", "persistenced_did_not_start"))

    def test_an_insufficient_permissions_answer_is_not_an_error(self):
        st, why = L.setters_verdict({"x": L.classify_setter(1, "Insufficient Permissions")}, True)
        self.assertEqual((st, why), ("PASS", "-"))

    def test_vk_dump_writes_leaves_without_uuids_and_digests_per_group(self):
        r = self.S.vk_dump({"probe": "-", "out": self.d})
        self.assertEqual(r["status"], "PASS", r)
        leaves = (self.d / "vk_leaves.tsv").read_text()
        self.assertNotIn("UUID", leaves)
        self.assertNotIn("LUID", leaves)
        self.assertIn("f7=True", leaves.splitlines())
        self.assertRegex(r["kv"]["tokens"], r"^\d+:[0-9a-f]{8}$")


class RunnerKv(unittest.TestCase):
    def test_a_case_keeps_only_its_own_kv_lines(self):
        t = GPR.L.parse_probe_output("GAPKV other k=v\nGAPKV mine a=1\nGAPPROBE case=mine status=PASS rc=0 secs=1 reason=-\n", "mine")
        self.assertEqual(t["kv"], {"a": "1"})

    def test_the_cli_run_of_a_destructive_case_without_the_flag_says_why(self):
        d = Path(tempfile.mkdtemp(prefix="gapdes"))
        self.addCleanup(shutil.rmtree, d, True)
        p = subprocess.run([sys.executable, str(GP / "gap_probes.py"), "run", "--side", "guest", "--out", str(d), "--only", "life_reset,life_busy_poweroff"], capture_output=True, text=True)
        rows = {l.split("\t")[0]: l.split("\t") for l in (d / "results.tsv").read_text().splitlines()[1:]}
        self.assertEqual(rows["life_reset"][7], "destructive_not_requested")
        self.assertEqual(rows["life_busy_poweroff"][7], "terminal_not_requested")


class CJudgeTable(unittest.TestCase):
    def test_the_whole_judge_table_is_exactly_this(self):
        out = subprocess.run([build_c(self, ("-DGAP_SELFTEST",))], capture_output=True, text=True).stdout.splitlines()
        self.assertEqual([l.split("-> ")[1] for l in out], ["PASS", "MISMATCH", "NOT_SUPPORTED", "FAIL", "FAIL", "FAIL"])


class TableMore(unittest.TestCase):
    def test_a_falsifier_of_a_few_words_is_refused(self):
        with self.assertRaises(ValueError):
            L.load_cases(HDR + case_line(fal="it breaks"))

    def test_an_id_repeated_across_kinds_is_refused(self):
        with self.assertRaises(ValueError):
            L.load_cases(HDR + case_line("x", kind="c") + case_line("x", kind="py"))


class WiringMore(unittest.TestCase):
    def test_run_apps_lists_the_row_but_all_does_not_expand_to_it(self):
        env = dict(os.environ, APPS_ROOT=tempfile.mkdtemp(prefix="gapapps"))
        self.addCleanup(shutil.rmtree, env["APPS_ROOT"], True)
        full = subprocess.run(["bash", str(ROOT / "scripts/apps/run_apps.sh"), "host", "list"], capture_output=True, text=True, env=env).stdout.split()
        gate = subprocess.run(["bash", str(ROOT / "scripts/apps/run_apps.sh"), "host", "listall"], capture_output=True, text=True, env=env).stdout.split()
        self.assertIn("gap_probes", full)
        self.assertNotIn("gap_probes", gate)

    def test_the_census_function_of_the_hook_answers_info_for_the_row(self):
        line = next(l for l in (ROOT / "scripts/apps/apps_hook.sh").read_text().splitlines() if l.startswith("census_need(){"))
        out = subprocess.run(["bash", "-c", line + '; census_need gap_probes; census_need nvcodec_matrix'], capture_output=True, text=True).stdout.split()
        self.assertEqual(out, ["info", "enc"])


class Tools(unittest.TestCase):
    def test_classes_of_the_answers_the_nvidia_tools_give(self):
        c = L.classify_tool
        self.assertEqual(c(0, "==PROF== Disconnected from process\nsm__cycles_elapsed.avg 1234"), "OK")
        self.assertEqual(c(1, "==ERROR== ERR_NVGPUCTRPERM - The user does not have permission to access NVIDIA GPU Performance Counters"), "NO_PERMISSION")
        self.assertEqual(c(1, "==ERROR== Profiling is not supported on this device"), "NOT_SUPPORTED")
        self.assertEqual(c(1, "Unable to start GPU metrics: unsupported GPU"), "NOT_SUPPORTED")
        self.assertEqual(c(2, "segmentation fault in the tool"), "ERROR")
        self.assertEqual(c(-6, ""), "CRASH")
        self.assertEqual(c(124, "", True), "TIMEOUT")

    def test_a_tool_that_breaks_fails_the_case_and_one_that_declines_is_not_supported(self):
        self.assertEqual(L.tools_verdict({"memcheck": "OK", "racecheck": "OK"}), ("PASS", "-"))
        self.assertEqual(L.tools_verdict({"metric": "NO_PERMISSION"}), ("NOT_SUPPORTED", "tool_answer_metric"))
        self.assertEqual(L.tools_verdict({"a": "OK", "b": "ERROR", "c": "NOT_SUPPORTED"}), ("FAIL", "b_error"))
        self.assertEqual(L.tools_verdict({"a": "TIMEOUT"}), ("FAIL", "a_timeout"))

    def test_the_ncu_limit_explains_a_lost_capability_and_nothing_else(self):
        limits = D.parse_limits((GP / "gap_known_limits.tsv").read_text())
        lost = D.compare({"tools_ncu": R("tools_ncu", "pass")}, {"tools_ncu": R("tools_ncu", "not_supported")}, limits)[0]
        self.assertEqual(lost["bucket"], "kayfabe_limitation")
        broke = D.compare({"tools_ncu": R("tools_ncu", "pass")}, {"tools_ncu": R("tools_ncu", "fail")}, limits)[0]
        self.assertEqual(broke["bucket"], "kayfabe_gap_candidate")
        other = D.compare({"tools_nsys": R("tools_nsys", "pass")}, {"tools_nsys": R("tools_nsys", "not_supported")}, limits)[0]
        self.assertEqual(other["bucket"], "kayfabe_gap_candidate")   # only the profiler row is a documented limitation

    def test_the_tool_cases_are_not_run_when_the_tool_is_absent_never_a_pass(self):
        import gap_scripts
        saved = gap_scripts.find_tool
        gap_scripts.find_tool = lambda name: None
        self.addCleanup(setattr, gap_scripts, "find_tool", saved)
        for fn in (gap_scripts.tools_sanitizer, gap_scripts.tools_ncu, gap_scripts.tools_nsys, gap_scripts.tools_nsys_gpumetrics):
            r = fn({"probe": "-", "out": Path(".")})
            self.assertEqual(r["status"], "NOT_RUN", fn.__name__)

    def test_a_silent_memcheck_fails_the_sanitizer_case_even_when_the_clean_runs_pass(self):
        import gap_scripts
        d = Path(tempfile.mkdtemp(prefix="gapsan"))
        self.addCleanup(shutil.rmtree, d, True)
        fake = d / "compute-sanitizer"
        fake.write_text("#!/bin/sh\necho '========= ERROR SUMMARY: 0 errors'\nexit 0\n")
        fake.chmod(0o755)
        saved = gap_scripts.find_tool
        gap_scripts.find_tool = lambda name: str(fake)
        self.addCleanup(setattr, gap_scripts, "find_tool", saved)
        r = gap_scripts.tools_sanitizer({"probe": "/bin/true", "out": d})
        self.assertEqual((r["status"], r["reason"]), ("FAIL", "memcheck_silent_about_an_illegal_store"))
        fake.write_text("#!/bin/sh\necho '========= Invalid __global__ write of size 4 bytes'\nexit 0\n")
        r = gap_scripts.tools_sanitizer({"probe": "/bin/true", "out": d})
        self.assertEqual(r["status"], "PASS")

    def test_tools_pushed_under_the_tools_directory_are_found_and_so_is_cupti(self):
        import gap_scripts
        d = Path(tempfile.mkdtemp(prefix="gaproot"))
        self.addCleanup(shutil.rmtree, d, True)
        (d / "usr/local/cuda-12.9/bin").mkdir(parents=True)
        (d / "usr/local/cuda-12.9/bin/compute-sanitizer").write_text("#!/bin/sh\n")
        (d / "usr/local/cuda-12.9/extras/CUPTI/lib64").mkdir(parents=True)
        (d / "usr/local/cuda-12.9/extras/CUPTI/lib64/libcupti.so.12").write_text("")
        saved = gap_scripts.TOOL_ROOTS
        gap_scripts.TOOL_ROOTS = (str(d),)
        self.addCleanup(setattr, gap_scripts, "TOOL_ROOTS", saved)
        self.assertTrue(gap_scripts.find_tool("compute-sanitizer").endswith("cuda-12.9/bin/compute-sanitizer"))
        self.assertIsNone(gap_scripts.find_tool("ncu"))
        self.assertTrue(gap_scripts.cupti_lib().endswith("libcupti.so.12"))

    def newest_fixture(self):
        import gap_scripts
        d = Path(tempfile.mkdtemp(prefix="gapver"))
        self.addCleanup(shutil.rmtree, d, True)
        for rel in ("cuda-12.6", "cuda-12.9"):
            (d / "usr/local" / rel / "bin").mkdir(parents=True)
            (d / "usr/local" / rel / "bin/compute-sanitizer").write_text("")
            (d / "usr/local" / rel / "extras/CUPTI/lib64").mkdir(parents=True)
            (d / "usr/local" / rel / "extras/CUPTI/lib64/libcupti.so.12").write_text("")
        (d / "usr/local/cuda-12").symlink_to(d / "usr/local/cuda-12.6")   # sorts after cuda-12.9 as a name and points at the older release
        saved = gap_scripts.TOOL_ROOTS
        gap_scripts.TOOL_ROOTS = (str(d),)
        self.addCleanup(setattr, gap_scripts, "TOOL_ROOTS", saved)
        saved_which = gap_scripts.shutil.which
        gap_scripts.shutil.which = lambda n: None
        self.addCleanup(setattr, gap_scripts.shutil, "which", saved_which)
        return gap_scripts

    def test_the_newest_release_wins_over_a_symlink_that_sorts_later(self):
        self.assertIn("cuda-12.9/bin/compute-sanitizer", self.newest_fixture().find_tool("compute-sanitizer"))

    def test_libcupti_is_also_found_in_lib64_where_cuda_12_9_installs_it(self):
        import gap_scripts
        d = Path(tempfile.mkdtemp(prefix="gaplib"))
        self.addCleanup(shutil.rmtree, d, True)
        (d / "usr/local/cuda-12.9/targets/x86_64-linux/lib").mkdir(parents=True)
        (d / "usr/local/cuda-12.9/targets/x86_64-linux/lib/libcupti.so.12").write_text("")
        (d / "usr/local/cuda-12.9/lib64").symlink_to("targets/x86_64-linux/lib")
        saved = gap_scripts.TOOL_ROOTS
        gap_scripts.TOOL_ROOTS = (str(d),)
        self.addCleanup(setattr, gap_scripts, "TOOL_ROOTS", saved)
        self.assertTrue(gap_scripts.cupti_lib().endswith("libcupti.so.12"))

    def test_the_newest_release_supplies_libcupti(self):
        self.assertIn("cuda-12.9/extras", self.newest_fixture().cupti_lib())

    def test_the_default_tool_roots_are_the_machine_and_the_directory_beside_the_script(self):
        import gap_scripts
        self.assertEqual(gap_scripts.TOOL_ROOTS[0], "")
        self.assertTrue(gap_scripts.TOOL_ROOTS[1].endswith("gap_probes/tools"))

    def test_a_tool_pushed_beside_the_script_is_found_with_the_default_roots(self):
        import gap_scripts
        tools = Path(gap_scripts.__file__).resolve().parent / "tools"
        existed = tools.exists()
        bindir = tools / "usr/local/cuda-12.9/bin"
        bindir.mkdir(parents=True, exist_ok=True)
        (bindir / "gap-fake-tool").write_text("#!/bin/sh\n")
        try:
            self.assertTrue(gap_scripts.find_tool("gap-fake-tool").endswith("cuda-12.9/bin/gap-fake-tool"))
        finally:
            shutil.rmtree(tools if not existed else bindir, True)

    def test_a_counter_permission_answer_from_ncu_is_a_declined_case_not_a_failure(self):
        cls = L.classify_tool(1, "==ERROR== ERR_NVGPUCTRPERM - The user does not have permission to access NVIDIA GPU Performance Counters")
        self.assertEqual(L.tools_verdict({"metric": cls}), ("NOT_SUPPORTED", "tool_answer_metric"))

    def test_a_sanitizer_that_errors_fails_the_case_with_the_tool_named(self):
        import gap_scripts
        d = Path(tempfile.mkdtemp(prefix="gapsan2"))
        self.addCleanup(shutil.rmtree, d, True)
        fake = d / "compute-sanitizer"
        fake.write_text("#!/bin/sh\necho 'boom: internal tool failure'\nexit 3\n")
        fake.chmod(0o755)
        saved = gap_scripts.find_tool
        gap_scripts.find_tool = lambda name: str(fake)
        self.addCleanup(setattr, gap_scripts, "find_tool", saved)
        r = gap_scripts.tools_sanitizer({"probe": "/bin/true", "out": d})
        self.assertEqual(r["status"], "FAIL")
        self.assertTrue(r["reason"].endswith("_error"), r["reason"])

    def test_the_provisioning_script_pins_versions_and_uses_only_the_official_repository(self):
        text = (GP / "provision_tools.sh").read_text()
        self.assertIn("set -euo pipefail", text)
        self.assertIn("OPENROWS_NVIDIA_REPO_BASE", text)
        self.assertNotRegex(text, r"https?://(?!developer\.download\.nvidia\.com)")
        for pin in ("SAN=", "CUPTI=", "NCU=", "NSYS="):
            self.assertRegex(text, pin + r"\d+\.\d+\.\d+-\d")
        self.assertLess(len(text.splitlines()), 50)



DOC = ROOT / "docs/design/V3_GPUPV_GAP_MATRIX.md"


def doc_rows():
    rows = []
    for line in DOC.read_text().splitlines():
        c = [x.strip() for x in line.strip().strip("|").split(" | ")]
        if len(c) == 7 and re.fullmatch(r"[A-Z]{2}-\d\d", c[0]):
            rows.append(c)
    return rows


class MatrixDoc(unittest.TestCase):
    def test_the_document_opens_with_a_dated_status_and_leaves_no_placeholder(self):
        text = DOC.read_text()
        self.assertRegex(text.split("\n", 3)[2], r"^\*\*STATUS: (RESEARCH|LIVE), 2026-\d\d-\d\d")
        self.assertNotIn("@@", text)

    def test_at_least_sixty_rows_with_unique_ids_and_a_known_status(self):
        rows = doc_rows()
        self.assertGreaterEqual(len(rows), 60)
        self.assertEqual(len({r[0] for r in rows}), len(rows))
        for r in rows:
            self.assertRegex(r[3], r"^(WORKS|BROKEN|UNKNOWN|N-A\(.{20,}\))$", r[0])

    def test_every_broken_and_unknown_row_has_a_closure_path_and_the_counts_match(self):
        rows = doc_rows()
        for r in rows:
            if r[3] in ("BROKEN", "UNKNOWN"):
                self.assertRegex(r[6], r"^(IN-TREE FIX|PATCH TIER|OWNER DECISION)", r[0])
            elif r[3].startswith("N-A"):
                self.assertEqual(r[6], "N-A-BY-DESIGN", r[0])
            else:
                self.assertEqual(r[6], "-", r[0])
        bu = [r for r in rows if r[3] in ("BROKEN", "UNKNOWN")]
        want = tuple(sum(1 for r in bu if r[6].startswith(k)) for k in ("IN-TREE FIX", "PATCH TIER", "OWNER DECISION"))
        m = re.search(r"Closure path of the BROKEN and UNKNOWN rows: IN-TREE FIX (\d+), PATCH TIER (\d+), OWNER DECISION (\d+) \(and (\d+) N-A-BY-DESIGN", DOC.read_text())
        self.assertEqual(tuple(map(int, m.groups()[:3])), want)
        self.assertIn("Target: BROKEN = 0. Patch-tier rows are BROKEN until the optional patch tier ships.", DOC.read_text())

    def test_the_patch_tier_section_lists_every_patch_tier_row(self):
        text = DOC.read_text()
        sec = text.split("## 6. Patch tier")[1].split("## 7.")[0]
        for r in doc_rows():
            if r[6].startswith("PATCH TIER"):
                self.assertIn(r[0], sec, r[0])
        self.assertIn("H3", sec)

    def test_the_public_summary_counts_match_the_rows(self):
        rows = doc_rows()
        want = {k: sum(1 for r in rows if r[3].startswith(k)) for k in ("WORKS", "BROKEN", "UNKNOWN", "N-A")}
        m = re.search(r"\*\*Summary \(public, counts per kayfabe status\): (\d+) rows: WORKS (\d+), BROKEN (\d+), UNKNOWN (\d+), N-A-BY-DESIGN (\d+)\.", DOC.read_text())
        self.assertEqual(tuple(map(int, m.groups())), (len(rows), want["WORKS"], want["BROKEN"], want["UNKNOWN"], want["N-A"]))

    def test_every_source_key_of_a_row_is_defined_with_a_url_or_marked_inferred(self):
        text = DOC.read_text()
        defs = {m[0]: m[2] for m in re.findall(r"^\| (\w+) \| ([^|]+) \| (\S+) \|", text.split("## 8. Sources")[1], re.M)}
        for r in doc_rows():
            for k in r[2].split(", "):
                self.assertIn(k, defs, r[0])
                self.assertTrue(k == "I" or defs[k].startswith("https://"), k)

    def test_a_row_that_says_a_probe_is_authored_names_a_real_case(self):
        ids = {c["id"] for c in L.load_cases(TABLE)}
        for r in doc_rows():
            if "authored" in r[4]:
                self.assertTrue(any(re.search(r"\b%s\b" % re.escape(i), r[4]) for i in ids), r[0])

    def test_a_works_or_broken_row_names_a_measurement_or_a_document(self):
        for r in doc_rows():
            if r[3] in ("WORKS", "BROKEN"):
                self.assertRegex(r[4], r"MEASURED|documented|reported|docs/|traces/|README", r[0])

    def test_no_case_of_the_table_is_missing_from_the_matrix(self):
        text = DOC.read_text()
        for c in L.load_cases(TABLE):
            if c["id"] not in ("canary",):
                self.assertIn(c["id"], text, c["id"])

    def test_the_document_names_no_private_path_and_no_identifier(self):
        text = DOC.read_text()
        for bad in ("/root/", "/workspace/", "@gmail", "licence key", "instance id"):
            self.assertNotIn(bad, text)


class CtsGroups(unittest.TestCase):
    def mustpass(self):
        d = Path(tempfile.mkdtemp(prefix="gapmp"))
        self.addCleanup(shutil.rmtree, d, True)
        (d / "vk-default/pipeline/monolithic").mkdir(parents=True)
        names = ["dEQP-VK.sparse_resources.%s.case_%d" % (sub, i) for sub in ("buffer.a", "buffer.b", "image.c", "image.d") for i in range(30)]
        (d / "vk-default/sparse-resources.txt").write_text("\n".join(names) + "\n")
        (d / "vk-default/pipeline/monolithic/depth.txt").write_text("dEQP-VK.pipeline.monolithic.depth.x1\ndEQP-VK.pipeline.monolithic.depth.x2\n")
        (d / "vk-default/api.txt").write_text("")
        (d / "vk-default/pipelines").mkdir()
        (d / "vk-default/pipelines/other.txt").write_text("dEQP-VK.pipelines.other.y1\n")
        (d / "vk-default.txt").write_text("vk-default/sparse-resources.txt vk-default/pipeline/monolithic/depth.txt vk-default/pipelines/other.txt vk-default/api.txt\n")
        return d

    def test_the_same_seed_gives_the_same_list_and_another_seed_another_one(self):
        d = self.mustpass()
        a = CG.build(d, ["sparse_resources"], 12, "1")[0]
        self.assertEqual(a, CG.build(d, ["sparse_resources"], 12, "1")[0])
        self.assertNotEqual(a, CG.build(d, ["sparse_resources"], 12, "2")[0])

    def test_the_sample_is_breadth_first_one_case_per_split_in_turn(self):
        d = self.mustpass()
        lines = CG.build(d, ["sparse_resources"], 8, "7")[0]
        splits = {CG.split_key(l.split("\t")[1]) for l in lines[2:]}
        self.assertEqual(len(lines) - 2, 8)
        self.assertEqual(len(splits), 4)   # 8 cases over 4 splits of 30 each: every split is reached before any is used twice

    def test_the_info_cases_come_first_and_a_group_is_capped_and_labelled(self):
        d = self.mustpass()
        lines, notes = CG.build(d, ["sparse_resources", "pipeline"], 5, "7")
        self.assertEqual([l.split("\t")[1] for l in lines[:2]], list(CG.INFO))
        self.assertEqual(sum(1 for l in lines if l.startswith("sparse_resources\t")), 5)
        self.assertEqual(sum(1 for l in lines if l.startswith("pipeline\t")), 2)   # a group smaller than the budget is taken whole
        self.assertIn("group sparse_resources: 5 of 120 cases sampled", notes)

    def test_a_missing_or_unknown_group_is_reported_not_invented(self):
        d = self.mustpass()
        lines, notes = CG.build(d, ["api", "no_such_group"], 5, "7")
        self.assertEqual(len(lines), 2)
        self.assertTrue(any("api has no case" in n for n in notes))
        self.assertTrue(any("unknown group no_such_group" in n for n in notes))

    def test_a_sample_as_small_as_the_number_of_splits_touches_each_split_once(self):
        d = self.mustpass()
        lines = CG.build(d, ["sparse_resources"], 4, "3")[0]
        self.assertEqual(sorted(CG.split_key(l.split("\t")[1]) for l in lines[2:]), sorted(["dEQP-VK.sparse_resources.buffer.a", "dEQP-VK.sparse_resources.buffer.b",
                                                                                           "dEQP-VK.sparse_resources.image.c", "dEQP-VK.sparse_resources.image.d"]))

    def test_the_pick_inside_one_split_depends_on_the_seed(self):
        d = self.mustpass()
        picks = {tuple(l for l in CG.build(d, ["sparse_resources"], 4, str(seed))[0] if ".buffer.a." in l) for seed in range(6)}
        self.assertGreater(len(picks), 1)

    def test_a_directory_whose_name_only_starts_like_the_group_is_not_part_of_it(self):
        d = self.mustpass()
        lines, notes = CG.build(d, ["pipeline"], 10, "7")
        self.assertEqual([l for l in lines if "pipelines" in l], [])
        self.assertIn("group pipeline: 2 of 2 cases sampled", notes)

    def with_bad_names(self):
        d = self.mustpass()
        long_name = "dEQP-VK.sparse_resources.buffer.a." + "x" * 130
        with open(d / "vk-default/sparse-resources.txt", "a") as f:
            f.write(long_name + "\ndEQP-VK.sparse_resources.buffer.a.bad name\n")
        return d, long_name

    def test_a_name_the_vkcts_row_would_refuse_is_left_out_of_the_group(self):
        d, long_name = self.with_bad_names()
        names = CG.read_group(d, "sparse_resources")
        self.assertNotIn(long_name, names)
        self.assertEqual(len(names), 120)

    def test_every_sampled_line_is_a_name_the_vkcts_row_accepts(self):
        d, _ = self.with_bad_names()
        sys.path.insert(0, str(ROOT / "scripts/apps/vkcts"))
        import vkcts_lib
        lines = CG.build(d, ["sparse_resources"], 500, "9")[0]
        self.assertEqual(len(lines), 2 + 120)
        for line in lines:
            self.assertRegex(line.split("\t")[1], vkcts_lib.CASE_RE)

    def test_a_group_picks_up_its_files_under_a_directory(self):
        self.assertEqual(CG.group_files("vk-default/pipeline/monolithic/depth.txt vk-default/pipelines.txt vk-default/api.txt", "pipeline"),
                         ["vk-default/pipeline/monolithic/depth.txt"])


VK_BLOCK = "VKCTS_RESULTS_BEGIN v1 side=%s\n#case\tstatus\tclass\tarea\tdetail\n%sVKCTS_RESULTS_END n=%d digest=x\n"


def vk_rows(rows):
    return "".join("%s\t%s\t%s\t%s\t-\n" % r for r in rows)


class CtsReport(unittest.TestCase):
    def logs(self, host_rows, guest_rows):
        d = Path(tempfile.mkdtemp(prefix="gapcts"))
        self.addCleanup(shutil.rmtree, d, True)
        out = []
        for side, rows in (("host", host_rows), ("guest", guest_rows)):
            paths = []
            for i, chunk in enumerate(rows):
                f = d / ("%s%d.log" % (side, i))
                f.write_text("noise\n" + VK_BLOCK % (side, vk_rows(chunk), len(chunk)))
                paths.append(str(f))
            out.append(paths)
        return d, out

    def run_report(self, host_rows, guest_rows):
        d, (h, g) = self.logs(host_rows, guest_rows)
        return subprocess.run([sys.executable, str(GP / "cts_report.py"), "--host", *h, "--guest", *g], capture_output=True, text=True)

    def test_chunks_of_one_side_are_joined_and_counted_per_area(self):
        p = self.run_report([[("dEQP-VK.a.x", "Pass", "pass", "sparse")], [("dEQP-VK.a.y", "NotSupported", "not_supported", "sparse"), ("dEQP-VK.b.z", "Pass", "pass", "api")]],
                            [[("dEQP-VK.a.x", "Pass", "pass", "sparse"), ("dEQP-VK.a.y", "NotSupported", "not_supported", "sparse")], [("dEQP-VK.b.z", "Pass", "pass", "api")]])
        self.assertEqual(p.returncode, 0, p.stdout)
        self.assertIn("AREA sparse host: pass=1 not_supported=1 | guest: pass=1 not_supported=1", p.stdout)
        self.assertIn("AREA api host: pass=1 | guest: pass=1", p.stdout)
        self.assertIn("CTS_REPORT cases=3 differing=0 candidates=0", p.stdout)

    def test_three_chunks_of_one_case_each_count_three_cases(self):
        rows = [("dEQP-VK.a.%s" % n, "Pass", "pass", "api") for n in "xyz"]
        p = self.run_report([[r] for r in rows], [[r] for r in rows])
        self.assertIn("CTS_REPORT cases=3 differing=0 candidates=0", p.stdout)

    def test_a_guest_that_ran_less_shows_not_run_in_the_area_line(self):
        rows = [("dEQP-VK.a.x", "Pass", "pass", "api"), ("dEQP-VK.a.y", "Pass", "pass", "api")]
        p = self.run_report([rows], [rows[:1]])
        self.assertIn("AREA api host: pass=2 | guest: pass=1 not_run=1", p.stdout)

    def test_a_guest_only_timeout_is_a_candidate_and_the_exit_status_says_so(self):
        p = self.run_report([[("dEQP-VK.a.x", "Pass", "pass", "sparse"), ("dEQP-VK.a.y", "Pass", "pass", "sparse")]],
                            [[("dEQP-VK.a.x", "Pass", "pass", "sparse"), ("dEQP-VK.a.y", "Timeout", "timeout", "sparse")]])
        self.assertEqual(p.returncode, 1)
        self.assertIn("dEQP-VK.a.y\tsparse\tpass\ttimeout\tGUEST_ONLY_FAIL\tkayfabe_gap_candidate", p.stdout)
        self.assertIn("candidates=1", p.stdout)

    def test_a_case_missing_from_the_guest_logs_is_not_run_there_not_a_pass(self):
        p = self.run_report([[("dEQP-VK.a.x", "Pass", "pass", "sparse")]], [[]])
        self.assertIn("guest: not_run=1", p.stdout)
        self.assertIn("INCOMPLETE", p.stdout)

    def test_a_case_that_fails_on_both_sides_is_the_harness_or_the_application(self):
        p = self.run_report([[("dEQP-VK.a.x", "Fail", "fail", "api")]], [[("dEQP-VK.a.x", "Fail", "fail", "api")]])
        self.assertEqual(p.returncode, 0)
        self.assertIn("BOTH_FAIL\tapp_or_harness_bug", p.stdout)


class Wiring(unittest.TestCase):
    def test_the_row_is_a_discovery_row_with_its_own_summary_line(self):
        text = (ROOT / "scripts/apps/run_apps.sh").read_text()
        disc = text.split("DISCOVERY=$(cat <<'EOF'")[1].split("\nEOF")[0]
        self.assertIn("gap_probes|3600|GAP_PROBES_SUMMARY side=|", disc)
        self.assertIn("--side $SIDE", disc)
        apps = text.split("APPS=$(cat <<'EOF'")[1].split("\nEOF")[0]
        self.assertNotIn("gap_probes", apps)   # not in `all`: a gate list never carries it

    def test_the_directory_reaches_the_bundle_the_host_and_the_guest(self):
        for f, needle in (("build_bundle.sh", "gap_probes"), ("apps_matrix.sh", '"$HERE/gap_probes"'), ("apps_hook.sh", "open_rows gap_probes")):
            self.assertIn(needle, (ROOT / "scripts/apps" / f).read_text(), f)

    def test_the_host_census_asks_for_the_row_and_the_compare_step_runs_it(self):
        self.assertIn("gap_probes) echo info", (ROOT / "scripts/apps/apps_hook.sh").read_text())
        self.assertIn("gap_probes.py\" compare", (ROOT / "scripts/apps/apps_matrix.sh").read_text())

    def test_the_recipe_pins_the_current_runner(self):
        rec = json.loads((ROOT / "scripts/apps/image/recipe.json").read_text())
        import hashlib
        want = hashlib.sha256((ROOT / "scripts/apps/run_apps.sh").read_bytes()).hexdigest()
        self.assertIn(want, json.dumps(rec))


if __name__ == "__main__":
    unittest.main(verbosity=1)
