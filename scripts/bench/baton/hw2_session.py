#!/usr/bin/env python3
"""One baton boot; caller holds the shared GPU flock. Signal owned children only."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time


def monitor_owned(proc, seconds, xid_count, baseline, sample):
    deadline = time.monotonic() + seconds
    reason = "exit"
    while proc.poll() is None:
        sample()
        if xid_count() > baseline:
            reason = "new_xid"
            break
        if time.monotonic() >= deadline:
            reason = "deadline"
            break
        try:
            proc.wait(timeout=min(0.25, max(0.001, deadline - time.monotonic())))
        except subprocess.TimeoutExpired:
            pass
    if proc.poll() is None:
        proc.terminate()
        try:
            proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            print("OWNED_CHILD_HUNG: ignored TERM after " + reason, flush=True)
            proc.kill()
            proc.wait()
    return reason, proc.returncode


def xid_count():
    r = subprocess.run(["dmesg"], capture_output=True, text=True, check=True)
    return sum("Xid" in line for line in r.stdout.splitlines())


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("tag")
    ap.add_argument("--binary", required=True)
    ap.add_argument("--guest", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--seq", default="pair:120,pair:120")
    ap.add_argument("--control", action="store_true")
    ap.add_argument("--infinite", action="store_true")
    ap.add_argument("--prof", action="store_true")
    args = ap.parse_args()
    out = Path(args.out).resolve() / args.tag
    out.mkdir(parents=True, exist_ok=False)
    guest = Path(args.guest).resolve()
    binary = Path(args.binary).resolve()
    for p in [binary, guest / "vmlinuz", guest / "initrd.cpio.gz"]:
        if not p.is_file():
            raise RuntimeError("missing input: " + str(p))
    budget = 90 + sum(int(item.split(":")[1]) + 25 for item in args.seq.split(","))
    env = {k: v for k, v in os.environ.items() if not k.startswith("KF3_")}
    env.update(KF3_DEBUG="1", KF3_DEBUG_FORCE_MIGRATABLE="1", KF3_TSG_MIRROR="0",
               KF3_DIAG_PROGRESS="1000", KAYFABE_GUEST_BAR1_MB="128")
    if args.control:
        env["KF3_NEGCTL_NO_BUSY_LOOK"] = "1"
    if args.infinite:
        env["KF3_WORKER_PARK_INFINITE"] = "1"
    if args.prof:
        env["KF3_PROF"] = "1"
    serial = out / ("fast_" + args.tag + "_serial.log")
    qlog = out / ("fast_" + args.tag + "_qemu.log")
    command = [str(binary), "-name", "kf-fastguest", "-object",
               "memory-backend-memfd,id=ram0,size=6144M,share=on", "-machine",
               "q35,accel=kvm,memory-backend=ram0", "-m", "6144", "-cpu", "host", "-smp", "4",
               "-kernel", str(guest / "vmlinuz"), "-initrd", str(guest / "initrd.cpio.gz"),
               "-append", "console=ttyS0 console=hvc0 panic=1 loglevel=4 KF_BATON_SEQ=" + args.seq
               + " KF_ARMS=--probe-launch-dma KF_IOCTL_TRACE=ring KF_BUDGET_S=" + str(budget),
               "-device", "kf3-gpu,fb-mb=8192,bar1-size=134217728,bar2-size=33554432,id=kf0",
               "-msg", "timestamp=on", "-serial", "file:" + str(out / "ttyS0.log"),
               "-device", "virtio-serial-pci,id=kfvs0", "-chardev", "file,id=kfcon0,path=" + str(serial),
               "-device", "virtconsole,chardev=kfcon0,bus=kfvs0.0", "-display", "none"]
    baseline = xid_count()
    start = time.time()
    receipt = dict(tag=args.tag, start_epoch=start, command=command,
                   env={k: v for k, v in env.items() if k.startswith("KF3_")}, xid_before=baseline)
    (out / "start.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print("SESSION_START " + args.tag + " " + time.strftime("%FT%T%z"), flush=True)
    last = {}
    with qlog.open("w") as log, (out / "qsample.tsv").open("w") as samples:
        proc = subprocess.Popen(command, env=env, stdout=log, stderr=subprocess.STDOUT)
        def sample():
            now = time.time()
            for task in Path("/proc", str(proc.pid), "task").glob("*"):
                try:
                    st = (task / "stat").read_text().rsplit(")", 1)
                    fields = st[1].split()
                    comm = st[0].split("(", 1)[1]
                    cpu = int(fields[11]) + int(fields[12])
                    delta = cpu - last.get(task.name, cpu)
                    last[task.name] = cpu
                    wchan = (task / "wchan").read_text().strip()
                    syscall = (task / "syscall").read_text().split()[0]
                    samples.write(f"{now:.3f}\t{task.name}\t{comm}\t{fields[0]}\t{wchan}\t{syscall}\t{delta}\n")
                except (OSError, IndexError, ValueError):
                    continue
            samples.flush()
        reason, rc = monitor_owned(proc, budget, xid_count, baseline, sample)
    after = xid_count()
    files = out / "files"
    files.mkdir()
    active = None
    if serial.exists():
        for line in serial.read_text(errors="replace").splitlines(keepends=True):
            if line.startswith("BATON_FILE_BEGIN "):
                name = line.split()[1]
                if Path(name).name != name or name in (".", ".."):
                    raise RuntimeError("invalid guest artifact name")
                active = (files / name).open("w")
            elif line.startswith("BATON_FILE_END "):
                if active:
                    active.close()
                active = None
            elif active:
                active.write(line)
        if active:
            active.close()
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import campaign_report
    pairs = [campaign_report.analyze_run(str(files), name, 1.0)
             for name in campaign_report.group_runs(str(files)) if name.startswith("pair")]
    text = qlog.read_text(errors="replace")
    summaries = [s for s in text.splitlines() if "USER_RING_LOOP SUMMARY" in s]
    workers = [s for s in text.splitlines() if "PROF workers " in s]
    strands = [s for s in text.splitlines() if re.search(r"PROGRESS_STRAND (?:for_ms=|relay=)", s)]
    work = re.findall(r"backstop_found\(work=(\d+)", summaries[-1] if summaries else "")
    timeouts = re.findall(r"timeouts_with_work=(\d+)", workers[-1] if workers else "")
    handoff = re.findall(r"handoff\(registered=(\d+) rung=(\d+) retried=(\d+)\)", summaries[-1] if summaries else "")
    gap = max((max(p["longest_gap_s"].values(), default=0) for p in pairs), default=0)
    expected = sum(x.startswith("pair:") for x in args.seq.split(","))
    failed = reason != "exit" or rc != 0 or after > baseline or len(pairs) != expected
    if not args.control:
        failed |= gap >= 1 or bool(strands) or not work or int(work[-1]) > 0
        failed |= bool(timeouts) and int(timeouts[-1]) > 0
        failed |= any(int(a) > int(b) + int(c) for a, b, c in handoff)
    result = dict(receipt, end_epoch=time.time(), reason=reason, qemu_rc=rc, xid_after=after,
                  pairs=pairs, max_gap_s=gap, strands=strands, final_summary=summaries[-1:] ,
                  final_workers=workers[-1:], failed=bool(failed), clk_tck=os.sysconf("SC_CLK_TCK"))
    (out / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(f"SESSION_END {args.tag} failed={failed} pairs={len(pairs)} max_gap={gap:.6f} xid={after}", flush=True)
    return int(failed)


if __name__ == "__main__":
    sys.exit(main())
