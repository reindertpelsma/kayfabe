#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Open Vulkan CTS row (deqp-vk): fetch the pinned sources, build deqp-vk, run the committed case list, diff host against guest.

STATUS: LIVE, 2026-11-17, discovery lane (the row reports, it does not gate). Design and the labels MEASURED / INFERRED:
docs/design/V3_VULKAN_CTS_ROW.md. Usage: docs/howto/vulkan-cts.md. Pure logic (parsers, classes, diff): vkcts_lib.py.

    vkcts.py check                              tools and libraries this machine needs
    vkcts.py build  [--root R] [--jobs N]       fetch the pinned sources, build deqp-vk once, install to R/<key>/ (R/current -> it)
    vkcts.py run    --side host|guest [--artifact A] [--out D] [--census auto|host|none] [--caselist F]
    vkcts.py census start|stop --dir D          host nvidia-smi sampler around a run (the guest run is sampled from the host)
    vkcts.py diff   RESULTS_DIR | --host LOG --guest LOG [--out F] [--strict]

Trust: standalone runs trust git only. The sources come from the pinned upstream commit (pins.json) and every external is checked
against the pinned CTS's own external/fetch_sources.py; nothing built here is pinned, cached remotely or downloaded as a binary.
An artifact given with --artifact is the user's own and is trusted as it is (its sha256 is recorded, not checked).
"""
import argparse
import collections
import hashlib
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import vkcts_lib as L  # noqa: E402

DEFAULT_ROOT = Path(os.environ.get("KF_APPS_ROOT", "/workspace/apps")) / "vkcts"
ICD_CANDIDATES = ("/etc/vulkan/icd.d/nvidia_icd.json", "/usr/share/vulkan/icd.d/nvidia_icd.json",
                  "/etc/vulkan/icd.d/nvidia_icd.x86_64.json", "/usr/share/vulkan/icd.d/nvidia_icd.x86_64.json")
EXIT_NOTRUN, EXIT_DEVICE, EXIT_GPU_UNUSED, EXIT_BAD_INPUT = 6, 3, 4, 2


def say(msg):
    print("[vkcts %s] %s" % (time.strftime("%H:%M:%S"), msg), flush=True)


def load_pins(path=None):
    return L.validate_pins(json.loads(Path(path or HERE / "pins.json").read_text()))


def read_caselist(path, pins, trusted):
    """The case list as (area, case) entries. The committed list must match the digest in pins.json; a list given on the command
    line is the user's own (trusted, its digest is recorded, not compared)."""
    text = Path(path).read_text()
    sha = L.caselist_sha256(text)
    want = pins["run"].get("caselist_sha256", "")
    if not trusted and sha != want:
        raise L.PinRefused("the committed case list has sha256 %s, pins.json says %s (the list is part of the experiment: change both)" % (sha, want))
    entries = L.read_caselist(text)
    L.validate_caselist(entries, info_cases=pins["run"]["info_cases"] if not trusted else ())
    return entries, sha


# ------------------------------------------------------------------------------------------------------- processes
def run_cmd(cmd, cwd=None, timeout=None, env=None, log=None):
    """Run a command in its own session; returns the exit code (124 on timeout, the group is killed). Output goes to `log` (a path) or the console."""
    out = open(log, "ab") if log else None
    try:
        p = subprocess.Popen(cmd, cwd=cwd, env=env, stdout=out, stderr=subprocess.STDOUT if out else None, stdin=subprocess.DEVNULL, start_new_session=True)
        try:
            return p.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            kill_group(p)
            return 124
    finally:
        if out:
            out.close()


def kill_group(p):
    for sig in (signal.SIGTERM, signal.SIGKILL):
        try:
            os.killpg(p.pid, sig)
        except (ProcessLookupError, PermissionError):
            break
        try:
            p.wait(timeout=5)
            return
        except subprocess.TimeoutExpired:
            continue


def git_out(args, cwd):
    return subprocess.run(["git"] + args, cwd=cwd, capture_output=True, text=True, timeout=120).stdout.strip()


# ------------------------------------------------------------------------------------------------------------- fetch
def retry(fn, attempts=3, backoff=(3, 10), sleep=time.sleep):
    for n in range(attempts):
        if fn():
            return True
        if n + 1 < attempts:
            sleep(backoff[min(n, len(backoff) - 1)])
    return False


def fetch_cts(src, pins, run=run_cmd, sleep=time.sleep):
    """`src` ends at exactly the pinned CTS commit or this raises PinRefused (same rule as scripts/apps/pin_lib.sh pin_git_checkout)."""
    src, want, url = Path(src), pins["cts"]["commit"], pins["cts"]["url"]
    if (src / ".git").is_dir() and git_out(["rev-parse", "HEAD"], src) == want:
        return
    shutil.rmtree(src, ignore_errors=True)
    src.mkdir(parents=True)
    if run(["git", "init", "-q", str(src)]) or run(["git", "remote", "add", "origin", url], cwd=src):
        raise L.PinRefused("git init failed")
    ok = retry(lambda: run(["git", "fetch", "-q", "--depth", "1", "origin", want], cwd=src, timeout=1800) == 0, sleep=sleep)
    if ok:
        run(["git", "checkout", "-q", "--detach", "FETCH_HEAD"], cwd=src)
    else:  # a server that refuses a fetch by commit: the tag is only a hint, the HEAD check below is the pin
        ref = "refs/tags/%s" % pins["cts"]["tag"]
        if not retry(lambda: run(["git", "fetch", "-q", "--depth", "1", "origin", "%s:%s" % (ref, ref)], cwd=src, timeout=1800) == 0, sleep=sleep):
            raise L.PinRefused("could not fetch the pinned CTS from %s" % url)
        run(["git", "checkout", "-q", "--detach", ref], cwd=src)
    L.head_matches(git_out(["rev-parse", "HEAD"], src), want)


def verify_externals(src, pins):
    """After the CTS's own fetch scripts ran: every pinned external is exactly at its commit, every pinned file is the pinned bytes."""
    src = Path(src)
    for name, row in sorted(pins["externals_git"].items()):
        d = src / "external" / name / "src"
        L.head_matches(git_out(["rev-parse", "HEAD"], d) if (d / ".git").exists() else "-", row["commit"])
    for name, row in sorted(pins.get("externals_files", {}).items()):
        base = row["url"].rsplit("/", 1)[-1]
        cand = [src / "external" / name / "packages" / base, src / "external" / name / "src" / base]
        found = [c for c in cand if c.is_file()]
        if not found:
            raise L.PinRefused("external file %s was not fetched" % name)
        if hashlib.sha256(found[0].read_bytes()).hexdigest() != row["sha256"]:
            raise L.PinRefused("external file %s does not match its pinned sha256" % name)


def fetch_all(src, pins, run=run_cmd, sleep=time.sleep):
    fetch_cts(src, pins, run=run, sleep=sleep)
    L.check_fetch_sources((Path(src) / "external/fetch_sources.py").read_text(), pins)
    for script in pins["fetch_scripts"]:
        say("running the pinned %s" % script)
        if not retry(lambda: run([sys.executable, script], cwd=src, timeout=3600) == 0, sleep=sleep):
            raise L.PinRefused("%s failed" % script)
    verify_externals(src, pins)


# -------------------------------------------------------------------------------------------------------------- build
def cpu_count():
    try:
        n = len(os.sched_getaffinity(0))
    except AttributeError:
        n = os.cpu_count() or 1
    try:  # cgroup v2 CPU quota ("max 100000" or "400000 100000")
        quota, period = Path("/sys/fs/cgroup/cpu.max").read_text().split()[:2]
        if quota != "max":
            n = min(n, max(1, int(int(quota) / int(period))))
    except (OSError, ValueError):
        pass
    return n


def mem_gb():
    total = 0.0
    try:
        for line in Path("/proc/meminfo").read_text().splitlines():
            if line.startswith("MemAvailable:"):
                total = int(line.split()[1]) / 1048576.0
    except (OSError, ValueError):
        pass
    for f in ("/sys/fs/cgroup/memory.max", "/sys/fs/cgroup/memory/memory.limit_in_bytes"):
        try:
            v = Path(f).read_text().strip()
            if v.isdigit() and int(v) < (1 << 50):
                total = min(total or 1e9, int(v) / 1073741824.0)
        except OSError:
            pass
    return total or 8.0


def plan_jobs(cpus, memory_gb, per_job_gb, override=None):
    if override:
        return max(1, int(override))
    return max(1, min(cpus, int(memory_gb / per_job_gb)))


def tool_status():
    rows = []
    for t in ("git", "cmake", "python3", "g++", "gcc", "make"):
        rows.append((t, bool(shutil.which(t))))
    rows.append(("ninja (optional, make is used without it)", bool(shutil.which("ninja"))))
    return rows


def build(root, pins, jobs=None, src=None, run=run_cmd):
    """Fetch, configure, build deqp-vk, install. Returns the artifact directory. The build is keyed by the content of the pins and
    the build configuration: the same key on this disk is reused, a changed pin builds again beside it."""
    key = L.content_key(pins)
    root = Path(root)
    art = root / key
    done = art / "BUILD_DONE"
    if done.is_file() and (art / "bin/deqp-vk").is_file():
        say("artifact %s exists (content key %s): not building again" % (art, key))
        return art
    root.mkdir(parents=True, exist_ok=True)
    free = shutil.disk_usage(root).free / 1073741824.0
    if free < pins["build"]["min_free_gb"]:
        raise L.PinRefused("only %.0f GB free, the build needs %d GB" % (free, pins["build"]["min_free_gb"]))
    src = Path(src or root / ("src-" + key))
    bdir = root / ("build-" + key)
    t0 = time.time()
    fetch_all(src, pins, run=run)
    t_fetch = time.time()
    njobs = plan_jobs(cpu_count(), mem_gb(), pins["build"]["mem_gb_per_job"], jobs)
    gen = ["-G", "Ninja"] if shutil.which("ninja") else []
    cfg = ["cmake", "-S", str(src), "-B", str(bdir)] + gen + list(pins["build"]["cmake_args"])
    say("configure: %s" % " ".join(cfg[3:]))
    if run(cfg, timeout=1800, log=root / "configure.log"):
        raise L.PinRefused("cmake configure failed (see %s)" % (root / "configure.log"))
    say("building %s with %d jobs (box time-box %ds)" % (pins["build"]["target"], njobs, pins["build"]["time_box_s"]))
    rc = run(["cmake", "--build", str(bdir), "--target", pins["build"]["target"], "-j", str(njobs)], timeout=pins["build"]["time_box_s"], log=root / "build.log")
    if rc:
        raise L.PinRefused("build failed rc=%d (see %s)" % (rc, root / "build.log"))
    t_build = time.time()
    binary = next(Path(bdir).rglob("deqp-vk"), None)
    data = src / "external/vulkancts/data/vulkan"
    if not binary or not binary.is_file() or not data.is_dir():
        raise L.PinRefused("deqp-vk or its data directory is missing after the build")
    shutil.rmtree(art, ignore_errors=True)
    (art / "bin").mkdir(parents=True)
    shutil.copy2(binary, art / "bin/deqp-vk")
    subprocess.run(["strip", str(art / "bin/deqp-vk")], capture_output=True)
    shutil.copytree(data, art / "data/vulkan", symlinks=False)
    size = (art / "bin/deqp-vk").stat().st_size
    manifest = {"key": key, "cts_commit": pins["cts"]["commit"], "cts_tag": pins["cts"]["tag"],
                "externals": {k: v["commit"] for k, v in pins["externals_git"].items()}, "cmake_args": pins["build"]["cmake_args"],
                "jobs": njobs, "cpus": cpu_count(), "mem_gb": round(mem_gb(), 1), "fetch_s": int(t_fetch - t0), "build_s": int(t_build - t_fetch),
                "binary_bytes": size, "binary_sha256_recorded_not_pinned": hashlib.sha256((art / "bin/deqp-vk").read_bytes()).hexdigest(),
                "compiler": subprocess.run(["g++", "--version"], capture_output=True, text=True).stdout.splitlines()[:1]}
    (art / "MANIFEST.json").write_text(json.dumps(manifest, indent=1))
    done.write_text("ok\n")
    cur = root / "current"
    try:
        cur.unlink()
    except FileNotFoundError:
        pass
    cur.symlink_to(art.name)
    print("VKCTS_BUILD key=%s fetch_s=%d build_s=%d jobs=%d binary_mb=%d" % (key, manifest["fetch_s"], manifest["build_s"], njobs, size >> 20))
    return art


# ---------------------------------------------------------------------------------------------------------------- run
def deqp_env(env):
    env = dict(env)
    if "VK_DRIVER_FILES" not in env and "VK_ICD_FILENAMES" not in env:
        for c in ICD_CANDIDATES:
            if os.path.isfile(c):
                env["VK_DRIVER_FILES"] = env["VK_ICD_FILENAMES"] = c
                break
    return env


def deqp_cmd(artifact, caselist, qpa, device_id):
    return [str(Path(artifact) / "bin/deqp-vk"), "--deqp-caselist-file=%s" % caselist, "--deqp-log-filename=%s" % qpa,
            "--deqp-archive-dir=%s" % (Path(artifact) / "data"), "--deqp-log-images=disable", "--deqp-log-shader-sources=disable",
            "--deqp-log-decompiled-spirv=disable", "--deqp-shadercache=disable", "--deqp-crashhandler=enable",
            "--deqp-vk-device-id=%d" % device_id]


def make_batch_runner(artifact, out, run_pins, env, keep, popen=subprocess.Popen, clock=time.monotonic, sleep=time.sleep):
    """Runs one deqp-vk per batch; a batch that makes no progress in `case_timeout_s` is killed (its running case is a Timeout)."""
    counter = [0]
    device_id = int(env.get("VKCTS_DEVICE_ID", "1"))

    def run_batch(batch):
        counter[0] += 1
        n = counter[0]
        cl, qpa, con = out / ("batch%03d.caselist" % n), out / ("batch%03d.qpa" % n), out / ("batch%03d.console" % n)
        cl.write_text("\n".join(batch) + "\n")
        cf = open(con, "wb")
        p = popen(deqp_cmd(artifact, cl, qpa, device_id), cwd=str(Path(artifact) / "data"), env=env, stdout=cf, stderr=subprocess.STDOUT,
                  stdin=subprocess.DEVNULL, start_new_session=True)
        killed, last, mark = False, -1, clock()
        while p.poll() is None:
            sleep(1.0)
            size = qpa.stat().st_size if qpa.exists() else 0
            if size != last:
                last, mark = size, clock()
            elif clock() - mark > run_pins["case_timeout_s"]:
                killed = True
                kill_group(p)
                break
        cf.close()
        lines = open(qpa, errors="replace") if qpa.exists() else []
        try:
            results, open_case = L.parse_qpa(lines, keep_bodies=keep)
        finally:
            if lines != []:
                lines.close()
        qpa.unlink(missing_ok=True)  # the evidence is the compact block and the failure details, not the full XML
        return L.BatchOutcome(results, open_case, p.returncode, killed)

    return run_batch


def load_gpu_assert():
    for d in (HERE, HERE.parents[1] / "bench/windows"):
        f = d / "gpu_assert.py"
        if f.is_file():
            sys.path.insert(0, str(d))
            import gpu_assert  # noqa: E402
            return gpu_assert
    raise ImportError("gpu_assert.py not found (scripts/bench/windows/gpu_assert.py)")


def census_start(directory):
    """Start the host nvidia-smi samplers (query CSV and dmon) detached; their pids go to <dir>/pids."""
    ga = load_gpu_assert()
    d = Path(directory)
    d.mkdir(parents=True, exist_ok=True)
    smi = shutil.which("nvidia-smi")
    if not smi:
        (d / "pids").write_text("")
        return False
    pids = []
    for name, cmd in (("host_query.csv", [smi, "--query-gpu=" + ",".join(ga.QUERY_FIELDS), "--format=csv,noheader,nounits", "-lms", str(ga.INTERVAL_MS)]),
                      ("host_dmon.txt", [smi, "dmon", "-s", "um", "-d", "1"])):
        with open(d / name, "wb") as f:
            pids.append(subprocess.Popen(cmd, stdout=f, stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL, start_new_session=True).pid)
    (d / "pids").write_text(" ".join(map(str, pids)))
    return True


def census_stop(directory, side, require=("gpu", "mem")):
    """Stop the samplers, judge GPU use from the host counters (gpu_assert.evaluate), write <dir>/gpu.json, return the record."""
    ga = load_gpu_assert()
    d = Path(directory)
    try:
        pids = [int(x) for x in (d / "pids").read_text().split()]
    except (OSError, ValueError):
        pids = []
    for pid in pids:
        try:
            os.killpg(pid, signal.SIGTERM)
        except (ProcessLookupError, PermissionError):
            pass
    time.sleep(0.5)
    rd = lambda n: (d / n).read_text(errors="replace") if (d / n).exists() else ""  # noqa: E731
    rec = ga.evaluate(ga.parse_query_csv(rd("host_query.csv")), ga.parse_dmon(rd("host_dmon.txt")), require)
    rec.update(source="host", side=side)
    (d / "gpu.json").write_text(json.dumps(rec, indent=1))
    return rec


def gpu_line(side, rec):
    peak = rec.get("peak", {})
    base = rec.get("baseline", {})
    rise = None if peak.get("mem_mb") is None or base.get("mem_mb") is None else int(peak["mem_mb"] - base["mem_mb"])
    return "VKCTS_GPU side=%s source=%s verdict=%s samples=%s busy_samples=%s peak_gpu_pct=%s mem_rise_mb=%s reasons=%s" % (
        side, rec.get("source", "host"), rec.get("verdict", "FAIL"), rec.get("samples", 0), peak.get("busy_samples"), peak.get("gpu"), rise,
        L.sanitize("; ".join(rec.get("reasons", [])) or "-", 120).replace(" ", "_"))


def evidence_lines(bodies, rows, limit=60):
    out = []
    for case, status, cls, _area, _d in rows:
        if cls in L.OK_CLASSES or cls == "not_supported" or case not in bodies:
            continue
        for line in [b for b in bodies[case] if b.strip() and not b.startswith("<?xml")][:4]:
            out.append("VKCTS_DETAIL case=%s %s" % (case, L.sanitize(line, 160)))
        if len(out) >= limit:
            break
    return out[:limit]


def cmd_run(a):
    pins = load_pins(a.pins)
    entries, sha = read_caselist(a.caselist or HERE / pins["run"]["caselist"], pins, trusted=bool(a.caselist))
    artifact = Path(a.artifact or DEFAULT_ROOT / "current")
    if not (artifact / "bin/deqp-vk").is_file():
        print("NOTRUN: deqp-vk is not built: No such file or directory: %s (run `vkcts.py build`)" % (artifact / "bin/deqp-vk"))
        return EXIT_NOTRUN
    out = Path(a.out or "vkcts_out_" + a.side)
    out.mkdir(parents=True, exist_ok=True)
    areas = {c: ar for ar, c in entries}
    cases = [c for _, c in entries]
    info = pins["run"]["info_cases"]
    env = deqp_env(os.environ)
    census = a.census == "host" or (a.census == "auto" and a.side == "host")
    probe = census_start(out / "census") if census else False
    if census:
        time.sleep(2.0)
    say("%d cases, caselist sha256 %s, binary sha256 %s (recorded)" % (len(cases), sha[:16], hashlib.sha256((artifact / "bin/deqp-vk").read_bytes()).hexdigest()[:16]))
    runner = make_batch_runner(artifact, out, pins["run"], env, set(info))
    t0 = time.monotonic()
    results, bodies, notes = L.execute(cases, runner, pins["run"]["batch_cases"], a.budget or pins["run"]["budget_s"], time.monotonic,
                                       pins["run"]["max_idle_restarts"], keep_bodies=set(info))
    secs = int(time.monotonic() - t0)
    rec = None
    if census:
        time.sleep(2.0)
        rec = census_stop(out / "census", a.side) if probe else {"verdict": "FAIL", "reasons": ["no nvidia-smi on this machine"], "source": "host"}
    rows = L.case_rows(results, areas)
    dev = L.device_info(bodies.get(info[0]), bodies.get(info[1]))
    gpu_verdict = rec["verdict"] if rec else "EXTERNAL"
    print(L.summary_line(a.side, rows, dev, gpu_verdict))
    print("VKCTS_DEVICE side=%s name=%s type=%s vendor=%s api=%s driver=%s extensions=%d" % (
        a.side, dev["name"].replace(" ", "_"), dev["type"], dev["vendor"], dev["api"], dev["driver"], len(dev["extensions"])))
    print("VKCTS_EXT side=%s %s" % (a.side, " ".join(dev["extensions"]) or "-"))
    print("VKCTS_RUN side=%s secs=%d caselist_sha256=%s notes=%s" % (a.side, secs, sha[:16], ",".join(notes)))
    if rec:
        print(gpu_line(a.side, rec))
    for line in evidence_lines(bodies, rows):
        print(line)
    sys.stdout.write(L.render_block(a.side, rows))
    print("DIGEST vkcts_cases %s" % L.digest(rows))
    (out / "results.tsv").write_text(L.render_block(a.side, rows))
    sys.stdout.flush()
    if not L.device_is_gpu(dev):
        print("VKCTS_CONFIG device is not a real GPU (name=%s type=%s): the row did not exercise the GPU" % (dev["name"], dev["type"]))
        return EXIT_DEVICE
    if rec and rec["verdict"] != "PASS":
        print("VKCTS_CONFIG GPU use was not shown by the host counters")
        return EXIT_GPU_UNUSED
    return 0


# ---------------------------------------------------------------------------------------------------------------- diff
def line_value(text, key, side):
    m = re.search(r"^%s side=%s (.*)$" % (key, side), text, re.M)
    return m.group(1) if m else None


def read_side(path):
    text = Path(path).read_text(errors="replace")
    side, rows, dg = L.parse_block(text)
    ext = line_value(text, "VKCTS_EXT", side) or "-"
    info = {"extensions": [] if ext == "-" else ext.split()}
    gpu = re.search(r"^VKCTS_GPU .*$", text, re.M)
    return side, rows, dg, info, (gpu.group(0) if gpu else None), text


def cmd_diff(a):
    if a.results_dir:
        d = Path(a.results_dir)
        host, guest = d / "vkcts.host.log", d / "vkcts.guest.log"
        out = Path(a.out) if a.out else d / "vkcts_diff.tsv"
        guest_gpu = d / "vkcts.guest.gpu.txt"
    else:
        host, guest, out, guest_gpu = Path(a.host), Path(a.guest), Path(a.out) if a.out else None, None
    try:
        hs, hrows, hdg, hinfo, hgpu, _ = read_side(host)
        gs, grows, gdg, ginfo, ggpu, _ = read_side(guest)
    except (OSError, L.CaselistError) as e:
        print("VKCTS_DIFF_ERROR %s" % L.sanitize(e, 160))
        return EXIT_BAD_INPUT
    if guest_gpu and guest_gpu.is_file():
        ggpu = guest_gpu.read_text().strip().splitlines()[-1]
    rows, summary = L.diff_rows(hrows, grows)
    lines = L.diff_lines(rows, summary, hdg, gdg)
    lost, gained = L.extension_diff(hinfo, ginfo)
    lines.append("VKCTS_EXT_DIFF host_only=%s guest_only=%s" % (" ".join(lost) or "-", " ".join(gained) or "-"))
    for g in (hgpu, ggpu):
        if g:
            lines.append(g)
    for area, cnt in summary["areas"].items():
        lines.append("VKCTS_AREA %s %s" % (area, " ".join("%s=%d" % kv for kv in sorted(cnt.items()))))
    text = "\n".join(lines) + "\n"
    if out:
        out.write_text(text)
    sys.stdout.write(text)
    return 1 if (a.strict and summary["candidates"]) else 0


# ------------------------------------------------------------------------------------------------------------- others
def cmd_check(_a):
    bad = 0
    for name, ok in tool_status():
        print("%s %s" % ("ok     " if ok else "MISSING", name))
        bad += (not ok) and "optional" not in name
    for lib in ("libvulkan.so.1",):
        found = any(Path(d, lib).exists() for d in ("/usr/lib/x86_64-linux-gnu", "/usr/lib64", "/usr/lib", "/lib/x86_64-linux-gnu"))
        print("%s %s (the Vulkan loader, needed to RUN deqp-vk)" % ("ok     " if found else "MISSING", lib))
    print("%s an NVIDIA Vulkan ICD file (%s)" % ("ok     " if any(os.path.isfile(c) for c in ICD_CANDIDATES) else "MISSING", ICD_CANDIDATES[0]))
    return 1 if bad else 0


def cmd_build(a):
    pins = load_pins(a.pins)
    try:
        art = build(a.root or DEFAULT_ROOT, pins, jobs=a.jobs, src=a.src)
    except L.PinRefused as e:
        print("PIN_REFUSED: %s" % e)
        print("BUILD_vkcts=FAIL rc=1")
        return 1
    print("BUILD_vkcts=ok")
    print("VKCTS_ARTIFACT %s" % art)
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("check").set_defaults(fn=cmd_check)
    b = sub.add_parser("build")
    b.add_argument("--root")
    b.add_argument("--jobs", type=int)
    b.add_argument("--src")
    b.add_argument("--pins")
    b.set_defaults(fn=cmd_build)
    r = sub.add_parser("run")
    r.add_argument("--side", choices=("host", "guest"), required=True)
    r.add_argument("--artifact")
    r.add_argument("--out")
    r.add_argument("--census", choices=("auto", "host", "none"), default="auto")
    r.add_argument("--caselist", help="your own case list (trusted as given, its digest is recorded)")
    r.add_argument("--budget", type=int)
    r.add_argument("--pins")
    r.set_defaults(fn=cmd_run)
    c = sub.add_parser("census")
    c.add_argument("action", choices=("start", "stop"))
    c.add_argument("--dir", required=True)
    c.add_argument("--side", default="guest")

    def cmd_census(a):
        if a.action == "start":
            print("VKCTS_CENSUS started=%s" % census_start(a.dir))
            return 0
        rec = census_stop(a.dir, a.side)
        line = gpu_line(a.side, rec)
        print(line)
        Path(a.dir, "gpu_line.txt").write_text(line + "\n")
        return 0

    c.set_defaults(fn=cmd_census)
    d = sub.add_parser("diff")
    d.add_argument("results_dir", nargs="?")
    d.add_argument("--host")
    d.add_argument("--guest")
    d.add_argument("--out")
    d.add_argument("--strict", action="store_true")
    d.set_defaults(fn=cmd_diff)
    a = ap.parse_args(argv)
    if a.cmd == "diff" and not (a.results_dir or (a.host and a.guest)):
        ap.error("diff needs a results directory or --host and --guest")
    return a.fn(a)


if __name__ == "__main__":
    sys.exit(main())
