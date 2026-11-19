#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The open ML rows of the Linux app matrix: vllm, tensorrt, jax (discovery lane). Stdlib only; the workloads run in their own venvs.

STATUS: LIVE, 2026-11-17 (docs/design/V3_OPEN_ML_ROWS.md, manual docs/howto/open-ml-rows.md). GPU-free tested (scripts/ci/test_mlrows.py); the native results are in the design doc.

    mlrows.py list
    mlrows.py check   [--rows vllm,tensorrt,jax]                     what this machine is, which flavour each row takes and why; changes nothing
    mlrows.py lockcheck                                              every lock is hashed, pinned, from an allowed index, and agrees with pins.json
    mlrows.py setup   --root DIR [--rows ...] [--flavor auto|cu12|cu13]    one venv per row from the hashed lock (pip --require-hashes), the model (vllm) from its origin
    mlrows.py run     ROW --root DIR [--out DIR] [--flavor ...]      run one row, judge it, print the summary, the GPU_USE_PROOF line and one MLROWS_RESULT line
    mlrows.py compare ROW HOST_LOG GUEST_LOG                         host against guest, from the MLROWS_RESULT lines of two logs
    mlrows.py push    --root DIR --rows ... --transport CMD...       copy the tool dir and the staged rows into a guest (tar | transport; --transport is last)

Trust (owner rulings 2026-11-17, open first, trust git only): no private object, no account, no secret. The packages are pinned by hashed locks (PyPI's own digests;
NVIDIA's own index for TensorRT) and installed with `pip --require-hashes --only-binary=:all: --no-deps`; the model is pinned by the upstream git commit of its repository
and checked against the sizes and digests the origin lists for that revision. Nothing is hosted, cached or pinned by us, and nothing NVIDIA ships is stored anywhere:
every machine downloads its own copy at setup time. NEGCTL (KF_NEGCTL=1): the CPU form of the row, which the GPU-use judge must FAIL.
"""
import argparse
import collections
import glob
import hashlib
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import sysconfig
import threading
import time
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import mlrows_lib as lib  # noqa: E402

SCRIPTS = {"vllm": "row_vllm.py", "tensorrt": "row_tensorrt.py", "jax": "row_jax.py"}
SUMMARY = {"vllm": "VLLM_SUMMARY", "tensorrt": "TENSORRT_SUMMARY", "jax": "JAX_SUMMARY"}
ENV_KEEP = ("PATH", "LANG", "LC_ALL", "TZ", "TERM")
SMI = "nvidia-smi"
MAX_LOG_LINES = 400


def say(*parts):
    print(*parts, flush=True)


def kv(**fields):
    return " ".join("%s=%s" % (k, re.sub(r"\s+", "_", str(v)) if v is not None else "-") for k, v in fields.items())


# ------------------------------------------------------------------------------------------------ processes (replaced by the tests)
def run_cmd(argv, env=None, timeout=60, cwd=None, stdin=None):
    """-> (rc, output text, timed_out). The whole process group is killed on a timeout."""
    try:
        p = subprocess.Popen(argv, env=env, cwd=cwd, stdin=stdin, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, start_new_session=True)
    except OSError as exc:
        return 127, str(exc), False
    try:
        out, _ = p.communicate(timeout=timeout)
        return p.returncode, out.decode("utf-8", "replace"), False
    except subprocess.TimeoutExpired:
        os.killpg(p.pid, signal.SIGKILL)
        out, _ = p.communicate()
        return 124, out.decode("utf-8", "replace"), True


def http_get(url, timeout=60, limit=1 << 30):
    """Bytes of an https URL (redirects followed), bounded. Replaced by the tests."""
    if not url.startswith("https://"):
        raise ValueError("not https: " + url)
    req = urllib.request.Request(url, headers={"User-Agent": "kayfabe-mlrows"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        data = r.read(limit + 1)
    if len(data) > limit:
        raise ValueError("response larger than %d bytes: %s" % (limit, url))
    return data


# ------------------------------------------------------------------------------------------------ facts
def collect_facts(disk_path="."):
    rc1, head, _ = run_cmd([SMI], timeout=30)
    rc2, query, _ = run_cmd([SMI, "--query-gpu=name,driver_version,compute_cap,memory.total", "--format=csv,noheader,nounits"], timeout=30)
    inc = sysconfig.get_paths().get("include") or ""
    near = Path(disk_path).resolve()
    while not near.exists() and near != near.parent:
        near = near.parent                      # the root may not exist yet: the filesystem it will be on is the nearest parent's
    free = shutil.disk_usage(str(near)).free / 1e9
    return lib.machine_facts(head if rc1 == 0 else "", query if rc2 == 0 else "", sys.version_info[:3],
                             has_gcc=bool(shutil.which("gcc") or shutil.which("cc")), has_python_h=os.path.exists(os.path.join(inc, "Python.h")), disk_free_gb=free)


def select(row, facts, pins, forced=None):
    """The flavour decision, with `--flavor` honoured only when it is a flavour of the row (dev and comparisons)."""
    d = lib.choose_flavour(row, facts, pins)
    if forced and forced != "auto":
        if forced not in pins["rows"][row]["flavours"]:
            return {"status": lib.NOT_RUN, "flavour": None, "reason": "flavour_not_pinned_for_this_row: %s (pinned: %s)" % (forced, ",".join(sorted(pins["rows"][row]["flavours"]))), "why_not": {}}
        if d["status"] in ("ok", lib.NOT_RUN) and d.get("flavour") is None and not d["reason"].startswith("driver_cuda_below"):
            return d                          # a hard NOT_RUN (no GPU, too old) is not overridden by a flag
        need = tuple(pins["rows"][row]["flavours"][forced]["driver_cuda_min"])
        if facts.get("driver_cuda") and tuple(facts["driver_cuda"]) < need:
            return {"status": lib.NOT_RUN, "flavour": None, "reason": "driver_cuda_below_forced_flavour: %s needs CUDA %d.%d" % (forced, need[0], need[1]), "why_not": {}}
        return {"status": "ok", "flavour": forced, "reason": "forced by --flavor", "why_not": d.get("why_not", {})}
    return d


def lock_path(row, flavour, pins):
    return HERE / pins["rows"][row]["flavours"][flavour]["lock"]


# ------------------------------------------------------------------------------------------------ the environment of a row
def lib_dirs(venv):
    """The directories a pip-installed CUDA stack needs on LD_LIBRARY_PATH: TensorRT's own libraries first, then every nvidia wheel's lib directory."""
    out = []
    for sp in sorted(glob.glob(os.path.join(str(venv), "lib", "python3*", "site-packages"))):
        out += sorted(glob.glob(os.path.join(sp, "tensorrt_libs")))
        out += sorted(glob.glob(os.path.join(sp, "nvidia", "*", "lib"))) + sorted(glob.glob(os.path.join(sp, "nvidia", "*", "lib64")))
    return out


def row_env(row, venv, cache, base_env, negctl=False, extra=None):
    """The environment of one row's process: an allowlist of the caller's (no token, no key), every cache under `cache` (a fresh directory per run, so a JIT or
    compile step really runs), telemetry off, offline for the model hub. GPU 0 only unless the caller chose."""
    env = {k: base_env[k] for k in ENV_KEEP if k in base_env}
    env.setdefault("PATH", "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin")
    c = Path(cache)
    for sub in ("home", "xdg", "tmp", "triton", "inductor", "torch_ext", "cuda", "vllm", "mpl"):
        (c / sub).mkdir(parents=True, exist_ok=True)
    env.update({"HOME": str(c / "home"), "XDG_CACHE_HOME": str(c / "xdg"), "XDG_CONFIG_HOME": str(c / "xdg"), "XDG_DATA_HOME": str(c / "xdg"), "TMPDIR": str(c / "tmp"),
                "TRITON_CACHE_DIR": str(c / "triton"), "TORCHINDUCTOR_CACHE_DIR": str(c / "inductor"), "TORCH_EXTENSIONS_DIR": str(c / "torch_ext"),
                "CUDA_CACHE_PATH": str(c / "cuda"), "VLLM_CACHE_ROOT": str(c / "vllm"), "MPLCONFIGDIR": str(c / "mpl"),
                "PYTHONNOUSERSITE": "1", "PYTHONDONTWRITEBYTECODE": "1", "PYTHONUNBUFFERED": "1",
                "HF_HUB_OFFLINE": "1", "TRANSFORMERS_OFFLINE": "1", "HF_HOME": str(c / "home"), "VLLM_NO_USAGE_STATS": "1", "DO_NOT_TRACK": "1",
                "CUDA_VISIBLE_DEVICES": base_env.get("CUDA_VISIBLE_DEVICES", "0")})
    dirs = lib_dirs(venv)
    rest = [d for d in base_env.get("LD_LIBRARY_PATH", "").split(":") if d]       # the driver's directory in a container (the venv's libraries come first)
    if dirs or rest:
        env["LD_LIBRARY_PATH"] = ":".join(dirs + rest)
    if negctl:
        # the CPU form of the row: the GPU is hidden (vllm has no CPU path in its wheel, so it fails by itself), jax is told to use its CPU backend, tensorrt runs its CPU reference only
        env["CUDA_VISIBLE_DEVICES"] = ""
        env["KF_NEGCTL"] = "1"
        if row == "jax":
            env["JAX_PLATFORMS"] = "cpu"
    env.update(extra or {})
    return env


# ------------------------------------------------------------------------------------------------ the device memory sampler (the runner's own, one for all rows)
class Sampler(threading.Thread):
    """`nvidia-smi --query-gpu=memory.used,memory.total -lms N` on GPU 0 while a row runs. Guest or host, this is what THIS machine's driver says; the host
    census of the kf3 log and the host nvidia-smi are the authority (gpu_use.py), this is the row's own record. Never raises."""

    def __init__(self, interval_ms=200, argv=None):
        super().__init__(daemon=True)
        self.argv = argv or [SMI, "-i", "0", "--query-gpu=memory.used,memory.total", "--format=csv,noheader,nounits", "-lms", str(int(interval_ms))]
        self.used, self.total, self.proc = [], None, None

    def run(self):
        try:
            self.proc = subprocess.Popen(self.argv, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        except OSError:
            return
        for line in self.proc.stdout:
            used, total = parse_mem_line(line)
            if used is not None:
                self.used.append(used)
                self.total = total or self.total

    def stop(self):
        if self.proc is not None and self.proc.poll() is None:
            self.proc.terminate()
        self.join(timeout=5)


def parse_mem_line(line):
    m = re.fullmatch(r"\s*(\d+)\s*,\s*(\d+)\s*", line or "")
    return (int(m.group(1)), int(m.group(2))) if m else (None, None)


def memory_record(used, total, baseline_samples=3):
    """Samples (MiB) -> {mem_base_mib, mem_peak_mib, mem_growth_mib, total_mib}. The base is the smallest of the first samples (the row had not started allocating)."""
    if not used:
        return {"mem_base_mib": None, "mem_peak_mib": None, "mem_growth_mib": None, "total_mib": total}
    base = min(used[:baseline_samples])
    return {"mem_base_mib": base, "mem_peak_mib": max(used), "mem_growth_mib": max(used) - base, "total_mib": total}


# ------------------------------------------------------------------------------------------------ setup
def setup_state_path(root, row):
    return Path(root) / row / "setup.json"


def read_setup(root, row):
    try:
        return json.loads(setup_state_path(root, row).read_text())
    except (OSError, ValueError):
        return None


def venv_python(root, row):
    return Path(root) / row / "venv" / "bin" / "python"


def pip_argv(py, lock):
    return [str(py), "-m", "pip", "install", "--no-input", "--disable-pip-version-check", "--no-cache-dir", "--only-binary=:all:", "--require-hashes", "--no-deps", "-r", str(lock)]


def fetch_model(dest, pins, get=None):
    """The model's files from their origin at the pinned revision, verified against the sizes and digests the origin lists for that revision. -> (ok, detail)."""
    get = get or http_get
    m = pins["model"]
    base = "%s/%s" % (m["origin"], m["repo"])
    try:
        tree = json.loads(get("%s/api/models/%s/tree/%s" % (m["origin"], m["repo"], m["revision"]), limit=1 << 22))
        plan = lib.model_plan(tree, pins)
    except (ValueError, OSError) as exc:
        return False, "tree: %s" % exc
    dest = Path(dest)
    dest.mkdir(parents=True, exist_ok=True)
    for entry in plan:
        target = dest / entry["path"]
        if target.is_file() and lib.check_model_bytes(entry, target.read_bytes()) is None:
            continue
        try:
            data = get("%s/resolve/%s/%s" % (base, m["revision"], entry["path"]), timeout=600, limit=entry["size"] + 1)
        except (ValueError, OSError) as exc:
            return False, "%s: %s" % (entry["path"], exc)
        bad = lib.check_model_bytes(entry, data)
        if bad:
            return False, "hash_mismatch: " + bad
        part = target.with_name(target.name + ".part")
        part.write_bytes(data)
        os.replace(part, target)
    (dest / "model.json").write_text(json.dumps({"repo": m["repo"], "revision": m["revision"], "license": m["license"], "origin": m["origin"], "files": plan}, indent=1, sort_keys=True) + "\n")
    return True, "%d files, %d bytes" % (len(plan), sum(p["size"] for p in plan))


def toolchain_problem(spec, facts):
    """vLLM's Triton and torch.compile build a launcher with the C compiler and the Python headers at the first start: without them the engine fails late and obscurely."""
    if spec.get("needs_c_compiler") and not (facts.get("gcc") and facts.get("python_h")):
        return "no_c_compiler_or_python_headers: install gcc and python3-dev (Triton and torch.compile build a launcher at first use)"
    return None


def setup_row(row, root, facts, pins, forced=None, get=None):
    """-> (status ok|NOT_RUN|FAIL, reason). Preconditions are named, the install is `pip --require-hashes`, nothing outside `root/row` is written."""
    spec = pins["rows"][row]
    choice = select(row, facts, pins, forced)
    if choice["status"] == lib.NOT_RUN:
        return lib.NOT_RUN, choice["reason"]
    if choice["status"] != "ok":
        return lib.FAIL, choice["reason"]
    flavour = choice["flavour"]
    problem = toolchain_problem(spec, facts)
    if problem:
        return lib.NOT_RUN, problem
    free = facts.get("disk_free_gb")
    if free is not None and free < spec["need_disk_gb"]:
        return lib.NOT_RUN, "disk_free_below_need: %.0f GB free, %d GB needed" % (free, spec["need_disk_gb"])
    lock = lock_path(row, flavour, pins)
    text = lock.read_text()
    problems = lib.lock_problems(text, require_nvidia_index=(row == "tensorrt")) + lib.pins_vs_lock_problems(row, flavour, text, pins)
    if problems:
        return lib.FAIL, "lock_refused: " + "; ".join(problems[:3])
    rowdir = Path(root) / row
    venv = rowdir / "venv"
    state = read_setup(root, row)
    lsha = lib.lock_sha256(text)
    if state and state.get("status") == "ok" and state.get("lock_sha256") == lsha and state.get("flavour") == flavour and venv_python(root, row).exists():
        return "ok", "already set up (flavour %s, lock %s)" % (flavour, lsha[:12])
    if rowdir.exists():
        shutil.rmtree(rowdir)                                  # a half-made or other-flavour directory is ours (under root/row): start clean
    rowdir.mkdir(parents=True)
    t0 = time.time()
    rc, out, _ = run_cmd([sys.executable, "-m", "venv", str(venv)], timeout=300)
    if rc != 0:
        return lib.FAIL, "venv_failed: " + out.strip().splitlines()[-1][:160] if out.strip() else "venv_failed"
    rc, out, to = run_cmd(pip_argv(venv_python(root, row), lock), timeout=3600)
    (rowdir / "pip.log").write_text(out[-200000:])
    if rc != 0:
        why = "pip_timeout" if to else ("hash_mismatch: " + _grep(out, r"(?i)hash|THESE PACKAGES DO NOT MATCH")) if re.search(r"(?i)do not match the hashes|hash mismatch", out) else "pip_failed: " + _grep(out, r"(?i)error")
        return lib.FAIL, why
    pip_s = time.time() - t0
    model = None
    if row == "vllm":
        ok, detail = fetch_model(rowdir / "model", pins, get)
        if not ok:
            return lib.FAIL, "model_fetch_failed: " + detail
        model = detail
    state = {"status": "ok", "row": row, "flavour": flavour, "lock_sha256": lsha, "python": ".".join(map(str, facts["python"])), "pip_seconds": round(pip_s, 1),
             "venv_bytes": tree_bytes(venv), "model": model, "driver_cuda": list(facts["driver_cuda"]) if facts.get("driver_cuda") else None}
    setup_state_path(root, row).write_text(json.dumps(state, indent=1, sort_keys=True) + "\n")
    return "ok", "flavour %s, %.0f s, %.2f GB" % (flavour, pip_s, state["venv_bytes"] / 1e9)


def _grep(text, rx):
    for line in reversed((text or "").splitlines()):
        if re.search(rx, line):
            return line.strip()[:160]
    return "see pip.log"


def tree_bytes(path):
    total = 0
    for dirpath, _, files in os.walk(path):
        for f in files:
            try:
                total += os.lstat(os.path.join(dirpath, f)).st_size
            except OSError:
                pass
    return total


# ------------------------------------------------------------------------------------------------ run
def row_args(row, root, out):
    if row == "vllm":
        return ["--model-dir", str(Path(root) / "vllm" / "model")]
    if row == "tensorrt":
        return ["--work", str(Path(out) / "work")]
    return []


def run_row(row, root, out, facts, pins, forced=None, negctl=False, base_env=None, popen=None):
    """Run one row and judge it. -> (status, reason, flavour, checks, result, log_tail_lines). Never raises for a row's failure."""
    spec = pins["rows"][row]
    choice = select(row, facts, pins, forced)
    if choice["status"] == lib.NOT_RUN:
        return lib.NOT_RUN, choice["reason"], None, [], None, []
    if choice["status"] != "ok":
        return lib.FAIL, choice["reason"], None, [], None, []
    flavour = choice["flavour"]
    problem = toolchain_problem(spec, facts)
    if problem:
        return lib.NOT_RUN, problem, flavour, [], None, []
    state = read_setup(root, row)
    py = venv_python(root, row)
    if not (state and state.get("status") == "ok" and py.exists()):
        return lib.NOT_RUN, "not_set_up: run `mlrows.py setup --root %s --rows %s` first (our own configuration, not a statement about the GPU)" % (root, row), flavour, [], None, []
    if state.get("flavour") != flavour and not forced:
        return lib.NOT_RUN, "set_up_for_another_flavour: the venv is %s, this machine takes %s" % (state.get("flavour"), flavour), flavour, [], None, []
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    cache = out / "cache"
    if cache.exists():
        shutil.rmtree(cache)                                   # fresh caches every run: the JIT and compile steps are part of what the row asks of the driver
    env = row_env(row, Path(root) / row / "venv", cache, base_env if base_env is not None else os.environ, negctl=negctl)
    work = out / "work"
    if work.exists():
        shutil.rmtree(work)                                    # a row reads files it wrote (the XLA autotuner dump): a stale one from an earlier run must never count
    work.mkdir(parents=True, exist_ok=True)
    argv = [str(py), "-B", str(HERE / SCRIPTS[row])] + row_args(row, root, out) + (["--negctl"] if negctl else [])
    sampler = Sampler()
    sampler.start()
    time.sleep(1.0)                                            # a baseline before the row allocates anything
    lines, log_path = collections.deque(maxlen=MAX_LOG_LINES), out / ("%s.venv.log" % row)
    fired = []
    t0 = time.time()
    try:
        proc = (popen or subprocess.Popen)(argv, env=env, cwd=str(work), stdout=subprocess.PIPE, stderr=subprocess.STDOUT, start_new_session=True, text=True, errors="replace")
        timer = threading.Timer(spec["timeout_s"], lambda: (fired.append(1), _kill(proc)))
        timer.start()
        with open(log_path, "w") as log:
            for line in proc.stdout:
                log.write(line)
                lines.append(line.rstrip("\n"))
        rc = proc.wait()
        timer.cancel()
    except OSError as exc:
        rc = 127
        lines.append("cannot start: %s" % exc)
    timed_out = bool(fired)
    secs = time.time() - t0
    time.sleep(0.5)
    sampler.stop()
    text = "\n".join(lines)
    full = log_path.read_text(errors="replace") if log_path.exists() else text
    result = lib.extract_result(full)
    mem = memory_record(sampler.used, sampler.total)
    if isinstance(result, dict):
        result.update(mem)
        result["secs"] = round(secs, 1)
        result["negctl"] = bool(negctl)
        if row == "vllm":
            result["log_facts"] = lib.vllm_log_facts(full)
    if timed_out:
        return lib.FAIL, "timeout: the row did not finish in %g s" % spec["timeout_s"], flavour, [], result, list(lines)
    if result is None:
        last = next((l for l in reversed(list(lines)) if l.strip()), "no output")
        return lib.FAIL, "no_result: rc=%s, last line: %s" % (rc, re.sub(r"\s+", " ", last)[:160]), flavour, [], None, list(lines)
    status, reason, checks = lib.JUDGES[row](result, (pins["rows"][row].get("expected_token_sha256") if row == "vllm" else None))
    return status, reason, flavour, checks, result, list(lines)


def _kill(proc):
    try:
        os.killpg(proc.pid, signal.SIGKILL)
    except (OSError, ProcessLookupError):
        pass


def cmd_run(a):
    pins = lib.load_pins()
    facts = collect_facts(a.root)
    negctl = os.environ.get("KF_NEGCTL") == "1"
    out = a.out or str(Path(a.root) / a.row / "out")
    status, reason, flavour, checks, result, tail = run_row(a.row, a.root, out, facts, pins, a.flavor, negctl)
    say("MLROWS_FACTS " + json.dumps({k: (list(v) if isinstance(v, tuple) else v) for k, v in facts.items()}, sort_keys=True))
    say("MLROWS_ROW " + kv(row=a.row, flavour=flavour, negctl=int(negctl), lock_sha256=(lib.lock_sha256(lock_path(a.row, flavour, pins).read_text())[:16] if flavour else None)))
    for line in tail[:60]:
        say("| " + line[:300])
    if len(tail) > 60:
        say("| ... (%d lines; the whole log is %s.venv.log)" % (len(tail), a.row))
        for line in tail[-100:]:
            say("| " + line[:300])
    if result is not None:
        say(lib.JSON_PREFIX + json.dumps(result, sort_keys=True, separators=(",", ":"), default=str)[: lib.MAX_JSON])
    for c in checks:
        say("MLROWS_CHECK " + kv(name=c["name"], ok=int(c["ok"]), detail=c["detail"]))
    proof = lib.proof_for(a.row, result, facts)
    if proof:
        say(proof)
    say("%s %s" % (SUMMARY[a.row], kv(status=status, flavour=flavour, reason=reason[:120], secs=(result or {}).get("secs"), mem_growth_mib=(result or {}).get("mem_growth_mib"),
                                       token_sha256=(result or {}).get("token_sha256") if a.row == "vllm" else None)))
    say(lib.judged_line(a.row, status, reason, flavour, checks, result))
    say("APPNOTE %s on %s: %s" % (status, flavour or "-", re.sub(r"[=|\s]+", " ", reason)[:110]))     # run_apps.sh puts this in the APPRES note (the engine log's own words would match its generic grep)
    if status == lib.NOT_RUN:
        say("APPNOTRUN " + reason[:150])
    return 1 if status == lib.FAIL else 0


# ------------------------------------------------------------------------------------------------ the other verbs
def cmd_list(_):
    pins = lib.load_pins()
    for row in lib.ROWS:
        say("%-9s %s | flavours %s | about %d GB download" % (row, pins["rows"][row]["title"], ",".join(sorted(pins["rows"][row]["flavours"])), pins["rows"][row]["setup_download_gb"]))
    return 0


def cmd_check(a):
    pins = lib.load_pins()
    facts = collect_facts(a.root or ".")
    say("MLROWS_FACTS " + json.dumps({k: (list(v) if isinstance(v, tuple) else v) for k, v in facts.items()}, sort_keys=True))
    for row in a.rows.split(","):
        c = select(row, facts, pins, a.flavor)
        st = read_setup(a.root, row) if a.root else None
        say("MLROWS_CHECK_ROW " + kv(row=row, status=c["status"], flavour=c["flavour"], reason=c["reason"][:160], set_up=int(bool(st and st.get("status") == "ok"))))
    return 0


def cmd_lockcheck(_):
    pins = lib.load_pins()
    bad = 0
    for row in lib.ROWS:
        for flavour in sorted(pins["rows"][row]["flavours"]):
            p = lock_path(row, flavour, pins)
            if not p.exists():
                say("LOCK_MISSING %s %s %s" % (row, flavour, p.name))
                bad += 1
                continue
            text = p.read_text()
            problems = lib.lock_problems(text, require_nvidia_index=(row == "tensorrt")) + lib.pins_vs_lock_problems(row, flavour, text, pins)
            say("LOCK %s %s packages=%d sha256=%s problems=%d" % (row, flavour, len(lib.parse_lock(text)[0]), lib.lock_sha256(text)[:16], len(problems)))
            for pr in problems:
                say("  " + pr)
            bad += bool(problems)
    return 1 if bad else 0


def cmd_setup(a):
    pins = lib.load_pins()
    facts = collect_facts(a.root)
    rc = 0
    for row in a.rows.split(","):
        status, reason = setup_row(row, a.root, facts, pins, a.flavor)
        say("MLROWS_SETUP " + kv(row=row, status=status, reason=reason[:200]))
        rc = max(rc, 0 if status in ("ok", lib.NOT_RUN) else 3)
    return rc


def cmd_compare(a):
    host = lib.extract_judged(Path(a.host).read_text(errors="replace")[: lib.MAX_TEXT * 2] if Path(a.host).exists() else "", a.row)
    guest = lib.extract_judged(Path(a.guest).read_text(errors="replace")[: lib.MAX_TEXT * 2] if Path(a.guest).exists() else "", a.row)
    findings = lib.compare_rows(a.row, host, guest)
    for f in findings:
        say("ML_COMPARE_%s %s: %s" % (f["kind"], a.row, f["what"]))
    cands = sum(f["kind"] == "CANDIDATE" for f in findings)
    say("ML_COMPARE_SUMMARY " + kv(row=a.row, host=(host or {}).get("status"), guest=(guest or {}).get("status"), candidates=cands))
    return 1 if cands else 0


def push_targets(root, rows):
    return [Path(root) / r for r in rows if (Path(root) / r).is_dir()]


def cmd_push(a):
    """tar | transport: the tool directory and each staged row into the guest's /opt/apps/bundle/{mlrows_tool,mlrows}. The transport is the argv that runs a shell
    command in the guest (scripts/bench/gssh_nv); its last argument is replaced by the remote tar command."""
    rc = 0
    jobs = [("mlrows_tool", HERE, ["--exclude=__pycache__", "--exclude=*.lock.txt.new"], "sudo mkdir -p {d}/mlrows_tool && sudo tar -C {d}/mlrows_tool -xf -")]
    for t in push_targets(a.root, a.rows.split(",")):
        jobs.append((t.name, Path(a.root), ["--exclude=%s/out" % t.name, "--exclude=%s/pip.log" % t.name], "sudo mkdir -p {d}/mlrows && sudo tar -C {d}/mlrows -xf -", t.name))
    for job in jobs:
        label, base, excl, remote = job[:4]
        sub = [job[4]] if len(job) > 4 else ["."]
        t0 = time.time()
        tar = subprocess.Popen(["tar", "-C", str(base), "-cf", "-"] + excl + sub, stdout=subprocess.PIPE)
        ssh = subprocess.Popen(list(a.transport) + [remote.format(d=a.dest)], stdin=tar.stdout)
        tar.stdout.close()
        r = ssh.wait()
        tar.wait()
        size = tree_bytes(base / sub[0]) if sub != ["."] else tree_bytes(base)
        say("MLROWS_PUSH " + kv(what=label, rc=r or tar.returncode, bytes=size, secs=round(time.time() - t0, 1)))
        rc = rc or r or tar.returncode
    return rc


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="verb", required=True)
    sub.add_parser("list")
    sub.add_parser("lockcheck")
    c = sub.add_parser("check")
    c.add_argument("--rows", default=",".join(lib.ROWS))
    c.add_argument("--root")
    c.add_argument("--flavor", default="auto")
    s = sub.add_parser("setup")
    s.add_argument("--root", required=True)
    s.add_argument("--rows", default=",".join(lib.ROWS))
    s.add_argument("--flavor", default="auto")
    r = sub.add_parser("run")
    r.add_argument("row", choices=lib.ROWS)
    r.add_argument("--root", required=True)
    r.add_argument("--out")
    r.add_argument("--flavor", default="auto")
    k = sub.add_parser("compare")
    k.add_argument("row", choices=lib.ROWS)
    k.add_argument("host")
    k.add_argument("guest")
    p = sub.add_parser("push")
    p.add_argument("--root", required=True)
    p.add_argument("--rows", default=",".join(lib.ROWS))
    p.add_argument("--dest", default="/opt/apps/bundle")
    p.add_argument("--transport", nargs=argparse.REMAINDER, required=True, help="the argv that runs one shell command in the guest; everything after it")
    a = ap.parse_args(argv)
    return {"list": cmd_list, "check": cmd_check, "lockcheck": cmd_lockcheck, "setup": cmd_setup, "run": cmd_run, "compare": cmd_compare, "push": cmd_push}[a.verb](a)


if __name__ == "__main__":
    sys.exit(main())
