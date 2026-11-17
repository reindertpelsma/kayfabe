#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""MATLAB GPU rows on the Linux proprietary image: boot, install the driver, inject the scripts, run each row, judge on the HOST (stdlib only).

STATUS: DESIGN-ONLY, 2026-11-17. GPU-free: every judging function is tested against fixture results and a fake guest transport
(scripts/ci/test_matlab_rows.py, mutants scripts/ci/matlab_rows_mutants.py). The real transport (`LinuxSession`) and the CLI have run on NO box;
what needs a KVM box with a GPU is listed in docs/design/V3_PROPIMAGE_LINUX.md section 14. Rows and their scripts: scripts/bench/linux/matlab/.

    matlab_rows.py plan                                           the rows, their time-boxes and what each must show
    matlab_rows.py verify RESULT.json --row ROW [--host-gpu-name N --host-mem-mib M]   re-judge one pulled result file (no VM)
    matlab_rows.py run --image ZIP|--dir D --workdir W --qemu BIN --driver-run F --driver-sha256 H [--rows a,b] [--isolation row|shared]
                       [--lane ROW=gate|discovery] [--strict] [--sha SHA] [--out FILE] [--evidence-dir DIR] [--deadline-epoch N]

What a row is. A MATLAB script (scripts/bench/linux/matlab/<row>.m) that computes its numbers on the GPU AND on the CPU in the same script and
writes one small JSON (`jsonencode`) with the raw numbers and the tolerances it used. The scripts live in the repository and are COPIED INTO THE RUNNING
OFFLINE CLONE at run time (`propimage_linux.py inject`); they are never baked into the image or stored in R2. The guest is hostile: its JSON is bounded,
parsed defensively (no NaN, no duplicate keys, no oversize), and every agreement flag is RECOMPUTED here from the raw numbers with this file's own
tolerances. Nothing else of the guest's output is kept: a row's stdout is not read, only its exit code and its JSON. The scripts never print account
or licence data (a test scans them), and this module never reads or prints any.

Verdict classes (per row): PASS; FAIL with a class (CRASH, HANG, XID, TDR, ASSERT, OUTPUT_MISMATCH, GPU_NOT_USED, BAD_RESULT); HARNESS (the run could not be
judged: transport, driver build, host counters missing, identity mismatch: never an application failure); NOT_RUN with a named reason (IMAGE_MISSING,
NO_GPU, GUEST_NO_DRIVER, GPU_UNSUPPORTED, DRIVER_MISSING, INJECT_UNAVAILABLE, GPU_INFO_FAILED, TIME, HOST_MEMORY_LOW...). GPU use is asserted from
HOST-side `nvidia-smi` counters (scripts/bench/windows/gpu_assert.py: HostProbe and evaluate, the record format of the Windows rows), never from the guest.
Every row is in the discovery lane unless told otherwise (`--lane ROW=gate`): a discovery failure is a listed breakage and never fails the run by itself;
a gate failure always fails it; strict policy (the Worker) additionally fails on any NOT_RUN. Timing is never a pass criterion.
"""
import argparse
import hashlib
import importlib.util
import json
import math
import os
import re
import shutil
import subprocess
import sys
import time
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
WINDOWS = HERE.parent / "windows"
SCRIPTS_DIR = HERE / "matlab"
SCHEMA_ROW = "kayfabe.matlab-row/1"
SCHEMA_RESULT = "kayfabe.matlab-rows/1"
RESULT_LINE = "KF_MATLAB_RESULT"
MAX_JSON_BYTES = 262144        # one row's JSON, read from the guest (the longest legitimate one is about 20 KB)
MAX_CHECKS = 64
MAX_LIST = 4096
MAX_STR = 200
MAX_RECORD_BYTES = 49152
GUEST_ROOT = "kfrows"          # relative: under the guest user's home (propimage_linux inject: --dest relative = under /home/<user>)
GUEST_SCRIPTS = GUEST_ROOT + "/scripts"
GUEST_OUT = GUEST_ROOT + "/out"
MATLAB_BIN = "matlab"
MIN_CC_X100 = 750              # MATLAB's GPU support starts at compute capability 7.5
BOOT_SLACK_S = 240             # DESIGN: boot, ssh, identity probe and the pull of the JSON around a row's own time-box
LANES = ("gate", "discovery")
FAIL_CLASSES = ("CRASH", "HANG", "XID", "TDR", "ASSERT", "OUTPUT_MISMATCH", "GPU_NOT_USED", "BAD_RESULT")
EXACT_PREFIXES = ("vecadd_", "sample_chunk_", "sum_ones", "no_write_past_n", "content_mismatches")
TOLERANCE = {"single": 1e-4, "double": 1e-10}        # relative; the host's own copy (kf_tol.m states the same numbers, never trusted)
PAIR_REL, PAIR_ABS = 0.10, 2e-3                      # dl_train: GPU and CPU final loss
LOSS_FALL = 0.5                                      # dl_train: the last epoch's mean loss must be below this fraction of the first
MEM_FRACTION = (0.70, 0.90)                          # mem_oversub: allocated bytes over the free memory reported at the start
MEM_RETURNED = 0.80                                  # mem_oversub: free memory after the release, over the start value
NAME_RE = re.compile(r"[A-Za-z0-9_]{1,64}")
BIN_RE = re.compile(r"[A-Za-z0-9_./-]{1,120}")
CC_RE = re.compile(r"(\d{1,2})\.(\d{1,2})")


class BadResult(Exception):
    """The guest's JSON cannot be used: `reason` is a short upper-case token."""

    def __init__(self, reason):
        super().__init__(reason)
        self.reason = reason


class BootRefusal(Exception):
    """A boot that cannot give the row a guest. verdict NOT_RUN (a named reason, not a failure) or HARNESS (could not be judged)."""

    def __init__(self, reason, verdict="HARNESS"):
        super().__init__(reason)
        self.reason, self.verdict = reason, verdict


def clean(text, limit=MAX_STR):
    """Anything that came from the guest or a tool: printable ASCII, bounded, never forwarded raw."""
    return re.sub(r"[^\x20-\x7e]", "?", str(text))[:limit]


def _load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


_MODS = {}


def _mod(key, path):
    if key not in _MODS:
        _MODS[key] = _load("kf_matlab_rows_" + key, path)
    return _MODS[key]


def gpu_assert():
    return _mod("gpu_assert", WINDOWS / "gpu_assert.py")


def census_mod():
    return _mod("census", WINDOWS / "kf3_census.py")


def prop():
    return _mod("propimage_linux", HERE / "propimage_linux.py")


# ---------------------------------------------------------------- the rows
def _names_matmul():
    out = ["matmul_%s_%d" % (dt, n) for dt in ("single", "double") for n in (256, 512, 1024)]
    return out + ["matmul_single_4096_after_300", "matmul_double_2048_after_60"]


def _names_fft():
    out = []
    for dt in ("single", "double"):
        for n in (65536, 1048576, 30030, 10007):
            out += ["fft_%s_%d" % (dt, n), "roundtrip_%s_%d" % (dt, n)]
        out += ["fft2_%s_1024" % dt, "roundtrip2_%s_1024" % dt]
    return out + ["fft2_single_2048_after_200"]


def _names_solve():
    out = []
    for dt in ("single", "double"):
        for n in (512, 1024):
            for stem in ("mldivide", "mldivide_resid_gpu", "mldivide_resid_cpu", "lu_resid_gpu", "lu_resid_cpu", "qr_resid_gpu", "qr_orth_gpu",
                         "qr_resid_cpu", "chol_resid_gpu"):
                out.append("%s_%s_%d" % (stem, dt, n))
        out.append("lstsq_%s_2000x500" % dt)
    return out + ["mldivide_double_2048_after_100"]


# id -> (script, time-box seconds for the matlab process, host engines that must show use, the check names the script must report)
# The time-boxes are DESIGN values (an estimate for a nested box, never measured); no pass criterion is a time.
ROWS = {
    "gpu_info": ("gpu_info.m", 300, (), ("sum_ones_1e6",)),
    "gpu_matmul": ("gpu_matmul.m", 900, ("gpu", "mem"), tuple(_names_matmul())),
    "gpu_fft": ("gpu_fft.m", 900, ("gpu", "mem"), tuple(_names_fft())),
    "gpu_solve": ("gpu_solve.m", 1200, ("gpu", "mem"), tuple(_names_solve())),
    "dl_train": ("dl_train.m", 1800, ("gpu", "mem"), ("train_loss_curve", "final_mse")),
    "parpool_gpu": ("parpool_gpu.m", 1200, ("gpu", "mem"), ("num_workers", "distinct_pids", "workers_with_a_device", "worker_task_1", "worker_task_2",
                                                              "worker_task_3", "worker_task_4")),
    "cuda_kernel": ("cuda_kernel.m", 900, ("gpu",), ("vecadd_1000", "vecadd_1048573", "vecadd_10000000", "vecadd_after_40000_launches", "no_write_past_n",
                                                    "vecadd_first_n_of_guarded")),
    "mem_oversub": ("mem_oversub.m", 900, ("mem",), ("content_mismatches", "sample_chunk_1")),
}
PREAMBLE = "gpu_info"
ROW_ORDER = tuple(ROWS)
SUPPORT_FILES = ("kf_atleast.m", "kf_cmp.m", "kf_count.m", "kf_fail.m", "kf_fp.m", "kf_outdir.m", "kf_resid.m", "kf_tol.m", "kf_worker_task.m",
                 "kf_write.m", "vecadd.ptx")


def row_timeout(row):
    return ROWS[row][1]


def row_require(row):
    return ROWS[row][2]


# ---------------------------------------------------------------- the guest's JSON (hostile): bounded, strict
def _no_constant(name):
    raise ValueError("constant " + name)


def _no_dups(pairs):
    out = {}
    for k, v in pairs:
        if k in out:
            raise ValueError("duplicate key")
        out[k] = v
    return out


def num(v):
    """A finite JSON number as a float, else None (bool, NaN, Infinity, huge integers and strings are not numbers)."""
    if isinstance(v, bool) or not isinstance(v, (int, float)):
        return None
    try:
        f = float(v)
    except OverflowError:
        return None
    return f if math.isfinite(f) and abs(f) < 1e300 else None


def _bounded(value, depth=0):
    if depth > 8:
        raise BadResult("TOO_DEEP")
    if isinstance(value, dict):
        if len(value) > 64:
            raise BadResult("TOO_MANY_KEYS")
        for k, v in value.items():
            if not isinstance(k, str) or len(k) > 64:
                raise BadResult("BAD_KEY")
            _bounded(v, depth + 1)
    elif isinstance(value, list):
        if len(value) > MAX_LIST:
            raise BadResult("LIST_TOO_LONG")
        for v in value:
            _bounded(v, depth + 1)
    elif isinstance(value, str):
        if len(value) > MAX_STR:
            raise BadResult("STRING_TOO_LONG")


def parse_row_json(raw, row):
    """Bytes read from the guest -> the document. Raises BadResult(reason) for anything that is not exactly one bounded JSON object of this
    row's schema: too large, not UTF-8, NaN or Infinity, duplicate keys, nesting, wrong schema or row name, more than MAX_CHECKS checks."""
    if not isinstance(raw, (bytes, bytearray)) or not raw:
        raise BadResult("EMPTY")
    if len(raw) > MAX_JSON_BYTES:
        raise BadResult("TOO_LARGE")
    try:
        doc = json.loads(bytes(raw).decode("utf-8"), parse_constant=_no_constant, object_pairs_hook=_no_dups)
    except (ValueError, UnicodeError, RecursionError):
        raise BadResult("NOT_JSON") from None
    if not isinstance(doc, dict):
        raise BadResult("NOT_OBJECT")
    _bounded(doc)
    if doc.get("schema") != SCHEMA_ROW or doc.get("row") != row:
        raise BadResult("SCHEMA")
    if doc.get("status") not in ("done", "error"):
        raise BadResult("STATUS")
    checks = doc.get("checks")
    if not isinstance(checks, list) or len(checks) > MAX_CHECKS or not all(isinstance(c, dict) for c in checks):
        raise BadResult("CHECKS")
    return doc


# ---------------------------------------------------------------- host-side recomputation of every check
def tolerance_for(name, dtype):
    """The host's tolerance: zero for the exact checks (integer content, element-wise float add), else by precision. None = unknown dtype."""
    if name.startswith(EXACT_PREFIXES):
        return 0.0
    return TOLERANCE.get(dtype) if isinstance(dtype, str) else None


def _vec(v, n):
    if not isinstance(v, list) or len(v) != n:
        return None
    out = [num(x) for x in v]
    return None if any(x is None for x in out) else out


def check_cmp(item):
    """GPU versus CPU: relative 2-norm of the difference, and the four-number fingerprints, both recomputed from the raw numbers."""
    name = item.get("name")
    gpu, cpu, diff, ref = _vec(item.get("gpu"), 4), _vec(item.get("cpu"), 4), num(item.get("diff")), num(item.get("ref"))
    tol = tolerance_for(name, item.get("dtype"))
    if None in (gpu, cpu, diff, ref) or tol is None or diff < 0 or ref < 0:
        return False, "malformed cmp"
    rel = diff / ref if ref > 0 else diff
    scale = max(abs(x) for x in cpu) or 1.0
    fp = max(abs(g - c) for g, c in zip(gpu, cpu)) / scale
    ok = rel <= tol and fp <= tol
    return ok, "rel=%.3g fp=%.3g tol=%.3g" % (rel, fp, tol)


def check_resid(item):
    value, tol = num(item.get("value")), tolerance_for(item.get("name", ""), _dtype_of(item.get("name", "")))
    if value is None or tol is None or value < 0:
        return False, "malformed resid"
    return value <= tol, "value=%.3g tol=%.3g" % (value, tol)


def _dtype_of(name):
    """A residual's precision is in its name (`..._single_512`): the guest's own dtype field is not trusted."""
    for dt in TOLERANCE:
        if "_%s_" % dt in name:
            return dt
    return None


def check_count(item):
    """Counts of mismatching elements: the host's expectation is zero, whatever the script says it expected."""
    value, expected = num(item.get("value")), num(item.get("expected"))
    if value is None or expected is None:
        return False, "malformed count"
    return value == 0.0, "value=%g (guest expected %g, host expects 0)" % (value, expected)


def check_atleast(item):
    value, minimum = num(item.get("value")), num(item.get("min"))
    if value is None or minimum is None:
        return False, "malformed atleast"
    return value >= minimum, "value=%g min=%g" % (value, minimum)


MIN_ATLEAST = {"num_workers": 2.0, "distinct_pids": 2.0, "workers_with_a_device": 2.0}


def host_atleast(item):
    """The minimum comes from the host's table, not from the guest's own `min` (a guest cannot lower its own bar)."""
    ok, why = check_atleast(item)
    want = MIN_ATLEAST.get(item.get("name"), 1.0)
    value = num(item.get("value"))
    return (ok and value is not None and value >= want), why + " host_min=%g" % want


def check_series(item):
    gpu, cpu = item.get("gpu"), item.get("cpu")
    if not (isinstance(gpu, list) and isinstance(cpu, list) and len(gpu) >= 3 and len(cpu) >= 3):
        return False, "malformed series"
    g, c = [num(x) for x in gpu], [num(x) for x in cpu]
    if None in g or None in c or min(g + c) < 0:
        return False, "malformed series"
    ok = g[-1] < LOSS_FALL * g[0] and c[-1] < LOSS_FALL * c[0]
    return ok, "gpu %.3g->%.3g cpu %.3g->%.3g (must fall below %.0f%% of the first epoch)" % (g[0], g[-1], c[0], c[-1], LOSS_FALL * 100)


def check_pair(item):
    g, c = num(item.get("gpu")), num(item.get("cpu"))
    if g is None or c is None or g < 0 or c < 0:
        return False, "malformed pair"
    return abs(g - c) <= PAIR_ABS + PAIR_REL * max(abs(c), abs(g)), "gpu=%.4g cpu=%.4g" % (g, c)


KINDS = {"cmp": check_cmp, "resid": check_resid, "count": check_count, "atleast": host_atleast, "series": check_series, "pair": check_pair}


def verify_checks(row, doc):
    """-> (items, findings). items: [{name, kind, ok, why, flag_disagrees}], one per reported check, each recomputed here. A required check
    that is missing, a duplicate name, an unknown kind or a malformed item is a finding (BAD_RESULT), a recomputed miss is OUTPUT_MISMATCH."""
    items, findings, seen = [], [], set()
    for raw in doc["checks"]:
        name = raw.get("name")
        if not (isinstance(name, str) and NAME_RE.fullmatch(name)) or name in seen:
            findings.append(("BAD_RESULT", "BAD_CHECK_NAME"))
            continue
        seen.add(name)
        kind = raw.get("kind")
        fn = KINDS.get(kind) if isinstance(kind, str) else None
        if fn is None:
            findings.append(("BAD_RESULT", "BAD_KIND:" + name))
            continue
        ok, why = fn(raw)
        flag = raw.get("guest_pass")
        items.append({"name": name, "kind": raw["kind"], "ok": bool(ok), "why": clean(why, 80),
                      "flag_disagrees": isinstance(flag, bool) and flag != bool(ok) if raw["kind"] != "series" and raw["kind"] != "pair" else False})
        if not ok:
            findings.append(("OUTPUT_MISMATCH", "%s: %s" % (name, clean(why, 80))))
    for name in ROWS[row][3]:
        if name not in seen:
            findings.append(("BAD_RESULT", "MISSING_CHECK:" + name))
    return items, findings


def _norm_gpu_name(name):
    return re.sub(r"[^a-z0-9]", "", str(name).lower().replace("nvidia", "").replace("geforce", ""))


def verify_gpu_info(doc, hw):
    """The device facts MATLAB reported against the host's own: the name, the memory, the compute capability MATLAB supports."""
    info = doc.get("info")
    findings, facts = [], {}
    if not isinstance(info, dict):
        return [("BAD_RESULT", "NO_INFO")], facts
    name = info.get("name")
    cc = info.get("compute_capability")
    m = CC_RE.fullmatch(cc) if isinstance(cc, str) else None
    total, avail = num(info.get("total_memory")), num(info.get("available_memory"))
    drv, tk, devs = num(info.get("driver_version")), num(info.get("toolkit_version")), num(info.get("device_count"))
    rel = info.get("matlab_release")
    if not (isinstance(name, str) and name and m and total and total > 0 and avail is not None and 0 <= avail <= total and drv and drv > 0 and tk and tk > 0
            and devs and devs >= 1 and isinstance(rel, str) and re.fullmatch(r"\d{4}[ab]", rel) and info.get("can_use_gpu") is True):
        return [("BAD_RESULT", "BAD_INFO")], facts
    facts = {"name": clean(name, 64), "compute_capability": cc, "driver_version": drv, "toolkit_version": tk, "total_memory": int(total),
             "device_count": int(devs), "matlab_release": rel}
    if hw:
        if hw.get("gpu_name") and _norm_gpu_name(hw["gpu_name"]) != _norm_gpu_name(name):
            findings.append(("OUTPUT_MISMATCH", "GPU_NAME_DIFFERS host=%s guest=%s" % (clean(hw["gpu_name"], 40), clean(name, 40))))
        mib = hw.get("memory_mib")
        if isinstance(mib, int) and mib > 0 and not (0.5 * mib * 2 ** 20 <= total <= 1.01 * mib * 2 ** 20):
            findings.append(("OUTPUT_MISMATCH", "TOTAL_MEMORY_OUT_OF_RANGE guest=%d host_mib=%d" % (total, mib)))
    return findings, facts


def cc_x100(info_facts):
    """"8.9" -> 890 (the same x100 form as vast_payload.hardware's compute_cap); 0 when unreadable."""
    m = CC_RE.fullmatch(str(info_facts.get("compute_capability", "")))
    return int(m.group(1)) * 100 + int(m.group(2)) * 10 if m else 0


def verify_mem(doc):
    mem = doc.get("memory")
    if not isinstance(mem, dict):
        return [("BAD_RESULT", "NO_MEMORY")]
    v = {k: num(mem.get(k)) for k in ("total", "avail_start", "avail_after_alloc", "avail_end", "chunk_bytes", "chunks", "allocated_bytes")}
    if None in v.values() or v["avail_start"] <= 0 or v["chunks"] < 1 or v["chunk_bytes"] <= 0:
        return [("BAD_RESULT", "BAD_MEMORY")]
    out = []
    if v["allocated_bytes"] != v["chunks"] * v["chunk_bytes"]:
        out.append(("BAD_RESULT", "ALLOCATED_BYTES_INCONSISTENT"))
    ratio = v["allocated_bytes"] / v["avail_start"]
    if not MEM_FRACTION[0] <= ratio <= MEM_FRACTION[1]:
        out.append(("OUTPUT_MISMATCH", "ALLOCATED_FRACTION %.2f outside %.2f..%.2f" % (ratio, *MEM_FRACTION)))
    if v["avail_start"] - v["avail_after_alloc"] < 0.5 * v["allocated_bytes"]:
        out.append(("OUTPUT_MISMATCH", "ALLOCATION_NOT_VISIBLE_IN_FREE_MEMORY"))
    if v["avail_end"] < MEM_RETURNED * v["avail_start"]:
        out.append(("OUTPUT_MISMATCH", "MEMORY_NOT_RETURNED %.2f of start" % (v["avail_end"] / v["avail_start"])))
    return out


def verify_doc(row, doc, hw=None):
    """One row's result document -> {ok, klass, findings (at most 8), n_checks, failed, flag_disagrees, facts}. klass: None, ASSERT (the script threw),
    BAD_RESULT (unusable) or OUTPUT_MISMATCH (a recomputed number missed). An error document is an ASSERT with the error id only."""
    if doc["status"] == "error":
        eid = doc.get("error_id")
        return {"ok": False, "klass": "ASSERT", "findings": ["ERROR_ID:" + (clean(eid, 80) if isinstance(eid, str) and re.fullmatch(r"[A-Za-z0-9:_.]{0,80}", eid) else "?")],
                "n_checks": 0, "failed": [], "flag_disagrees": 0, "facts": {}}
    items, found = verify_checks(row, doc)
    facts = {}
    if row == "gpu_info":
        extra, facts = verify_gpu_info(doc, hw)
        found += extra
    elif row == "mem_oversub":
        found += verify_mem(doc)
    elif row == "dl_train":
        ep = num(doc.get("epochs"))
        if ep is None or ep < 3:
            found.append(("BAD_RESULT", "BAD_EPOCHS"))
    klass = "BAD_RESULT" if any(k == "BAD_RESULT" for k, _ in found) else ("OUTPUT_MISMATCH" if found else None)
    return {"ok": not found, "klass": klass, "findings": [clean(t, 100) for _, t in found][:8], "n_checks": len(items),
            "failed": [i["name"] for i in items if not i["ok"]][:8], "flag_disagrees": sum(1 for i in items if i["flag_disagrees"]), "facts": facts}


# ---------------------------------------------------------------- GPU use from the HOST counters
def assess_gpu_use(row, query_text, dmon_text, **thresholds):
    """The host's nvidia-smi samples of the row's window -> the record of gpu_assert.evaluate (source host). No sample at all means the counters could
    not be read: that is `unreadable`, a HARNESS matter, never a pass and never a GPU_NOT_USED finding about the guest."""
    ga = gpu_assert()
    required = row_require(row)
    q, d = ga.parse_query_csv(query_text or ""), ga.parse_dmon(dmon_text or "")
    rec = ga.evaluate(q, d, required, **thresholds)
    rec["source"] = "host"
    rec["name"] = row
    rec["unreadable"] = bool(required) and rec.get("samples", 0) == 0 and rec.get("dmon_samples", 0) == 0
    return rec


# ---------------------------------------------------------------- the verdict of one row
def judge(row, execution, parsed, verified, gpu, census, host_xid):
    """-> (verdict, failure_class, reason). The order is the point: a guest that is gone or hung is judged before its numbers, host Xids before a
    PASS, the numbers before the GPU-use assertion, and a GPU counter that could not be read is HARNESS (the row could not be judged).
    execution {rc, timed_out, lost}; parsed: the document or None; verified: verify_doc's dict or None; gpu: assess_gpu_use record or None."""
    xid = (host_xid or 0) + int((census or {}).get("xid", 0) or 0)
    if execution.get("lost") and not parsed:
        return "FAIL", "CRASH", "GUEST_LOST"
    if execution.get("timed_out"):
        return "FAIL", "HANG", "ROW_TIMEBOX"
    if xid:
        return "FAIL", "XID", "HOST_XID:%d" % xid
    if verified is None:
        rc = execution.get("rc")
        return "FAIL", ("CRASH" if rc not in (0, None) or execution.get("lost") else "BAD_RESULT"), "NO_USABLE_RESULT rc=%s" % rc
    if not verified["ok"]:
        return "FAIL", verified["klass"], (verified["findings"] or ["?"])[0]
    if execution.get("rc") not in (0, None):
        return "FAIL", "ASSERT", "MATLAB_EXIT:%s" % execution["rc"]
    if row_require(row):
        if gpu is None or gpu.get("unreadable"):
            return "HARNESS", None, "HOST_COUNTERS_MISSING"
        if gpu["verdict"] != "PASS":
            return "FAIL", "GPU_NOT_USED", clean((gpu.get("reasons") or ["?"])[0], 100)
    return "PASS", None, ""


def grade(rows, strict):
    """-> (verdict, reason, exit_code). A gate-lane failure always fails; a discovery failure never does; HARNESS is RETRY (not an application
    verdict); NOT_RUN is a skip: fine standalone, a failure under strict (zero skips)."""
    gate_fail = [r for r in rows if r["lane"] == "gate" and r["verdict"] == "FAIL"]
    if gate_fail:
        return "FAIL", "APP_FAIL:" + gate_fail[0]["row"], 1
    harness = [r for r in rows if r["verdict"] == "HARNESS"]
    if any(r["verdict"] == "NOT_RUN" and r["reason"] == "TIME" for r in rows):
        return "RETRY", "TIME", 3
    if harness:
        return "RETRY", "HARNESS:" + harness[0]["row"], 3
    skipped = [r for r in rows if r["verdict"] == "NOT_RUN"]
    if strict and skipped:
        return "FAIL", "STRICT_SKIP:%s:%s" % (skipped[0]["row"], skipped[0]["reason"]), 4
    if not any(r["verdict"] in ("PASS", "FAIL") for r in rows):
        return "SKIPPED", "NOTHING_RAN", 0
    return "PASS", "", 0


def breakages(rows):
    return [{"row": r["row"], "class": r["failure_class"], "reason": r["reason"], "host_xid": r.get("host_xid") or 0}
            for r in rows if r["lane"] == "discovery" and r["verdict"] == "FAIL"]


# ---------------------------------------------------------------- one row, one boot
class Execution(dict):
    """{rc, timed_out, lost, secs}: what running the row on the guest did (rc None when the transport itself failed)."""


def remote_command(row, timeout_s, matlab=MATLAB_BIN):
    """The one command line the guest runs for a row. The matlab process is time-boxed by `timeout` (TERM, then KILL 20 s later); its output goes to
    a file in the guest that nothing reads; only the exit code comes back."""
    assert row in ROWS and BIN_RE.fullmatch(matlab) and isinstance(timeout_s, int) and 0 < timeout_s <= 7200   # nothing here is ever shell-quoted
    script = "%s/%s.m" % ("$HOME/" + GUEST_SCRIPTS, row)
    return ("mkdir -p $HOME/%s && rm -f $HOME/%s/%s.json && KF_ROWS_OUT=$HOME/%s timeout -k 20 %d %s -batch \"run('%s')\" >/var/tmp/kf-%s.log 2>&1; echo KF_RC=$?"
            % (GUEST_OUT, GUEST_OUT, row, GUEST_OUT, timeout_s, matlab, script, row))


def parse_rc(stdout):
    m = re.findall(r"^KF_RC=(\d{1,3})$", stdout or "", re.M)
    return int(m[-1]) if len(m) == 1 else None


def run_row(sess, row, ctx):
    """Run one row on a booted, driver-installed, injected guest and judge it. Returns the row's record (text only, bounded)."""
    spec_timeout = row_timeout(row)
    lane = ctx.lanes.get(row, "discovery")
    rec = {"row": row, "lane": lane, "verdict": "HARNESS", "failure_class": None, "reason": "", "rc": None, "secs": 0, "checks": {"n": 0, "failed": [], "flag_disagrees": 0},
           "gpu": {}, "census": {}, "host_xid": None, "findings": [], "facts": {}, "result": {"sha256": None, "bytes": 0}}
    probe = ctx.probe_factory(Path(ctx.evidence_dir or ctx.workdir) / row) if row_require(row) else None
    started_probe = bool(probe and probe.start())
    ctx.sleep(ctx.settle_s if started_probe else 0)
    qlog = sess.qemu_log()
    edge0 = ctx.census.count_lines(qlog) if qlog else 0
    xid0 = ctx.xid_mark()
    t0 = ctx.clock()
    execution = Execution(rc=None, timed_out=False, lost=False, secs=0)
    try:
        res = sess.exec(remote_command(row, spec_timeout, ctx.matlab), spec_timeout + 60)
        execution["rc"] = parse_rc(res[1]) if res[0] == 0 else None
        execution["timed_out"] = execution["rc"] in (124, 137)
        execution["lost"] = execution["rc"] is None        # ssh failed, or its output was not exactly one KF_RC line: the guest is not trusted to say
    except TimeoutError:
        execution["timed_out"] = True
    except OSError:
        execution["lost"] = True
    execution["secs"] = int(ctx.clock() - t0)
    ctx.sleep(ctx.settle_s if started_probe else 0)
    texts = probe.stop() if started_probe else None
    window = ctx.census.window(qlog, edge0 + 1, None) if qlog else []
    census = ctx.census.census(window)
    xid = ctx.xid_since(xid0)
    parsed, verified, raw = None, None, None
    if not execution["lost"]:
        try:
            raw = sess.fetch(GUEST_OUT + "/" + row + ".json", MAX_JSON_BYTES + 1)
        except (OSError, TimeoutError):
            raw, execution["lost"] = None, True
    if raw is not None:
        rec["result"] = {"sha256": hashlib.sha256(raw).hexdigest(), "bytes": len(raw)}
        try:
            parsed = parse_row_json(raw, row)
            verified = verify_doc(row, parsed, ctx.hw)
        except BadResult as exc:
            verified = {"ok": False, "klass": "BAD_RESULT", "findings": [exc.reason], "n_checks": 0, "failed": [], "flag_disagrees": 0, "facts": {}}
            parsed = None
    gpu = assess_gpu_use(row, texts[0], texts[1]) if texts is not None else None
    verdict, klass, reason = judge(row, execution, parsed, verified, gpu, census, xid)
    rec.update(verdict=verdict, failure_class=klass, reason=clean(reason, 120), rc=execution["rc"], secs=execution["secs"], host_xid=xid)
    if verified:
        rec["checks"] = {"n": verified["n_checks"], "failed": verified["failed"], "flag_disagrees": verified["flag_disagrees"]}
        rec["findings"] = verified["findings"]
        rec["facts"] = verified["facts"]
    if gpu:
        rec["gpu"] = {"required": list(gpu.get("required", [])), "verdict": gpu.get("verdict"), "samples": gpu.get("samples"), "peak": {k: v for k, v in (gpu.get("peak") or {}).items() if num(v) is not None or v is None},
                      "baseline": gpu.get("baseline"), "reasons": [clean(x, 100) for x in (gpu.get("reasons") or [])][:4], "source": "host"}
    rec["census"] = {"lines": census.get("lines", 0), "counts": census.get("counts", {}), "xid": census.get("xid", 0)}
    if ctx.evidence_dir:
        write_evidence(Path(ctx.evidence_dir) / row, rec, parsed, texts)
    return rec, execution


KEEP_INFO = ("name", "compute_capability", "driver_version", "toolkit_version", "total_memory", "available_memory", "multiprocessors", "device_count", "matlab_release",
             "can_use_gpu")
KEEP_MEMORY = ("total", "avail_start", "avail_after_alloc", "avail_end", "chunk_bytes", "chunks", "allocated_bytes")
KEEP_ITEM = ("name", "kind", "dtype", "numel", "tol", "gpu", "cpu", "diff", "ref", "value", "expected", "min", "guest_pass")


def _plain(v):
    """A number, a bool, or a short printable string; a list of those (bounded). Anything else is dropped."""
    if isinstance(v, bool) or v is None:
        return v
    if num(v) is not None:
        return v
    if isinstance(v, str):
        return clean(v, 64)
    if isinstance(v, list):
        return [_plain(x) for x in v[:MAX_LIST] if not isinstance(x, (dict, list))]
    return None


def sanitize_result(doc):
    """The part of a USABLE result worth keeping as evidence, rebuilt from a whitelist of keys: the guest may have added anything, and none of it is kept."""
    out = {"schema": SCHEMA_ROW, "row": doc.get("row"), "status": doc.get("status"),
           "checks": [{k: _plain(c.get(k)) for k in KEEP_ITEM if k in c} for c in doc.get("checks", [])[:MAX_CHECKS]]}
    for key, keep in (("info", KEEP_INFO), ("memory", KEEP_MEMORY)):
        if isinstance(doc.get(key), dict):
            out[key] = {k: _plain(v) for k, v in doc[key].items() if k in keep and not isinstance(v, (dict, list))}
    if num(doc.get("epochs")) is not None:
        out["epochs"] = doc["epochs"]
    return out


def write_evidence(directory, rec, parsed, texts):
    """Text only: the verdict, the re-serialised numbers of a USABLE result (never the raw guest bytes), the host's sampler output (bounded)."""
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "verdict.json").write_text(json.dumps(rec, indent=1, sort_keys=True) + "\n")
    if parsed is not None:
        (directory / "result.json").write_text(json.dumps(sanitize_result(parsed), indent=1, sort_keys=True)[:MAX_JSON_BYTES] + "\n")
    if texts:
        (directory / "host_query.csv").write_text((texts[0] or "")[:200000])
        (directory / "host_dmon.txt").write_text((texts[1] or "")[:200000])


# ---------------------------------------------------------------- the run
class Context:
    """The run's collaborators, every one replaceable (the tests pass fakes). `probe_factory(dir)` -> an object with start() and stop();
    `xid_mark()` and `xid_since(mark)` count host kernel Xid lines (None when unknown); `census` is kf3_census (count_lines, window, census)."""

    def __init__(self, lanes=None, hw=None, probe_factory=None, evidence_dir=None, workdir=".", sleep=time.sleep, clock=time.monotonic, settle_s=2.0,
                 census=None, xid_mark=None, xid_since=None, deadline_epoch=0.0, now=time.time, isolation="row", strict=False, matlab=MATLAB_BIN):
        self.lanes, self.hw, self.evidence_dir, self.workdir = dict(lanes or {}), hw or {}, evidence_dir, workdir
        self.probe_factory = probe_factory or (lambda d: gpu_assert().HostProbe(d))
        self.sleep, self.clock, self.settle_s = sleep, clock, settle_s
        self.census = census or census_mod()
        self.xid_mark = xid_mark or (lambda: None)
        self.xid_since = xid_since or (lambda mark: None)
        self.deadline_epoch, self.now, self.isolation, self.strict, self.matlab = deadline_epoch, now, isolation, strict, matlab


def plan_boots(rows, isolation):
    """Each row boots alone in a fresh clone (the runner's discovery rule) or all rows share one boot (`shared`: quicker, a crashed row then
    sends the remaining rows to a fresh boot). The preamble is always first."""
    ordered = [PREAMBLE] + [r for r in rows if r != PREAMBLE]
    return [[r] for r in ordered] if isolation == "row" else [ordered]


def not_run_record(row, lane, reason):
    return {"row": row, "lane": lane, "verdict": "NOT_RUN", "failure_class": None, "reason": clean(reason, 120), "rc": None, "secs": 0,
            "checks": {"n": 0, "failed": [], "flag_disagrees": 0}, "gpu": {}, "census": {}, "host_xid": None, "findings": [], "facts": {},
            "result": {"sha256": None, "bytes": 0}}


def refused_record(row, lane, refusal):
    rec = not_run_record(row, lane, refusal.reason)
    rec["verdict"] = refusal.verdict
    return rec


def run_rows(rows, make_session, ctx, log=print):
    """-> {rows: [...], boots: [...]}. A boot that is refused gives each of its rows the refusal (NOT_RUN or HARNESS) and the run goes on with the
    next boot. After the preamble: no usable GPU (GPU_INFO_FAILED) or a compute capability MATLAB does not support (GPU_UNSUPPORTED) -> the rest are NOT_RUN."""
    out, boots, gate_reason, harness_refusals = [], [], None, 0
    pending = [list(b) for b in plan_boots(rows, ctx.isolation)]
    while pending:
        group = pending.pop(0)
        if gate_reason:
            out += [not_run_record(r, ctx.lanes.get(r, "discovery"), gate_reason) for r in group]
            continue
        need = sum(row_timeout(r) for r in group) + BOOT_SLACK_S
        if ctx.deadline_epoch and ctx.now() + need + 900 > ctx.deadline_epoch:
            out += [not_run_record(r, ctx.lanes.get(r, "discovery"), "TIME") for r in group]
            continue
        boot = {"index": len(boots) + 1, "rows": list(group), "secs": 0, "refused": None}
        boots.append(boot)
        t0 = ctx.clock()
        sess = make_session(boot["index"])
        try:
            try:
                sess.boot()
                sess.install_driver()
                sess.inject(SCRIPTS_DIR)
            except BootRefusal as refusal:
                boot["refused"] = refusal.reason
                out += [refused_record(r, ctx.lanes.get(r, "discovery"), refusal) for r in group]
                harness_refusals = harness_refusals + 1 if refusal.verdict == "HARNESS" else 0
                if refusal.verdict == "NOT_RUN":
                    gate_reason = refusal.reason          # a missing driver, GPU or verb is the same for every boot: do not repeat it
                elif harness_refusals >= 2:
                    gate_reason = "HARNESS_REPEATED"      # two boots in a row could not be set up: stop paying for boots
                continue
            for i, row in enumerate(group):
                rec, execution = run_row(sess, row, ctx)
                out.append(rec)
                log("KF_MATLAB_ROW %s lane=%s verdict=%s class=%s reason=%s" % (row, rec["lane"], rec["verdict"], rec["failure_class"] or "-", rec["reason"] or "-"))
                if row == PREAMBLE:
                    gate_reason = gate_after_preamble(rec)
                if rec["verdict"] == "FAIL" and rec["failure_class"] in ("CRASH", "HANG", "XID") and i + 1 < len(group):
                    pending.insert(0, group[i + 1:])    # a guest that crashed, hung or raised an Xid does not run the next row: it gets a fresh boot
                    break
                if gate_reason:
                    out += [not_run_record(r, ctx.lanes.get(r, "discovery"), gate_reason) for r in group[i + 1:]]
                    break
        finally:
            boot["secs"] = int(ctx.clock() - t0)
            try:
                sess.shutdown()
            except Exception:  # noqa: BLE001 - a shutdown that fails must not hide the row records
                pass
    out.sort(key=lambda r: ROW_ORDER.index(r["row"]) if r["row"] in ROW_ORDER else 99)
    return {"rows": out, "boots": boots}


def gate_after_preamble(rec):
    """The reason the rest of the rows cannot run, from the preamble's record, or None."""
    if rec["verdict"] != "PASS":
        return "GPU_INFO_FAILED"
    if cc_x100(rec.get("facts") or {}) < MIN_CC_X100:
        return "GPU_UNSUPPORTED"
    return None


def build_record(result, ctx, sha, image, driver, family=None):
    rows = result["rows"]
    verdict, reason, code = grade(rows, ctx.strict)
    counts = {}
    for r in rows:
        counts[r["verdict"]] = counts.get(r["verdict"], 0) + 1
    rec = {"schema": SCHEMA_RESULT, "verdict": verdict, "reason": reason, "sha": sha, "strict": ctx.strict, "isolation": ctx.isolation,
           "gpu": {k: ctx.hw.get(k) for k in ("gpu_name", "compute_cap", "memory_mib")}, "family": family, "image": image, "driver": driver,
           "rows": rows, "breakages": breakages(rows), "boots": result["boots"],
           "summary": "rows %d: %s" % (len(rows), ", ".join("%s %d" % (k, counts[k]) for k in sorted(counts))), "exit_code": code}
    return rec


def serialise(rec):
    """One bounded JSON line; the evidence and census detail shrink before the verdict does."""
    data = json.dumps(rec, separators=(",", ":"), sort_keys=True)
    for r in rec["rows"]:
        if len(data.encode()) <= MAX_RECORD_BYTES:
            break
        r["findings"], r["gpu"] = r["findings"][:2], {k: v for k, v in r["gpu"].items() if k in ("required", "verdict", "samples")}
        data = json.dumps(rec, separators=(",", ":"), sort_keys=True)
    return data


# ---------------------------------------------------------------- the real transport (RUN ON NO BOX; thin on purpose)
STATE_ERRORS = (("DRIVER_SHA_MISMATCH", "HARNESS"), ("KERNEL_CC_MISSING", "HARNESS"), ("GUEST_DRIVER_MISMATCH", "NOT_RUN"),
                ("SSH_TIMEOUT", "HARNESS"), ("HOST_MEMORY_LOW", "NOT_RUN"), ("GUEST_DRIVER_REQUIRED", "HARNESS"))


def refusal_from(exc):
    """A propimage_linux PropError (its message starts with a named token) or a ClockError -> BootRefusal. GUEST_DRIVER_MISMATCH is the guest having no
    working driver (NOT_RUN GUEST_NO_DRIVER); a clock refusal is a harness problem."""
    text = str(exc)
    for token, verdict in STATE_ERRORS:
        if text.startswith(token):
            return BootRefusal("GUEST_NO_DRIVER" if token == "GUEST_DRIVER_MISMATCH" else token, verdict)
    if "KF_CLOCK" in text or exc.__class__.__name__ == "ClockError":
        return BootRefusal("CLOCK_REFUSED", "HARNESS")
    return BootRefusal("BOOT_FAILED", "HARNESS")


class LinuxSession:
    """One fresh throwaway clone of the sealed image on this box: offline, recorded clock, the seed identity, the kf3 GPU device with the run's
    driver version. Uses propimage_linux's own functions (build_argv, clone_cmd, wait_ssh, run_probe, compare, install_driver) and its `inject` verb."""

    def __init__(self, st, p, name, qemu, driver_run, driver_sha256, created_utc=None, popen=subprocess.Popen, runner=subprocess.run, log=print):
        self.st, self.p, self.name, self.qemu = st, p, name, str(qemu)
        self.driver_run, self.driver_sha, self.created = str(driver_run), driver_sha256, created_utc
        self.popen, self.runner, self.log, self.proc, self.wd, self.t0, self.logf = popen, runner, log, None, None, None, None
        self.pl = prop()

    def qemu_log(self):
        return self.wd / "qemu.log" if self.wd else None

    def boot(self):
        pl = self.pl
        try:
            pl.guard_memory()
            version = pl.driver_version_from_name(self.driver_run)
            self.wd = self.p["run"] / self.name
            shutil.rmtree(self.wd, ignore_errors=True)
            (self.wd / "firmware").mkdir(parents=True)
            pl.restore_files(self.st, self.wd)
            self.runner(pl.clone_cmd(self.p["sealed"] / "image.qcow2", self.wd / "clone.qcow2"), check=True, capture_output=True)
            clock = pl.driver_clock(self.st, self.driver_run, self.created)       # the recorded clock, never earlier than the installer's creation time
            argv = pl.build_argv(self.st, "run", self.wd / "clone.qcow2", self.wd, net="offline", clock=clock, qemu=self.qemu, gpu="kf3",
                                 driver_version=version, daemonize=False)
            argv += ["-pidfile", str(self.wd / "qemu.pid")]
            self.logf = open(self.wd / "qemu.log", "ab")
            self.t0 = time.monotonic()
            self.proc = self.popen(argv, stdout=self.logf, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL, start_new_session=True)
            pl.wait_ssh(self.st, self.p["key"])
            self.identity("pre-driver")
        except BootRefusal:
            raise
        except Exception as exc:  # noqa: BLE001 - propimage_linux raises PropError, ClockError, OSError, CalledProcessError
            raise refusal_from(exc) from None

    def identity(self, when):
        pl = self.pl
        rows = pl.compare(self.st, pl.run_probe(self.st, self.p["key"], runner=self.runner), elapsed_s=time.monotonic() - self.t0, net="offline")
        bad = [r for r in rows if r[1] == "MISMATCH" and r[0] not in ("kernel_headers", "driver_build_tools", "cloud_init_disabled")]
        if bad:
            raise BootRefusal("IDENTITY_MISMATCH:" + bad[0][0], "HARNESS")

    def install_driver(self):
        try:
            self.pl.install_driver(self.st, self.p["key"], self.driver_run, self.driver_sha, self.created, runner=self.runner, log=self.log)
            self.identity("post-driver")        # the guest's GPU UUID must now equal the seed's (the kf3 device property)
        except BootRefusal:
            raise
        except Exception as exc:  # noqa: BLE001
            raise refusal_from(exc) from None

    def inject(self, src):
        if not hasattr(self.pl, "cmd_inject"):
            raise BootRefusal("INJECT_UNAVAILABLE", "NOT_RUN")       # the `inject` verb is propimage_linux's (docs/design/V3_PROPIMAGE_LINUX.md section 13)
        cmd = [sys.executable, "-B", "-I", str(HERE / "propimage_linux.py"), "inject", "--dir", str(self.p["dir"]), "--name", self.name,
               "--src", str(src), "--dest", GUEST_SCRIPTS, "--replace"]
        done = self.runner(cmd, capture_output=True, text=True, timeout=600)
        if done.returncode != 0:
            raise BootRefusal("INJECT_FAILED", "HARNESS")

    def exec(self, command, timeout):
        try:
            done = self.runner(self.pl.ssh_argv(self.st, self.p["key"], command), capture_output=True, text=True, timeout=timeout)
        except subprocess.TimeoutExpired:
            raise TimeoutError("ssh") from None
        return done.returncode, done.stdout[-4096:]

    def fetch(self, remote, max_bytes):
        """The first max_bytes of one file under the guest user's home, as bytes; None when it does not exist."""
        if not re.fullmatch(r"kfrows/out/[A-Za-z0-9_]{1,40}\.json", remote):
            raise OSError("path")
        argv = self.pl.ssh_argv(self.st, self.p["key"], "test -f $HOME/%s && head -c %d $HOME/%s" % (remote, max_bytes, remote))
        done = self.runner(argv, capture_output=True, timeout=120)
        return done.stdout if done.returncode == 0 else None

    def shutdown(self):
        try:
            if self.proc:
                self.pl.stop_clone(self.st, self.p, self.wd, self.proc.pid)
        finally:
            if self.logf:
                self.logf.close()


# ---------------------------------------------------------------- the image on disk
def prepare_dir(image, workdir):
    """A packed image zip -> a state directory `propimage_linux` commands understand: <w>/state.json, <w>/sealed/image.qcow2, <w>/ssh/id_ed25519 (0600,
    from the state: it is the throwaway key of the local transport). Returns (state, paths). The zip's members are sha256-checked by unpack."""
    pl = prop()
    w = Path(workdir)
    sealed = w / "sealed"
    st = pl.unpack(image, sealed)
    shutil.copyfile(sealed / "state.json", w / "state.json")
    (w / "ssh").mkdir(parents=True, exist_ok=True)
    key = w / "ssh" / "id_ed25519"
    key.write_text(st["guest"]["ssh_private_key"])
    key.chmod(0o600)
    return st, pl.paths(w)


def host_gpu_facts(runner=subprocess.run):
    """gpu_name, compute_cap (x100), memory_mib of the host's GPU, or {} when nvidia-smi says nothing usable."""
    try:
        done = runner(["nvidia-smi", "--query-gpu=name,compute_cap,memory.total", "--format=csv,noheader,nounits"], capture_output=True, text=True, timeout=30)
        lines = [l for l in done.stdout.strip().splitlines() if l.strip()]
        name, cc, mem = [x.strip() for x in lines[0].split(",")]
        if done.returncode or len(lines) != 1:
            return {}
        return {"gpu_name": clean(name, 64), "compute_cap": int(round(float(cc) * 100)), "memory_mib": int(float(mem))}
    except (OSError, subprocess.SubprocessError, ValueError, IndexError):
        return {}


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 22), b""):
            h.update(chunk)
    return h.hexdigest()


def preflight_refusal(image_path, hw, qemu, kvm="/dev/kvm"):
    """The cheap reasons a run cannot start at all (each is NOT_RUN for every row): -> reason or None."""
    if not image_path or not Path(image_path).exists():
        return "IMAGE_MISSING"
    if not hw or not hw.get("gpu_name"):
        return "NO_GPU"
    if not os.access(kvm, os.R_OK | os.W_OK):
        return "KVM_MISSING"
    if not qemu or not os.access(qemu, os.X_OK):
        return "QEMU_MISSING"
    return None


def parse_lanes(items, rows):
    lanes = {r: "discovery" for r in rows}
    for item in items or []:
        row, _, lane = item.partition("=")
        if row not in ROWS or lane not in LANES:
            raise ValueError("bad --lane " + clean(item, 40))
        lanes[row] = lane
    return lanes


def select_rows(text):
    rows = [r for r in (text or "").split(",") if r] or [r for r in ROW_ORDER if r != PREAMBLE]
    for r in rows:
        if r not in ROWS:
            raise ValueError("unknown row " + clean(r, 40))
    return [r for r in ROW_ORDER if r in rows and r != PREAMBLE]


def cmd_plan(_):
    for r in ROW_ORDER:
        script, timeout, require, checks = ROWS[r]
        print("%-12s %-16s time-box %4ds  host-asserted use: %-9s %d checks" % (r, script, timeout, ",".join(require) or "-", len(checks)))
    return 0


def cmd_verify(a):
    raw = Path(a.result).read_bytes()[:MAX_JSON_BYTES + 1]
    hw = {"gpu_name": a.host_gpu_name, "memory_mib": a.host_mem_mib} if a.host_gpu_name else {}
    try:
        doc = parse_row_json(raw, a.row)
    except BadResult as exc:
        print("BAD_RESULT " + exc.reason)
        return 1
    v = verify_doc(a.row, doc, hw)
    print("%s %s checks=%d failed=%s findings=%s" % ("OK" if v["ok"] else "FAIL:" + str(v["klass"]), a.row, v["n_checks"], ",".join(v["failed"]) or "-", "; ".join(v["findings"]) or "-"))
    return 0 if v["ok"] else 1


def check_pin(image_sha, pin, strict):
    """A user-supplied pin on an image is enforced under --strict only; otherwise the computed sha256 is recorded and nothing is refused."""
    return "IMAGE_SHA_MISMATCH" if (strict and pin and image_sha != pin) else None


def cmd_run(a):
    rows = select_rows(a.rows)
    lanes = parse_lanes(a.lane, ROW_ORDER)
    hw = host_gpu_facts()
    image = a.image or str(Path(a.dir) / "sealed" / "image.qcow2")
    ctx = Context(lanes=lanes, hw=hw, evidence_dir=a.evidence_dir, workdir=a.workdir, isolation=a.isolation, strict=a.strict, deadline_epoch=a.deadline_epoch or 0.0, matlab=a.matlab)
    refusal = preflight_refusal(image, hw, a.qemu)
    image_rec = {"sha256": None, "size": None, "source": "user", "pin_enforced": bool(a.image_sha256 and a.strict)}
    driver_rec = {"version": None, "sha256": a.driver_sha256, "source": "user"}
    if refusal is None and not Path(a.driver_run).is_file():
        refusal = "DRIVER_MISSING"
    if refusal is None and a.image:
        image_rec.update(sha256=sha256_file(a.image), size=Path(a.image).stat().st_size)     # computed and recorded: a user-supplied image is not pinned
        refusal = check_pin(image_rec["sha256"], a.image_sha256, a.strict)
    if refusal is not None:
        result = {"rows": [not_run_record(r, lanes.get(r, "discovery"), refusal) for r in [PREAMBLE] + rows], "boots": []}
    else:
        if a.image:
            st, p = prepare_dir(a.image, a.workdir)
        else:
            pl = prop()
            p = pl.paths(a.dir)
            st = pl.load_state(p["state"])
        driver_rec["version"] = prop().driver_version_from_name(a.driver_run)
        result = run_rows(rows, lambda index: LinuxSession(st, p, "matlab%d" % index, a.qemu, a.driver_run, a.driver_sha256, a.driver_created_utc), ctx)
    rec = build_record(result, ctx, a.sha, image_rec, driver_rec)
    data = serialise(rec)
    if a.out:
        Path(a.out).write_text(data + "\n")
    print("%s %s" % (RESULT_LINE, data), flush=True)
    return rec["exit_code"]


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("plan")
    v = sub.add_parser("verify")
    v.add_argument("result")
    v.add_argument("--row", required=True, choices=ROW_ORDER)
    v.add_argument("--host-gpu-name")
    v.add_argument("--host-mem-mib", type=int)
    r = sub.add_parser("run")
    r.add_argument("--image")
    r.add_argument("--image-sha256", help="a pin for --image: enforced with --strict only (otherwise the computed sha256 is just recorded)")
    r.add_argument("--dir")
    r.add_argument("--workdir", required=True)
    r.add_argument("--qemu", required=True)
    r.add_argument("--driver-run", required=True)
    r.add_argument("--driver-sha256", required=True)
    r.add_argument("--driver-created-utc")
    r.add_argument("--rows")
    r.add_argument("--matlab", default=MATLAB_BIN, help="the matlab binary in the guest (a path or a name on its PATH)")
    r.add_argument("--lane", action="append", default=[])
    r.add_argument("--isolation", choices=("row", "shared"), default="row")
    r.add_argument("--strict", action="store_true")
    r.add_argument("--sha", default="0" * 40)
    r.add_argument("--out")
    r.add_argument("--evidence-dir")
    r.add_argument("--deadline-epoch", type=float, default=0.0)
    a = ap.parse_args(argv)
    if a.cmd == "run" and bool(a.image) == bool(a.dir):
        ap.error("give exactly one of --image and --dir")
    try:
        return {"plan": cmd_plan, "verify": cmd_verify, "run": cmd_run}[a.cmd](a)
    except (ValueError, OSError) as exc:
        print("KF_MATLAB_USAGE " + clean(exc, 100), flush=True)
        return 2


if __name__ == "__main__":
    sys.exit(main())
