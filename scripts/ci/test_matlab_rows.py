#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""scripts/bench/linux/matlab_rows.py fixtures: no VM, no GPU, no network, no MATLAB. A fake guest transport, fixture result documents (good, a tolerance
miss, hostile garbage and oversize), fixture host GPU counters, and the repository's own MATLAB scripts read as text."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import types
import unittest

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "scripts/bench/linux/matlab_rows.py"
MATLAB_DIR = ROOT / "scripts/bench/linux/matlab"
PUB = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAItestkeytestkeytestkeytestkeytestkeytestkey kfimage-propimage"
SEED = "kayfabe-matlab-test-v1"


def load():
    spec = importlib.util.spec_from_file_location("fixture_matlab_rows", SOURCE)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


M = load()


# ---------------------------------------------------------------- fixture documents
def kind_of(name):
    if name.startswith(("mldivide_resid", "lu_resid", "qr_resid", "qr_orth", "chol_resid")):
        return "resid"
    if name in ("content_mismatches", "no_write_past_n"):
        return "count"
    if name in ("num_workers", "distinct_pids", "workers_with_a_device"):
        return "atleast"
    if name == "train_loss_curve":
        return "series"
    if name == "final_mse":
        return "pair"
    if name in ("kernel_output_gpuarray", "chunks_gpuarray", "train_env_gpu"):
        return "gpuarray"
    return "cmp"


def dtype_of(name):
    for dt in ("single", "double"):
        if "_%s_" % dt in name:
            return dt
    if name.startswith("sample_chunk"):
        return "uint32"
    if name.startswith("sum_ones"):
        return "double"
    return "single"


def item(name, **over):
    kind = kind_of(name)
    if kind == "cmp":
        it = {"name": name, "kind": "cmp", "dtype": dtype_of(name), "numel": 1000, "tol": 0, "gpu": [10.0, 5.0, 1.0, 2.0], "cpu": [10.0, 5.0, 1.0, 2.0],
              "diff": 0.0, "ref": 10.0, "guest_pass": True, "on_gpu": True, "exists_on_gpu": True}
    elif kind == "resid":
        it = {"name": name, "kind": "resid", "value": 1e-13, "tol": 1e-10, "guest_pass": True}
    elif kind == "count":
        it = {"name": name, "kind": "count", "value": 0, "expected": 0, "guest_pass": True}
    elif kind == "atleast":
        it = {"name": name, "kind": "atleast", "value": 2, "min": 2, "guest_pass": True}
    elif kind == "gpuarray":
        it = {"name": name, "kind": "gpuarray", "on_gpu": True, "exists_on_gpu": True, "underlying": "single", "device_index": 1, "device_name": "NVIDIA GeForce RTX 4090",
              "guest_pass": True}
    elif kind == "series":
        it = {"name": name, "kind": "series", "gpu": [1.0, 0.5, 0.2, 0.1], "cpu": [1.0, 0.5, 0.2, 0.1]}
    else:
        it = {"name": name, "kind": "pair", "gpu": 0.0100, "cpu": 0.0101}
    it.update(over)
    return it


DECLARED_FB_MB = 4096          # what the launch declares: propimage_linux.init_state's `fb_mb`, written by build_argv as the kf3 device's `fb-mb=`
MEASURED_TOTAL = 3968073728    # MEASURED 2026-11-18 (Vast RTX 4090, kf3): gpuDevice TotalMemory of a guest booted with fb-mb=4096 (3784 MiB: 92.4 percent of it)
INFO = {"name": "NVIDIA GeForce RTX 4090", "compute_capability": "8.9", "driver_version": 13.0, "toolkit_version": 12.9, "total_memory": MEASURED_TOTAL,
        "available_memory": 3_700_000_000, "multiprocessors": 128, "device_count": 1, "matlab_release": "2026b", "can_use_gpu": True}
MEM = {"total": 25757220864, "avail_start": 24000000000, "avail_after_alloc": 3000000000, "avail_end": 23900000000, "chunk_bytes": 134217728, "chunks": 152,
       "allocated_bytes": 152 * 134217728}
HW = {"gpu_name": "NVIDIA GeForce RTX 4090", "compute_cap": 890, "memory_mib": 24564}


def good(row):
    doc = {"schema": M.SCHEMA_ROW, "row": row, "status": "done", "checks": [item(n) for n in M.ROWS[row][3]],
           "gpu_device": {"name": "NVIDIA GeForce RTX 4090", "index": 1, "can_use_gpu": True}}
    if row == "gpu_info":
        doc["info"] = dict(INFO)
    if row == "mem_oversub":
        doc["memory"] = dict(MEM)
    if row == "dl_train":
        doc["epochs"] = 30
    return doc


def raw(doc):
    return json.dumps(doc).encode()


def set_check(doc, name, **over):
    for it in doc["checks"]:
        if it["name"] == name:
            it.update(over)
            return doc
    raise KeyError(name)


# ---------------------------------------------------------------- fixture host counters and the fake guest
def smi_csv(busy=True, mem_rise=True, n=12, na=False):
    """The host's nvidia-smi rows, the format gpu_assert reads: timestamp, gpu%, mem%, mem MiB, encoder sessions, encoder fps."""
    lines = []
    for i in range(n):
        gpu = ("N/A" if na else (60 if busy and 2 <= i < n - 1 else 0))
        mem = 1000 + (3000 if mem_rise and i >= 2 else 0)
        lines.append("2026/11/17 10:00:%02d.000, %s, 5, %d, 0, 0" % (i, gpu, mem))
    return "\n".join(lines) + "\n"


class FakeProbe:
    def __init__(self, text=None, starts=True):
        self.text, self.starts = smi_csv() if text is None else text, starts
        self.started = self.stopped = False

    def start(self):
        self.started = True
        return self.starts

    def stop(self):
        self.stopped = True
        return self.text, ""


def status_line(births, live=""):
    return ("kf3: family=Ampere phase=Running trapped=1 chan[births=%d pt_births=0 acts=1/0refused nsi=[] served=1 parks=1 host_rings=0 contended=0 poisoned=0 "
            "tokens=[%s]] unserviced=[]" % (births, live))


BASE_LINE = status_line(2)
WORK_LINES = ["kf3: chan token 0x801 (host 0x1001d) RETIRED, forwarded=40 submissions=411 splits=0/0 serves=9 last_put=Some(1) gp_get=Some(1) dead=None",
              status_line(8)]


class FakeCensus:
    """kf3_census with a canned QEMU log (the module is the real one; only the file reading is canned). The window holds GPU work and a baseline line
    precedes it unless `work=False` (a window where nothing was submitted) or `base=False` (no status line before the window)."""

    def __init__(self, lines=(), work=True, base=True):
        self.lines, self.real = list(lines) + (WORK_LINES if work else [status_line(8)]), M.census_mod()
        self.base_lines = [BASE_LINE] if base else ["kf3: booting"]

    def count_lines(self, path):
        return 3

    def window(self, path, lo, hi):
        return self.base_lines if hi is not None else self.lines

    def census(self, lines):
        return self.real.census(lines)


class FakeSession:
    """Plays a guest: `results[row]` is (rc, json bytes or None) or an exception to raise from exec."""

    def __init__(self, results, refuse=None, stages=None, index=1, log=None):
        self.results, self.refuse, self.index = results, refuse, index
        self.calls = log if log is not None else []
        self.last = None

    def qemu_log(self):
        return Path("/nonexistent/qemu.log")

    def boot(self):
        self.calls.append(("boot", self.index))
        if self.refuse and self.refuse[0] == "boot":
            raise self.refuse[1]

    def install_driver(self):
        self.calls.append(("install_driver", self.index))
        if self.refuse and self.refuse[0] == "driver":
            raise self.refuse[1]

    def inject(self, src):
        self.calls.append(("inject", Path(src)))
        if self.refuse and self.refuse[0] == "inject":
            raise self.refuse[1]

    def exec(self, command, timeout):
        row = re.search(r"/(\w+)\.m'\)", command).group(1)
        self.calls.append(("exec", row, timeout))
        res = self.results[row]
        if isinstance(res, BaseException):
            raise res
        self.last = res
        return 0, "garbage output\nKF_RC=%d\n" % res[0]

    def fetch(self, remote, max_bytes):
        self.calls.append(("fetch", remote, max_bytes))
        return self.last[1] if self.last else None

    def shutdown(self):
        self.calls.append(("shutdown", self.index))


def make_ctx(lanes=None, probes=None, census_lines=(), xid=None, **kw):
    probes = probes if probes is not None else {}
    made = []

    def factory(directory):
        p = probes.get(Path(directory).name) or FakeProbe()
        made.append(p)
        return p

    kw.setdefault("declared_fb_mb", DECLARED_FB_MB)
    ctx = M.Context(lanes=lanes, hw=dict(HW), probe_factory=factory, sleep=lambda s: None, census=FakeCensus(census_lines),
                    xid_mark=lambda: 0, xid_since=(lambda mark: xid) if xid is not None else (lambda mark: None), **kw)
    ctx.made = made
    return ctx


def all_good():
    return {r: (0, raw(good(r))) for r in M.ROW_ORDER}


def run(results, ctx=None, rows=None, refuse=None, calls=None):
    ctx = ctx or make_ctx()
    calls = calls if calls is not None else []
    rows = rows or [r for r in M.ROW_ORDER if r != "gpu_info"]
    out = M.run_rows(rows, lambda i: FakeSession(results, refuse=refuse, index=i, log=calls), ctx, log=lambda *_: None)
    return out, ctx, calls


def by_row(out):
    return {r["row"]: r for r in out["rows"]}


# ---------------------------------------------------------------- the hostile document
class ParseTests(unittest.TestCase):
    def test_good_documents_pass_for_every_row(self):
        for row in M.ROW_ORDER:
            v = M.verify_doc(row, M.parse_row_json(raw(good(row)), row), HW, DECLARED_FB_MB)
            self.assertTrue(v["ok"], (row, v))
            self.assertEqual(v["n_checks"], len(M.ROWS[row][3]))

    def test_oversized_json_is_refused_before_parsing(self):
        big = b'{"schema":"%s","row":"gpu_matmul","status":"done","checks":[],"pad":"%s"}' % (M.SCHEMA_ROW.encode(), b"x" * M.MAX_JSON_BYTES)
        with self.assertRaises(M.BadResult) as c:
            M.parse_row_json(big, "gpu_matmul")
        self.assertEqual(c.exception.reason, "TOO_LARGE")

    def test_garbage_and_empty_are_refused(self):
        for blob, reason in ((b"", "EMPTY"), (b"\x00\x01garbage", "NOT_JSON"), (b"\xff\xfe", "NOT_JSON"), (b"[1,2]", "NOT_OBJECT"), (b"{", "NOT_JSON")):
            with self.assertRaises(M.BadResult) as c:
                M.parse_row_json(blob, "gpu_matmul")
            self.assertEqual(c.exception.reason, reason, blob)

    def test_nan_infinity_and_duplicate_keys_are_refused(self):
        for text in ('{"schema":"%s","row":"gpu_matmul","status":"done","checks":[],"x":NaN}' % M.SCHEMA_ROW,
                     '{"schema":"%s","row":"gpu_matmul","status":"done","checks":[],"x":Infinity}' % M.SCHEMA_ROW,
                     '{"schema":"%s","row":"gpu_matmul","status":"done","status":"done","checks":[]}' % M.SCHEMA_ROW):
            with self.assertRaises(M.BadResult):
                M.parse_row_json(text.encode(), "gpu_matmul")

    def test_deep_nesting_and_long_lists_are_refused(self):
        deep = {"schema": M.SCHEMA_ROW, "row": "gpu_matmul", "status": "done", "checks": [], "x": 1}
        node = deep
        for _ in range(12):
            node["x"] = {"x": 1}
            node = node["x"]
        with self.assertRaises(M.BadResult):
            M.parse_row_json(raw(deep), "gpu_matmul")
        longer = good("gpu_matmul")
        longer["pad"] = list(range(M.MAX_LIST + 1))
        with self.assertRaises(M.BadResult):
            M.parse_row_json(raw(longer), "gpu_matmul")

    def test_wrong_schema_row_name_status_and_check_count(self):
        d = good("gpu_matmul")
        for edit in ({"schema": "other"}, {"row": "gpu_fft"}, {"status": "weird"}, {"checks": "no"}, {"checks": [item("matmul_single_256")] * (M.MAX_CHECKS + 1)}):
            with self.assertRaises(M.BadResult):
                M.parse_row_json(raw(dict(d, **edit)), "gpu_matmul")

    def test_numbers_that_are_not_numbers(self):
        self.assertIsNone(M.num(True))
        self.assertIsNone(M.num("1"))
        self.assertIsNone(M.num(10 ** 400))
        self.assertIsNone(M.num(float("nan")))
        self.assertIsNone(M.num(1e301))
        self.assertEqual(M.num(3), 3.0)

    def test_hostile_strings_are_cleaned_and_bounded(self):
        self.assertEqual(M.clean("a\x00b\n\xe9" + "z" * 500, 20), "a?b??" + "z" * 15)
        d = good("gpu_matmul")
        d["checks"].append({"name": "x" * 100, "kind": "cmp"})
        v = M.verify_doc("gpu_matmul", M.parse_row_json(raw(d), "gpu_matmul"), HW)
        self.assertFalse(v["ok"])
        self.assertEqual(v["klass"], "BAD_RESULT")


class NumbersTests(unittest.TestCase):
    def verify(self, row, doc):
        return M.verify_doc(row, M.parse_row_json(raw(doc), row), HW, DECLARED_FB_MB)

    def test_tolerance_miss_is_output_mismatch(self):
        d = set_check(good("gpu_matmul"), "matmul_single_512", diff=0.01, ref=10.0)   # relative 1e-3 against 1e-4
        v = self.verify("gpu_matmul", d)
        self.assertFalse(v["ok"])
        self.assertEqual((v["klass"], v["failed"]), ("OUTPUT_MISMATCH", ["matmul_single_512"]))

    def test_the_same_miss_passes_in_double_only_below_its_own_tolerance(self):
        d = set_check(good("gpu_matmul"), "matmul_double_512", diff=1e-12, ref=10.0)
        self.assertTrue(self.verify("gpu_matmul", d)["ok"])
        d = set_check(good("gpu_matmul"), "matmul_double_512", diff=1e-8, ref=10.0)
        self.assertFalse(self.verify("gpu_matmul", d)["ok"])

    def test_exact_checks_allow_no_difference(self):
        d = set_check(good("cuda_kernel"), "vecadd_1000", diff=1e-30, ref=10.0)
        v = self.verify("cuda_kernel", d)
        self.assertEqual((v["ok"], v["failed"]), (False, ["vecadd_1000"]))

    def test_guest_cannot_loosen_its_own_tolerance(self):
        d = set_check(good("gpu_matmul"), "matmul_single_512", diff=5.0, ref=10.0, tol=1.0, guest_pass=True)
        v = self.verify("gpu_matmul", d)
        self.assertFalse(v["ok"])
        self.assertEqual(v["flag_disagrees"], 1)

    def test_agreement_flag_is_recomputed_from_the_raw_numbers(self):
        d = set_check(good("gpu_matmul"), "matmul_single_512", guest_pass=False)      # numbers are fine, the guest's flag says no
        v = self.verify("gpu_matmul", d)
        self.assertTrue(v["ok"])
        self.assertEqual(v["flag_disagrees"], 1)

    def test_fingerprint_disagreement_alone_fails(self):
        d = set_check(good("gpu_matmul"), "matmul_single_512", gpu=[10.0, 5.5, 1.0, 2.0])   # the difference norm claims agreement, the sums do not
        self.assertFalse(self.verify("gpu_matmul", d)["ok"])

    def test_zero_reference_uses_the_absolute_difference(self):
        d = set_check(good("gpu_matmul"), "matmul_single_512", ref=0.0, diff=0.5, cpu=[0.0] * 4, gpu=[0.0] * 4)
        self.assertFalse(self.verify("gpu_matmul", d)["ok"])

    def test_residual_precision_comes_from_the_name_not_the_guest(self):
        d = set_check(good("gpu_solve"), "lu_resid_gpu_single_512", value=5e-4, dtype="double", tol=1e-3)
        self.assertEqual(self.verify("gpu_solve", d)["failed"], ["lu_resid_gpu_single_512"])
        d = set_check(good("gpu_solve"), "lu_resid_gpu_double_512", value=5e-4)
        self.assertEqual(self.verify("gpu_solve", d)["failed"], ["lu_resid_gpu_double_512"])

    def test_counts_must_be_zero_whatever_the_guest_expected(self):
        d = set_check(good("mem_oversub"), "content_mismatches", value=3, expected=3, guest_pass=True)
        self.assertEqual(self.verify("mem_oversub", d)["failed"], ["content_mismatches"])

    def test_worker_minimums_come_from_the_host(self):
        d = set_check(good("parpool_gpu"), "num_workers", value=1, min=0)
        self.assertEqual(self.verify("parpool_gpu", d)["failed"], ["num_workers"])

    def test_loss_must_fall_on_both_devices(self):
        d = set_check(good("dl_train"), "train_loss_curve", gpu=[1.0, 0.9, 0.8, 0.9])
        self.assertEqual(self.verify("dl_train", d)["failed"], ["train_loss_curve"])
        d = set_check(good("dl_train"), "train_loss_curve", cpu=[1.0, 1.0, 1.0, 1.0])
        self.assertEqual(self.verify("dl_train", d)["failed"], ["train_loss_curve"])

    def test_final_losses_must_agree(self):
        d = set_check(good("dl_train"), "final_mse", gpu=0.01, cpu=0.5)
        self.assertEqual(self.verify("dl_train", d)["failed"], ["final_mse"])

    def test_missing_required_check_is_a_bad_result(self):
        d = good("gpu_fft")
        d["checks"] = d["checks"][:-1]
        v = self.verify("gpu_fft", d)
        self.assertEqual(v["klass"], "BAD_RESULT")
        self.assertTrue(v["findings"][0].startswith("MISSING_CHECK:"))

    def test_duplicate_unknown_kind_and_bad_names(self):
        d = good("gpu_matmul")
        d["checks"].append(dict(d["checks"][0]))
        self.assertEqual(self.verify("gpu_matmul", d)["klass"], "BAD_RESULT")
        d = good("gpu_matmul")
        d["checks"][0]["kind"] = ["cmp"]
        self.assertEqual(self.verify("gpu_matmul", d)["klass"], "BAD_RESULT")
        d = good("gpu_matmul")
        d["checks"][0]["dtype"] = ["single"]
        self.assertFalse(self.verify("gpu_matmul", d)["ok"])

    def test_error_document_keeps_only_the_error_id(self):
        d = {"schema": M.SCHEMA_ROW, "row": "gpu_fft", "status": "error", "error_id": "parallel:gpu:device:NoDriver", "checks": [], "message": "SECRET text"}
        v = self.verify("gpu_fft", d)
        self.assertEqual((v["ok"], v["klass"]), (False, "ASSERT"))
        self.assertEqual(v["findings"], ["ERROR_ID:parallel:gpu:device:NoDriver"])
        d["error_id"] = "has spaces and \x00 junk"
        self.assertEqual(self.verify("gpu_fft", d)["findings"], ["ERROR_ID:?"])

    def test_gpu_info_checks_the_host_facts(self):
        d = good("gpu_info")
        d["info"]["name"] = "NVIDIA GeForce RTX 3060"
        v = self.verify("gpu_info", d)
        self.assertEqual(v["klass"], "OUTPUT_MISMATCH")
        d = good("gpu_info")
        d["info"]["total_memory"] = 2 * DECLARED_FB_MB * 2 ** 20           # twice what the launch declared
        self.assertEqual(self.verify("gpu_info", d)["klass"], "OUTPUT_MISMATCH")
        d = good("gpu_info")
        d["info"]["can_use_gpu"] = False
        self.assertEqual(self.verify("gpu_info", d)["klass"], "BAD_RESULT")
        d = good("gpu_info")
        d["info"]["compute_capability"] = "eight"
        self.assertEqual(self.verify("gpu_info", d)["klass"], "BAD_RESULT")

    def test_gpu_info_facts_carry_no_account_data(self):
        facts = self.verify("gpu_info", good("gpu_info"))["facts"]
        self.assertEqual(sorted(facts), ["compute_capability", "declared_fb_mb", "device_count", "driver_version", "matlab_release", "name", "toolkit_version", "total_memory"])
        self.assertEqual(facts["declared_fb_mb"], DECLARED_FB_MB)

    def test_compute_capability_gate(self):
        self.assertEqual(M.cc_x100({"compute_capability": "8.9"}), 890)
        self.assertEqual(M.cc_x100({"compute_capability": "7.5"}), 750)
        self.assertEqual(M.cc_x100({"compute_capability": "12.0"}), 1200)
        self.assertEqual(M.cc_x100({"compute_capability": "x"}), 0)

    def test_memory_row_bounds(self):
        for edit, klass in (({"allocated_bytes": 1}, "BAD_RESULT"), ({"chunks": 10, "allocated_bytes": 10 * 134217728}, "OUTPUT_MISMATCH"),
                            ({"avail_end": 1000}, "OUTPUT_MISMATCH"), ({"avail_after_alloc": 23000000000}, "OUTPUT_MISMATCH"), ({"avail_start": "x"}, "BAD_RESULT")):
            d = good("mem_oversub")
            d["memory"].update(edit)
            self.assertEqual(self.verify("mem_oversub", d)["klass"], klass, edit)


# ---------------------------------------------------------------- the preamble's total memory is judged against the DECLARED fb-mb
class DeclaredMemoryTests(unittest.TestCase):
    """2026-11-18, MEASURED on a Vast RTX 4090 under kf3: the preamble failed `TOTAL_MEMORY_OUT_OF_RANGE guest=3968073728 host_mib=24564` and no other row ran.
    The guest reports the VRAM carve-out the LAUNCH declares (the kf3 property `fb-mb`, 4096 MiB), never the host's 24 GB, so the host's memory is no limit."""

    def info(self, total, declared=DECLARED_FB_MB, hw=HW):
        d = good("gpu_info")
        d["info"]["total_memory"] = total
        d["info"]["available_memory"] = min(d["info"]["available_memory"], total)           # the document stays consistent (free is never above total)
        return M.verify_doc("gpu_info", M.parse_row_json(raw(d), "gpu_info"), hw, declared)

    @staticmethod
    def bytes_of(mib):
        return mib * 2 ** 20

    def test_the_measured_report_of_the_4090_run_passes_against_the_declared_4096(self):
        v = self.info(MEASURED_TOTAL)
        self.assertEqual((v["ok"], v["klass"], v["findings"]), (True, None, []))
        self.assertEqual(v["facts"]["declared_fb_mb"], 4096)
        self.assertEqual(v["facts"]["total_memory"], MEASURED_TOTAL)

    def test_the_hosts_memory_is_never_a_limit(self):
        for hw_mib in (24564, 8192, 4096, 100, None):
            hw = dict(HW, memory_mib=hw_mib)
            self.assertTrue(self.info(MEASURED_TOTAL, hw=hw)["ok"], hw_mib)
        self.assertTrue(self.info(MEASURED_TOTAL, hw={})["ok"])                      # no host facts at all: the declared bound still applies
        v = self.info(self.bytes_of(HW["memory_mib"]))                              # a guest that reports the HOST's total sees more than the launch declared
        self.assertEqual((v["ok"], v["klass"]), (False, "OUTPUT_MISMATCH"))
        self.assertTrue(v["findings"][0].startswith("TOTAL_MEMORY_OVER_DECLARED"), v["findings"])
        self.assertFalse(self.info(self.bytes_of(1024), hw={})["ok"])               # and the bound holds without host facts

    def test_zero_and_tiny_reports_fail(self):
        for total in (0, 1, 4096, self.bytes_of(1), self.bytes_of(255)):
            v = self.info(total)
            self.assertFalse(v["ok"], total)
            self.assertIn(v["klass"], ("BAD_RESULT", "OUTPUT_MISMATCH"))
        v = self.info(self.bytes_of(255))
        self.assertTrue(v["findings"][0].startswith("TOTAL_MEMORY_BELOW_FLOOR"), v["findings"])

    def test_a_gib_of_a_declared_4096_fails_below_the_floor(self):
        v = self.info(self.bytes_of(1024))
        self.assertEqual((v["ok"], v["klass"]), (False, "OUTPUT_MISMATCH"))
        self.assertTrue(v["findings"][0].startswith("TOTAL_MEMORY_BELOW_FLOOR"), v["findings"])
        self.assertIn("declared_fb_mb=4096", v["findings"][0])

    def test_more_than_declared_fails_by_its_own_name(self):
        v = self.info(5 * 2 ** 30)
        self.assertEqual((v["ok"], v["klass"]), (False, "OUTPUT_MISMATCH"))
        self.assertTrue(v["findings"][0].startswith("TOTAL_MEMORY_OVER_DECLARED"), v["findings"])
        self.assertTrue(self.info(self.bytes_of(DECLARED_FB_MB))["ok"])             # exactly the declared size is the upper edge and passes
        v = self.info(self.bytes_of(DECLARED_FB_MB) + 1)
        self.assertEqual((v["ok"], v["findings"][0].split()[0]), (False, "TOTAL_MEMORY_OVER_DECLARED"))

    def test_between_the_floor_and_the_tolerance_is_out_of_range(self):
        declared = self.bytes_of(DECLARED_FB_MB)
        low = -(-int(M.GPU_INFO_TOTAL_FRACTION[0] * declared * 1000) // 1000)        # the smallest passing integer, rounded up
        self.assertTrue(self.info(low)["ok"], low)
        for total in (low - 1, int(0.60 * declared), int(0.51 * declared)):
            v = self.info(total)
            self.assertEqual((v["ok"], v["klass"], v["findings"][0].split()[0]), (False, "OUTPUT_MISMATCH", "TOTAL_MEMORY_OUT_OF_RANGE"), total)
        v = self.info(int(0.49 * declared))
        self.assertEqual(v["findings"][0].split()[0], "TOTAL_MEMORY_BELOW_FLOOR")   # under half of it: the hard floor, a different defect than a carve-out

    def test_the_tolerance_is_the_documented_one_and_holds_the_measured_value(self):
        lo, hi = M.GPU_INFO_TOTAL_FRACTION
        self.assertEqual((lo, hi), (0.80, 1.00))
        self.assertLess(M.GPU_INFO_FLOOR_FRACTION, lo)
        self.assertEqual(M.GPU_INFO_FLOOR_BYTES, 256 * 2 ** 20)
        self.assertTrue(lo <= MEASURED_TOTAL / self.bytes_of(DECLARED_FB_MB) <= hi)
        self.assertAlmostEqual(MEASURED_TOTAL / self.bytes_of(DECLARED_FB_MB), 0.924, places=3)      # 3784 of 4096 MiB

    def test_a_declared_8192_judges_the_same_way(self):
        self.assertTrue(self.info(7_700_000_000, declared=8192)["ok"])
        v = self.info(MEASURED_TOTAL, declared=8192)                                # the 4096 report is under half of 8192: the launch declared more than the guest saw
        self.assertEqual((v["ok"], v["findings"][0].split()[0]), (False, "TOTAL_MEMORY_BELOW_FLOOR"))
        v = self.info(7_700_000_000, declared=4096)
        self.assertEqual((v["ok"], v["findings"][0].split()[0]), (False, "TOTAL_MEMORY_OVER_DECLARED"))

    def test_a_declared_value_that_is_missing_is_a_harness_matter_never_a_pass(self):
        for declared in (None, 0, -4096, "4096", 4096.0, True, [4096]):
            v = self.info(MEASURED_TOTAL, declared=declared)
            self.assertEqual((v["ok"], v["klass"]), (False, "HARNESS"), declared)
            self.assertEqual(v["findings"], ["DECLARED_FB_MB_MISSING"])
            self.assertIsNone(v["facts"]["declared_fb_mb"])

    def test_a_bad_info_block_still_outranks_a_missing_declared_value(self):
        d = good("gpu_info")
        d["info"]["can_use_gpu"] = False
        v = M.verify_doc("gpu_info", M.parse_row_json(raw(d), "gpu_info"), HW, None)
        self.assertEqual(v["klass"], "BAD_RESULT")
        d = good("gpu_info")
        d["info"]["name"] = "NVIDIA GeForce RTX 3060"
        v = M.verify_doc("gpu_info", M.parse_row_json(raw(d), "gpu_info"), HW, None)
        self.assertEqual(v["klass"], "OUTPUT_MISMATCH")                              # a real failure is reported as one even when the memory could not be judged

    def test_the_other_rows_do_not_need_the_declared_value(self):
        for row in M.ROW_ORDER:
            if row != "gpu_info":
                self.assertTrue(M.verify_doc(row, M.parse_row_json(raw(good(row)), row), HW, None)["ok"], row)

    def test_the_preamble_still_checks_everything_else(self):
        d = good("gpu_info")
        d["info"]["name"] = "NVIDIA GeForce RTX 3060"
        v = M.verify_doc("gpu_info", M.parse_row_json(raw(d), "gpu_info"), HW, DECLARED_FB_MB)
        self.assertEqual((v["klass"], v["findings"][0].split()[0]), ("OUTPUT_MISMATCH", "GPU_NAME_DIFFERS"))
        d = good("gpu_info")
        d["info"]["available_memory"] = d["info"]["total_memory"] + 1
        self.assertEqual(M.verify_doc("gpu_info", M.parse_row_json(raw(d), "gpu_info"), HW, DECLARED_FB_MB)["klass"], "BAD_RESULT")

    def test_the_judge_maps_a_harness_klass_to_harness_not_a_failure(self):
        verified = self.info(MEASURED_TOTAL, declared=None)
        verdict = M.judge("gpu_info", {"rc": 0, "timed_out": False, "lost": False, "secs": 1}, {"status": "done"}, verified, None, {}, None)
        self.assertEqual(verdict, ("HARNESS", None, "DECLARED_FB_MB_MISSING"))
        failing = M.judge("gpu_info", {"rc": 0, "timed_out": False, "lost": False, "secs": 1}, {"status": "done"}, self.info(5 * 2 ** 30), None, {}, None)
        self.assertEqual(failing[:2], ("FAIL", "OUTPUT_MISMATCH"))

    # ---- at run level: the context carries the declared value to the judge
    def test_a_good_preamble_passes_the_run_with_the_declared_value_in_its_facts(self):
        out, ctx, _ = run(all_good())
        rec = by_row(out)["gpu_info"]
        self.assertEqual((rec["verdict"], rec["facts"]["declared_fb_mb"]), ("PASS", 4096))
        self.assertEqual(M.grade(out["rows"], True)[2], 0)

    def test_the_run_judges_the_preamble_against_the_context_value_not_a_constant(self):
        results = all_good()
        results["gpu_info"] = (0, raw(dict(good("gpu_info"), info=dict(INFO, total_memory=7_700_000_000))))
        out, _, _ = run(results, ctx=make_ctx(declared_fb_mb=8192))
        self.assertEqual(by_row(out)["gpu_info"]["verdict"], "PASS")
        out, _, _ = run(results, ctx=make_ctx(declared_fb_mb=4096))
        rec = by_row(out)["gpu_info"]
        self.assertEqual((rec["verdict"], rec["failure_class"]), ("FAIL", "OUTPUT_MISMATCH"))
        self.assertTrue(rec["reason"].startswith("TOTAL_MEMORY_OVER_DECLARED"), rec["reason"])

    def test_a_guest_above_the_declared_size_fails_the_preamble_and_nothing_else_runs(self):
        results = all_good()
        results["gpu_info"] = (0, raw(dict(good("gpu_info"), info=dict(INFO, total_memory=5 * 2 ** 30))))
        out, _, calls = run(results)
        rows = by_row(out)
        self.assertEqual((rows["gpu_info"]["verdict"], rows["gpu_info"]["failure_class"]), ("FAIL", "OUTPUT_MISMATCH"))
        self.assertEqual({r["reason"] for k, r in rows.items() if k != "gpu_info"}, {"GPU_INFO_FAILED"})
        self.assertEqual([c[1] for c in calls if c[0] == "exec"], ["gpu_info"])

    def test_no_declared_value_in_the_context_is_a_harness_retry_never_a_pass(self):
        out, _, calls = run(all_good(), ctx=make_ctx(declared_fb_mb=None))
        rows = by_row(out)
        self.assertEqual((rows["gpu_info"]["verdict"], rows["gpu_info"]["failure_class"], rows["gpu_info"]["reason"]), ("HARNESS", None, "DECLARED_FB_MB_MISSING"))
        self.assertEqual({r["reason"] for k, r in rows.items() if k != "gpu_info"}, {"GPU_INFO_FAILED"})
        self.assertEqual(M.grade(out["rows"], True)[:3], ("RETRY", "HARNESS:gpu_info", 3))

    def test_the_context_defaults_to_no_declared_value(self):
        self.assertIsNone(M.Context().declared_fb_mb)
        self.assertEqual(M.Context(declared_fb_mb=4096).declared_fb_mb, 4096)

    # ---- the one source of truth: the state the boot is launched from
    def execute(self, state, results, rows=("gpu_matmul",)):
        from unittest import mock
        with tempfile.TemporaryDirectory(prefix="matlab-rows-exec-") as t:
            img, drv = Path(t) / "i.zip", Path(t) / "NVIDIA-Linux-x86_64-580.126.09.run"
            img.write_bytes(b"x")
            drv.write_bytes(b"x")
            ctx = make_ctx(declared_fb_mb=1)                      # a stale or foreign value: execute() must replace it with the state's
            sessions = []

            def make(i):
                sessions.append(FakeSession(results, index=i))
                return sessions[-1]

            with mock.patch.object(M, "preflight_refusal", lambda *a, **k: None), mock.patch.object(M, "prepare_dir", lambda *a, **k: (state, {})):
                rec = M.execute(str(img), None, t, "/q", str(drv), "0" * 64, list(rows), ctx.lanes, ctx, "0" * 40, make_session=make)
        return rec, ctx, sessions

    def state(self, fb_mb=None):
        st = M.prop().init_state(SEED, ram_mb=4096, smp=2, disk_gb=30, keypair=("PRIVATE", PUB))
        if fb_mb is not None:
            st["qemu"]["fb_mb"] = fb_mb
        return st

    def test_execute_takes_the_declared_value_from_the_image_state_the_boot_uses(self):
        rec, ctx, sessions = self.execute(self.state(), all_good())
        self.assertEqual(ctx.declared_fb_mb, 4096)
        self.assertEqual(ctx.declared_fb_mb, M.prop().declared_fb_mb(self.state()))
        self.assertEqual(rec["rows"][0]["facts"]["declared_fb_mb"], 4096)
        self.assertEqual(rec["rows"][0]["verdict"], "PASS")
        rec, ctx, sessions = self.execute(self.state(8192), all_good())
        self.assertEqual((ctx.declared_fb_mb, rec["rows"][0]["verdict"], rec["rows"][0]["failure_class"]), (8192, "FAIL", "OUTPUT_MISMATCH"))   # 3.97e9 is under half of 8192 MiB

    def test_execute_refuses_a_state_that_declares_nothing_before_any_boot(self):
        for broken in ({}, {"qemu": {}}, {"qemu": {"fb_mb": 0}}, {"qemu": {"fb_mb": "4096"}}, {"qemu": {"fb_mb": None}}):
            rec, ctx, sessions = self.execute(broken, all_good())
            self.assertEqual(sessions, [], broken)
            self.assertEqual({(r["verdict"], r["reason"]) for r in rec["rows"]}, {("HARNESS", "DECLARED_FB_MB_MISSING")}, broken)
            self.assertEqual((rec["verdict"], rec["exit_code"]), ("RETRY", 3))
            self.assertIsNone(ctx.declared_fb_mb)
            self.assertEqual([r["row"] for r in rec["rows"]], ["gpu_info", "gpu_matmul"])

    def test_the_boot_argv_carries_the_value_the_judge_uses(self):
        for fb_mb in (None, 8192, 6144):
            s, calls, popens, p = make_session(self)
            if fb_mb is not None:
                s.st["qemu"]["fb_mb"] = fb_mb
            s.boot()
            device = next(a for a in popens[0] if a.startswith("kf3-gpu,"))
            self.assertIn(",fb-mb=%d," % M.prop().declared_fb_mb(s.st), device + ",")
            self.assertEqual(int(re.search(r"fb-mb=(\d+)", device).group(1)), fb_mb or 4096)

    def test_verify_cli_takes_the_declared_value_and_no_host_memory(self):
        with tempfile.TemporaryDirectory(prefix="matlab-rows-cli-") as t:
            f = Path(t) / "r.json"
            f.write_bytes(raw(good("gpu_info")))
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                code = M.main(["verify", str(f), "--row", "gpu_info", "--host-gpu-name", "NVIDIA GeForce RTX 4090", "--declared-fb-mb", "4096"])
            self.assertEqual((code, buf.getvalue().split()[0]), (0, "OK"))
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                code = M.main(["verify", str(f), "--row", "gpu_info", "--declared-fb-mb", "8192"])
            self.assertEqual(code, 1)
            self.assertIn("TOTAL_MEMORY_BELOW_FLOOR", buf.getvalue())
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                code = M.main(["verify", str(f), "--row", "gpu_info"])
            self.assertEqual(code, 1)
            self.assertIn("DECLARED_FB_MB_MISSING", buf.getvalue())
            with self.assertRaises(SystemExit), contextlib.redirect_stderr(io.StringIO()):
                M.main(["verify", str(f), "--row", "gpu_info", "--host-mem-mib", "24564"])         # the host's memory is not an input any more


# ---------------------------------------------------------------- GPU use from the host counters, the verdict order
class GpuUseTests(unittest.TestCase):
    def test_busy_gpu_and_rising_memory_pass(self):
        rec = M.assess_gpu_use("gpu_matmul", smi_csv(), "")
        self.assertEqual((rec["verdict"], rec["source"], rec["unreadable"]), ("PASS", "host", False))

    def test_idle_gpu_is_not_used(self):
        rec = M.assess_gpu_use("gpu_matmul", smi_csv(busy=False), "")
        self.assertEqual(rec["verdict"], "FAIL")
        self.assertIn("idle", rec["reasons"][0])

    def test_memory_that_never_rises_is_not_used(self):
        rec = M.assess_gpu_use("mem_oversub", smi_csv(mem_rise=False), "")
        self.assertEqual(rec["verdict"], "FAIL")

    def test_no_samples_is_unreadable_not_a_failure_of_the_guest(self):
        for text in ("", "garbage\n"):
            rec = M.assess_gpu_use("gpu_matmul", text, "")
            self.assertTrue(rec["unreadable"])

    def test_rows_without_a_requirement_need_no_counters(self):
        self.assertFalse(M.assess_gpu_use("gpu_info", "", "")["unreadable"])

    def test_required_engines_per_row(self):
        self.assertEqual(M.row_require("cuda_kernel"), ("gpu",))
        self.assertEqual(M.row_require("mem_oversub"), ("mem",))
        self.assertEqual(M.row_require("gpu_info"), ())
        self.assertTrue(all(set(M.row_require(r)) <= {"gpu", "mem"} for r in M.ROW_ORDER))

    def test_all_rows_discovery_by_default(self):
        out, ctx, _ = run(all_good())
        self.assertEqual({r["lane"] for r in out["rows"]}, {"discovery"})


class JudgeTests(unittest.TestCase):
    OK = {"ok": True, "klass": None, "findings": [], "n_checks": 1, "failed": [], "flag_disagrees": 0, "facts": {}}
    USED = {"verdict": "PASS", "unreadable": False, "reasons": []}

    WORK = {"verdict": "PASS", "why": "host census: +411 submission(s)"}

    def j(self, row="gpu_matmul", rc=0, timed_out=False, lost=False, parsed=True, verified=OK, gpu=USED, census=None, xid=None, work=WORK):
        return M.judge(row, {"rc": rc, "timed_out": timed_out, "lost": lost}, {"x": 1} if parsed else None, verified, gpu, census or {}, xid, work)

    def test_pass(self):
        self.assertEqual(self.j(), ("PASS", None, ""))

    def test_the_host_census_saying_no_work_fails_a_row_whose_guest_numbers_and_engines_are_fine(self):
        no_work = {"verdict": "FAIL", "why": "host census: no GPU work in the window"}
        self.assertEqual(self.j(work=no_work)[:2], ("FAIL", "GPU_NOT_USED"))
        self.assertEqual(self.j(work=no_work, gpu=None)[:2], ("FAIL", "GPU_NOT_USED"))       # the census is the authority even when the sampler is unreadable

    def test_a_census_that_could_not_be_read_is_harness_never_a_pass(self):
        for work in (None, {"verdict": "NOT_RUN", "why": "no status line"}):
            self.assertEqual(self.j(work=work), ("HARNESS", None, "HOST_CENSUS_MISSING"))

    def test_every_row_needs_the_census_even_the_preamble_and_the_memory_row(self):
        for row in ("gpu_info", "mem_oversub"):
            self.assertEqual(self.j(row=row, work=None, gpu={"verdict": "PASS", "unreadable": False, "reasons": []})[0], "HARNESS")

    def test_lost_guest_is_a_crash_before_anything_else(self):
        self.assertEqual(self.j(lost=True, parsed=False, verified=None)[:2], ("FAIL", "CRASH"))

    def test_timeout_is_a_hang_and_beats_the_numbers(self):
        self.assertEqual(self.j(timed_out=True)[:2], ("FAIL", "HANG"))

    def test_host_xid_fails_a_row_whose_numbers_are_fine(self):
        self.assertEqual(self.j(xid=1)[:2], ("FAIL", "XID"))
        self.assertEqual(self.j(census={"xid": 2})[:2], ("FAIL", "XID"))

    def test_no_result_file_after_a_nonzero_exit_is_a_crash_and_after_zero_a_bad_result(self):
        self.assertEqual(self.j(rc=139, parsed=False, verified=None)[:2], ("FAIL", "CRASH"))
        self.assertEqual(self.j(rc=0, parsed=False, verified=None)[:2], ("FAIL", "BAD_RESULT"))

    def test_numbers_before_gpu_use(self):
        bad = dict(self.OK, ok=False, klass="OUTPUT_MISMATCH", findings=["x: rel=1"])
        self.assertEqual(self.j(verified=bad, gpu={"verdict": "FAIL", "unreadable": False, "reasons": ["idle"]})[:2], ("FAIL", "OUTPUT_MISMATCH"))

    def test_exit_status_zero_alone_is_never_a_pass_and_nonzero_with_good_numbers_is_not_either(self):
        self.assertEqual(self.j(rc=1)[:2], ("FAIL", "ASSERT"))

    def test_unreadable_counters_are_harness_never_pass(self):
        self.assertEqual(self.j(gpu=None), ("HARNESS", None, "HOST_COUNTERS_MISSING"))
        self.assertEqual(self.j(gpu={"verdict": "FAIL", "unreadable": True, "reasons": []})[0], "HARNESS")

    def test_idle_gpu_is_a_failure_class(self):
        self.assertEqual(self.j(gpu={"verdict": "FAIL", "unreadable": False, "reasons": ["gpu engine idle"]})[:2], ("FAIL", "GPU_NOT_USED"))


# ---------------------------------------------------------------- the run, with a fake guest
class RunTests(unittest.TestCase):
    def test_every_row_passes_in_the_discovery_lane(self):
        out, ctx, calls = run(all_good())
        self.assertEqual(len(out["rows"]), 8)
        self.assertTrue(all(r["verdict"] == "PASS" for r in out["rows"]), [(r["row"], r["verdict"], r["reason"]) for r in out["rows"]])
        rec = M.build_record(out, ctx, "a" * 40, {}, {})
        self.assertEqual((rec["verdict"], rec["exit_code"], rec["breakages"]), ("PASS", 0, []))

    def test_each_row_boots_alone_installs_the_driver_then_injects_then_runs(self):
        out, ctx, calls = run(all_good())
        boots = [c for c in calls if c[0] == "boot"]
        self.assertEqual(len(boots), 8)
        first = [c[0] for c in calls[:6]]
        self.assertEqual(first, ["boot", "install_driver", "inject", "exec", "fetch", "shutdown"])
        self.assertEqual(calls[3][1], "gpu_info")

    def test_the_injected_directory_is_the_repository_scripts_directory(self):
        out, ctx, calls = run(all_good())
        self.assertEqual({c[1] for c in calls if c[0] == "inject"}, {M.SCRIPTS_DIR})
        self.assertEqual(M.SCRIPTS_DIR, ROOT / "scripts/bench/linux/matlab")

    def test_shared_isolation_uses_one_boot(self):
        out, ctx, calls = run(all_good(), ctx=make_ctx(isolation="shared"))
        self.assertEqual(len([c for c in calls if c[0] == "boot"]), 1)
        self.assertEqual(len(out["rows"]), 8)

    def test_a_crash_in_a_shared_boot_sends_the_rest_to_a_fresh_boot(self):
        results = all_good()
        results["gpu_fft"] = (139, None)
        out, ctx, calls = run(results, ctx=make_ctx(isolation="shared"))
        r = by_row(out)
        self.assertEqual((r["gpu_fft"]["verdict"], r["gpu_fft"]["failure_class"]), ("FAIL", "CRASH"))
        self.assertEqual(r["gpu_solve"]["verdict"], "PASS")
        self.assertEqual(len([c for c in calls if c[0] == "boot"]), 2)

    def test_tolerance_miss_is_a_discovery_breakage_that_does_not_fail_the_run(self):
        results = all_good()
        results["gpu_matmul"] = (0, raw(set_check(good("gpu_matmul"), "matmul_single_512", diff=1.0, ref=10.0)))
        out, ctx, _ = run(results)
        r = by_row(out)["gpu_matmul"]
        self.assertEqual((r["verdict"], r["failure_class"], r["lane"]), ("FAIL", "OUTPUT_MISMATCH", "discovery"))
        rec = M.build_record(out, ctx, "a" * 40, {}, {})
        self.assertEqual((rec["verdict"], rec["exit_code"]), ("PASS", 0))
        self.assertEqual([(b["row"], b["class"]) for b in rec["breakages"]], [("gpu_matmul", "OUTPUT_MISMATCH")])

    def test_the_same_miss_in_the_gate_lane_fails_the_run(self):
        results = all_good()
        results["gpu_matmul"] = (0, raw(set_check(good("gpu_matmul"), "matmul_single_512", diff=1.0, ref=10.0)))
        out, ctx, _ = run(results, ctx=make_ctx(lanes={"gpu_matmul": "gate"}))
        rec = M.build_record(out, ctx, "a" * 40, {}, {})
        self.assertEqual((rec["verdict"], rec["reason"], rec["exit_code"]), ("FAIL", "APP_FAIL:gpu_matmul", 1))

    def test_oversized_or_garbage_json_is_a_bad_result_and_the_run_goes_on(self):
        results = all_good()
        results["gpu_fft"] = (0, b"x" * (M.MAX_JSON_BYTES + 1))
        results["gpu_solve"] = (0, b"\x00not json")
        out, ctx, _ = run(results)
        r = by_row(out)
        self.assertEqual([(r[n]["verdict"], r[n]["failure_class"]) for n in ("gpu_fft", "gpu_solve")], [("FAIL", "BAD_RESULT")] * 2)
        self.assertEqual(r["dl_train"]["verdict"], "PASS")
        self.assertEqual(r["gpu_fft"]["result"]["bytes"], M.MAX_JSON_BYTES + 1)

    def test_missing_host_counters_make_the_row_harness_and_the_run_retry(self):
        ctx = make_ctx(probes={"gpu_matmul": FakeProbe(starts=False)})
        out, ctx, _ = run(all_good(), ctx=ctx)
        r = by_row(out)["gpu_matmul"]
        self.assertEqual((r["verdict"], r["reason"]), ("HARNESS", "HOST_COUNTERS_MISSING"))
        rec = M.build_record(out, ctx, "a" * 40, {}, {})
        self.assertEqual((rec["verdict"], rec["exit_code"]), ("RETRY", 3))

    def test_an_idle_gpu_fails_the_row_as_gpu_not_used(self):
        ctx = make_ctx(probes={"cuda_kernel": FakeProbe(smi_csv(busy=False))})
        out, ctx, _ = run(all_good(), ctx=ctx)
        r = by_row(out)["cuda_kernel"]
        self.assertEqual((r["verdict"], r["failure_class"]), ("FAIL", "GPU_NOT_USED"))
        self.assertEqual(r["gpu"]["source"], "host")

    def test_gpu_use_is_assessed_only_inside_the_rows_own_window(self):
        out, ctx, _ = run(all_good())
        self.assertTrue(all(p.started and p.stopped for p in ctx.made))
        self.assertEqual(len(ctx.made), 7)        # gpu_info needs no counters

    def test_driver_missing_is_not_run_for_every_row(self):
        out, ctx, calls = run(all_good(), refuse=("driver", M.BootRefusal("GUEST_NO_DRIVER", "NOT_RUN")))
        self.assertTrue(all(r["verdict"] == "NOT_RUN" and r["reason"] == "GUEST_NO_DRIVER" for r in out["rows"]))
        self.assertEqual(len([c for c in calls if c[0] == "boot"]), 1)      # the second boot is not paid for
        rec = M.build_record(out, ctx, "a" * 40, {}, {})
        self.assertEqual((rec["verdict"], rec["exit_code"]), ("SKIPPED", 0))

    def test_strict_policy_fails_on_any_skip(self):
        out, ctx, _ = run(all_good(), ctx=make_ctx(strict=True), refuse=("driver", M.BootRefusal("GUEST_NO_DRIVER", "NOT_RUN")))
        rec = M.build_record(out, ctx, "a" * 40, {}, {})
        self.assertEqual((rec["verdict"], rec["exit_code"]), ("FAIL", 4))
        self.assertTrue(rec["reason"].startswith("STRICT_SKIP:gpu_info:GUEST_NO_DRIVER"))

    def test_a_harness_boot_failure_twice_stops_paying_for_boots(self):
        out, ctx, calls = run(all_good(), refuse=("boot", M.BootRefusal("SSH_TIMEOUT", "HARNESS")))
        self.assertEqual(len([c for c in calls if c[0] == "boot"]), 2)
        self.assertEqual({r["verdict"] for r in out["rows"]}, {"HARNESS", "NOT_RUN"})
        rec = M.build_record(out, ctx, "a" * 40, {}, {})
        self.assertEqual(rec["exit_code"], 3)

    def test_repeated_harness_boots_end_in_harness_repeated(self):
        out, ctx, calls = run(all_good(), refuse=("boot", M.BootRefusal("SSH_TIMEOUT", "HARNESS")))
        self.assertEqual(sorted({r["reason"] for r in out["rows"]}), ["HARNESS_REPEATED", "SSH_TIMEOUT"])

    def test_evidence_result_file_is_the_sanitized_form(self):
        d = good("gpu_matmul")
        d["extra"] = "keep out"
        results = all_good()
        results["gpu_matmul"] = (0, raw(d))
        with tempfile.TemporaryDirectory(prefix="matlab-rows-ev-") as t:
            run(results, ctx=make_ctx(evidence_dir=t))
            got = json.loads((Path(t) / "gpu_matmul" / "result.json").read_text())
            self.assertEqual(got, M.sanitize_result(M.parse_row_json(raw(d), "gpu_matmul")))
            self.assertNotIn("extra", got)

    def test_an_unforeseen_boot_error_is_a_harness_refusal_not_a_crashed_run(self):
        out, ctx, calls = run(all_good(), refuse=("boot", RuntimeError("boom with /secret/path")))
        self.assertEqual({r["verdict"] for r in out["rows"]}, {"HARNESS", "NOT_RUN"})
        self.assertIn("BOOT_FAILED", {r["reason"] for r in out["rows"]})
        self.assertNotIn("secret", json.dumps(out))

    def test_an_unforeseen_row_error_keeps_the_other_rows(self):
        class Boom(FakeCensus):
            calls = 0

            def window(self, path, lo, hi):
                Boom.calls += 1
                if Boom.calls == 3:         # per row: the window read, then the baseline read; the third read is gpu_fft's window
                    raise RuntimeError("census broke")
                return self.base_lines if hi is not None else self.lines

        ctx = make_ctx()
        ctx.census = Boom()
        out = M.run_rows(["gpu_fft", "gpu_solve"], lambda i: FakeSession(all_good(), index=i), ctx, log=lambda *_: None)
        r = by_row(out)
        self.assertEqual((r["gpu_fft"]["verdict"], r["gpu_fft"]["reason"]), ("HARNESS", "INTERNAL"))
        self.assertEqual((r["gpu_info"]["verdict"], r["gpu_solve"]["verdict"]), ("PASS", "PASS"))

    def test_inject_missing_is_a_named_not_run(self):
        out, ctx, _ = run(all_good(), refuse=("inject", M.BootRefusal("INJECT_UNAVAILABLE", "NOT_RUN")))
        self.assertEqual({r["reason"] for r in out["rows"]}, {"INJECT_UNAVAILABLE"})

    def test_a_failed_preamble_stops_the_remaining_rows(self):
        results = all_good()
        results["gpu_info"] = (1, raw({"schema": M.SCHEMA_ROW, "row": "gpu_info", "status": "error", "error_id": "parallel:gpu:NoDevice", "checks": []}))
        out, ctx, calls = run(results)
        r = by_row(out)
        self.assertEqual((r["gpu_info"]["verdict"], r["gpu_info"]["failure_class"]), ("FAIL", "ASSERT"))
        self.assertEqual({v["reason"] for k, v in r.items() if k != "gpu_info"}, {"GPU_INFO_FAILED"})
        self.assertEqual(len([c for c in calls if c[0] == "boot"]), 1)

    def test_a_compute_capability_below_matlabs_floor_is_not_run(self):
        results = all_good()
        d = good("gpu_info")
        d["info"]["compute_capability"] = "6.1"
        results["gpu_info"] = (0, raw(d))
        out, ctx, _ = run(results)
        self.assertEqual({v["reason"] for k, v in by_row(out).items() if k != "gpu_info"}, {"GPU_UNSUPPORTED"})

    def test_a_row_that_exceeds_its_timebox_is_a_hang(self):
        results = all_good()
        results["gpu_solve"] = (124, None)
        out, ctx, _ = run(results)
        self.assertEqual(by_row(out)["gpu_solve"]["failure_class"], "HANG")
        results["gpu_fft"] = TimeoutError("ssh")
        out, ctx, _ = run(results)
        self.assertEqual(by_row(out)["gpu_fft"]["failure_class"], "HANG")

    def test_a_lost_guest_is_a_crash(self):
        results = all_good()
        results["dl_train"] = ConnectionResetError("ssh")
        out, ctx, _ = run(results)
        self.assertEqual((by_row(out)["dl_train"]["verdict"], by_row(out)["dl_train"]["failure_class"]), ("FAIL", "CRASH"))

    def test_xids_in_the_rows_window_fail_it(self):
        for ctx in (make_ctx(xid=1), make_ctx(census_lines=["NVRM: Xid (PCI:0000:01:00): 31, pid=1"])):
            out, ctx, _ = run(all_good(), ctx=ctx)
            r = by_row(out)
            self.assertEqual((r["gpu_info"]["verdict"], r["gpu_info"]["failure_class"]), ("FAIL", "XID"))     # the preamble's window already has one
            self.assertEqual({v["reason"] for k, v in r.items() if k != "gpu_info"}, {"GPU_INFO_FAILED"})

    def test_census_counts_are_kept_and_uuids_stripped(self):
        lines = ["GSP rpc UNSERVICED code 76 for GPU-fa4efa4e-fa4e-4fa4-8fa4-fa4efa4e0001", "ForeignClass 0xcafe refused"]
        out, ctx, _ = run(all_good(), ctx=make_ctx(census_lines=lines))
        c = by_row(out)["gpu_matmul"]["census"]
        self.assertEqual(c["counts"]["unserviced"], 1)
        self.assertNotIn("GPU-fa4efa4e", json.dumps(out))

    def test_deadline_gives_not_run_time_and_a_retry(self):
        ctx = make_ctx(deadline_epoch=1000.0, now=lambda: 990.0)
        out, ctx, calls = run(all_good(), ctx=ctx)
        self.assertEqual({r["reason"] for r in out["rows"]}, {"TIME"})
        self.assertEqual(calls, [])
        self.assertEqual(M.build_record(out, ctx, "a" * 40, {}, {})["exit_code"], 3)

    def test_rows_come_back_in_table_order(self):
        out, _, _ = run(all_good())
        self.assertEqual([r["row"] for r in out["rows"]], list(M.ROW_ORDER))

    def test_restricting_rows_keeps_the_preamble(self):
        self.assertEqual(M.plan_boots(["gpu_fft"], "row"), [["gpu_info"], ["gpu_fft"]])
        self.assertEqual(M.select_rows("gpu_fft,gpu_matmul"), ["gpu_matmul", "gpu_fft"])
        with self.assertRaises(ValueError):
            M.select_rows("nope")

    def test_lanes_option(self):
        self.assertEqual(M.parse_lanes(["gpu_fft=gate"], M.ROW_ORDER)["gpu_fft"], "gate")
        for bad in ("gpu_fft=weird", "nope=gate"):
            with self.assertRaises(ValueError):
                M.parse_lanes([bad], M.ROW_ORDER)


class MoreRunTests(unittest.TestCase):
    """Run-level checks that give the rule mutants a second, independent killer."""

    def test_exact_prefixes_cover_the_exact_checks_of_the_scripts(self):
        for name in ("vecadd_1000", "vecadd_after_40000_launches", "vecadd_first_n_of_guarded", "sample_chunk_1", "sum_ones_1e6", "no_write_past_n", "content_mismatches"):
            self.assertEqual(M.tolerance_for(name, "single"), 0.0, name)
        self.assertEqual(M.tolerance_for("matmul_single_256", "single"), 1e-4)
        self.assertIsNone(M.tolerance_for("matmul_x_256", "int8"))

    def test_first_and_last_elements_are_compared(self):
        for i in (2, 3):
            gpu = [10.0, 5.0, 1.0, 2.0]
            gpu[i] += 0.01
            v = M.verify_doc("gpu_matmul", M.parse_row_json(raw(set_check(good("gpu_matmul"), "matmul_single_512", gpu=gpu)), "gpu_matmul"), HW)
            self.assertFalse(v["ok"], i)

    def test_an_all_zero_reference_fingerprint_uses_a_unit_scale(self):
        d = set_check(good("gpu_matmul"), "matmul_single_512", cpu=[0.0] * 4, gpu=[0.0, 0.0, 0.0, 0.5], diff=0.5, ref=0.0)
        self.assertFalse(M.verify_doc("gpu_matmul", M.parse_row_json(raw(d), "gpu_matmul"), HW)["ok"])
        d = set_check(good("gpu_matmul"), "matmul_single_512", cpu=[0.0] * 4, gpu=[0.0] * 4, diff=0.0, ref=0.0)
        self.assertTrue(M.verify_doc("gpu_matmul", M.parse_row_json(raw(d), "gpu_matmul"), HW)["ok"])

    def test_a_nonzero_mismatch_count_fails_the_cuda_row(self):
        results = all_good()
        results["cuda_kernel"] = (0, raw(set_check(good("cuda_kernel"), "no_write_past_n", value=1, expected=1)))
        out, _, _ = run(results)
        self.assertEqual((by_row(out)["cuda_kernel"]["verdict"], by_row(out)["cuda_kernel"]["failure_class"]), ("FAIL", "OUTPUT_MISMATCH"))

    def test_one_worker_fails_the_parpool_row(self):
        results = all_good()
        results["parpool_gpu"] = (0, raw(set_check(good("parpool_gpu"), "num_workers", value=1, min=1)))
        out, _, _ = run(results)
        self.assertEqual(by_row(out)["parpool_gpu"]["failure_class"], "OUTPUT_MISMATCH")

    def test_a_loss_that_barely_falls_fails_and_the_pair_edges(self):
        d = set_check(good("dl_train"), "train_loss_curve", gpu=[1.0, 0.9, 0.8, 0.7], cpu=[1.0, 0.5, 0.2, 0.1])
        self.assertEqual(M.verify_doc("dl_train", M.parse_row_json(raw(d), "dl_train"), HW)["failed"], ["train_loss_curve"])
        for g, c, ok in ((0.010, 0.0105, True), (0.010, 0.0121, True), (0.010, 0.0150, False), (0.0, 0.0015, True), (0.0, 0.01, False)):
            d = set_check(good("dl_train"), "final_mse", gpu=g, cpu=c)
            self.assertEqual(M.verify_doc("dl_train", M.parse_row_json(raw(d), "dl_train"), HW)["ok"], ok, (g, c))

    def test_the_preamble_gate_unit(self):
        self.assertIsNone(M.gate_after_preamble({"verdict": "PASS", "facts": {"compute_capability": "7.5"}}))
        self.assertEqual(M.gate_after_preamble({"verdict": "PASS", "facts": {"compute_capability": "7.4"}}), "GPU_UNSUPPORTED")
        self.assertEqual(M.gate_after_preamble({"verdict": "FAIL", "facts": {}}), "GPU_INFO_FAILED")

    def test_a_forged_second_exit_line_makes_the_guest_untrusted(self):
        class Forging(FakeSession):
            def exec(self, command, timeout):
                rc, text = super().exec(command, timeout)
                return (0, "KF_RC=0\nKF_RC=0\n") if "gpu_fft" in command else (rc, text)

        out = M.run_rows(["gpu_fft"], lambda i: Forging(all_good(), index=i), make_ctx(), log=lambda *_: None)
        r = by_row(out)["gpu_fft"]
        self.assertEqual((r["verdict"], r["failure_class"], r["reason"]), ("FAIL", "CRASH", "GUEST_LOST"))

    def test_hostile_documents_at_run_level_name_their_reason(self):
        results = all_good()
        nan = ('{"schema":"%s","row":"gpu_fft","status":"done","checks":[],"x":NaN}' % M.SCHEMA_ROW).encode()
        dup = ('{"schema":"%s","row":"gpu_solve","status":"done","status":"done","checks":[]}' % M.SCHEMA_ROW).encode()
        big = b'{"pad":"' + b"x" * (M.MAX_JSON_BYTES + 10) + b'"}'
        results["gpu_fft"], results["gpu_solve"], results["dl_train"] = (0, nan), (0, dup), (0, big)
        out, _, _ = run(results)
        r = by_row(out)
        self.assertEqual(r["gpu_fft"]["findings"], ["NOT_JSON"])
        self.assertEqual(r["gpu_solve"]["findings"], ["NOT_JSON"])
        self.assertEqual(r["dl_train"]["findings"], ["TOO_LARGE"])

    def test_a_wrong_gpu_name_fails_the_preamble_and_nothing_else_runs(self):
        results = all_good()
        d = good("gpu_info")
        d["info"]["name"] = "NVIDIA GeForce RTX 3060"
        results["gpu_info"] = (0, raw(d))
        out, _, _ = run(results)
        r = by_row(out)
        self.assertEqual((r["gpu_info"]["verdict"], r["gpu_info"]["failure_class"]), ("FAIL", "OUTPUT_MISMATCH"))
        self.assertEqual(r["gpu_matmul"]["reason"], "GPU_INFO_FAILED")

    def test_a_memory_row_that_allocated_too_little_fails(self):
        results = all_good()
        d = good("mem_oversub")
        d["memory"].update(chunks=20, allocated_bytes=20 * 134217728)
        results["mem_oversub"] = (0, raw(d))
        out, _, _ = run(results)
        self.assertEqual(by_row(out)["mem_oversub"]["failure_class"], "OUTPUT_MISMATCH")

    def test_a_hang_in_a_shared_boot_sends_the_rest_to_a_fresh_boot(self):
        results = all_good()
        results["gpu_solve"] = (124, None)
        out, ctx, calls = run(results, ctx=make_ctx(isolation="shared"))
        self.assertEqual(len([c for c in calls if c[0] == "boot"]), 2)
        self.assertEqual(by_row(out)["dl_train"]["verdict"], "PASS")

    def test_the_deadline_cuts_later_boots_only(self):
        ticks = iter([0.0])
        ctx = make_ctx(deadline_epoch=2500.0, now=lambda: next(ticks, 5000.0))
        out, _, calls = run(all_good(), ctx=ctx)
        r = by_row(out)
        self.assertEqual(r["gpu_info"]["verdict"], "PASS")
        self.assertEqual({v["reason"] for k, v in r.items() if k != "gpu_info"}, {"TIME"})

    def test_sanitize_keeps_only_whitelisted_numbers_and_short_text(self):
        d = good("gpu_info")
        d["extra"] = "ACCOUNT-SECRET-STRING"
        d["info"]["owner"] = "ACCOUNT-SECRET-STRING"
        d["checks"][0]["note"] = "ACCOUNT-SECRET-STRING"
        out = M.sanitize_result(d)
        self.assertNotIn("ACCOUNT-SECRET-STRING", json.dumps(out))
        self.assertEqual(out["info"]["name"], "NVIDIA GeForce RTX 4090")

    def test_time_not_run_is_a_retry_and_a_gate_failure_beats_it(self):
        rows = [{"row": "a", "lane": "discovery", "verdict": "NOT_RUN", "reason": "TIME", "failure_class": None}]
        self.assertEqual(M.grade(rows, False)[2], 3)
        rows.append({"row": "b", "lane": "gate", "verdict": "FAIL", "reason": "x", "failure_class": "XID"})
        self.assertEqual(M.grade(rows, False)[2], 1)


class FinalRuleTests(unittest.TestCase):
    def test_nothing_ran_is_skipped_and_still_exit_zero_and_strict_is_not(self):
        rows = [{"row": "r", "lane": "discovery", "verdict": "NOT_RUN", "reason": "NO_GPU", "failure_class": None}]
        self.assertEqual(M.grade(rows, False)[2], 0)
        self.assertEqual(M.grade(rows, True)[:2], ("FAIL", "STRICT_SKIP:r:NO_GPU"))

    def test_user_supplied_pins_are_enforced_under_strict_only_for_the_cli(self):
        from unittest import mock
        with tempfile.TemporaryDirectory(prefix="matlab-rows-cli-") as t:
            img = Path(t) / "i.zip"
            img.write_bytes(b"x")
            common = ["run", "--image", str(img), "--workdir", t, "--qemu", "/q", "--driver-run", str(img), "--driver-sha256", "0" * 64, "--image-sha256", "f" * 64]
            seen = []

            def fake_run_rows(rows, make, ctx, log=print):
                seen.append(rows)
                return {"rows": [], "boots": []}

            outcomes = {}
            for strict in (False, True):
                buf = io.StringIO()
                with mock.patch.object(M, "host_gpu_facts", lambda: dict(HW)), mock.patch.object(M, "preflight_refusal", lambda *a, **k: None), \
                        mock.patch.object(M, "prepare_dir", lambda *a, **k: ({}, {})), mock.patch.object(M, "run_rows", fake_run_rows), \
                        mock.patch.object(M, "prop", lambda: types.SimpleNamespace(driver_version_from_name=lambda p: "580.126.09", declared_fb_mb=lambda st: 4096)), contextlib.redirect_stdout(buf):
                    M.main(common + (["--strict"] if strict else []))
                outcomes[strict] = json.loads([l for l in buf.getvalue().splitlines() if l.startswith(M.RESULT_LINE)][0].split(" ", 1)[1])
            self.assertEqual(len(seen), 1)                                              # the permissive run went on to run the rows
            self.assertFalse(outcomes[False]["image"]["pin_enforced"])
            self.assertEqual(outcomes[False]["image"]["sha256"], M.sha256_file(img))   # computed and recorded
            self.assertTrue(outcomes[True]["image"]["pin_enforced"])
            self.assertEqual({r["reason"] for r in outcomes[True]["rows"]}, {"IMAGE_SHA_MISMATCH"})

    def test_fetch_path_whitelist_is_the_rows_output_directory(self):
        s = make_session(self)[0]
        s.runner = lambda argv, **kw: types.SimpleNamespace(returncode=0, stdout=b"{}", stderr=b"")
        self.assertEqual(s.fetch("kfrows/out/mem_oversub.json", 10), b"{}")
        for bad in ("kfrows/scripts/x.json", "kfrows/out/x.txt", "kfrows/out/", "kfrows/out/" + "a" * 50 + ".json"):
            with self.assertRaises(OSError):
                s.fetch(bad, 10)

    def test_matlab_binary_option_is_validated(self):
        for ok in ("matlab", "/usr/local/MATLAB/R2026b/bin/matlab"):
            self.assertIn(ok, M.remote_command("gpu_info", 60, ok))
        with self.assertRaises(AssertionError):
            M.remote_command("gpu_info", 60, "matlab && id")


class GradeTests(unittest.TestCase):
    def rows(self, *spec):
        return [{"row": "r%d" % i, "lane": lane, "verdict": v, "reason": reason, "failure_class": None} for i, (lane, v, reason) in enumerate(spec)]

    def test_gate_fail_beats_everything(self):
        self.assertEqual(M.grade(self.rows(("gate", "FAIL", "x"), ("discovery", "HARNESS", "y")), False)[2], 1)

    def test_discovery_fail_is_listed_not_fatal(self):
        self.assertEqual(M.grade(self.rows(("discovery", "FAIL", "x"), ("discovery", "PASS", "")), False)[:3], ("PASS", "", 0))

    def test_nothing_ran_is_skipped_and_still_exit_zero(self):
        self.assertEqual(M.grade(self.rows(("discovery", "NOT_RUN", "IMAGE_MISSING")), False), ("SKIPPED", "NOTHING_RAN", 0))
        self.assertEqual(M.grade(self.rows(("discovery", "NOT_RUN", "IMAGE_MISSING")), True)[2], 4)

    def test_user_supplied_pins_are_enforced_under_strict_only(self):
        self.assertIsNone(M.check_pin("a" * 64, "b" * 64, False))
        self.assertEqual(M.check_pin("a" * 64, "b" * 64, True), "IMAGE_SHA_MISMATCH")
        self.assertIsNone(M.check_pin("a" * 64, "a" * 64, True))
        self.assertIsNone(M.check_pin("a" * 64, None, True))


def make_session(case, **pl_over):
    pl = M.prop()
    st = pl.init_state(SEED, ram_mb=4096, smp=2, disk_gb=30, keypair=("PRIVATE", PUB))
    st["clock"]["last_time_at_stop_utc"] = "2026-11-16T20:00:00Z"
    tmp = tempfile.TemporaryDirectory(prefix="matlab-rows-test-")
    case.addCleanup(tmp.cleanup)
    p = pl.paths(tmp.name)
    calls, popens, clock_calls, compare_calls = [], [], [], []

    def runner(argv, **kw):
        calls.append(list(argv))
        return types.SimpleNamespace(returncode=0, stdout="", stderr="")

    def popen(argv, **kw):
        popens.append(list(argv))
        return types.SimpleNamespace(pid=4242)

    shim = types.SimpleNamespace()
    base = {"guard_memory": lambda *a, **k: 9999, "wait_ssh": lambda *a, **k: 1.0, "run_probe": lambda *a, **k: {},
            "compare": lambda *a, **k: (compare_calls.append(1), [("machine_id", "MATCH", "")])[1], "restore_files": lambda st_, wd: None,
            "driver_clock": lambda state, run_file, created=None, *a, **k: (clock_calls.append((run_file, created)), pl.clock_for_mode(st, "run", "offline"))[1]}
    base.update(pl_over)
    for name in dir(pl):
        if not name.startswith("__"):
            setattr(shim, name, base.get(name, getattr(pl, name)))
    for k, v in base.items():
        setattr(shim, k, v)
    s = M.LinuxSession(st, p, "matlab1", "/opt/kf3/qemu-system-x86_64", "/tmp/NVIDIA-Linux-x86_64-580.126.09.run", "0" * 64, popen=popen, runner=runner)
    s.pl = shim
    shim.clock_calls, shim.compare_calls = clock_calls, compare_calls
    return s, calls, popens, p


class TransportTests(unittest.TestCase):
    def test_remote_command_shape(self):
        c = M.remote_command("gpu_matmul", 900)
        self.assertIn("timeout -k 20 900 matlab -batch", c)
        self.assertIn("run('$HOME/kfrows/scripts/gpu_matmul.m')", c)
        self.assertTrue(c.endswith("echo KF_RC=$?"))
        self.assertIn(">/var/tmp/kf-gpu_matmul.log 2>&1", c)        # the guest's stdout is never read back
        self.assertIn("/opt/matlab/bin/matlab", M.remote_command("gpu_fft", 10, "/opt/matlab/bin/matlab"))

    def test_nothing_unquoted_reaches_the_shell(self):
        for bad in ("matlab; rm -rf /", "$(id)", "a b", ""):
            with self.assertRaises(AssertionError):
                M.remote_command("gpu_fft", 10, bad)
        with self.assertRaises(AssertionError):
            M.remote_command("gpu_fft; id", 10)

    def test_exit_code_parsing(self):
        self.assertEqual(M.parse_rc("noise\nKF_RC=124\n"), 124)
        self.assertIsNone(M.parse_rc("KF_RC=1\nKF_RC=0\n"))         # a guest cannot print a second line to change the answer
        self.assertIsNone(M.parse_rc("nothing"))

    def test_refusal_mapping(self):
        r = M.refusal_from(M.prop().PropError("GUEST_DRIVER_MISMATCH: modinfo="))
        self.assertEqual((r.reason, r.verdict), ("GUEST_NO_DRIVER", "NOT_RUN"))
        r = M.refusal_from(M.prop().PropError("DRIVER_SHA_MISMATCH: x"))
        self.assertEqual((r.reason, r.verdict), ("DRIVER_SHA_MISMATCH", "HARNESS"))
        r = M.refusal_from(M.prop().PropError("HOST_MEMORY_LOW: x"))
        self.assertEqual((r.reason, r.verdict), ("HOST_MEMORY_LOW", "NOT_RUN"))
        self.assertEqual(M.refusal_from(OSError("x")).reason, "BOOT_FAILED")

    def session(self, **pl_over):
        return make_session(self, **pl_over)

    def test_boot_argv_is_offline_recorded_clock_with_the_kf3_device_and_the_runs_driver(self):
        s, calls, popens, p = self.session()
        s.boot()
        argv = popens[0]
        self.assertEqual(argv[0], "/opt/kf3/qemu-system-x86_64")
        joined = " ".join(argv)
        self.assertIn("restrict=on", joined)
        self.assertIn("guest-driver=580.126.09", joined)
        self.assertIn("kf3-gpu", joined)
        self.assertIn("-pidfile", argv)
        self.assertNotIn("-daemonize", argv)                    # kf3 logs go to qemu.log through our own stderr redirect
        self.assertTrue(any(a.startswith("base=2026-11-16") for a in argv))
        self.assertNotIn("base=utc", joined)
        self.assertEqual(s.pl.clock_calls, [("/tmp/NVIDIA-Linux-x86_64-580.126.09.run", None)])      # the clock is the driver-floor decision, not a bare recorded base

    def test_a_missing_driver_is_not_run_and_a_wrong_sha_is_harness(self):
        from_pl = M.prop().PropError
        for message, reason, verdict in (("GUEST_DRIVER_MISMATCH: modinfo='' libcuda=''", "GUEST_NO_DRIVER", "NOT_RUN"), ("DRIVER_SHA_MISMATCH: x", "DRIVER_SHA_MISMATCH", "HARNESS"),
                                         ("KERNEL_CC_MISSING: x", "KERNEL_CC_MISSING", "HARNESS")):
            def boom(*a, _m=message, **k):
                raise from_pl(_m)

            s, calls, popens, p = self.session(install_driver=boom)
            with self.assertRaises(M.BootRefusal) as c:
                s.install_driver()
            self.assertEqual((c.exception.reason, c.exception.verdict), (reason, verdict))

    def test_boot_and_driver_stage_each_run_the_identity_comparison(self):
        s, calls, popens, p = self.session(install_driver=lambda *a, **k: {})
        s.boot()
        s.install_driver()
        self.assertEqual(len(s.pl.compare_calls), 2)

    def test_the_driver_stage_re_checks_the_identity_so_the_gpu_uuid_must_match(self):
        s, calls, popens, p = self.session(install_driver=lambda *a, **k: {}, compare=lambda *a, **k: [("gpu_uuid", "MISMATCH", "")])
        s.t0 = 0.0
        with self.assertRaises(M.BootRefusal) as c:
            s.install_driver()
        self.assertEqual(c.exception.reason, "IDENTITY_MISMATCH:gpu_uuid")

    def test_the_boot_uses_the_driver_clock_floor(self):
        seen = []
        pl = M.prop()
        s, calls, popens, p = self.session(driver_clock=lambda st, run_file, created=None, **k: (seen.append((run_file, created)), pl.clock_for_mode(st, "run", "offline"))[1])
        s.created = "2026-11-10T00:00:00Z"
        s.boot()
        self.assertEqual(seen, [("/tmp/NVIDIA-Linux-x86_64-580.126.09.run", "2026-11-10T00:00:00Z")])

    def test_boot_refuses_an_identity_mismatch(self):
        s, calls, popens, p = self.session(compare=lambda *a, **k: [("gpu_uuid", "MISMATCH", "")])
        with self.assertRaises(M.BootRefusal) as c:
            s.boot()
        self.assertEqual((c.exception.reason, c.exception.verdict), ("IDENTITY_MISMATCH:gpu_uuid", "HARNESS"))

    def test_inject_needs_the_inject_verb(self):
        s, calls, popens, p = self.session()
        s.pl = types.SimpleNamespace()                          # no cmd_inject
        with self.assertRaises(M.BootRefusal) as c:
            s.inject(M.SCRIPTS_DIR)
        self.assertEqual((c.exception.reason, c.exception.verdict), ("INJECT_UNAVAILABLE", "NOT_RUN"))
        s, calls, popens, p = self.session(cmd_inject=lambda a: 0)
        s.inject(M.SCRIPTS_DIR)
        argv = calls[-1]
        self.assertEqual(argv[argv.index("inject") + 1:argv.index("inject") + 3], ["--dir", str(p["dir"])])
        self.assertEqual(argv[argv.index("--src") + 1], str(M.SCRIPTS_DIR))
        self.assertEqual(argv[argv.index("--dest") + 1], "kfrows/scripts")

    def test_fetch_reads_only_the_rows_own_json_and_bounds_it(self):
        s, calls, popens, p = self.session()
        s.fetch("kfrows/out/gpu_fft.json", 1000)
        self.assertIn("head -c 1000", calls[-1][-1])
        for bad in ("../etc/passwd", "kfrows/out/../../x.json", "kfrows/out/a b.json", "/etc/passwd", "kfrows/out/x.json; id"):
            with self.assertRaises(OSError):
                s.fetch(bad, 10)

    def test_exec_bounds_what_comes_back(self):
        s, calls, popens, p = self.session()
        s.runner = lambda argv, **kw: types.SimpleNamespace(returncode=0, stdout="x" * 100000, stderr="")
        self.assertEqual(len(s.exec("true", 5)[1]), 4096)

    def test_prepare_dir_lays_out_the_state_directory_and_a_private_key(self):
        pl = M.prop()
        st = pl.init_state(SEED, ram_mb=4096, smp=2, disk_gb=30, keypair=("PRIVATE-KEY-TEXT", PUB))
        with tempfile.TemporaryDirectory(prefix="matlab-rows-test-") as t:
            t = Path(t)
            pl.save_state(st, t / "state.json")
            (t / "image.qcow2").write_bytes(b"not a disk")
            pl.pack(t / "state.json", t / "image.qcow2", t / "image.zip")
            got, p = M.prepare_dir(t / "image.zip", t / "work")
            self.assertEqual(got["seed"], SEED)
            self.assertTrue((p["sealed"] / "image.qcow2").is_file())
            self.assertEqual(p["key"].read_text(), "PRIVATE-KEY-TEXT")
            self.assertEqual(oct(p["key"].stat().st_mode & 0o777), "0o600")
            self.assertTrue(p["state"].is_file())


# ---------------------------------------------------------------- evidence and the CLI
class EvidenceTests(unittest.TestCase):
    def test_evidence_is_text_only_and_never_holds_raw_guest_bytes(self):
        d = good("gpu_matmul")
        d["leak"] = "ACCOUNT-SECRET-STRING"
        d["checks"][0]["note"] = "ACCOUNT-SECRET-STRING"
        results = all_good()
        results["gpu_matmul"] = (0, raw(d))
        results["gpu_fft"] = (0, b"ACCOUNT-SECRET-STRING \x00 garbage")
        with tempfile.TemporaryDirectory(prefix="matlab-rows-ev-") as t:
            out, ctx, _ = run(results, ctx=make_ctx(evidence_dir=t))
            blob = "".join(p.read_text() for p in Path(t).rglob("*") if p.is_file()) + json.dumps(out)
            self.assertNotIn("ACCOUNT-SECRET-STRING", blob)
            self.assertTrue((Path(t) / "gpu_matmul" / "verdict.json").is_file())
            self.assertTrue((Path(t) / "gpu_matmul" / "host_query.csv").is_file())
            self.assertFalse((Path(t) / "gpu_fft" / "result.json").exists())    # an unusable result is not kept

    def test_record_is_bounded_and_one_line(self):
        out, ctx, _ = run(all_good())
        rec = M.build_record(out, ctx, "a" * 40, {"sha256": "1" * 64}, {"version": "580.126.09"})
        for r in rec["rows"]:
            r["findings"] = ["f" * 100] * 8
            r["gpu"]["reasons"] = ["g" * 100] * 4
        data = M.serialise(rec)
        self.assertLessEqual(len(data.encode()), M.MAX_RECORD_BYTES)
        self.assertNotIn("\n", data)
        self.assertEqual(json.loads(data)["schema"], M.SCHEMA_RESULT)

    def run_cli(self, *argv):
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            code = M.main(list(argv))
        return code, buf.getvalue()

    def test_verify_cli(self):
        with tempfile.TemporaryDirectory(prefix="matlab-rows-cli-") as t:
            f = Path(t) / "r.json"
            f.write_bytes(raw(good("gpu_fft")))
            code, out = self.run_cli("verify", str(f), "--row", "gpu_fft")
            self.assertEqual((code, out.split()[0]), (0, "OK"))
            f.write_bytes(raw(set_check(good("gpu_fft"), "fft_single_65536", diff=9.0, ref=10.0)))
            code, out = self.run_cli("verify", str(f), "--row", "gpu_fft")
            self.assertEqual(code, 1)
            self.assertIn("fft_single_65536", out)
            f.write_bytes(b"x")
            self.assertEqual(self.run_cli("verify", str(f), "--row", "gpu_fft")[0], 1)

    def test_run_with_no_image_is_not_run_image_missing_and_exits_zero_standalone(self):
        with tempfile.TemporaryDirectory(prefix="matlab-rows-cli-") as t:
            code, out = self.run_cli("run", "--image", str(Path(t) / "nope.zip"), "--workdir", t, "--qemu", "/nonexistent", "--driver-run",
                                     str(Path(t) / "NVIDIA-Linux-x86_64-580.126.09.run"), "--driver-sha256", "0" * 64)
            line = [l for l in out.splitlines() if l.startswith(M.RESULT_LINE + " ")][0]
            rec = json.loads(line.split(" ", 1)[1])
            self.assertEqual((code, rec["verdict"]), (0, "SKIPPED"))
            self.assertEqual({r["reason"] for r in rec["rows"]}, {"IMAGE_MISSING"})
            self.assertEqual(len(rec["rows"]), 8)
            code, out = self.run_cli("run", "--image", str(Path(t) / "nope.zip"), "--workdir", t, "--qemu", "/nonexistent", "--driver-run", "x",
                                     "--driver-sha256", "0" * 64, "--strict")
            self.assertEqual(code, 4)

    def test_preflight_reasons(self):
        with tempfile.TemporaryDirectory(prefix="matlab-rows-cli-") as t:
            img = Path(t) / "i.zip"
            img.write_bytes(b"x")
            self.assertEqual(M.preflight_refusal(str(img), {}, "/x"), "NO_GPU")
            self.assertEqual(M.preflight_refusal(str(img), HW, "/nonexistent", kvm=str(img)), "QEMU_MISSING")
            self.assertEqual(M.preflight_refusal(str(img), HW, "/x", kvm=str(Path(t) / "nokvm")), "KVM_MISSING")
            self.assertEqual(M.preflight_refusal(None, HW, "/x"), "IMAGE_MISSING")

    def test_host_gpu_facts_parse(self):
        ok = lambda argv, **kw: types.SimpleNamespace(returncode=0, stdout="NVIDIA GeForce RTX 4090, 8.9, 24564\n", stderr="")
        self.assertEqual(M.host_gpu_facts(ok), {"gpu_name": "NVIDIA GeForce RTX 4090", "compute_cap": 890, "memory_mib": 24564})
        two = lambda argv, **kw: types.SimpleNamespace(returncode=0, stdout="a, 8.9, 1\nb, 8.9, 1\n", stderr="")
        self.assertEqual(M.host_gpu_facts(two), {})

        def gone(argv, **kw):
            raise FileNotFoundError

        self.assertEqual(M.host_gpu_facts(gone), {})


# ---------------------------------------------------------------- the MATLAB scripts themselves, read as text
class GpuUseProofTests(unittest.TestCase):
    """The owner requirement of 2026-11-17: a row never passes because MATLAB computed on the CPU. gpuDevice present, an operation on a gpuArray (recorded
    before the gather), and the host's kf3 census window showing GPU work: each is a separate check with its own failure."""

    def verify(self, row, doc, hw=HW):
        return M.verify_doc(row, doc, hw, DECLARED_FB_MB)

    def test_a_cpu_result_compared_with_the_cpu_result_agrees_numerically_and_still_fails(self):
        doc = good("gpu_matmul")
        for it in doc["checks"]:
            it["on_gpu"] = False                     # kf_cmp.m saw a regular array: gather() of it is a no-op, so the numbers agree trivially
        v = self.verify("gpu_matmul", doc)
        self.assertEqual((v["ok"], v["klass"]), (False, "GPU_NOT_USED"))
        self.assertTrue(any("not a gpuArray" in f for f in v["findings"]))

    def test_a_script_that_does_not_record_on_gpu_fails_closed(self):
        doc = good("gpu_fft")
        for it in doc["checks"]:
            del it["on_gpu"]
            del it["exists_on_gpu"]
        v = self.verify("gpu_fft", doc)
        self.assertEqual(v["klass"], "GPU_NOT_USED")

    def test_one_comparison_on_the_cpu_among_gpu_ones_is_enough_to_fail(self):
        doc = set_check(good("gpu_solve"), "lstsq_single_2000x500", on_gpu=False)
        self.assertEqual(self.verify("gpu_solve", doc)["klass"], "GPU_NOT_USED")

    def test_a_gpuarray_proof_item_that_says_cpu_fails_for_the_three_rows_that_have_one(self):
        for row, name in (("cuda_kernel", "kernel_output_gpuarray"), ("mem_oversub", "chunks_gpuarray"), ("dl_train", "train_env_gpu")):
            doc = set_check(good(row), name, on_gpu=False)
            self.assertEqual(self.verify(row, doc)["klass"], "GPU_NOT_USED", row)
            doc = set_check(good(row), name, exists_on_gpu=False)
            self.assertEqual(self.verify(row, doc)["klass"], "GPU_NOT_USED", row)
            self.assertTrue(self.verify(row, good(row))["ok"], row)

    def test_a_false_gpuarray_item_is_named_in_the_findings(self):
        doc = set_check(good("cuda_kernel"), "kernel_output_gpuarray", on_gpu=False, exists_on_gpu=False)
        self.assertIn("kernel_output_gpuarray: on_gpu=False exists_on_gpu=False", self.verify("cuda_kernel", doc)["findings"])

    def test_a_document_of_counts_only_has_no_gpu_proof_and_says_so(self):
        doc = good("gpu_info")
        doc["checks"] = [item("content_mismatches")]
        self.assertIn("NO_GPU_ARRAY_PROOF", self.verify("gpu_info", doc)["findings"])

    def test_gpuarray_flags_must_be_booleans(self):
        doc = set_check(good("cuda_kernel"), "kernel_output_gpuarray", on_gpu="true")
        self.assertEqual(self.verify("cuda_kernel", doc)["klass"], "GPU_NOT_USED")

    def test_no_gpu_proof_item_at_all_is_a_failure_even_when_every_check_passes(self):
        doc = good("gpu_info")
        doc["checks"] = [{k: v for k, v in c.items() if k not in ("on_gpu", "exists_on_gpu")} for c in doc["checks"]]
        v = self.verify("gpu_info", doc)
        self.assertIn("NO_GPU_ARRAY_PROOF", v["findings"])

    def test_gpudevice_must_be_present_usable_and_the_hosts_gpu(self):
        for bad in (None, {}, {"name": "", "index": 0, "can_use_gpu": False}, {"name": "NVIDIA GeForce RTX 4090", "index": 1, "can_use_gpu": False},
                    {"name": "NVIDIA GeForce RTX 4090", "index": 0, "can_use_gpu": True}):
            doc = good("gpu_matmul")
            if bad is None:
                del doc["gpu_device"]
            else:
                doc["gpu_device"] = bad
            self.assertEqual(self.verify("gpu_matmul", doc)["klass"], "GPU_NOT_USED", bad)
        doc = good("gpu_matmul")
        doc["gpu_device"]["name"] = "NVIDIA GeForce RTX 3060"
        self.assertTrue(any("GPU_DEVICE_NAME_DIFFERS" in f for f in self.verify("gpu_matmul", doc)["findings"]))
        self.assertTrue(self.verify("gpu_matmul", doc, hw={})["ok"])        # no host name known: the block alone is checked

    def test_the_scripts_write_a_device_block_and_the_host_requires_it(self):
        self.assertIn("doc.gpu_device = struct(", (MATLAB_DIR / "kf_write.m").read_text())
        doc = good("cuda_kernel")
        del doc["gpu_device"]
        self.assertIn("NO_GPU_DEVICE_BLOCK", self.verify("cuda_kernel", doc)["findings"])

    def test_gpu_not_used_outranks_a_numeric_miss_for_a_foreign_device(self):
        doc = set_check(good("gpu_fft"), "fft_single_65536", diff=9.0)
        doc["gpu_device"]["name"] = "NVIDIA GeForce RTX 3060"
        self.assertEqual(self.verify("gpu_fft", doc)["klass"], "GPU_NOT_USED")

    def test_gpu_not_used_outranks_a_numeric_miss(self):
        doc = set_check(good("gpu_matmul"), "matmul_single_256", on_gpu=False, diff=5.0)
        self.assertEqual(self.verify("gpu_matmul", doc)["klass"], "GPU_NOT_USED")

    def test_the_census_window_with_no_submissions_fails_the_row_end_to_end(self):
        ctx = make_ctx()
        ctx.census = FakeCensus(work=False)
        r, _ = M.run_row(FakeSession(all_good()), "gpu_matmul", ctx)
        self.assertEqual((r["verdict"], r["failure_class"]), ("FAIL", "GPU_NOT_USED"))
        self.assertIn("no GPU work in the window", r["reason"])
        self.assertEqual(r["census"]["work"]["verdict"], "FAIL")

    def test_a_missing_baseline_line_is_a_harness_matter_not_a_pass_and_not_a_failure(self):
        ctx = make_ctx()
        ctx.census = FakeCensus(work=False, base=False)
        r, _ = M.run_row(FakeSession(all_good()), "gpu_matmul", ctx)
        self.assertEqual((r["verdict"], r["reason"]), ("HARNESS", "HOST_CENSUS_MISSING"))

    def test_the_good_run_records_the_census_work(self):
        out, ctx, _ = run(all_good())
        r = by_row(out)["gpu_matmul"]
        self.assertEqual((r["verdict"], r["census"]["work"]["verdict"]), ("PASS", "PASS"))
        self.assertEqual(r["census"]["work"]["submissions"], 411)

    def test_a_guest_claiming_gpu_work_cannot_beat_a_host_that_saw_none_for_any_row(self):
        for row in M.ROW_ORDER:
            ctx = make_ctx()
            ctx.census = FakeCensus(work=False)
            rec, _ = M.run_row(FakeSession(all_good()), row, ctx)
            self.assertEqual((rec["verdict"], rec["failure_class"]), ("FAIL", "GPU_NOT_USED"), row)

    def test_evidence_keeps_the_gpu_proof_fields_and_nothing_else_of_the_device_block(self):
        doc = good("cuda_kernel")
        doc["gpu_device"]["extra_secret"] = "x"
        doc["gpu_device"]["path"] = "/home/u"
        out = M.sanitize_result(doc)
        self.assertEqual(set(out["gpu_device"]), {"name", "index", "can_use_gpu"})
        kept = {c["name"]: c for c in out["checks"]}["kernel_output_gpuarray"]
        self.assertTrue(kept["on_gpu"] and kept["exists_on_gpu"])


class ScriptTests(unittest.TestCase):
    FORBIDDEN = (r"\blicen[sc]e\b", r"\bver\b", r"\bwhoami\b", r"\busername\b", r"\buserpath\b", r"\bprefdir\b", r"\bmatlabroot\b", r"\bdisp\s*\(", r"\bfprintf\s*\(",
                 r"\bdisplay\s*\(", r"\bsystem\s*\(", r"\bunix\s*\(", r"\bdos\s*\(", r"getenv\s*\(\s*['\"](USER|LOGNAME|USERNAME)", r"\bnet\.|\bweb(read|write|save)\b|\burlread\b|\bhttp",
                 r"@[A-Za-z0-9-]+\.[A-Za-z]{2,}", r"\bgetpwuid\b", r"\bhostname\b")

    def text(self, name):
        return (MATLAB_DIR / name).read_text()

    def test_every_row_has_its_script_and_no_stray_files(self):
        files = {p.name for p in MATLAB_DIR.iterdir()}
        scripts = {v[0] for v in M.ROWS.values()}
        self.assertEqual(files, scripts | set(M.SUPPORT_FILES))

    def test_no_script_prints_or_reads_account_or_licence_data(self):
        for p in sorted(MATLAB_DIR.iterdir()):
            body = "\n".join(l.split("%")[0] for l in p.read_text().splitlines()) if p.suffix == ".m" else ""
            for pattern in self.FORBIDDEN:
                self.assertIsNone(re.search(pattern, body), "%s matches %s" % (p.name, pattern))

    def test_scripts_never_write_a_path_or_a_user_name_into_the_json(self):
        for row, spec in M.ROWS.items():
            body = self.text(spec[0])
            self.assertNotRegex(body, r"\bpwd\b|\bfullfile\(.*\)\s*;?\s*%?\s*info\.")
            self.assertIn("row = '%s'" % row, body)
            self.assertIn("kf_write(row, doc)", body)
            self.assertIn("kf_fail(row, ME)", body)
            self.assertIn("rethrow(ME)", body)

    def test_kf_cmp_records_the_gpuarray_flags_before_it_gathers(self):
        body = self.text("kf_cmp.m")
        self.assertLess(body.index("isa(xg, 'gpuArray')"), body.index("gather(xg)"))
        self.assertIn("existsOnGPU(xg)", body)
        self.assertIn("c.on_gpu = on_gpu", body)

    def test_the_writer_records_gpudevice_and_the_proof_helper_exists(self):
        self.assertIn("gpuDevice()", self.text("kf_write.m"))
        self.assertIn("canUseGPU", self.text("kf_write.m"))
        body = self.text("kf_gpu.m")
        self.assertIn("existsOnGPU", body)
        self.assertIn("kf_gpu.m", M.SUPPORT_FILES)

    def test_no_row_gathers_the_gpu_side_before_kf_cmp(self):
        for row, spec in M.ROWS.items():
            for line in self.text(spec[0]).splitlines():
                code = line.split("%")[0]
                if "kf_cmp(" in code:
                    first_arg = code.split("kf_cmp(", 1)[1]
                    self.assertNotIn("gather(", first_arg.split(",", 2)[1] if first_arg.count(",") >= 2 else "", "%s: %s" % (row, line.strip()))

    def test_the_rows_that_need_a_proof_item_call_kf_gpu(self):
        for row, name in (("cuda_kernel", "kernel_output_gpuarray"), ("mem_oversub", "chunks_gpuarray"), ("dl_train", "train_env_gpu")):
            self.assertIn("kf_gpu('%s'" % name, self.text(M.ROWS[row][0]))
            self.assertIn(name, M.ROWS[row][3])

    def test_dl_train_trains_with_an_explicit_gpu_environment_never_auto(self):
        body = self.text("dl_train.m")
        self.assertIn("'ExecutionEnvironment', envs{e}", body)
        self.assertIn("envs = {'gpu', 'cpu'}", body)
        self.assertNotIn("'auto'", body)

    def test_the_error_writer_records_only_the_identifier(self):
        body = self.text("kf_fail.m")
        self.assertIn("ME.identifier", body)
        self.assertNotIn("ME.message", body)
        self.assertNotIn("getReport", body)

    def test_each_required_check_is_produced_by_its_script(self):
        for row, spec in M.ROWS.items():
            body = self.text(spec[0]) + "".join(self.text(f) for f in ("kf_worker_task.m",) if row == "parpool_gpu")
            for name in spec[3]:
                parts = name.split("_")
                k = next((i for i, w in enumerate(parts) if w in ("single", "double") or w[:1].isdigit()), len(parts))
                stem = "_".join(parts[:k]) if k else name
                self.assertIn(stem, body, "%s: %s" % (row, name))

    def test_scripts_use_the_documented_tolerance_numbers(self):
        body = self.text("kf_tol.m")
        self.assertIn("1e-4", body)
        self.assertIn("1e-10", body)
        self.assertEqual(M.TOLERANCE, {"single": 1e-4, "double": 1e-10})

    def test_loops_are_bounded_by_counts_not_by_time(self):
        for p in sorted(MATLAB_DIR.glob("*.m")):
            body = p.read_text()
            self.assertNotRegex(body, r"\bwhile\b", p.name)
            self.assertNotRegex(body, r"\b(tic|toc|datetime|now|clock|cputime|pause)\b", p.name)     # no timing criterion anywhere

    def test_cuda_kernel_prototype_matches_the_ptx(self):
        ptx = (MATLAB_DIR / "vecadd.ptx").read_text()
        body = self.text("cuda_kernel.m")
        self.assertIn("'const float *, const float *, float *, int', 'vecadd'", body)
        self.assertEqual(re.findall(r"\.visible \.entry (\w+)\(", ptx), ["vecadd"])
        self.assertEqual(re.findall(r"\.param \.(\w+) vecadd_param_\d", ptx), ["u64", "u64", "u64", "u32"])
        self.assertRegex(ptx, r"\.target sm_(7[5-9]|[89]\d)")
        self.assertIn("vecadd.ptx", body)
        self.assertNotIn("mexcuda", body)

    def test_ptx_registers_stay_inside_their_declarations(self):
        ptx = (MATLAB_DIR / "vecadd.ptx").read_text()
        decl = {m.group(1): int(m.group(2)) for m in re.finditer(r"\.reg \.\w+\s+%(\w+)<(\d+)>;", ptx)}
        self.assertEqual(sorted(decl), ["f", "p", "r", "rd"])
        for kind, count in decl.items():
            used = {int(m.group(1)) for m in re.finditer(r"%%%s(\d+)\b" % kind, ptx)}
            self.assertTrue(used and max(used) < count, (kind, used, count))
        self.assertEqual(ptx.count("{") , ptx.count("}"))
        self.assertIn("ret;", ptx)
        self.assertIn("DONE:", ptx)

    def test_sustained_phases_have_fixed_counts(self):
        self.assertIn("for k = 1:300", self.text("gpu_matmul.m"))
        self.assertIn("for k = 1:200", self.text("gpu_fft.m"))
        self.assertIn("for r = 1:40000", self.text("cuda_kernel.m"))
        self.assertIn("min(floor(target / chunkBytes), 512)", self.text("mem_oversub.m"))
        self.assertIn("0.85 * avail0", self.text("mem_oversub.m"))

    def test_parpool_row_uses_a_pool_with_a_device_per_worker_and_parfeval(self):
        body = self.text("parpool_gpu.m")
        for needle in ("parpool('Processes', 2)", "gpuDevice(1)", "parfeval(", "addAttachedFiles"):
            self.assertIn(needle, body)

    def test_dl_row_trains_on_both_devices_with_the_same_seed(self):
        body = self.text("dl_train.m")
        self.assertIn("{'gpu', 'cpu'}", body)
        self.assertIn("trainnet(", body)
        self.assertIn("'Shuffle', 'never'", body)
        self.assertEqual(body.count("rng(11, 'twister')"), 1)         # inside the loop: both runs start from the same seed

    def test_scripts_are_listed_for_injection(self):
        self.assertTrue(set(M.SUPPORT_FILES) <= {p.name for p in MATLAB_DIR.iterdir()})


if __name__ == "__main__":
    unittest.main()
