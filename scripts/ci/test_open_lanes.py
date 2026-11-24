#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""GPU-free tests of the nvbandwidth row and the llama.cpp Vulkan row (2026-11-23).

STATUS: LIVE, 2026-11-23 (docs/design/V3_OPEN_LANES_NVBW_LLAMAVK_LHM.md section 7). Fixtures: scripts/ci/fixtures/open_lanes/ (README there says which are captured and
which are constructed); nvbandwidth's JSON is built here from the writer of the pinned source. The fakes are small scripts that print what the real tool prints.
Mutants of the code under test: scripts/ci/open_lanes_mutants.py (two named killers each).
"""
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import tempfile
import textwrap
import unittest

ROOT = Path(__file__).resolve().parents[2]
OR = ROOT / "scripts/apps/open_rows"
FX = ROOT / "scripts/ci/fixtures/open_lanes"
sys.path.insert(0, str(OR))
sys.path.insert(0, str(ROOT / "scripts/apps"))
sys.path.insert(0, str(ROOT / "scripts/bench"))
import gpu_use as gu  # noqa: E402
import gpu_use_rows as rows  # noqa: E402
import llama_vulkan_row as LV  # noqa: E402
import nvbandwidth_row as NV  # noqa: E402
import rowio  # noqa: E402

# the 35 non-multinode testcases of nvbandwidth at the pinned commit (testcase.h of 4a49bdae, class names stripped), the ones that need peer pairs marked
NVB_SINGLE = """host_to_device_memcpy_ce device_to_host_memcpy_ce host_to_device_bidirectional_memcpy_ce device_to_host_bidirectional_memcpy_ce device_local_copy
all_to_host_memcpy_ce all_to_host_bidirectional_memcpy_ce host_to_all_memcpy_ce host_to_all_bidirectional_memcpy_ce host_to_device_memcpy_sm device_to_host_memcpy_sm
host_to_device_bidirectional_memcpy_sm device_to_host_bidirectional_memcpy_sm all_to_host_memcpy_sm all_to_host_bidirectional_memcpy_sm host_to_all_memcpy_sm
host_to_all_bidirectional_memcpy_sm host_device_latency_sm""".split()
NVB_PEER = """device_to_device_memcpy_read_ce device_to_device_memcpy_write_ce device_to_device_bidirectional_memcpy_read_ce device_to_device_bidirectional_memcpy_write_ce
all_to_one_write_ce all_to_one_read_ce one_to_all_write_ce one_to_all_read_ce device_to_device_memcpy_read_sm device_to_device_latency_sm device_to_device_memcpy_write_sm
device_to_device_bidirectional_memcpy_read_sm device_to_device_bidirectional_memcpy_write_sm all_to_one_write_sm all_to_one_read_sm one_to_all_write_sm one_to_all_read_sm""".split()
GPU_NAME = "NVIDIA GeForce RTX 4090"
CASES = NV.load_cases((OR / "nvbandwidth_cases.tsv").read_text())
BY_ID = {c["id"]: c for c in CASES}


# ------------------------------------------------------------------------------------------------ nvbandwidth output, as json_output.cpp writes it
def nvb_doc(testcases, devices=(GPU_NAME,), extra=None):
    body = {"version": "v0.9", "git_version": "v0.9", "CUDA Runtime Version": 12060, "Driver Version": "580.0",
            "GPU Device list": ["%d: %s (00000000:%02x:00): (fakehost)" % (i, n, i + 1) for i, n in enumerate(devices)], "testcases": testcases}
    body.update(extra or {})
    return json.dumps({"nvbandwidth": body}, indent=4)


def passed(name, cells=(("26.03",),), desc="memcpy CE CPU(row) -> GPU(column) bandwidth (GB/s)"):
    return {"name": name, "status": "Passed", "bandwidth_matrix": [list(r) for r in cells], "sum": 1.0, "bandwidth_description": desc}


def waived(name):
    return {"name": name, "status": "Waived"}


class Table(unittest.TestCase):
    def test_the_committed_table_is_the_35_testcases_of_the_pinned_source(self):
        self.assertEqual({c["id"] for c in CASES}, set(NVB_SINGLE) | set(NVB_PEER))
        self.assertEqual(len(CASES), 35)

    def test_peer_pair_cases_need_two_gpus_and_the_others_one(self):
        self.assertEqual({c["id"] for c in CASES if c["min_gpus"] == 2}, set(NVB_PEER))

    def test_every_class_the_row_exists_for_is_in_the_table(self):
        self.assertEqual({c["class"] for c in CASES}, set(NV.CLASSES))
        self.assertEqual(set(NV.REQUIRED_CLASSES), set(NV.CLASSES))

    def test_the_latency_and_local_cases_are_what_their_names_say(self):
        self.assertEqual({c["id"] for c in CASES if c["class"] == "latency"}, {"host_device_latency_sm", "device_to_device_latency_sm"})
        self.assertEqual({c["id"] for c in CASES if c["class"] == "local"}, {"device_local_copy"})
        self.assertTrue(all(c["id"].endswith("_ce") for c in CASES if c["class"] == "ce"))
        self.assertTrue(all(c["id"].endswith("_sm") for c in CASES if c["class"] == "sm"))

    def test_bad_tables_are_refused(self):
        good = (OR / "nvbandwidth_cases.tsv").read_text()
        for bad in (good + "host_to_device_memcpy_ce\tce\t1\n", good + "x\tnope\t1\n", good + "y\tce\t3\n", good + "z\tce\n",
                    good.replace("id\tclass\tmin_gpus", "id\tclass"), good + "Bad Id\tce\t1\n"):
            with self.assertRaises(ValueError, msg=bad[-30:]):
                NV.load_cases(bad)

    def test_the_table_names_no_die_or_driver(self):
        text = (OR / "nvbandwidth_cases.tsv").read_text().lower()
        for word in ("rtx", "geforce", "ada", "ampere", "hopper", "blackwell", "580", "575"):
            self.assertNotIn(word, text)


class Parsing(unittest.TestCase):
    def test_listing(self):
        text = "Index, Name:\n\tDescription\n0, host_to_device_memcpy_ce:\n\tA host to device copy\n1, device_to_host_memcpy_ce:\n\tx\n"
        self.assertEqual(NV.parse_listing(text), ["host_to_device_memcpy_ce", "device_to_host_memcpy_ce"])
        self.assertEqual(NV.parse_listing(""), [])
        self.assertEqual(NV.parse_listing(None), [])

    def test_json_after_a_warning_line(self):
        body = NV.parse_json("NOTE: something\n" + nvb_doc([passed("a")]))
        self.assertEqual(body["testcases"][0]["name"], "a")

    def test_json_garbage_is_none(self):
        for bad in ("", "no json", "{not json", json.dumps({"other": {}}), json.dumps([1]), json.dumps({"nvbandwidth": [1]})):
            self.assertIsNone(NV.parse_json(bad), bad)

    def test_device_names_drop_the_pci_address_and_the_host_name(self):
        names = NV.device_names(NV.parse_json(nvb_doc([], devices=(GPU_NAME, "NVIDIA RTX A6000"))))
        self.assertEqual(names, [GPU_NAME, "NVIDIA RTX A6000"])
        self.assertEqual(NV.device_names({"GPU Device list": []}), [])
        self.assertEqual(NV.device_names({}), [])
        self.assertEqual(NV.device_names(None), [])

    def test_matrix_values_skip_not_available_and_zero(self):
        tc = {"bandwidth_matrix": [["N/A", "276.07"], ["276.19", "N/A"], ["0", "x"], ["1e2"]]}
        self.assertEqual(NV.matrix_values(tc), [276.07, 276.19, 100.0])
        self.assertEqual(NV.matrix_values({}), [])


class Judge(unittest.TestCase):
    def judge(self, cid, tc, ngpus=1, rc=0, timed_out=False, doc=None):
        body = doc if doc is not None else NV.parse_json(nvb_doc([tc] if tc else []))
        return NV.judge_case(BY_ID[cid], ngpus, rc, body, timed_out)

    def test_a_passed_case_with_a_positive_figure_passes(self):
        r = self.judge("host_to_device_memcpy_ce", passed("host_to_device_memcpy_ce"))
        self.assertEqual(r[:3], ("PASS", "ok", "Passed"))
        self.assertEqual(r[3], [26.03])

    def test_a_case_that_needs_two_gpus_is_waived_on_one(self):
        self.assertEqual(self.judge("device_to_device_memcpy_read_ce", waived("device_to_device_memcpy_read_ce"))[:2], ("WAIVED", "needs_2_gpus_has_1"))

    def test_a_case_that_needs_one_gpu_and_was_waived_is_a_failure(self):
        self.assertEqual(self.judge("host_to_device_memcpy_ce", waived("host_to_device_memcpy_ce"))[:2], ("FAIL", "unexpected_waive"))

    def test_a_peer_case_waived_on_two_gpus_is_a_failure(self):
        self.assertEqual(self.judge("device_to_device_memcpy_read_ce", waived("device_to_device_memcpy_read_ce"), ngpus=2)[:2], ("FAIL", "unexpected_waive"))

    def test_a_peer_case_that_passed_without_peers_is_a_mismatch(self):
        r = self.judge("device_to_device_memcpy_read_ce", passed("device_to_device_memcpy_read_ce"))
        self.assertEqual(r[:2], ("MISMATCH", "passed_without_2_gpus"))

    def test_an_error_status_is_a_failure(self):
        r = self.judge("host_to_device_memcpy_ce", {"name": "host_to_device_memcpy_ce", "status": "Error"})
        self.assertEqual(r[:2], ("FAIL", "status_error"))

    def test_a_passed_case_without_a_positive_figure_fails(self):
        for cells in ((), (("N/A",),), (("0",),)):
            r = self.judge("host_to_device_memcpy_ce", passed("host_to_device_memcpy_ce", cells))
            self.assertEqual(r[:2], ("FAIL", "no_positive_figure"), cells)

    def test_a_figure_below_the_floor_fails_but_a_small_latency_does_not(self):
        low = "%g" % (NV.MIN_GBPS / 2)
        r = self.judge("host_to_device_memcpy_ce", passed("host_to_device_memcpy_ce", ((low,),)))
        self.assertEqual(r[0], "FAIL")
        self.assertTrue(r[1].startswith("below_floor"))
        self.assertEqual(self.judge("host_device_latency_sm", passed("host_device_latency_sm", ((low,),)))[0], "PASS")
        self.assertEqual(self.judge("host_to_device_memcpy_ce", passed("host_to_device_memcpy_ce", ((str(NV.MIN_GBPS),),)))[0], "PASS")

    def test_timeout_crash_missing_binary_and_missing_json(self):
        self.assertEqual(NV.judge_case(BY_ID["device_local_copy"], 1, 124, None, True)[:2], ("FAIL", "timeout"))
        self.assertEqual(NV.judge_case(BY_ID["device_local_copy"], 1, -11, None)[:2], ("FAIL", "crash_rc-11"))
        self.assertEqual(NV.judge_case(BY_ID["device_local_copy"], 1, None, None)[:2], ("NOT_RUN", "binary_missing"))
        self.assertEqual(NV.judge_case(BY_ID["device_local_copy"], 1, 0, None)[:2], ("FAIL", "no_json"))

    def test_a_document_with_an_error_key_fails(self):
        self.assertEqual(NV.judge_case(BY_ID["device_local_copy"], 1, 0, {"error": "boom", "testcases": []})[:2], ("FAIL", "binary_error"))

    def test_a_document_that_does_not_report_the_case_fails(self):
        self.assertEqual(self.judge("device_local_copy", passed("another_case"))[:2], ("FAIL", "case_not_reported"))

    def test_a_status_that_never_finished_is_a_failure(self):
        r = self.judge("host_to_device_memcpy_ce", {"name": "host_to_device_memcpy_ce", "status": "Running"})
        self.assertEqual(r[:2], ("FAIL", "status_running"))

    def test_a_latency_in_nanoseconds_is_not_judged_against_the_bandwidth_floor(self):
        self.assertEqual(self.judge("host_device_latency_sm", passed("host_device_latency_sm", (("0.01",),)))[:2], ("PASS", "ok"))

    def test_the_hidden_gpu_form_fails_every_case(self):
        """CUDA_VISIBLE_DEVICES empty: no device in the list; a case that nevertheless reports Passed has no figure, and a figure with no device is a mismatch."""
        doc = NV.parse_json(nvb_doc([passed("host_to_device_memcpy_ce", ())], devices=()))
        self.assertEqual(NV.judge_case(BY_ID["host_to_device_memcpy_ce"], 0, 0, doc)[0], "MISMATCH")
        self.assertEqual(NV.judge_case(BY_ID["host_to_device_memcpy_ce"], 0, 0, NV.parse_json(nvb_doc([waived("host_to_device_memcpy_ce")], devices=())))[:2],
                         ("WAIVED", "needs_1_gpus_has_0"))


def results_all_ok(ngpus=1):
    out = []
    for c in CASES:
        if c["min_gpus"] > ngpus:
            out.append({"id": c["id"], "class": c["class"], "result": "WAIVED", "figures": []})
        else:
            out.append({"id": c["id"], "class": c["class"], "result": "PASS", "figures": [26.0] if c["class"] != "latency" else [900.0]})
    return out


class Summary(unittest.TestCase):
    def test_all_single_gpu_cases_passing_is_a_pass(self):
        status, counts, why = NV.summarize(results_all_ok())
        self.assertEqual((status, why), ("PASS", "ok"))
        self.assertEqual((counts["PASS"], counts["WAIVED"]), (18, 17))

    def test_one_failure_or_mismatch_fails_the_row(self):
        for bad in ("FAIL", "MISMATCH"):
            r = results_all_ok()
            r[0]["result"] = bad
            self.assertEqual(NV.summarize(r)[0], "FAIL", bad)

    def test_a_row_where_nothing_ran_is_not_run_never_a_pass(self):
        r = results_all_ok()
        for x in r:
            x["result"] = "NOT_RUN"
        self.assertEqual(NV.summarize(r)[0], "NOT_RUN")
        self.assertEqual(NV.summarize([])[0], "NOT_RUN")

    def test_every_class_must_have_a_passed_case(self):
        for cls in NV.REQUIRED_CLASSES:
            r = results_all_ok()
            for x in r:
                if x["class"] == cls and x["result"] == "PASS":
                    x["result"] = "WAIVED"
            status, _, why = NV.summarize(r)
            self.assertEqual(status, "FAIL", cls)
            self.assertIn(cls, why)

    def test_the_proof_line_is_what_the_judge_reads(self):
        line = NV.proof_line(GPU_NAME, 1, results_all_ok())
        self.assertRegex(line, r"^NVBANDWIDTH_PROOF device=NVIDIA_GeForce_RTX_4090 gpus=1 passed=18 ce=8 sm=8 latency=1 local=1 min_gbps=26$")
        proof = gu.proof_nvbandwidth(line)
        self.assertIs(proof["ok"], True)
        self.assertEqual(proof["device"], GPU_NAME)

    def test_the_proof_line_of_a_hidden_gpu_run_is_judged_a_failure(self):
        r = results_all_ok()
        for x in r:
            x["result"] = "FAIL"
        proof = gu.proof_nvbandwidth(NV.proof_line("", 0, r))
        self.assertIs(proof["ok"], False)
        self.assertTrue(any("no GPU device" in w for w in proof["reasons"]))

    def test_the_latency_figure_is_not_a_bandwidth_in_the_proof(self):
        self.assertIn("min_gbps=26", NV.proof_line(GPU_NAME, 1, results_all_ok()))


class ProofJudge(unittest.TestCase):
    def line(self, **kw):
        base = dict(device="NVIDIA_GeForce_RTX_4090", gpus=1, passed=18, ce=8, sm=8, latency=1, local=1, min_gbps="26")
        base.update(kw)
        return "NVBANDWIDTH_PROOF device=%(device)s gpus=%(gpus)s passed=%(passed)s ce=%(ce)s sm=%(sm)s latency=%(latency)s local=%(local)s min_gbps=%(min_gbps)s" % base

    def test_a_good_line_passes(self):
        self.assertIs(gu.proof_nvbandwidth(self.line())["ok"], True)

    def test_every_way_to_fail(self):
        for kw, word in (({"gpus": 0}, "no GPU device"), ({"device": "llvmpipe"}, "not an NVIDIA"), ({"ce": 0}, "copy-engine"), ({"sm": 0}, "SM-copy"),
                         ({"min_gbps": "0.01"}, "below")):
            p = gu.proof_nvbandwidth(self.line(**kw))
            self.assertIs(p["ok"], False, kw)
            self.assertTrue(any(word in w for w in p["reasons"]), (kw, p["reasons"]))

    def test_a_log_without_the_proof_line_says_so_in_its_reason(self):
        p = gu.proof_nvbandwidth("nothing here")
        self.assertIsNone(p["ok"])
        self.assertTrue(any("dropped its device proof" in r for r in p["reasons"]))

    def test_no_gpu_device_fails(self):
        self.assertIs(gu.proof_nvbandwidth(self.line(gpus=0))["ok"], False)

    def test_a_non_nvidia_device_name_fails(self):
        self.assertIs(gu.proof_nvbandwidth(self.line(device="llvmpipe"))["ok"], False)

    def test_no_copy_engine_case_fails(self):
        self.assertIs(gu.proof_nvbandwidth(self.line(ce=0))["ok"], False)

    def test_no_sm_copy_case_fails(self):
        self.assertIs(gu.proof_nvbandwidth(self.line(sm=0))["ok"], False)

    def test_a_figure_below_any_real_link_fails(self):
        self.assertIs(gu.proof_nvbandwidth(self.line(min_gbps="0.01"))["ok"], False)
        self.assertIs(gu.proof_nvbandwidth(self.line(min_gbps=str(gu.NVB_MIN_GBPS)))["ok"], True)

    def test_a_dropped_proof_line_is_none_so_the_row_fails(self):
        self.assertIsNone(gu.proof_nvbandwidth("nothing here")["ok"])
        self.assertEqual(rows.evaluate("nvbandwidth", "host", "no proof line")["verdict"], gu.FAIL)

    def test_the_row_is_classified_and_asserts_the_gpu_engines(self):
        self.assertEqual(rows.row_class("nvbandwidth"), "a+proof")
        self.assertIn("nvbandwidth", rows.WIRED)
        self.assertEqual(rows.ROWS["nvbandwidth"][1], "nvbandwidth")
        self.assertIn("gpu", rows.ROWS["nvbandwidth"][3])
        self.assertEqual(rows.ROWS["nvbandwidth"][5], "FAIL")     # a log without its proof is a failure, not a pass

    def test_a_guest_run_is_judged_by_the_host_census_first(self):
        rec = rows.evaluate("nvbandwidth", "guest", self.line(), kf3_lines=[], kf3_base=None)
        self.assertIn(rec["verdict"], (gu.FAIL, gu.NOT_RUN))      # no census window: the app's own line alone never passes a guest run
        self.assertNotEqual(rec["verdict"], gu.PASS)


# ------------------------------------------------------------------------------------------------ fakes
FAKE_NVB = '''#!/usr/bin/env python3
import json, os, sys
a = sys.argv[1:]
PEER = %(peer)r
hidden = os.environ.get("CUDA_VISIBLE_DEVICES") == ""
mode = os.environ.get("FAKE_NVB_MODE", "ok")
ngpus = 0 if hidden else int(os.environ.get("FAKE_NVB_GPUS", "1"))
if "-l" in a:
    for i, n in enumerate(%(names)r):
        print("%%d, %%s:\\n\\tdesc" %% (i, n))
    sys.exit(0)
if "-s" in a:
    sys.exit(2)                                   # the row must never skip nvbandwidth's own data verification
name = a[a.index("-t") + 1]
if mode == "crash" and name == "device_local_copy":
    os.abort()
if mode == "hang" and name == "device_local_copy":
    import time
    time.sleep(60)
tcs = []
if mode == "error":
    print(json.dumps({"nvbandwidth": {"error": "boom", "testcases": []}}))
    sys.exit(0)
if mode == "othercase":
    name = "something_else"
if name in PEER and ngpus < 2:
    tcs.append({"name": name, "status": "Waived"})
elif mode == "waive_h2d" and name == "host_to_device_memcpy_ce":
    tcs.append({"name": name, "status": "Waived"})
else:
    cells = [["%%.2f" %% (26.0 + i)] for i in range(ngpus)] if "latency" not in name else [["%%.1f" %% 900.0]] * ngpus
    if mode == "nofigure":
        cells = []
    if mode == "slow":
        cells = [["0.01"]] * ngpus
    tcs.append({"name": name, "status": "Passed", "bandwidth_matrix": cells, "sum": 1.0, "bandwidth_description": "d"})
devs = ["%%d: NVIDIA GeForce RTX 4090 (00000000:%%02x:00): (fakehost)" %% (i, i + 1) for i in range(ngpus)]
print(json.dumps({"nvbandwidth": {"version": "v0.9", "git_version": "v0.9", "GPU Device list": devs, "testcases": tcs}}, indent=4))
'''

FAKE_LLAMA = '''#!/usr/bin/env python3
import os, sys
mode = os.environ.get("FAKE_LV_MODE", "ok")
hidden = os.environ.get("VK_DRIVER_FILES", "").startswith("/nonexistent")
tps = {"zero_tps": ("0.00", "200.5"), "no_tg": ("9000.1", None)}.get(mode, ("9000.1", "200.5"))
ngl = int(sys.argv[sys.argv.index("-ngl") + 1])
if hidden or mode == "no_device":
    print("ggml_vulkan: No devices found.")
    backend = "CPU"
elif mode == "llvmpipe":
    print("ggml_vulkan: Found 1 Vulkan devices:")
    print("ggml_vulkan: 0 = llvmpipe (LLVM 15.0.7, 256 bits) (llvmpipe) | uma: 0 | matrix cores: none")
    backend = "Vulkan"
else:
    print("ggml_vulkan: Found 1 Vulkan devices:")
    print("ggml_vulkan: 0 = NVIDIA GeForce RTX 4090 (NVIDIA) | uma: 0 | fp16: 1 | matrix cores: KHR_coopmat")
    backend = "CPU" if mode == "cpu_backend" else "Vulkan"
if mode == "no_table":
    sys.exit(0)
print("| model | size | params | backend | ngl | test | t/s |")
print("| --- | ---: | ---: | --- | --: | ---: | ---: |")
for test, v in (("pp512", tps[0] + " +- 1.0"), ("tg64", None if tps[1] is None else tps[1] + " +- 0.5")):
    if v is None:
        continue
    print("| qwen2 1.5B Q4_K - Medium | 1.04 GiB | 1.78 B | %-10s | %3d | %15s | %20s |" % (backend, ngl, test, v))
sys.exit(int(os.environ.get("FAKE_LV_RC", "0")))
'''


def write_exec(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    path.chmod(path.stat().st_mode | stat.S_IXUSR)


def fake_nvbandwidth(path):
    write_exec(path, FAKE_NVB % {"peer": set(NVB_PEER), "names": [c["id"] for c in CASES]})


def fake_llama(path):
    write_exec(path, FAKE_LLAMA)


class Args:
    def __init__(self, **kw):
        self.__dict__.update(dict(bin=None, bundle=None, only=None, buffer_mib=8, samples=1, emit=False, model=None, data=None, ngl=99), **kw)


class RunnerEndToEnd(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self.tmp.name)
        self.bin = self.dir / "nvbandwidth"
        fake_nvbandwidth(self.bin)
        self.env = {k: os.environ.get(k) for k in ("CUDA_VISIBLE_DEVICES", "FAKE_NVB_MODE", "FAKE_NVB_GPUS")}
        for k in self.env:
            os.environ.pop(k, None)

    def tearDown(self):
        for k, v in self.env.items():
            os.environ.pop(k, None)
            if v is not None:
                os.environ[k] = v
        self.tmp.cleanup()

    def run_row(self, **kw):
        import contextlib
        import io
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = NV.cmd_run(Args(**dict(dict(out=str(self.dir / "out"), bin=str(self.bin)), **kw)))
        return rc, buf.getvalue()

    def test_a_single_gpu_run_passes_with_the_peer_cases_waived(self):
        rc, out = self.run_row()
        self.assertEqual(rc, 0)
        summary = out.strip().splitlines()[-1]
        self.assertRegex(summary, r"^NVBANDWIDTH_SUMMARY status=PASS reason=ok pass=18 waived=17 fail=0 mismatch=0 not_run=0 cases=35 gpus=1 ")
        self.assertIn("device=NVIDIA_GeForce_RTX_4090", summary)
        self.assertRegex(out, r"NVBANDWIDTH_PROOF device=NVIDIA_GeForce_RTX_4090 gpus=1 passed=18 ce=8 sm=8 latency=1 local=1 ")
        self.assertIn("CASE device_to_device_memcpy_read_ce class=ce result=WAIVED reason=needs_2_gpus_has_1", out)
        self.assertTrue((self.dir / "out" / "results.tsv").is_file())
        self.assertEqual(len(list((self.dir / "out" / "logs").glob("*.log"))), 36)       # one per case plus the listing

    def test_the_hidden_gpu_negative_control_fails_and_the_judge_agrees(self):
        os.environ["CUDA_VISIBLE_DEVICES"] = ""
        rc, out = self.run_row()
        self.assertEqual(rc, 1)
        self.assertRegex(out.strip().splitlines()[-1], r"status=FAIL .* gpus=0 ")
        proof = gu.proof_nvbandwidth(out)
        self.assertIs(proof["ok"], False)
        self.assertEqual(gu.control_verdict(rows.evaluate("nvbandwidth", "host", out)), "CONTROL_OK")

    def test_the_same_run_with_the_gpu_visible_is_not_a_control(self):
        rc, out = self.run_row()
        self.assertEqual(gu.control_verdict(rows.evaluate("nvbandwidth", "host", out)), "CONTROL_BROKEN")

    def test_a_crash_is_booked_on_its_case_and_the_row_fails(self):
        os.environ["FAKE_NVB_MODE"] = "crash"
        rc, out = self.run_row()
        self.assertEqual(rc, 1)
        self.assertIn("CASE device_local_copy class=local result=FAIL reason=crash_rc-6", out)
        self.assertIn("fail=1", out)

    def test_an_unexpected_waive_fails_the_row(self):
        os.environ["FAKE_NVB_MODE"] = "waive_h2d"
        rc, out = self.run_row()
        self.assertEqual(rc, 1)
        self.assertIn("host_to_device_memcpy_ce class=ce result=FAIL reason=unexpected_waive", out)

    def test_a_binary_that_reports_another_case_fails_the_row(self):
        os.environ["FAKE_NVB_MODE"] = "othercase"
        rc, out = self.run_row()
        self.assertEqual(rc, 1)
        self.assertIn("reason=case_not_reported", out)

    def test_a_binary_that_reports_an_error_fails_the_row(self):
        os.environ["FAKE_NVB_MODE"] = "error"
        rc, out = self.run_row()
        self.assertEqual(rc, 1)
        self.assertIn("reason=binary_error", out)

    def test_the_hidden_gpu_proof_line_says_no_device(self):
        os.environ["CUDA_VISIBLE_DEVICES"] = ""
        rc, out = self.run_row()
        self.assertRegex(out, r"NVBANDWIDTH_PROOF device=none gpus=0 passed=0 ")

    def test_compare_logs_front_end_compares_two_runs(self):
        self.run_row(emit=True)
        shutil_copy = __import__("shutil").copytree
        shutil_copy(self.dir / "out", self.dir / "guest")
        cmd = [sys.executable, "-B", "-I", str(OR / "compare_logs.py"), "nvbandwidth", str(self.dir / "out"), str(self.dir / "guest")]
        r = subprocess.run(cmd, capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("NVBANDWIDTH_COMPARE cases=35 agree=35 disagree=0", r.stdout)
        tsv = self.dir / "guest" / "results.tsv"
        tsv.write_text(tsv.read_text().replace("host_to_device_memcpy_ce\tce\tPASS", "host_to_device_memcpy_ce\tce\tFAIL"))
        r = subprocess.run(cmd, capture_output=True, text=True)
        self.assertEqual(r.returncode, 1)
        self.assertIn("disagree=1", r.stdout)

    def test_cases_that_pass_with_no_figure_fail_the_row(self):
        os.environ["FAKE_NVB_MODE"] = "nofigure"
        rc, out = self.run_row()
        self.assertEqual(rc, 1)
        self.assertIn("result=FAIL reason=no_positive_figure", out)
        self.assertIs(gu.proof_nvbandwidth(out)["ok"], False)

    def test_a_figure_below_the_floor_fails_the_row(self):
        os.environ["FAKE_NVB_MODE"] = "slow"
        rc, out = self.run_row()
        self.assertEqual(rc, 1)
        self.assertIn("reason=below_floor_0.01", out)

    def test_a_hung_case_is_killed_booked_as_a_timeout_and_fails_the_row(self):
        os.environ["FAKE_NVB_MODE"] = "hang"
        saved = NV.CASE_TIMEOUT_S
        NV.CASE_TIMEOUT_S = 2
        try:
            rc, out = self.run_row()
        finally:
            NV.CASE_TIMEOUT_S = saved
        self.assertEqual(rc, 1)
        self.assertIn("CASE device_local_copy class=local result=FAIL reason=timeout status=Timeout", out)

    def test_nvbandwidth_is_never_told_to_skip_its_data_verification(self):
        rc, out = self.run_row()                      # the fake exits 2 on -s: every case would be a crash
        self.assertEqual(rc, 0)
        self.assertNotIn("reason=crash", out)

    def test_two_gpus_run_the_peer_cases_too(self):
        os.environ["FAKE_NVB_GPUS"] = "2"
        rc, out = self.run_row()
        self.assertEqual(rc, 0)
        self.assertRegex(out.strip().splitlines()[-1], r"pass=35 waived=0 fail=0 mismatch=0 not_run=0 cases=35 gpus=2 ")

    def test_a_missing_binary_is_not_run_with_an_appnotrun_line_and_exit_zero(self):
        rc, out = self.run_row(bin=str(self.dir / "absent"))
        self.assertEqual(rc, 0)
        self.assertIn("NVBANDWIDTH_SUMMARY status=NOT_RUN", out)
        self.assertIn("APPNOTRUN nvbandwidth:binary_missing", out)
        self.assertNotIn("status=PASS", out)

    def test_only_selects_cases(self):
        rc, out = self.run_row(only="host_to_device_memcpy_*")
        self.assertEqual(len([l for l in out.splitlines() if l.startswith("CASE ")]), 2)
        self.assertEqual(rc, 1)                       # a subset has no latency case: the row says so and does not pass

    def test_emit_carries_the_files_through_the_log(self):
        rc, out = self.run_row(emit=True)
        files = rowio.extract(out)
        self.assertEqual(set(files), {"results.tsv", "summary.txt"})
        self.assertEqual(len(NV.parse_results(files["results.tsv"])), 35)

    def test_compare_flags_a_changed_result_and_reports_the_ratio(self):
        self.run_row()
        host = (self.dir / "out" / "results.tsv").read_text()
        guest = host.replace("host_to_device_memcpy_ce\tce\tPASS", "host_to_device_memcpy_ce\tce\tFAIL")
        h, g = NV.parse_results(host), NV.parse_results(guest)
        cmp = NV.compare_rows(h, g)
        self.assertEqual([r[0] for r in cmp if r[3] == "DISAGREE"], ["host_to_device_memcpy_ce"])
        self.assertEqual(NV.compare_rows(h, h)[0][3], "AGREE")
        g2 = dict(h)
        g2["host_to_device_memcpy_ce"] = dict(h["host_to_device_memcpy_ce"], best=h["host_to_device_memcpy_ce"]["best"] / 2)
        ratio = [r for r in NV.compare_rows(h, g2) if r[0] == "host_to_device_memcpy_ce"][0][4]
        self.assertAlmostEqual(ratio, 0.5)
        del g2["device_local_copy"]
        self.assertIn(("device_local_copy", "PASS", None, "DISAGREE", None), NV.compare_rows(h, g2))

    def test_case_commands_never_go_through_a_shell(self):
        argv = NV.case_argv("/x/nvbandwidth", "device_local_copy", 512, 3)
        self.assertEqual(argv, ["/x/nvbandwidth", "-j", "-t", "device_local_copy", "-b", "512", "-i", "3"])
        self.assertNotIn("-s", argv)                  # data verification stays on: nvbandwidth checks what it copied


class LlamaVulkanParsing(unittest.TestCase):
    def fx(self, name):
        return (FX / name).read_text()

    def test_the_real_nvidia_log_passes_and_records_the_matrix_cores(self):
        status, why, facts = LV.judge(self.fx("llama_vulkan_nvidia_windows_real.log"))
        self.assertEqual((status, why), ("PASS", "ok"))
        self.assertEqual(facts["device"], "NVIDIA GeForce RTX 4060 Ti")
        self.assertEqual(facts["coopmat"], "NV_coopmat2")
        self.assertAlmostEqual(facts["pp512"], 10332.92)
        self.assertAlmostEqual(facts["tg64"], 181.28)

    def test_a_cpu_vulkan_driver_fails(self):
        self.assertEqual(LV.judge(self.fx("llama_vulkan_llvmpipe.log"))[:2], ("FAIL", "device_is_not_nvidia:llvmpipe"))

    def test_no_device_fails(self):
        self.assertEqual(LV.judge(self.fx("llama_vulkan_no_device_cpu.log"))[:2], ("FAIL", "no_vulkan_device"))

    def test_ngl_zero_fails(self):
        self.assertEqual(LV.judge(self.fx("llama_vulkan_nvidia_ngl0.log"))[:2], ("FAIL", "ngl_0"))

    def test_a_cpu_backend_row_next_to_an_nvidia_device_fails(self):
        text = self.fx("llama_vulkan_nvidia_windows_real.log").replace("Vulkan     |  99 |            tg64", "CPU        |  99 |            tg64")
        self.assertEqual(LV.judge(text)[0:2], ("FAIL", "backend_CPU"))

    def test_a_missing_generation_row_fails(self):
        text = "\n".join(l for l in self.fx("llama_vulkan_nvidia_windows_real.log").splitlines() if "tg64" not in l)
        self.assertEqual(LV.judge(text)[:2], ("FAIL", "missing_pp512_or_tg64"))

    def test_a_zero_throughput_row_fails(self):
        text = self.fx("llama_vulkan_nvidia_windows_real.log").replace("10332.92", "0.00")
        self.assertEqual(LV.judge(text)[:2], ("FAIL", "no_throughput_pp512"))

    def test_no_table_fails(self):
        text = "\n".join(l for l in self.fx("llama_vulkan_nvidia_windows_real.log").splitlines() if not l.startswith("|"))
        self.assertEqual(LV.judge(text)[:2], ("FAIL", "no_table_row"))

    def test_a_die_without_cooperative_matrix_still_passes_with_coopmat_none(self):
        text = self.fx("llama_vulkan_nvidia_windows_real.log").replace("NV_coopmat2", "none")
        status, why, facts = LV.judge(text)
        self.assertEqual((status, facts["coopmat"]), ("PASS", "none"))
        self.assertEqual(LV.coopmat([]), "unknown")

    def test_the_judge_of_the_row_and_the_judge_of_the_gpu_use_module_agree_on_every_fixture(self):
        for name in sorted(p.name for p in FX.glob("llama_vulkan_*.log")):
            text = self.fx(name)
            self.assertEqual(LV.judge(text)[0] == "PASS", gu.proof_llama_vulkan(text)["ok"] is True, name)

    def test_the_gpu_use_module_names_the_reason(self):
        self.assertTrue(any("not an NVIDIA" in r for r in gu.proof_llama_vulkan(self.fx("llama_vulkan_llvmpipe.log"))["reasons"]))
        self.assertTrue(any("no Vulkan device" in r for r in gu.proof_llama_vulkan(self.fx("llama_vulkan_no_device_cpu.log"))["reasons"]))
        self.assertTrue(any("ngl 0" in r for r in gu.proof_llama_vulkan(self.fx("llama_vulkan_nvidia_ngl0.log"))["reasons"]))
        self.assertIsNone(gu.proof_llama_vulkan("nothing")["ok"])

    def test_the_cuda_judge_is_not_fooled_into_accepting_a_vulkan_log_as_cuda(self):
        self.assertIsNot(gu.proof_llama(self.fx("llama_vulkan_nvidia_windows_real.log"), mode="bench")["ok"], True)

    def test_the_judge_of_the_rows_module_fails_every_fallback_and_passes_the_real_log(self):
        self.assertEqual(rows.evaluate("llama_vulkan", "host", self.fx("llama_vulkan_nvidia_windows_real.log"))["verdict"], gu.PASS)
        for name in ("llama_vulkan_llvmpipe.log", "llama_vulkan_no_device_cpu.log", "llama_vulkan_nvidia_ngl0.log"):
            self.assertEqual(rows.evaluate("llama_vulkan", "host", self.fx(name))["verdict"], gu.FAIL, name)
            self.assertEqual(gu.control_verdict(rows.evaluate("llama_vulkan", "host", self.fx(name))), "CONTROL_OK", name)

    def test_a_device_line_without_a_table_is_not_a_proof(self):
        text = self.fx("llama_vulkan_nvidia_no_table.log")
        self.assertIsNot(gu.proof_llama_vulkan(text)["ok"], True)
        self.assertEqual(rows.evaluate("llama_vulkan", "host", text)["verdict"], gu.FAIL)

    def test_a_log_with_no_vulkan_proof_at_all_is_a_failure_not_a_not_run(self):
        self.assertEqual(rows.evaluate("llama_vulkan", "host", "nothing here")["verdict"], gu.FAIL)

    def test_the_row_is_classified_as_able_to_fall_back(self):
        self.assertEqual(rows.row_class("llama_vulkan"), "b")
        self.assertIn("llama_vulkan", rows.WIRED)
        self.assertEqual(rows.ROWS["llama_vulkan"][1], "llama_vulkan")
        self.assertEqual(rows.ROWS["llama_vulkan"][5], "FAIL")     # a log without its proof is a failure, not a pass


class LlamaVulkanRunner(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self.tmp.name)
        fake_llama(self.dir / "llama_vk" / "llama-bench")
        (self.dir / "data").mkdir()
        (self.dir / "data" / LV.MODEL_FILE).write_text("x")
        self.saved = {k: os.environ.pop(k, None) for k in ("FAKE_LV_MODE", "FAKE_LV_RC", "VK_DRIVER_FILES")}

    def tearDown(self):
        for k, v in self.saved.items():
            os.environ.pop(k, None)
            if v is not None:
                os.environ[k] = v
        self.tmp.cleanup()

    def run_row(self, **kw):
        import contextlib
        import io
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = LV.cmd_run(Args(**dict(dict(out=str(self.dir / "out"), bundle=str(self.dir), data=str(self.dir / "data")), **kw)))
        return rc, buf.getvalue()

    def test_a_run_on_an_nvidia_device_passes(self):
        rc, out = self.run_row()
        self.assertEqual(rc, 0)
        self.assertRegex(out.strip().splitlines()[-1], r"^LLAMA_VULKAN_SUMMARY status=PASS reason=ok device=NVIDIA_GeForce_RTX_4090 coopmat=KHR_coopmat pp512_tps=9000\.10 tg64_tps=200\.50 ngl=99 ")
        self.assertEqual(rows.evaluate("llama_vulkan", "host", out)["app"]["ok"], True)

    def test_the_negative_control_forms_fail_and_the_judge_agrees(self):
        os.environ["VK_DRIVER_FILES"] = "/nonexistent/x.json"
        rc, out = self.run_row(ngl=0)
        self.assertEqual(rc, 1)
        self.assertIn("status=FAIL reason=no_vulkan_device", out)
        self.assertEqual(gu.control_verdict(rows.evaluate("llama_vulkan", "host", out)), "CONTROL_OK")

    def test_ngl_zero_alone_is_a_failure_too(self):
        rc, out = self.run_row(ngl=0)
        self.assertEqual(rc, 1)
        self.assertIn("reason=ngl_0", out)

    def test_a_software_vulkan_driver_fails(self):
        os.environ["FAKE_LV_MODE"] = "llvmpipe"
        rc, out = self.run_row()
        self.assertEqual(rc, 1)
        self.assertIn("status=FAIL reason=device_is_not_nvidia", out)

    def test_a_cpu_backend_row_next_to_an_nvidia_device_fails(self):
        os.environ["FAKE_LV_MODE"] = "cpu_backend"
        rc, out = self.run_row()
        self.assertEqual(rc, 1)
        self.assertIn("reason=backend_CPU", out)

    def test_a_zero_throughput_fails(self):
        os.environ["FAKE_LV_MODE"] = "zero_tps"
        rc, out = self.run_row()
        self.assertEqual(rc, 1)
        self.assertIn("reason=no_throughput_pp512", out)

    def test_a_table_without_the_generation_row_fails(self):
        os.environ["FAKE_LV_MODE"] = "no_tg"
        rc, out = self.run_row()
        self.assertEqual(rc, 1)
        self.assertIn("reason=missing_pp512_or_tg64", out)

    def test_a_log_without_a_table_says_no_table_row(self):
        os.environ["FAKE_LV_MODE"] = "no_table"
        rc, out = self.run_row()
        self.assertEqual(rc, 1)
        self.assertIn("reason=no_table_row", out)

    def test_the_bench_arguments_equal_those_of_the_cuda_llama_bench_row(self):
        line = next(l for l in (ROOT / "scripts/apps/run_apps.sh").read_text().splitlines() if l.startswith("llama_bench|"))
        tail = line.split("-ngl ${KF_NGL:-99} ", 1)[1]
        self.assertEqual(tuple(tail.split()), LV.BENCH_ARGS)

    def test_a_nonzero_exit_status_fails_even_with_a_good_table(self):
        os.environ["FAKE_LV_RC"] = "3"
        rc, out = self.run_row()
        self.assertEqual(rc, 1)
        self.assertIn("status=FAIL reason=rc_3", out)

    def test_a_missing_binary_or_model_is_not_run_never_a_failure_or_a_pass(self):
        for kw, reason in (({"bundle": str(self.dir / "nowhere")}, "binary_missing"), ({"data": str(self.dir / "nodata")}, "model_missing")):
            rc, out = self.run_row(**kw)
            self.assertEqual(rc, 0)
            self.assertIn("status=NOT_RUN reason=" + reason, out)
            self.assertIn("APPNOTRUN llama_vulkan:" + reason, out)

    def test_the_command_line_is_the_bench_the_cuda_row_runs(self):
        argv = LV.bench_argv("/b/llama-bench", "/d/m.gguf", 99)
        self.assertEqual(argv, ["/b/llama-bench", "-m", "/d/m.gguf", "-ngl", "99", "-p", "512", "-n", "64", "-r", "2"])
        run_apps = (ROOT / "scripts/apps/run_apps.sh").read_text()
        self.assertIn("-ngl ${KF_NGL:-99} -p 512 -n 64 -r 2", run_apps)       # llama_bench (CUDA)
        self.assertEqual(LV.MODEL_FILE, "qwen2.5-1.5b-instruct-q4_k_m.gguf")
        self.assertIn(LV.MODEL_FILE, run_apps)


# ------------------------------------------------------------------------------------------------ run_apps.sh, with the fakes
class RunAppsShell(unittest.TestCase):
    """The real run_apps.sh with the fake tools: the default command PASSES, KF_NEGCTL=1 runs the hidden-GPU form of the same row and the judge says FAIL."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        for sub in ("bundle/share", "bundle/bin", "bundle/llama_vk", "data", "out"):
            (self.root / sub).mkdir(parents=True)
        subprocess.run(["cp", "-r", str(OR), str(self.root / "bundle/share/")], check=True)
        fake_nvbandwidth(self.root / "bundle/bin/nvbandwidth")
        fake_llama(self.root / "bundle/llama_vk/llama-bench")
        (self.root / "data" / LV.MODEL_FILE).write_text("x")

    def tearDown(self):
        self.tmp.cleanup()

    def run_row(self, app, negctl=False, **extra):
        env = {k: v for k, v in os.environ.items() if not k.startswith(("KF_", "FAKE_", "VK_", "CUDA_VISIBLE"))}
        env.update(APPS_ROOT=str(self.root), KF_NEGCTL="1" if negctl else "0", **extra)
        p = subprocess.run(["bash", str(ROOT / "scripts/apps/run_apps.sh"), "guest", app], capture_output=True, text=True, stdin=subprocess.DEVNULL, timeout=300, env=env)
        log = (self.root / "out" / "guest" / (app + ".log")).read_text()
        return [l for l in p.stdout.splitlines() if l.startswith("APPRES ")][-1], log

    def test_nvbandwidth_passes_by_default_and_its_negative_control_fails_by_itself(self):
        res, log = self.run_row("nvbandwidth")
        self.assertIn("verdict=PASS", res)
        self.assertEqual(rows.evaluate("nvbandwidth", "host", log)["app"]["ok"], True)
        res, log = self.run_row("nvbandwidth", True)
        self.assertIn("verdict=FAIL", res)
        self.assertEqual(gu.control_verdict(rows.evaluate("nvbandwidth", "host", log)), "CONTROL_OK")

    def test_llama_vulkan_passes_by_default_and_its_negative_control_fails_by_itself(self):
        res, log = self.run_row("llama_vulkan")
        self.assertIn("verdict=PASS", res)
        self.assertEqual(rows.evaluate("llama_vulkan", "host", log)["app"]["ok"], True)
        res, log = self.run_row("llama_vulkan", True)
        self.assertIn("verdict=FAIL", res)
        self.assertIn("ngl=0", log)
        self.assertIn("reason=no_vulkan_device", log)      # the ICD is hidden too: -ngl 0 alone would have said ngl_0
        self.assertEqual(gu.control_verdict(rows.evaluate("llama_vulkan", "host", log)), "CONTROL_OK")

    def test_a_row_whose_tool_is_missing_is_notrun_with_the_reason(self):
        (self.root / "bundle/bin/nvbandwidth").unlink()
        res, _ = self.run_row("nvbandwidth")
        self.assertIn("verdict=NOTRUN", res)
        self.assertIn("binary_missing", res)

    def test_the_vulkan_row_without_its_binary_is_notrun_with_the_reason(self):
        (self.root / "bundle/llama_vk/llama-bench").unlink()
        res, _ = self.run_row("llama_vulkan")
        self.assertIn("verdict=NOTRUN", res)
        self.assertIn("binary_missing", res)

    def test_the_summary_regexes_demand_status_pass(self):
        run_apps = (ROOT / "scripts/apps/run_apps.sh").read_text()
        for name, tag in (("nvbandwidth", "NVBANDWIDTH_SUMMARY"), ("llama_vulkan", "LLAMA_VULKAN_SUMMARY")):
            rx = next(l for l in run_apps.splitlines() if l.startswith(name + "|")).split("|")[2]
            self.assertRegex(tag + " status=PASS reason=ok", rx)
            for bad in ("status=FAIL", "status=NOT_RUN"):
                self.assertNotRegex("%s %s reason=x" % (tag, bad), rx)

    def test_a_row_that_prints_a_pass_summary_but_fails_its_exit_status_is_not_a_pass(self):
        res, _ = self.run_row("llama_vulkan", FAKE_LV_RC="3")
        self.assertIn("verdict=FAIL", res)


class Wiring(unittest.TestCase):
    """Text pins (they prove the line is there, not that it works; the shell tests above show that it works)."""

    def test_the_rows_are_discovery_rows_in_no_gate_list(self):
        run_apps = (ROOT / "scripts/apps/run_apps.sh").read_text()
        disc = run_apps.split("DISCOVERY=$(cat <<'EOF'")[1].split("\nEOF")[0]
        for name in ("nvbandwidth", "llama_vulkan"):
            self.assertRegex(disc, r"(?m)^%s\|\d+\|" % name)
        for f in ("gate_list.txt", "gate_list_middle.txt", "expected_rows.txt"):
            text = (ROOT / "scripts/apps" / f).read_text()
            for name in ("nvbandwidth", "llama_vulkan"):
                self.assertNotIn(name, text, f)

    def test_the_negative_control_hides_the_gpu_in_the_run_apps_block(self):
        run_apps = (ROOT / "scripts/apps/run_apps.sh").read_text()
        block = run_apps.split('if [ "${KF_NEGCTL:-0}" = 1 ]; then')[1].split("\nfi\n")[0]
        self.assertIn('KF_GPU_ENV="CUDA_VISIBLE_DEVICES="', block)
        self.assertIn("KF_VK_ENV=", block)
        self.assertIn("VK_DRIVER_FILES=", block)
        self.assertIn("KF_NGL=0", block)
        self.assertIn("${KF_GPU_ENV:+env $KF_GPU_ENV }python3", run_apps.split("\nnvbandwidth|")[1].split("\n")[0])
        self.assertIn("${KF_VK_ENV:+env $KF_VK_ENV }python3", run_apps.split("\nllama_vulkan|")[1].split("\n")[0])
        self.assertIn("--ngl ${KF_NGL:-99}", run_apps.split("\nllama_vulkan|")[1].split("\n")[0])

    def test_provision_builds_both_tools_from_pins(self):
        text = (OR / "provision_lanes.sh").read_text()
        for needle in ("-DGGML_VULKAN=ON", "-DGGML_CUDA=OFF", "want nvbandwidth && step nvbandwidth nvbandwidth", "want llama_vulkan && step llama_vulkan llama_vulkan",
                       "pin_install_cuda_toolkit"):
            self.assertIn(needle, text)
        body = re.search(r"^lunarg_sdk\(\) \{\n(.*?)^\}\n", text, re.S | re.M).group(1)
        self.assertEqual(text.count("curl -"), 2)                       # the two downloads of the LunarG SDK, nothing else
        self.assertEqual(body.count("curl -"), 2)
        self.assertEqual(body.count("sdk.sha"), 2)                                # fetched into a file and read back for the comparison
        self.assertIn('"$OPENROWS_LUNARG_CHECKSUM_BASE/$v/linux/vulkan_sdk.tar.xz.txt"', body)   # from LunarG's checksum endpoint, not from the download's own host
        self.assertLess(body.index("sdk.sha"), body.index("sha256sum"))          # the checksum is fetched from LunarG and compared ...
        self.assertLess(body.index("SDK_HASH_MISMATCH"), body.index("tar -C"))   # ... before anything is unpacked
        self.assertNotIn("wget", text)
        self.assertNotIn("git clone", text)
        self.assertIsNone(re.search(r"command\s+apt-get", text))             # the one sanctioned bypass stays in pin_lib.sh

    def test_the_vulkan_build_is_vulkan_only_and_builds_the_bench(self):
        body = re.search(r"^llama_vulkan\(\) \{\n(.*?)^\}\n", (OR / "provision_lanes.sh").read_text(), re.S | re.M).group(1)
        for needle in ("-DGGML_VULKAN=ON", "-DGGML_CUDA=OFF", "--target llama-bench", "build-vk"):
            self.assertIn(needle, body)

    def test_every_pin_variable_the_script_uses_is_defined_and_the_commit_pins_are_used(self):
        text = (OR / "provision_lanes.sh").read_text()
        used = set(re.findall(r"\$\{?((?:OPENROWS|PIN)_[A-Z0-9_]+)", text))
        defined = {l.split("=", 1)[0] for f in (OR / "pins.env", ROOT / "scripts/apps/pins.sh") for l in f.read_text().splitlines() if re.match(r"^[A-Z_0-9]+=", l)}
        self.assertEqual(set(), used - defined)
        self.assertTrue({"OPENROWS_NVB_COMMIT", "PIN_LLAMA_COMMIT", "OPENROWS_NVB_URL", "PIN_LLAMA_URL"} <= used)

    def run_lunarg_sdk(self, good_checksum):
        """The real lunarg_sdk function of provision_lanes.sh, with a fake curl that serves a small tarball and a checksum text."""
        import hashlib
        import io
        import tarfile
        text = (OR / "provision_lanes.sh").read_text()
        fn = re.search(r"^lunarg_sdk\(\) \{\n.*?^\}\n", text, re.S | re.M).group(0)
        with tempfile.TemporaryDirectory() as t:
            t = Path(t)
            buf = io.BytesIO()
            with tarfile.open(fileobj=buf, mode="w:xz") as tf:
                data = b"#!/bin/sh\n"
                info = tarfile.TarInfo("1.4.1.1/x86_64/bin/glslc")
                info.size, info.mode = len(data), 0o755
                tf.addfile(info, io.BytesIO(data))
            tarball = buf.getvalue()
            (t / "served.tar.xz").write_bytes(tarball)
            digest = hashlib.sha256(tarball).hexdigest() if good_checksum else "0" * 64
            (t / "served.sha").write_text(digest + "  vulkansdk-linux-x86_64-1.4.1.1.tar.xz\n")
            write_exec(t / "bin" / "curl", '#!/bin/bash\nwhile [ $# -gt 0 ]; do case "$1" in -o) out=$2; shift;; http*) url=$1;; esac; shift; done\n'
                                           'case "$url" in */sha/*) cp "%s/served.sha" "$out";; *) cp "%s/served.tar.xz" "$out";; esac\n' % (t, t))
            script = ('ROOT=%s/root; OPENROWS_LUNARG_SDK_VERSION=1.4.1.1; OPENROWS_LUNARG_SDK_BASE=https://sdk.example/dl; OPENROWS_LUNARG_CHECKSUM_BASE=https://sdk.example/sha\n%s\n'
                      'lunarg_sdk; echo "rc=$?"; ls "$ROOT/vulkan-sdk" 2>/dev/null\n') % (t, fn)
            r = subprocess.run(["bash", "-c", script], capture_output=True, text=True, env=dict(os.environ, PATH=str(t / "bin") + os.pathsep + os.environ["PATH"]))
            return r.stdout, (t / "root" / "vulkan-sdk" / "1.4.1.1" / "x86_64" / "bin" / "glslc").exists()

    def test_the_lunarg_sdk_is_unpacked_when_it_hashes_to_what_lunarg_lists(self):
        out, unpacked = self.run_lunarg_sdk(True)
        self.assertIn("rc=0", out)
        self.assertTrue(unpacked)

    def test_the_lunarg_sdk_download_is_refused_and_not_unpacked_when_the_hash_differs(self):
        out, unpacked = self.run_lunarg_sdk(False)
        self.assertIn("SDK_HASH_MISMATCH", out)
        self.assertIn("rc=1", out)
        self.assertFalse(unpacked)

    def test_provision_checks_every_source_out_at_its_pinned_commit(self):
        text = (OR / "provision_lanes.sh").read_text()
        self.assertIn('pin_git_checkout "$src" "$OPENROWS_NVB_URL" "$OPENROWS_NVB_TAG" "$OPENROWS_NVB_COMMIT" || return 1', text)
        self.assertIn('pin_git_checkout "$src" "$PIN_LLAMA_URL" "" "$PIN_LLAMA_COMMIT" || return 1', text)

    def test_every_command_of_a_step_ends_in_a_return_because_set_e_is_off_inside_if(self):
        text = (OR / "provision_lanes.sh").read_text()
        for fn in ("nvbandwidth", "llama_vulkan", "llama_model"):
            body = re.search(r"^%s\(\) \{\n(.*?)^\}\n" % fn, text, re.S | re.M).group(1)
            logical, cur = [], ""
            for line in body.splitlines():
                if line.strip().startswith("#"):
                    continue
                cur += line.strip().rstrip("\\") + " "
                if not line.rstrip().endswith("\\"):
                    logical.append(cur.strip())
                    cur = ""
            for cmd in logical[:-1]:                    # the last command's status is the function's
                if cmd.startswith(("local ", "nvcc=$(find_nvcc)", "if ", "fi")):
                    continue
                self.assertIn("return 1", cmd, "%s: %s" % (fn, cmd[:90]))

    def test_the_nvbandwidth_commit_is_one_pin_git_checkout_accepts(self):
        pins = dict(l.split("=", 1) for l in (OR / "pins.env").read_text().splitlines() if "=" in l and not l.startswith("#"))
        r = subprocess.run(["bash", "-c", '. "%s"; pin_is_commit "$1"' % (ROOT / "scripts/apps/pin_lib.sh"), "x", pins["OPENROWS_NVB_COMMIT"]], capture_output=True)
        self.assertEqual(r.returncode, 0)
        r = subprocess.run(["bash", "-c", '. "%s"; pin_is_commit "$1"' % (ROOT / "scripts/apps/pin_lib.sh"), "x", "v0.9"], capture_output=True)
        self.assertNotEqual(r.returncode, 0)

    def test_the_pins_are_commits_and_there_is_no_second_llama_pin(self):
        pins = dict(l.split("=", 1) for l in (OR / "pins.env").read_text().splitlines() if "=" in l and not l.startswith("#"))
        self.assertRegex(pins["OPENROWS_NVB_COMMIT"], r"^[0-9a-f]{40}$")
        self.assertEqual(pins["OPENROWS_NVB_URL"], "https://github.com/NVIDIA/nvbandwidth.git")
        self.assertRegex(pins["OPENROWS_NVB_TAG"], r"^v\d+\.\d+$")
        self.assertFalse([k for k in pins if "LLAMA" in k])
        self.assertIn("PIN_LLAMA_COMMIT", (ROOT / "scripts/apps/pins.sh").read_text())
        for p in (OR / "pins.env", OR / "provision_lanes.sh", OR / "nvbandwidth_row.py", OR / "llama_vulkan_row.py", OR / "nvbandwidth_cases.tsv"):
            self.assertNotRegex(p.read_text(), r"\b[0-9a-f]{64}\b", p.name)       # no sha256 of a binary in git (owner rule 2026-11-17)

    def test_a_gate_bundle_never_downloads_the_model(self):
        self.assertNotIn("llama_model", re.search(r"^STEPS=.*$", (OR / "provision_lanes.sh").read_text(), re.M).group(0))
        self.assertNotIn("llama_model", (ROOT / "scripts/apps/build_bundle.sh").read_text())

    def test_no_file_of_the_lanes_pins_a_digest(self):
        for p in (OR / "pins.env", OR / "provision_lanes.sh", OR / "run_standalone.sh", OR / "nvbandwidth_row.py", OR / "llama_vulkan_row.py", OR / "nvbandwidth_cases.tsv"):
            self.assertNotRegex(p.read_text(), r"(?i)\b[0-9a-f]{64}\b|sha-?256\s*=", p.name)
        pins = (OR / "pins.env").read_text()
        self.assertNotRegex(pins, r"(?im)^OPENROWS_[A-Z_]*(SHA|HASH|DIGEST)")

    def test_a_failed_lane_step_prints_a_line_in_the_bundle_build_and_nothing_else(self):
        self.assertRegex((ROOT / "scripts/apps/build_bundle.sh").read_text(), r'provision_lanes\.sh" "\$B" \|\| say "open lanes')

    def test_each_row_regex_names_the_summary_the_row_prints_with_its_pass_status(self):
        run_apps = (ROOT / "scripts/apps/run_apps.sh").read_text()
        for name, tag, src in (("nvbandwidth", "NVBANDWIDTH_SUMMARY", "nvbandwidth_row.py"), ("llama_vulkan", "LLAMA_VULKAN_SUMMARY", "llama_vulkan_row.py")):
            rx = next(l for l in run_apps.splitlines() if l.startswith(name + "|")).split("|")[2]
            self.assertEqual(rx, tag + " status=PASS")
            self.assertIn('"%s status=%%s' % tag, (OR / src).read_text())

    def test_the_model_step_is_not_a_default_step_and_run_standalone_asks_for_it(self):
        prov = (OR / "provision_lanes.sh").read_text()
        self.assertIn("STEPS=${2:-nvbandwidth llama_vulkan}", prov)
        self.assertIn('pin_fetch_file "$PIN_GGUF_URL"', prov)
        self.assertIn("llama_model", (OR / "run_standalone.sh").read_text())

    def test_the_provision_script_is_glue_and_the_bundle_calls_it_without_being_able_to_fail(self):
        self.assertLess(len((OR / "provision_lanes.sh").read_text().splitlines()), 100)
        self.assertIn('open_rows/provision_lanes.sh" "$B" ||', (ROOT / "scripts/apps/build_bundle.sh").read_text())
        self.assertIn("provision_lanes.sh", (OR / "run_standalone.sh").read_text())

    def test_the_hook_pushes_the_vulkan_build_of_llama_bench_when_the_host_built_one(self):
        hook = (ROOT / "scripts/apps/apps_hook.sh").read_text()
        self.assertRegex(hook, r"\[ -d /workspace/apps/bundle/llama_vk \] && tar -C /workspace/apps/bundle -cf - llama_vk \| \$G")

    def test_the_hook_unpacks_into_the_bundle_of_the_guest(self):
        hook = (ROOT / "scripts/apps/apps_hook.sh").read_text()
        self.assertRegex(hook, r"llama_vk \| \$G 'sudo tar -C /opt/apps/bundle -xf -'")

    def test_the_matrix_table_names_the_rows(self):
        self.assertIn('"nvbandwidth":', (ROOT / "scripts/apps/matrix_table.py").read_text())
        self.assertIn('"llama_vulkan":', (ROOT / "scripts/apps/matrix_table.py").read_text())

    def test_compare_logs_knows_the_nvbandwidth_row(self):
        text = (OR / "compare_logs.py").read_text()
        self.assertIn('"nvbandwidth"', text)
        self.assertIn("nvbandwidth_row.cmd_compare", text)

    def test_no_environment_variable_is_read_as_user_config_by_the_new_rows(self):
        for name in ("nvbandwidth_row.py", "llama_vulkan_row.py"):
            text = (OR / name).read_text()
            self.assertNotIn("os.environ.get", text, name)
            self.assertNotIn("getenv", text, name)

    def test_the_run_standalone_script_runs_both_rows_without_a_new_variable(self):
        text = (OR / "run_standalone.sh").read_text()
        self.assertIn("nvbandwidth_row.py", text)
        self.assertIn("llama_vulkan_row.py", text)
        self.assertNotIn("KF_APPS_DATA", text)


if __name__ == "__main__":
    unittest.main()
