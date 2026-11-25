#!/usr/bin/env python3
"""Arms of the host debugger-session cap measurement (decision A8).

Design and falsifiers: traces/debugger_cap_20261125/PREREGISTRATION.md. The workers are
`kf-debugger-cap-probe` processes (one host RM client each); this controller starts N of them per arm,
samples host FB (`nvidia-smi`), `/proc/meminfo` and `dmesg`, stops an arm at the first new Xid, at the
session ceiling, at the FB growth limit or at the wall limit, then has every worker free its sessions
and exit, and checks that host FB comes back (a leak is a finding).

    python3 scripts/bench/debugger_cap_arms.py --probe target/release/kf-debugger-cap-probe \
        --out DIR [--arms a1,a2,...] [--ceiling 20000] [--fb-limit-mib 4096] [--wall-s 900]

Standard library only. No GPU identity is recorded: dmesg lines are scrubbed of UUIDs and PCI addresses
before they are written.
"""
import argparse
import json
import re
import subprocess
import sys
import threading
import time
from pathlib import Path

# name: (processes, GR objects per process, sessions per GR object; 0 = no limit of ours)
ARMS = {
    "a1": (1, 1, 0),
    "a2": (1, 4, 0),
    "a3": (1, 16, 0),
    "a4": (4, 1, 0),
    "a5": (16, 1, 0),
    "k1": (1, 64, 4),
    "k2": (1, 448, 4),
}
DEFAULT_BUDGET = 1789  # the derived channel-budget default seen in kf3 logs on an idle host
MIN_BUDGET = 256

_KV = re.compile(r"(\w+)=(\S+)")
_UUID = re.compile(r"GPU-[0-9a-fA-F]{8}(?:-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12}")
_BDF = re.compile(r"\b[0-9a-fA-F]{4}:[0-9a-fA-F]{2}:[0-9a-fA-F]{2}\.[0-7]\b")


def parse_line(line):
    """One worker line `TAG key=value ...` as (tag, {key: value}); None for a blank line."""
    line = line.strip()
    if not line:
        return None
    tag, _, rest = line.partition(" ")
    return tag, dict(_KV.findall(rest))


def parse_meminfo(text):
    """`/proc/meminfo` text as {field: kB}."""
    out = {}
    for ln in text.splitlines():
        m = re.match(r"(\w+):\s+(\d+)\s*kB", ln)
        if m:
            out[m.group(1)] = int(m.group(2))
    return out


def count_xids(dmesg_text):
    """NVRM Xid lines in a dmesg text."""
    return sum(1 for ln in dmesg_text.splitlines() if "NVRM: Xid" in ln)


def scrub(line):
    """A dmesg line without GPU UUIDs or PCI addresses."""
    return _BDF.sub("<bdf>", _UUID.sub("<uuid>", line))


def nvrm_lines(dmesg_text):
    """The scrubbed NVRM lines of a dmesg text."""
    return [scrub(ln) for ln in dmesg_text.splitlines() if "NVRM" in ln]


def fb_limit_hit(baseline_mib, now_mib, limit_mib):
    """True when host FB grew by `limit_mib` or more since the arm started."""
    return now_mib - baseline_mib >= limit_mib


def leak_verdict(before_mib, after_mib, tol_mib):
    """`ok` when FB is within `tol_mib` above `before_mib`, else `LEAK +N MiB`."""
    extra = after_mib - before_mib
    return "ok" if extra <= tol_mib else "LEAK +%d MiB" % extra


def kib_per_session(from_mib, to_mib, sessions):
    """FB growth per session in KiB; None without sessions."""
    if sessions <= 0:
        return None
    return round((to_mib - from_mib) * 1024.0 / sessions, 2)


def recommend(refusal_total, kib_per, default_budget=DEFAULT_BUDGET, min_budget=MIN_BUDGET):
    """The cap the preregistration derives (owner decides).

    A global refusal count R gives floor(R / 4) rounded down to 64; with none below the ceiling, the
    count whose measured FB cost stays under 1 GiB, rounded down to 64 and never below one VM at the
    default budget.
    """
    if refusal_total is not None:
        cap = max(0, refusal_total // 4 // 64 * 64)
        basis = "refusal %d / 4" % refusal_total
    elif kib_per and kib_per > 0:
        cap = max(default_budget, int(1024 * 1024 / kib_per) // 64 * 64)
        basis = "1 GiB / %.2f KiB per session, at least one default VM" % kib_per
    else:
        return {"cap": None, "basis": "no per-session cost measured", "vms_default": None, "vms_min": None}
    return {
        "cap": cap,
        "basis": basis,
        "vms_default": cap // default_budget,
        "vms_min": cap // min_budget,
    }


class Worker:
    """One probe process and what it has said."""

    def __init__(self, idx, argv, log):
        self.idx = idx
        self.log = log
        self.proc = subprocess.Popen(
            argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1
        )
        self.lines = []
        self.n = 0
        self.tags = {}
        self.refusals = []
        self.progress = []
        self.lock = threading.Lock()
        self.t0 = time.time()
        self.thread = threading.Thread(target=self._read, daemon=True)
        self.thread.start()

    def _read(self):
        for raw in self.proc.stdout:
            with self.lock:
                self.log.write("%7.2f p%d %s\n" % (time.time() - self.t0, self.idx, raw.rstrip()))
                self.log.flush()
                parsed = parse_line(raw)
                if not parsed:
                    continue
                tag, kv = parsed
                self.lines.append((tag, kv))
                self.tags.setdefault(tag, kv)
                if tag == "PROGRESS" and "n" in kv:
                    self.n = int(kv["n"])
                    self.progress.append(kv)
                elif tag == "ALLOC_DONE" and "total" in kv:
                    self.n = int(kv["total"])
                elif tag == "REFUSED":
                    self.refusals.append(kv)

    def has(self, tag):
        with self.lock:
            return tag in self.tags

    def alive(self):
        return self.proc.poll() is None

    def say(self, word):
        try:
            self.proc.stdin.write(word + "\n")
            self.proc.stdin.flush()
        except (BrokenPipeError, OSError):
            pass

    def kill(self):
        if self.alive():
            self.proc.kill()

    def close(self):
        for f in (self.proc.stdin, self.proc.stdout):
            try:
                f.close()
            except OSError:
                pass


class Sources:
    """The host facts the controller samples. Replaced in tests."""

    def __init__(self, smi_index):
        self.smi_index = smi_index

    def fb_mib(self):
        out = subprocess.run(
            ["nvidia-smi", "-i", str(self.smi_index), "--query-gpu=memory.used", "--format=csv,noheader,nounits"],
            capture_output=True, text=True, timeout=60, check=True,
        ).stdout
        return int(out.strip().splitlines()[0])

    def meminfo(self):
        return parse_meminfo(Path("/proc/meminfo").read_text())

    def dmesg(self):
        return subprocess.run(["dmesg"], capture_output=True, text=True, timeout=60).stdout


def wait_all(workers, tag, timeout, stop_on_death=True):
    """Wait until every worker has said `tag`; a dead worker without it ends the wait."""
    end = time.time() + timeout
    while time.time() < end:
        if all(w.has(tag) for w in workers):
            return True
        if stop_on_death and any(not w.alive() and not w.has(tag) for w in workers):
            time.sleep(0.2)
            return all(w.has(tag) for w in workers)
        time.sleep(0.1)
    return False


def run_arm(name, spec, probe, out, src, ceiling, fb_limit_mib, wall_s, settle_s, leak_tol_mib, gpu, sample_s=0.5,
            progress_every=100, pause_s=2.0):
    procs, grs, per_gr = spec
    out.mkdir(parents=True, exist_ok=True)
    log = open(out / ("%s.log" % name), "w")
    summary = {"arm": name, "procs": procs, "grs_per_proc": grs, "per_gr": per_gr, "ceiling": ceiling,
               "fb_limit_mib": fb_limit_mib}
    base_fb = src.fb_mib()
    base_mem = src.meminfo()
    base_dmesg = src.dmesg()
    base_xids = count_xids(base_dmesg)
    base_lines = set(nvrm_lines(base_dmesg))
    summary["fb_baseline_mib"] = base_fb
    argv = [str(probe), "--gpu", str(gpu), "--grs", str(grs), "--per-gr", str(per_gr),
            "--max-sessions", str(ceiling), "--progress-every", str(progress_every)]
    workers = []
    timeline = []
    stop_cause = None
    try:
        for i in range(procs):
            workers.append(Worker(i, argv, log))
        ready = wait_all(workers, "READY", 900)
        summary["ready"] = ready
        setups = [dict(w.tags.get("SETUP", {})) for w in workers]
        summary["setup"] = setups
        if not ready:
            stop_cause = "setup_failed"
            raise RuntimeError("workers not ready")
        time.sleep(pause_s / 2)
        setup_fb = src.fb_mib()
        summary["fb_after_setup_mib"] = setup_fb
        t0 = time.time()
        next_dmesg = t0
        for w in workers:
            w.say("go")
        peak_fb = setup_fb
        min_avail = base_mem.get("MemAvailable", 0)
        max_sunrec = base_mem.get("SUnreclaim", 0)
        while True:
            now = time.time()
            total = sum(w.n for w in workers)
            fb = src.fb_mib()
            mem = src.meminfo()
            peak_fb = max(peak_fb, fb)
            min_avail = min(min_avail, mem.get("MemAvailable", min_avail))
            max_sunrec = max(max_sunrec, mem.get("SUnreclaim", 0))
            timeline.append((round(now - t0, 2), total, fb, mem.get("MemAvailable", 0), mem.get("SUnreclaim", 0)))
            if all(w.has("ALLOC_DONE") for w in workers):
                stop_cause = stop_cause or "workers_done"
                break
            if any(not w.alive() and not w.has("ALLOC_DONE") for w in workers):
                stop_cause = stop_cause or "worker_died"
                break
            cause = None
            if total >= ceiling:
                cause = "ceiling"
            elif fb_limit_hit(base_fb, fb, fb_limit_mib):
                cause = "fb_limit"
            elif now - t0 > wall_s:
                cause = "wall"
            elif now >= next_dmesg:
                next_dmesg = now + max(pause_s, 4 * sample_s)
                if count_xids(src.dmesg()) > base_xids:
                    cause = "xid"
            if cause and not stop_cause:
                stop_cause = cause
                for w in workers:
                    w.say("stop")
            time.sleep(sample_s)
        wait_all(workers, "ALLOC_DONE", 120, stop_on_death=False)
        time.sleep(pause_s)
        peak_fb = max(peak_fb, src.fb_mib())
        mem = src.meminfo()
        summary["fb_peak_mib"] = peak_fb
        summary["fb_at_done_mib"] = src.fb_mib()
        summary["stop_cause"] = stop_cause
        summary["sessions_total"] = sum(w.n for w in workers)
        summary["meminfo_kb"] = {
            "MemAvailable_start": base_mem.get("MemAvailable"), "MemAvailable_min": min_avail,
            "MemAvailable_at_done": mem.get("MemAvailable"),
            "SUnreclaim_start": base_mem.get("SUnreclaim"), "SUnreclaim_max": max_sunrec,
            "SUnreclaim_at_done": mem.get("SUnreclaim"),
            "Slab_start": base_mem.get("Slab"), "Slab_at_done": mem.get("Slab"),
        }
        for w in workers:
            w.say("release")
        freed = wait_all(workers, "FREE_DONE", wall_s, stop_on_death=False)
        time.sleep(pause_s)
        summary["free_ok"] = freed
        summary["fb_after_free_mib"] = src.fb_mib()
        summary["leak_after_free_vs_setup"] = leak_verdict(setup_fb, summary["fb_after_free_mib"], leak_tol_mib)
        for w in workers:
            w.say("exit")
        for w in workers:
            try:
                w.proc.wait(timeout=180)
            except subprocess.TimeoutExpired:
                w.kill()
        end = time.time() + settle_s
        after = src.fb_mib()
        while after - base_fb > leak_tol_mib and time.time() < end:
            time.sleep(1.0)
            after = src.fb_mib()
        summary["fb_after_exit_mib"] = after
        summary["leak_after_exit_vs_baseline"] = leak_verdict(base_fb, after, leak_tol_mib)
    finally:
        for w in workers:
            w.kill()
        for w in workers:
            w.thread.join(timeout=5)
            w.close()
        log.close()
    # per-worker facts
    per = []
    for w in workers:
        done = w.tags.get("ALLOC_DONE", {})
        fd = w.tags.get("FREE_DONE", {})
        per.append({"proc": w.idx, "alloc_done": done, "free_done": fd, "rc": w.proc.poll(),
                    "refusals": w.refusals[:8], "refusal_count": len(w.refusals),
                    "first_progress": w.progress[0] if w.progress else None,
                    "last_progress": w.progress[-1] if w.progress else None})
    summary["workers"] = per
    summary["sessions_total"] = sum(w.n for w in workers)
    summary["refusing_workers"] = sum(1 for w in workers if w.refusals)
    if "fb_peak_mib" in summary and "fb_after_setup_mib" in summary:
        summary["fb_kib_per_session"] = kib_per_session(
            summary["fb_after_setup_mib"], summary["fb_peak_mib"], summary["sessions_total"])
        gr_total = procs * grs
        summary["fb_setup_mib_per_gr"] = round((summary["fb_after_setup_mib"] - base_fb) / gr_total, 2)
    dm = src.dmesg()
    summary["new_xids"] = count_xids(dm) - base_xids
    new = [ln for ln in nvrm_lines(dm) if ln not in base_lines]
    (out / ("%s_dmesg.txt" % name)).write_text("\n".join(new[:200]) + ("\n" if new else ""))
    with open(out / ("%s_timeline.tsv" % name), "w") as f:
        f.write("t_s\tsessions\tfb_used_mib\tmem_available_kb\tsunreclaim_kb\n")
        for row in timeline:
            f.write("\t".join(str(c) for c in row) + "\n")
    (out / ("%s.json" % name)).write_text(json.dumps(summary, indent=1, sort_keys=True) + "\n")
    return summary


def table(summaries):
    """A short text table over arm summaries."""
    head = ["arm", "procs x GR", "sessions", "stop", "refused", "FB KiB/session", "peak-base MiB", "after free", "after exit"]
    rows = [head]
    for s in summaries:
        rows.append([
            s["arm"], "%dx%d" % (s["procs"], s["grs_per_proc"]), str(s.get("sessions_total")), str(s.get("stop_cause")),
            "%d/%d procs" % (s.get("refusing_workers", 0), s["procs"]), str(s.get("fb_kib_per_session")),
            str(s.get("fb_peak_mib", 0) - s["fb_baseline_mib"]), str(s.get("leak_after_free_vs_setup")),
            str(s.get("leak_after_exit_vs_baseline")),
        ])
    w = [max(len(r[i]) for r in rows) for i in range(len(head))]
    return "\n".join("  ".join(c.ljust(w[i]) for i, c in enumerate(r)) for r in rows)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--probe", required=True, help="path of kf-debugger-cap-probe")
    ap.add_argument("--out", required=True)
    ap.add_argument("--arms", default="a1,a2,a3,a4,a5,k1,k2")
    ap.add_argument("--ceiling", type=int, default=20000, help="sessions per arm, all processes")
    ap.add_argument("--fb-limit-mib", type=int, default=4096)
    ap.add_argument("--wall-s", type=int, default=900)
    ap.add_argument("--settle-s", type=int, default=60)
    ap.add_argument("--leak-tol-mib", type=int, default=2)
    ap.add_argument("--gpu", type=int, default=0, help="host GPU minor")
    ap.add_argument("--smi-index", type=int, default=0, help="nvidia-smi -i index of the same GPU")
    a = ap.parse_args(argv)
    names = [n for n in a.arms.split(",") if n]
    unknown = [n for n in names if n not in ARMS]
    if unknown:
        ap.error("unknown arm(s): %s" % ",".join(unknown))
    src = Sources(a.smi_index)
    out = Path(a.out)
    done = []
    for n in names:
        print("ARM_START", n, time.strftime("%H:%M:%S"), flush=True)
        s = run_arm(n, ARMS[n], a.probe, out, src, a.ceiling, a.fb_limit_mib, a.wall_s, a.settle_s, a.leak_tol_mib, a.gpu)
        done.append(s)
        print("ARM_DONE", n, "sessions=%s stop=%s kib_per_session=%s after_free=%s after_exit=%s xids=%s" % (
            s.get("sessions_total"), s.get("stop_cause"), s.get("fb_kib_per_session"),
            s.get("leak_after_free_vs_setup"), s.get("leak_after_exit_vs_baseline"), s.get("new_xids")), flush=True)
        if s.get("new_xids"):
            print("STOP_ARMS: an Xid appeared in", n, flush=True)
            break
    print(table(done))
    (out / "summary.json").write_text(json.dumps(done, indent=1, sort_keys=True) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
