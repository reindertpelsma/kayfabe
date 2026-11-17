#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The decisions of the three open ML rows (vllm, tensorrt, jax): pure functions, stdlib only, GPU-free tested (scripts/ci/test_mlrows.py).

STATUS: LIVE, 2026-11-17 (docs/design/V3_OPEN_ML_ROWS.md). Everything here is a decision over text a machine printed: the driver and GPU facts, the
flavour a driver can take, the hashed locks, the model files, the result one row script printed, and the host-against-guest comparison. The runner
(mlrows.py) does the I/O and calls these; a row script (row_*.py) only measures and prints one `MLROW_JSON` line, it never decides a verdict.

Vocabulary (the same as kfml's and the other open rows'): PASS, FAIL, NOT_RUN. NOT_RUN is a statement about the machine or our own configuration, with a
named reason (the GPU is older than Turing, the driver cannot take the wheel, the environment was not set up), never a statement about kayfabe.
"""
import hashlib
import json
import re
from pathlib import Path

HERE = Path(__file__).resolve().parent
PASS, FAIL, NOT_RUN = "PASS", "FAIL", "NOT_RUN"
ROWS = ("vllm", "tensorrt", "jax")
MAX_TEXT = 1 << 22              # one log read at most (the guest is hostile: its output is bounded before any regex runs)
MAX_JSON = 1 << 20              # one MLROW_JSON line
JSON_PREFIX = "MLROW_JSON "


def load_pins(path=None):
    return json.loads(Path(path or HERE / "pins.json").read_text())


# ------------------------------------------------------------------------------------------------ the machine's facts
SMI_CUDA_RX = re.compile(r"CUDA Version:\s*(\d+)\.(\d+)")
SMI_DRIVER_RX = re.compile(r"Driver Version:\s*([0-9.]+)")
# NVIDIA's CUDA release notes: the first driver of a CUDA major line (only used when nvidia-smi does not print `CUDA Version`)
DRIVER_FOR_CUDA = ((13, 0, (580, 65)), (12, 0, (525, 60)))


def version_tuple(text):
    return tuple(int(x) for x in re.findall(r"\d+", str(text))[:3])


def parse_smi_header(text):
    """`nvidia-smi` (the default table) -> {driver, cuda (major, minor) or None}. The CUDA version is the highest the driver supports."""
    text = (text or "")[:MAX_TEXT]
    d, c = SMI_DRIVER_RX.search(text), SMI_CUDA_RX.search(text)
    return {"driver": d.group(1) if d else None, "cuda": (int(c.group(1)), int(c.group(2))) if c else None}


def cuda_from_driver(driver):
    """The highest CUDA major.minor line a driver version is documented to support (floor of the table above), or None."""
    if not driver:
        return None
    v = version_tuple(driver)
    for major, minor, floor in DRIVER_FOR_CUDA:
        if v[:2] >= floor:
            return (major, minor)
    return None


def parse_query_csv(text):
    """`nvidia-smi --query-gpu=name,driver_version,compute_cap,memory.total --format=csv,noheader,nounits` -> [{name, driver, cc (major, minor) or None, total_mib}]."""
    out = []
    for line in (text or "")[:MAX_TEXT].splitlines():
        parts = [p.strip() for p in line.split(",")]
        if len(parts) < 4 or not parts[0]:
            continue
        cc = re.fullmatch(r"(\d+)\.(\d+)", parts[2])
        mem = re.fullmatch(r"\d+", parts[3].split(".")[0])
        out.append({"name": parts[0], "driver": parts[1], "cc": (int(cc.group(1)), int(cc.group(2))) if cc else None, "total_mib": int(mem.group(0)) if mem else None})
    return out


def machine_facts(smi_header, query_csv, python_version, has_gcc=False, has_python_h=False, disk_free_gb=None):
    """Everything the flavour choice needs, from text and numbers the runner collected. GPU 0 is the one the rows use."""
    head, gpus = parse_smi_header(smi_header), parse_query_csv(query_csv)
    g = gpus[0] if gpus else {}
    driver = g.get("driver") or head["driver"]
    return {"gpus": len(gpus), "name": g.get("name"), "driver": driver, "driver_cuda": head["cuda"] or cuda_from_driver(driver), "cc": g.get("cc"),
            "total_mib": g.get("total_mib"), "python": tuple(python_version)[:3], "gcc": bool(has_gcc), "python_h": bool(has_python_h), "disk_free_gb": disk_free_gb}


def choose_flavour(row, facts, pins=None):
    """-> {status: ok|NOT_RUN, flavour, reason, why_not {flavour: reason}}. The newest flavour whose driver floor the machine meets.

    NOT_RUN (a named reason, never FAIL): no GPU or no nvidia-smi, a GPU older than Turing, a Python outside the pinned range, a driver below every
    flavour's CUDA floor, too little video memory. Unknown facts are NOT a NOT_RUN: a driver whose CUDA line cannot be derived is `driver_cuda_unknown`
    (an env problem the user can read), because nothing is known to be unsupported."""
    pins = pins or load_pins()
    spec = pins["rows"][row]
    py, pmin, pmax = facts.get("python") or (0, 0, 0), tuple(pins["python"]["min"]), tuple(pins["python"]["max_exclusive"])
    if not facts.get("gpus") or not facts.get("name"):
        return _nr("no_gpu_or_no_nvidia_smi")
    if tuple(py[:2]) < pmin:
        return _nr("python_too_old: %d.%d < %d.%d" % (py[0], py[1], pmin[0], pmin[1]))
    if tuple(py[:2]) >= pmax:
        return _nr("python_too_new: %d.%d >= %d.%d (no wheel is pinned for it)" % (py[0], py[1], pmax[0], pmax[1]))
    cc, floor = facts.get("cc"), tuple(pins["min_compute_capability"])
    if cc is None:
        return {"status": "env", "flavour": None, "reason": "compute_capability_unknown: nvidia-smi does not print compute_cap (update the driver tools)", "why_not": {}}
    if tuple(cc) < floor:
        return _nr("gpu_older_than_turing: sm_%d%d < sm_%d%d" % (cc[0], cc[1], floor[0], floor[1]))
    if facts.get("total_mib") is not None and facts["total_mib"] < spec["min_vram_mib"]:
        return _nr("vram_below_floor: %d MiB < %d MiB" % (facts["total_mib"], spec["min_vram_mib"]))
    cuda = facts.get("driver_cuda")
    if cuda is None:
        return {"status": "env", "flavour": None, "reason": "driver_cuda_unknown: no `CUDA Version` in nvidia-smi and the driver is not in the table", "why_not": {}}
    why = {}
    for name in sorted(spec["flavours"], key=lambda n: -int(re.sub(r"\D", "", n))):
        need = tuple(spec["flavours"][name]["driver_cuda_min"])
        if tuple(cuda) >= need:
            return {"status": "ok", "flavour": name, "reason": "driver CUDA %d.%d >= %d.%d" % (cuda[0], cuda[1], need[0], need[1]), "why_not": why}
        why[name] = "driver supports CUDA %d.%d < %d.%d" % (cuda[0], cuda[1], need[0], need[1])
    return {"status": NOT_RUN, "flavour": None, "reason": "driver_cuda_below_every_wheel: " + "; ".join("%s: %s" % kv for kv in sorted(why.items())), "why_not": why}


def _nr(reason):
    return {"status": NOT_RUN, "flavour": None, "reason": reason, "why_not": {}}


# ------------------------------------------------------------------------------------------------ the hashed locks
HASH_RX = re.compile(r"^\s+--hash=sha256:([0-9a-f]{64})(?: \\)?\s*$")
REQ_RX = re.compile(r"^([A-Za-z0-9][A-Za-z0-9._-]*)==([A-Za-z0-9][A-Za-z0-9.+!_-]*)(?: \\)?\s*$")
INDEX_RX = re.compile(r"^--(index-url|extra-index-url)\s+(\S+)\s*$")
ALLOWED_INDEX_HOSTS = ("pypi.org", "pypi.nvidia.com")


def parse_lock(text):
    """The requirement list of a pip hashed lock -> [{name, version, hashes}] and the index lines -> [(kind, url)]. Comment and blank lines are skipped."""
    reqs, indexes, cur = [], [], None
    for raw in (text or "")[:MAX_TEXT * 4].splitlines():
        line = raw.rstrip("\n")
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        m = INDEX_RX.match(line)
        if m:
            indexes.append((m.group(1), m.group(2)))
            continue
        h = HASH_RX.match(line)
        if h and cur is not None:
            cur["hashes"].append(h.group(1))
            continue
        r = REQ_RX.match(line)
        if r:
            cur = {"name": r.group(1).lower().replace("_", "-"), "version": r.group(2), "hashes": []}
            reqs.append(cur)
            continue
        reqs.append({"name": None, "version": None, "hashes": [], "bad_line": line[:120]})
        cur = None
    return reqs, indexes


def lock_problems(text, require_nvidia_index=False):
    """Why a lock may not be installed with `pip --require-hashes`: any requirement without a digest, an unpinned or URL requirement, an unknown index,
    an index line that is not https, a duplicate name. [] when the lock is sound."""
    reqs, indexes = parse_lock(text)
    out = []
    if not reqs:
        out.append("the lock has no requirement")
    names = set()
    for r in reqs:
        if r.get("bad_line"):
            out.append("a line that is not `name==version` plus hashes: " + r["bad_line"])
            continue
        if not r["hashes"]:
            out.append("%s==%s has no --hash=sha256" % (r["name"], r["version"]))
        if r["name"] in names:
            out.append("%s appears twice" % r["name"])
        names.add(r["name"])
    for kind, url in indexes:
        host = re.match(r"https://([^/]+)/", url)
        if not host:
            out.append("index is not https: " + url)
        elif host.group(1) not in ALLOWED_INDEX_HOSTS:
            out.append("index host is not one of %s: %s" % (", ".join(ALLOWED_INDEX_HOSTS), host.group(1)))
    if not any(k == "index-url" for k, _ in indexes):
        out.append("the lock names no --index-url (the index pip uses would be the machine's)")
    if require_nvidia_index and not any("pypi.nvidia.com" in u for _, u in indexes):
        out.append("the lock needs NVIDIA's own index (pypi.nvidia.com) for the TensorRT wheels")
    return out


def lock_sha256(text):
    return hashlib.sha256((text or "").encode()).hexdigest()


def lock_versions(text):
    return {r["name"]: r["version"] for r in parse_lock(text)[0] if r.get("name")}


def pins_vs_lock_problems(row, flavour, lock_text, pins=None):
    """Every package pinned for a flavour in pins.json must be in its lock at exactly that version."""
    pins = pins or load_pins()
    have = lock_versions(lock_text)
    out = []
    for name, ver in pins["rows"][row]["flavours"][flavour]["packages"].items():
        got = have.get(name.lower().replace("_", "-"))
        if got != ver:
            out.append("%s: pins.json says %s, the %s lock has %s" % (name, ver, flavour, got))
    return out


# ------------------------------------------------------------------------------------------------ the model (weights come from the model host)
def git_blob_sha1(data):
    return hashlib.sha1(b"blob %d\0" % len(data) + data).hexdigest()


def model_plan(tree, pins=None):
    """The origin's tree listing at the pinned revision -> [{path, size, kind (sha256 | git-blob), digest}] for exactly the pinned files, or raises ValueError.

    `tree` is the JSON of `GET /api/models/<repo>/tree/<revision>` (a list of {type, path, size, oid, lfs: {oid (sha256), size}})."""
    pins = pins or load_pins()
    m = pins["model"]
    by = {e.get("path"): e for e in tree if isinstance(e, dict) and e.get("type") == "file"}
    plan, total = [], 0
    for path in m["files"]:
        if path.startswith("/") or ".." in path.split("/"):
            raise ValueError("unsafe model path in pins.json: " + path)
        e = by.get(path)
        if e is None:
            raise ValueError("the origin's tree at revision %s has no %s" % (m["revision"][:12], path))
        lfs = e.get("lfs")
        if lfs and re.fullmatch(r"[0-9a-f]{64}", str(lfs.get("oid", ""))):
            plan.append({"path": path, "size": int(lfs.get("size", e.get("size", -1))), "kind": "sha256", "digest": lfs["oid"]})
        elif re.fullmatch(r"[0-9a-f]{40}", str(e.get("oid", ""))):
            plan.append({"path": path, "size": int(e.get("size", -1)), "kind": "git-blob", "digest": e["oid"]})
        else:
            raise ValueError("no digest for %s in the origin's listing" % path)
        total += plan[-1]["size"]
    if any(p["size"] < 0 for p in plan):
        raise ValueError("a file in the listing has no size")
    if total > m["max_bytes"]:
        raise ValueError("the model is %d bytes, more than the %d pinned" % (total, m["max_bytes"]))
    return plan


def check_model_bytes(entry, data):
    """-> None when `data` is exactly the planned file, else the named mismatch."""
    if len(data) != entry["size"]:
        return "%s: %d bytes, the origin lists %d" % (entry["path"], len(data), entry["size"])
    got = hashlib.sha256(data).hexdigest() if entry["kind"] == "sha256" else git_blob_sha1(data)
    if got != entry["digest"]:
        return "%s: %s %s, the origin lists %s" % (entry["path"], entry["kind"], got, entry["digest"])
    return None


# ------------------------------------------------------------------------------------------------ one row's printed result
def extract_result(text):
    """The last `MLROW_JSON {...}` line of a log -> dict, or None (a missing or unparsable line is `None`, never a partial dict)."""
    found = None
    for line in (text or "")[:MAX_TEXT].splitlines():
        if line.startswith(JSON_PREFIX) and len(line) <= MAX_JSON:
            try:
                value = json.loads(line[len(JSON_PREFIX):])
            except ValueError:
                continue
            if isinstance(value, dict):
                found = value
    return found


def canon_sha256(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def _chk(name, ok, detail=""):
    return {"name": name, "ok": bool(ok), "detail": str(detail)[:200]}


def _num(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool) and value == value and abs(value) != float("inf")


# --- vllm
VLLM_PROMPTS = 8
VLLM_MIN_GRAPH_SIZES = 8
VLLM_MIN_PEAK_MIB = 300          # UNCALIBRATED until the first native run (the weights alone are 270 MiB)


def judge_vllm(res, expected_sha=None):
    """-> (status, reason, checks). The row passes when a real engine produced greedy tokens, twice identically, with graphs captured and the engine core in
    its own process, and device memory grew. FAIL names the first check that did not hold."""
    if not isinstance(res, dict):
        return FAIL, "no_result: the row script printed no MLROW_JSON", [_chk("result_present", False)]
    ids = res.get("token_ids")
    checks = [_chk("result_present", True)]
    good = isinstance(ids, list) and len(ids) == VLLM_PROMPTS and all(isinstance(t, list) and len(t) >= 1 and all(isinstance(i, int) and i >= 0 for i in t) for t in ids)
    checks.append(_chk("tokens_for_every_prompt", good, "n=%s" % (len(ids) if isinstance(ids, list) else None)))
    checks.append(_chk("token_sha256_matches_ids", good and res.get("token_sha256") == canon_sha256(ids), res.get("token_sha256")))
    checks.append(_chk("greedy_repeat_identical", res.get("repeat_identical") is True, "the same batch twice, temperature 0"))
    checks.append(_chk("engine_in_its_own_process", bool(res.get("engine_children")), res.get("engine_children")))
    n = res.get("cudagraph_n_sizes")
    checks.append(_chk("cuda_graph_sizes", _num(n) and n >= VLLM_MIN_GRAPH_SIZES, "n_sizes=%s mode=%s" % (n, res.get("cudagraph_mode"))))
    peak = res.get("mem_growth_mib")
    checks.append(_chk("device_memory_grew", _num(peak) and peak >= VLLM_MIN_PEAK_MIB, "growth=%s MiB, floor %d" % (peak, VLLM_MIN_PEAK_MIB)))
    if expected_sha:
        checks.append(_chk("tokens_equal_committed_hash", res.get("token_sha256") == expected_sha, "got %s" % res.get("token_sha256")))
    bad = next((c for c in checks if not c["ok"]), None)
    return (FAIL, "%s: %s" % (bad["name"], bad["detail"]), checks) if bad else (PASS, "ok", checks)


# --- tensorrt
TRT_REL_TOL = 0.05               # engine output versus the ONNX Runtime CPU reference, as a fraction of the reference's largest magnitude (FP16)  UNCALIBRATED


def judge_tensorrt(res):
    if not isinstance(res, dict):
        return FAIL, "no_result: the row script printed no MLROW_JSON", [_chk("result_present", False)]
    b, run, ort = res.get("builder") or {}, res.get("engine") or {}, res.get("ort_trt") or {}
    checks = [_chk("result_present", True)]
    checks.append(_chk("onnx_model_generated", bool(res.get("onnx_sha256")) and _num(res.get("onnx_params")) and res["onnx_params"] > 0, res.get("onnx_params")))
    checks.append(_chk("builder_ran_fp16", b.get("built") is True and b.get("fp16") is True, "built=%s fp16=%s" % (b.get("built"), b.get("fp16"))))
    checks.append(_chk("tactics_were_timed", _num(b.get("timing_cache_bytes")) and b["timing_cache_bytes"] > 0, "timing cache %s bytes" % b.get("timing_cache_bytes")))
    checks.append(_chk("engine_deserialized_and_ran", run.get("ran") is True, run.get("error")))
    err, scale = run.get("max_abs_err"), run.get("ref_max_abs")
    checks.append(_chk("engine_equals_onnxruntime_cpu", _num(err) and _num(scale) and scale > 0 and err <= TRT_REL_TOL * scale, "max_abs_err=%s ref_max_abs=%s tol=%s" % (err, scale, TRT_REL_TOL)))
    checks.append(_chk("engine_repeat_identical", run.get("repeat_identical") is True))
    st = ort.get("status")
    if st == PASS:
        ok_ep = ort.get("first_provider") == "TensorrtExecutionProvider" and _num(ort.get("trt_node_events")) and ort["trt_node_events"] >= 1
        checks.append(_chk("ort_trt_ep_ran_on_tensorrt", ok_ep, "first=%s events=%s" % (ort.get("first_provider"), ort.get("trt_node_events"))))
        e2, s2 = ort.get("max_abs_err"), ort.get("ref_max_abs")
        checks.append(_chk("ort_trt_equals_onnxruntime_cpu", _num(e2) and _num(s2) and s2 > 0 and e2 <= TRT_REL_TOL * s2, "max_abs_err=%s" % e2))
    elif st == FAIL:
        checks.append(_chk("ort_trt_ep", False, ort.get("reason")))
    elif st != NOT_RUN:
        checks.append(_chk("ort_trt_status_stated", False, "status=%r" % st))
    bad = next((c for c in checks if not c["ok"]), None)
    return (FAIL, "%s: %s" % (bad["name"], bad["detail"]), checks) if bad else (PASS, "ok", checks)


# --- jax
JAX_STRICT_REL = 1e-4            # float32 at HIGHEST precision, as a fraction of the CPU result's largest magnitude
JAX_DEFAULT_REL = 2e-2           # the default (TF32-capable) precision
JAX_PREALLOC_LOW, JAX_PREALLOC_HIGH = 0.70, 0.78   # XLA_PYTHON_CLIENT_MEM_FRACTION defaults to 0.75
JAX_MIN_PEAK_MIB = 32            # UNCALIBRATED


def judge_jax(res):
    if not isinstance(res, dict):
        return FAIL, "no_result: the row script printed no MLROW_JSON", [_chk("result_present", False)]
    checks = [_chk("result_present", True)]
    devs = res.get("devices") or []
    gpu = bool(devs) and all(d.get("platform") in ("gpu", "cuda") for d in devs) and res.get("default_backend") in ("gpu", "cuda")
    checks.append(_chk("jax_devices_are_gpu", gpu, "backend=%s devices=%s" % (res.get("default_backend"), [d.get("platform") for d in devs][:4])))
    for op in res.get("ops") or []:
        tol = JAX_STRICT_REL if op.get("precision") == "highest" else JAX_DEFAULT_REL
        e, s = op.get("max_abs_err"), op.get("ref_max_abs")
        checks.append(_chk("%s_equals_cpu" % op.get("name"), _num(e) and _num(s) and s > 0 and e <= tol * s and op.get("on_gpu") is True, "err=%s scale=%s tol=%s on_gpu=%s" % (e, s, tol, op.get("on_gpu"))))
    names = {op.get("name") for op in res.get("ops") or []}
    checks.append(_chk("matmul_conv_scan_all_ran", {"matmul_f32_highest", "matmul_default", "conv", "scan"} <= names, sorted(n for n in names if n)))
    lim, tot = res.get("bytes_limit_mib"), res.get("total_mib")
    ratio = (lim / tot) if _num(lim) and _num(tot) and tot > 0 else None
    checks.append(_chk("preallocation_is_75_percent", ratio is not None and JAX_PREALLOC_LOW <= ratio <= JAX_PREALLOC_HIGH, "bytes_limit %s MiB of %s MiB = %s" % (lim, tot, None if ratio is None else round(ratio, 3))))
    grown = res.get("mem_growth_mib")
    checks.append(_chk("preallocation_observed_on_the_device", _num(grown) and _num(tot) and grown >= 0.6 * tot, "device memory grew %s MiB of %s MiB" % (grown, tot)))
    checks.append(_chk("autotuner_ran", _num(res.get("autotune_bytes")) and res["autotune_bytes"] > 0, "autotune results %s bytes" % res.get("autotune_bytes")))
    peak = res.get("peak_bytes_in_use_mib")
    checks.append(_chk("work_allocated_on_the_device", _num(peak) and peak >= JAX_MIN_PEAK_MIB, "peak_bytes_in_use %s MiB, floor %d" % (peak, JAX_MIN_PEAK_MIB)))
    bad = next((c for c in checks if not c["ok"]), None)
    return (FAIL, "%s: %s" % (bad["name"], bad["detail"]), checks) if bad else (PASS, "ok", checks)


JUDGES = {"vllm": judge_vllm, "tensorrt": judge_tensorrt, "jax": judge_jax}


def proof_for(row, res, facts):
    """The one line gpu_use.proof_framework reads (guest self-report, the host census is the authority): device, the GPU's name from nvidia-smi, and how far the
    device's memory grew while the row ran. None when the row measured nothing (then the proof is missing and the judge says FAIL)."""
    if not isinstance(res, dict) or not facts.get("name"):
        return None
    grown = res.get("mem_growth_mib")
    if not _num(grown):
        return None
    cpu = res.get("platform") == "cpu" or res.get("device") == "cpu"
    dev = "cpu:0" if cpu else "cuda:0"
    name = "cpu" if cpu else re.sub(r"\s+", " ", str(facts["name"]))[:100]
    return "GPU_USE_PROOF %s device=%s name=%s peak_mib=%d" % (row, dev, name, max(0, int(grown)))


# ------------------------------------------------------------------------------------------------ the vllm engine's own log
KV_RX = re.compile(r"GPU KV cache size:\s*([\d,]+)\s*tokens")
GRAPH_RX = re.compile(r"Graph capturing finished in\s*([\d.]+)\s*secs?(?:,\s*took\s*([\d.]+)\s*GiB)?")
LOAD_RX = re.compile(r"Model loading took\s*([\d.]+)\s*GiB(?:.*?and\s*([\d.]+)\s*seconds)?")
CAPTURE_RX = re.compile(r"Capturing CUDA graphs[^\n]*?(\d+)/(\d+)")


def vllm_log_facts(text):
    """What the engine core printed about itself (INFO log lines): KV-cache size in tokens, graph capture seconds and GiB, model load GiB. Keys are absent when
    the line was not there; the format is vLLM's and may change, so these are recorded and never a verdict (the verdict reads the row script's own facts)."""
    text = (text or "")[:MAX_TEXT]
    out = {}
    m = KV_RX.search(text)
    if m:
        out["kv_cache_tokens"] = int(m.group(1).replace(",", ""))
    m = GRAPH_RX.search(text)
    if m:
        out["graph_capture_s"] = float(m.group(1))
        if m.group(2):
            out["graph_capture_gib"] = float(m.group(2))
    m = LOAD_RX.search(text)
    if m:
        out["model_load_gib"] = float(m.group(1))
    caps = CAPTURE_RX.findall(text)
    if caps:
        out["graphs_captured"] = max(int(a) for a, _ in caps)
        out["graphs_planned"] = max(int(b) for _, b in caps)
    return out


# ------------------------------------------------------------------------------------------------ host against guest
def compare_rows(row, host, guest):
    """Two judged results of one row (each {status, reason, result}) -> [{kind, what}]. kind: CANDIDATE (the guest differs from the host in a way kayfabe may own),
    INFO (a difference the design says is expected, with the reason), NOTE (an observation)."""
    out = []
    hs, gs = (host or {}).get("status"), (guest or {}).get("status")
    if host is None or guest is None:
        return [{"kind": "NOTE", "what": "no %s result in the %s log" % (row, "host" if host is None else "guest")}]
    if hs == PASS and gs == FAIL:
        out.append({"kind": "CANDIDATE", "what": "host PASS, guest FAIL: %s" % guest.get("reason")})
    elif hs == PASS and gs == NOT_RUN:
        out.append({"kind": "NOTE", "what": "host PASS, guest NOT_RUN (%s): our own configuration, re-run" % guest.get("reason")})
    elif hs == FAIL and gs == PASS:
        out.append({"kind": "NOTE", "what": "host FAIL, guest PASS: the host control is the broken side (%s)" % host.get("reason")})
    elif hs != gs:
        out.append({"kind": "NOTE", "what": "host %s, guest %s" % (hs, gs)})
    h, g = (host.get("result") or {}), (guest.get("result") or {})
    if hs == PASS and gs == PASS:
        out += {"vllm": _cmp_vllm, "tensorrt": _cmp_trt, "jax": _cmp_jax}[row](h, g)
    return out


def _cmp_vllm(h, g):
    out = []
    if h.get("token_sha256") != g.get("token_sha256"):
        first = next((i for i, (a, b) in enumerate(zip(h.get("token_ids") or [], g.get("token_ids") or [])) if a != b), None)
        out.append({"kind": "CANDIDATE", "what": "greedy token ids differ between host and guest (host %s, guest %s; first differing prompt: %s)" % (h.get("token_sha256"), g.get("token_sha256"), first)})
    if h.get("cudagraph_n_sizes") != g.get("cudagraph_n_sizes"):
        out.append({"kind": "CANDIDATE", "what": "the number of captured CUDA graph sizes differs (host %s, guest %s): the engine sized its capture from a different memory or device report" % (h.get("cudagraph_n_sizes"), g.get("cudagraph_n_sizes"))})
    if h.get("unbatched_equal") is not g.get("unbatched_equal"):
        out.append({"kind": "NOTE", "what": "batch invariance differs (host %s, guest %s)" % (h.get("unbatched_equal"), g.get("unbatched_equal"))})
    if h.get("total_mib") != g.get("total_mib"):
        out.append({"kind": "INFO", "what": "device memory the process saw: host %s MiB, guest %s MiB (the guest's frame buffer is a kayfabe policy, GAPS 2)" % (h.get("total_mib"), g.get("total_mib"))})
    return out


def _cmp_trt(h, g):
    out = []
    if (h.get("onnx_sha256") != g.get("onnx_sha256")) or (h.get("ref_sha256") != g.get("ref_sha256")):
        out.append({"kind": "NOTE", "what": "the generated model or the CPU reference differs between the sides (onnx %s/%s, reference %s/%s): the inputs are not the same experiment" % (str(h.get("onnx_sha256"))[:12], str(g.get("onnx_sha256"))[:12], str(h.get("ref_sha256"))[:12], str(g.get("ref_sha256"))[:12])})
    hb, gb = h.get("builder") or {}, g.get("builder") or {}
    if _num(hb.get("engine_bytes")) and _num(gb.get("engine_bytes")) and hb["engine_bytes"] > 0 and abs(gb["engine_bytes"] - hb["engine_bytes"]) > 0.25 * hb["engine_bytes"]:
        out.append({"kind": "CANDIDATE", "what": "the engine is %s bytes on the host and %s on the guest: the builder chose other tactics (a different device report?)" % (hb["engine_bytes"], gb["engine_bytes"])})
    ho, go = (h.get("ort_trt") or {}).get("status"), (g.get("ort_trt") or {}).get("status")
    if ho == PASS and go != PASS:
        out.append({"kind": "CANDIDATE", "what": "ONNX Runtime TensorRT EP: host %s, guest %s" % (ho, go)})
    return out


def _cmp_jax(h, g):
    out = []
    hr, gr = _ratio(h), _ratio(g)
    if hr is not None and gr is not None and abs(hr - gr) > 0.02:
        out.append({"kind": "CANDIDATE", "what": "the preallocated fraction differs (host %.3f, guest %.3f)" % (hr, gr)})
    if h.get("total_mib") != g.get("total_mib"):
        out.append({"kind": "INFO", "what": "device memory the process saw: host %s MiB, guest %s MiB (the guest's frame buffer is a kayfabe policy, GAPS 2)" % (h.get("total_mib"), g.get("total_mib"))})
    if _num(h.get("autotune_bytes")) and h["autotune_bytes"] > 0 and not (_num(g.get("autotune_bytes")) and g["autotune_bytes"] > 0):
        out.append({"kind": "CANDIDATE", "what": "the XLA autotuner left results on the host and none in the guest"})
    return out


def _ratio(r):
    lim, tot = r.get("bytes_limit_mib"), r.get("total_mib")
    return lim / tot if _num(lim) and _num(tot) and tot > 0 else None


# ------------------------------------------------------------------------------------------------ the judged line a run prints (what `compare` reads)
RESULT_PREFIX = "MLROWS_RESULT "


def judged_line(row, status, reason, flavour, checks, result):
    """One line, JSON: the verdict, why, the checks and the row's own measurements. `compare` reads nothing else from a log."""
    return RESULT_PREFIX + json.dumps({"row": row, "status": status, "reason": str(reason)[:300], "flavour": flavour, "checks": checks, "result": result},
                                      sort_keys=True, separators=(",", ":"), default=str)


def extract_judged(text, row=None):
    """The last `MLROWS_RESULT {...}` line of a log (of the named row) -> dict, or None."""
    found = None
    for line in (text or "")[:MAX_TEXT * 2].splitlines():
        if line.startswith(RESULT_PREFIX) and len(line) <= MAX_JSON * 2:
            try:
                value = json.loads(line[len(RESULT_PREFIX):])
            except ValueError:
                continue
            if isinstance(value, dict) and value.get("status") in (PASS, FAIL, NOT_RUN) and (row is None or value.get("row") == row):
                found = value
    return found
