#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""The `py` cases of the GPU-PV gap probes: the ones that drive nvidia-smi, vulkaninfo, kernel modules and process kills (stdlib only).

STATUS: RESEARCH, 2026-11-18 (docs/design/V3_GPUPV_GAP_MATRIX.md section 3). The decisions are in gap_lib.py (GPU-free tested); the functions here run commands and are
first exercised on a box. Each returns dict(status, reason, kv). A case never touches anything but this machine's GPU stack, and a disruptive case is guest_only.
"""
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import tempfile
import time

import gap_lib as L


def run(argv, timeout, env=None, cwd=None):
    """-> (rc, output, timed_out). Own process group, killed whole on timeout; never a shell."""
    try:
        p = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors="replace", stdin=subprocess.DEVNULL,
                             start_new_session=True, env=env, cwd=cwd)
    except OSError as e:
        return 127, "cannot execute %s: %s" % (argv[0], e), False
    try:
        out, _ = p.communicate(timeout=timeout)
        return p.returncode, out or "", False
    except subprocess.TimeoutExpired:
        os.killpg(p.pid, signal.SIGKILL)
        out, _ = p.communicate()
        return 124, out or "", True


def result(status, reason="-", **kv):
    return {"status": status, "reason": reason, "kv": {k: L.clean(v) for k, v in kv.items()}}


def canary(ctx):
    rc, out, to = run([ctx["probe"], "canary"], 90)
    p = L.parse_probe_output(out, "canary")
    return (not to) and p is not None and p["status"] == "PASS"


# ------------------------------------------------------------------------------------------------------------------ vulkan caps
def vk_dump(ctx):
    exe = shutil.which("vulkaninfo")
    if not exe:
        return result("NOT_RUN", "no_vulkaninfo")
    with tempfile.TemporaryDirectory() as d:
        rc, out, to = run([exe, "--json=0", "-o", str(Path(d) / "vk.json")], 120, cwd=d)
        files = sorted(Path(d).glob("*.json"))
        if not files:   # an older vulkaninfo writes vulkaninfo_<device>.json to the working directory, or prints it
            rc, out, to = run([exe, "--json"], 120, cwd=d)
            files = sorted(Path(d).glob("*.json"))
        text = files[0].read_text() if files else out
        try:
            leaves = L.flatten_vulkan(json.loads(text))
        except ValueError:
            return result("FAIL", "no_json_from_vulkaninfo_rc_%s" % rc)
    (ctx["out"] / "vk_leaves.tsv").write_text("\n".join(leaves) + "\n")
    groups = L.vulkan_groups(leaves)
    kv = {"leaves": len(leaves), "groups": len(groups)}
    for g, ls in groups.items():   # one digest per group: a difference is named by group in the comparison, the leaf-level diff is in vk_leaves.tsv
        kv["group." + g] = "%d:%08x" % (len(ls), _crc("\n".join(ls)))
    return result("PASS" if len(leaves) > 100 else "FAIL", "-" if len(leaves) > 100 else "too_few_leaves", **kv)


def _crc(s):
    import zlib
    return zlib.crc32(s.encode()) & 0xFFFFFFFF


# ------------------------------------------------------------------------------------------------------------------ nvidia-smi setters
SMI_FIELDS = "compute_mode,persistence_mode,power.limit,clocks.applications.graphics,clocks.current.graphics"


def smi_query():
    rc, out, to = run(["nvidia-smi", "--query-gpu=" + SMI_FIELDS, "--format=csv,noheader,nounits", "-i", "0"], 30)
    return tuple(x.strip() for x in out.strip().splitlines()[0].split(",")) if rc == 0 and out.strip() else None


def smi_setters(ctx):
    if not shutil.which("nvidia-smi"):
        return result("NOT_RUN", "no_nvidia_smi")
    base = smi_query()
    if base is None:
        return result("FAIL", "nvidia_smi_query_failed")
    classes, kv = {}, {"baseline": ",".join(base)}
    pl = base[2] if base[2].replace(".", "", 1).isdigit() else None
    gr = base[4] if base[4].isdigit() else None
    steps = [("pm_on", ["-pm", "1"], ["-pm", "0" if base[1].lower() == "disabled" else "1"]), ("compute_exclusive", ["-c", "EXCLUSIVE_PROCESS"], ["-c", "DEFAULT"]),
             ("compute_prohibited", ["-c", "PROHIBITED"], ["-c", "DEFAULT"]), ("ecc_off", ["-e", "0"], None), ("accounting", ["-am", "1"], ["-am", "0"])]
    if pl:
        steps.append(("power_same", ["-pl", str(int(float(pl)))], None))
    if gr:
        steps.append(("lock_gr_clock", ["-lgc", "%s,%s" % (gr, gr)], ["-rgc"]))
    for name, args, undo in steps:
        rc, out, to = run(["nvidia-smi", "-i", "0"] + args, 60)
        classes[name] = L.classify_setter(rc, out, to)
        kv["setter." + name] = classes[name]
        if undo:
            run(["nvidia-smi", "-i", "0"] + undo, 60)
    after = smi_query()
    kv["unchanged_after_undo"] = int(after == base)
    ok = canary(ctx)
    st, why = L.setters_verdict(classes, ok)
    return result(st, why, **kv)


# ------------------------------------------------------------------------------------------------------------------ lifecycle
def start_target(ctx):
    p = subprocess.Popen([ctx["probe"], "kill_target"], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, start_new_session=True)
    t = time.time()
    while time.time() - t < 60:
        line = p.stdout.readline()
        if "GAPREADY" in line:
            return p
        if not line and p.poll() is not None:
            return None
    os.killpg(p.pid, signal.SIGKILL)
    return None


def life_kill_kernel(ctx, rounds=5):
    done = 0
    for i in range(rounds):
        p = start_target(ctx)
        if p is None:
            return result("FAIL", "target_did_not_start_round_%d" % i, rounds_ok=done)
        time.sleep(2.5)
        os.killpg(p.pid, signal.SIGKILL)
        p.wait()
        if not canary(ctx):
            return result("FAIL", "device_unusable_after_kill_round_%d" % i, rounds_ok=done)
        done += 1
    return result("PASS", rounds_ok=done)


def life_kill_gl(ctx, rounds=5):
    egl = ctx.get("egl")
    if not egl or not os.access(egl, os.X_OK):
        return result("NOT_RUN", "no_egl_offscreen")
    rc, out, to = run([egl], 120)
    if to or "CHECK" not in out:
        return result("FAIL", "egl_client_does_not_run_before_the_kill", control_rc=rc)
    for i in range(rounds):
        p = subprocess.Popen([egl], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        time.sleep(0.3 + 0.4 * i)
        try:
            os.killpg(p.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        p.wait()
        rc, out, to = run([egl], 120)
        if to or "CHECK" not in out:
            return result("FAIL", "egl_client_fails_after_kill_round_%d" % i, rounds_ok=i)
        if not canary(ctx):
            return result("FAIL", "cuda_unusable_after_egl_kill_round_%d" % i, rounds_ok=i)
    return result("PASS", rounds_ok=rounds)


def life_persistenced(ctx):
    exe = shutil.which("nvidia-persistenced")
    if not exe:
        return result("NOT_RUN", "no_nvidia_persistenced")
    p = subprocess.Popen([exe, "--no-persistence-mode", "--verbose"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    time.sleep(3)
    alive = p.poll() is None
    c = canary(ctx) if alive else False
    if alive:
        p.terminate()
        try:
            p.wait(20)
        except subprocess.TimeoutExpired:
            p.kill()
            return result("FAIL", "persistenced_ignored_sigterm", alive=1, canary=int(c))
    if not alive:
        return result("FAIL", "persistenced_exited_early_rc_%s" % p.returncode, alive=0)
    return result("PASS" if c else "FAIL", "-" if c else "cuda_fails_while_persistenced_runs", alive=1, canary=int(c))


MODULES = ["nvidia_drm", "nvidia_modeset", "nvidia_uvm", "nvidia"]


def modprobe(args, timeout=60):
    return run(["modprobe"] + args, timeout)


def life_module_cycle(ctx, cycles=3):
    """Guest only. With a CUDA client alive the unload must be REFUSED quickly (module in use) and the client must survive; without one the modules must unload and reload,
    and CUDA must work after each reload. A hang is the result this case exists to find."""
    if os.geteuid() != 0 or not shutil.which("modprobe"):
        return result("NOT_RUN", "needs_root_and_modprobe")
    run(["pkill", "nvidia-persistenced"], 10)
    ok_cycles = 0
    for i in range(cycles):
        p = start_target(ctx)
        if p is None:
            return result("FAIL", "target_did_not_start_cycle_%d" % i, cycles_ok=ok_cycles)
        rc, out, to = modprobe(["-r", "nvidia"], 60)
        alive = p.poll() is None
        os.killpg(p.pid, signal.SIGKILL)
        p.wait()
        if to:
            return result("FAIL", "unload_with_client_hung_cycle_%d" % i, cycles_ok=ok_cycles)
        if rc == 0 or not alive:
            return result("FAIL", "unload_succeeded_under_a_live_client_cycle_%d" % i, cycles_ok=ok_cycles, unload_rc=rc)
        time.sleep(1)
        for m in MODULES:
            rc, out, to = modprobe(["-r", m], 60)
            if to:
                return result("FAIL", "unload_%s_hung_cycle_%d" % (m, i), cycles_ok=ok_cycles)
            if rc != 0 and "not currently loaded" not in out.lower():
                return result("FAIL", "unload_%s_failed_cycle_%d" % (m, i), cycles_ok=ok_cycles, unload_out=out)
        rc, out, to = modprobe(["nvidia"], 120)
        if to or rc != 0:
            return result("FAIL", "reload_failed_or_hung_cycle_%d" % i, cycles_ok=ok_cycles, reload_out=out)
        modprobe(["nvidia_uvm"])
        modprobe(["nvidia_modeset"])
        if not canary(ctx):
            return result("FAIL", "cuda_unusable_after_reload_cycle_%d" % i, cycles_ok=ok_cycles)
        ok_cycles += 1
    return result("PASS", cycles_ok=ok_cycles)


def life_reset(ctx):
    if os.geteuid() != 0 or not shutil.which("nvidia-smi"):
        return result("NOT_RUN", "needs_root_and_nvidia_smi")
    rc, out, to = run(["nvidia-smi", "-r", "-i", "0"], 120)
    cls = L.classify_setter(rc, out, to)
    time.sleep(3)
    ok = canary(ctx)
    kv = {"reset_class": cls, "reset_rc": rc, "canary_after": int(ok)}
    if to:
        return result("FAIL", "reset_hung", **kv)
    if cls not in L.SETTER_OK:
        return result("FAIL", "reset_answer_%s" % cls.lower(), **kv)
    return result("PASS" if ok else "FAIL", "-" if ok else "device_unusable_after_reset", **kv)


def life_busy_poweroff(ctx):
    """Terminal. Starts a busy kernel and powers the machine off. Nothing is judged here: the host reads its census (slots, VA pool, channels) afterwards, and the next boot
    on the same device must pass the canary. The line below is the last one this machine prints."""
    p = start_target(ctx)
    if p is None:
        return result("FAIL", "target_did_not_start")
    print("GAPTERMINAL poweroff with a busy GPU: the host census decides", flush=True)
    subprocess.run(["systemctl", "poweroff"], check=False)
    time.sleep(60)
    return result("FAIL", "machine_did_not_power_off_in_60s")


# ------------------------------------------------------------------------------------------------------------------ the NVIDIA tools
# They come from the official CUDA apt repository at run time (provision_tools.sh): never stored in the repo, an image or R2.
# the tools are searched on the machine itself and under tools/ beside this file: a guest has no network at run time, so a run may push the tools the host installed
# (from the official repository, by provision_tools.sh) into that directory; nothing there is ever committed
TOOL_ROOTS = ("", str(Path(__file__).resolve().parent / "tools"))


def newest(paths):
    """The path under the highest CUDA release, judged by the real path (cuda and cuda-12 are symlinks to a release, and sorting the names would prefer them)."""
    def key(p):
        m = re.findall(r"cuda-(\d+)\.(\d+)", os.path.realpath(p))
        return tuple(int(x) for x in m[-1]) if m else (0, 0)
    return max(paths, key=lambda p: (key(p), p)) if paths else None


def find_tool(name):
    import glob
    found = shutil.which(name)
    if found:
        return found
    for root in TOOL_ROOTS:
        hits = []
        for pat in ("/usr/local/cuda*/bin/" + name, "/opt/nvidia/nsight-systems/*/bin/" + name, "/opt/nvidia/nsight-compute/*/" + name):
            hits += glob.glob(root + pat)
        best = newest(hits)
        if best:
            return best
    return None


def cupti_lib():
    import glob
    for root in TOOL_ROOTS:
        best = newest(glob.glob(root + "/usr/local/cuda*/extras/CUPTI/lib64/libcupti.so.*"))
        if best:
            return best
    return ""


def tool_case(ctx, runs, extra_kv=None):
    """runs: {name: argv or None}. None -> the tool is not installed -> NOT_RUN (never a pass). -> result dict; kv holds one class per run."""
    classes, kv, outs = {}, dict(extra_kv or {}), {}
    for name, argv in runs.items():
        if argv is None:
            return result("NOT_RUN", "no_tool_for_" + name)
        rc, out, to = run(argv, 240)
        classes[name] = L.classify_tool(rc, out, to)
        kv["tool." + name] = classes[name]
        outs[name] = out
    st, why = L.tools_verdict(classes)
    return dict(result(st, why, **kv), outs=outs)


def tools_sanitizer(ctx):
    exe = find_tool("compute-sanitizer")
    if not exe:
        return result("NOT_RUN", "no_compute_sanitizer")
    base = [exe, "--error-exitcode", "99"]
    runs = {t: base + ["--tool", t, ctx["probe"], "canary"] for t in ("memcheck", "racecheck", "synccheck", "initcheck")}
    r = tool_case(ctx, runs)
    # positive control: the same tool must REPORT an illegal store. A silent memcheck is blind, whatever it says about the clean run
    rc, out, to = run(base + ["--tool", "memcheck", ctx["probe"], "fault_child", "fault"], 240)
    seen = "invalid __global__ write" in out.lower() or "invalid __global__" in out.lower()
    r["kv"]["memcheck_reports_the_illegal_store"] = str(int(seen))
    if r["status"] == "PASS" and not seen:
        return dict(r, status="FAIL", reason="memcheck_silent_about_an_illegal_store")
    return r


def tools_ncu(ctx):
    exe = find_tool("ncu")
    if not exe:
        return result("NOT_RUN", "no_ncu")
    return tool_case(ctx, {"metric": [exe, "--metrics", "sm__cycles_elapsed.avg", "-k", "regex:addone", "--launch-count", "1", ctx["probe"], "canary"]})


def tools_nsys(ctx):
    exe = find_tool("nsys")
    if not exe:
        return result("NOT_RUN", "no_nsys")
    d = Path(tempfile.mkdtemp(prefix="gapnsys"))
    try:
        rep = str(d / "r")
        r = tool_case(ctx, {"profile": [exe, "profile", "--trace=cuda,nvtx,osrt", "--stats=false", "-f", "true", "-o", rep, ctx["probe"], "canary"]})
        if r["status"] != "PASS":
            return r
        rc, out, to = run([exe, "stats", "--report", "cuda_gpu_kern_sum", "--format", "csv", rep + ".nsys-rep"], 120)
        seen = "addone" in out
        r["kv"]["kernel_in_report"] = str(int(seen))
        return r if seen else dict(r, status="FAIL", reason="nsys_report_has_no_kernel")
    finally:
        shutil.rmtree(d, True)


def tools_nsys_gpumetrics(ctx):
    exe = find_tool("nsys")
    if not exe:
        return result("NOT_RUN", "no_nsys")
    d = Path(tempfile.mkdtemp(prefix="gapnsysm"))
    try:
        return tool_case(ctx, {"gpu_metrics": [exe, "profile", "--gpu-metrics-device=0", "--stats=false", "-f", "true", "-o", str(d / "r"), ctx["probe"], "canary"]})
    finally:
        shutil.rmtree(d, True)


CASES = {"tools_sanitizer": tools_sanitizer, "tools_ncu": tools_ncu, "tools_nsys": tools_nsys, "tools_nsys_gpumetrics": tools_nsys_gpumetrics, "vk_dump": vk_dump, "smi_setters": smi_setters, "life_persistenced": life_persistenced, "life_kill_kernel": life_kill_kernel, "life_kill_gl": life_kill_gl,
         "life_module_cycle": life_module_cycle, "life_reset": life_reset, "life_busy_poweroff": life_busy_poweroff}
