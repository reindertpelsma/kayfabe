#!/usr/bin/env python3
"""qsampler.py - host-side sampler of the kayfabe VMM's threads while a baton session runs.

  python3 -I qsampler.py OUT.tsv [PERIOD_S=0.25]

Waits for a process whose command line contains 'kf-fastguest', then every PERIOD_S writes one line per
thread that is interesting: CPU time advanced since the last sample, or the thread is not asleep in a
plain wait.  Columns: epoch  tid  comm  state  wchan  syscall_nr  d_cpu_ticks.  For a thread in an
ioctl (syscall 16) or in D state, the kernel stack (first 6 frames, ';'-joined) is appended.
Pure observation (reads /proc only).  Stops when the process is gone.
"""
import glob
import os
import sys
import time

QUIET_WCHAN = {"futex_do_wait", "futex_wait_queue", "do_epoll_wait", "ep_poll", "do_sys_poll",
               "poll_schedule_timeout.constprop.0", "hrtimer_nanosleep", "do_nanosleep", "0", ""}


def find_pid():
    for p in glob.glob("/proc/[0-9]*"):
        try:
            with open(p + "/cmdline", "rb") as f:
                c = f.read()
        except OSError:
            continue
        if b"kf-fastguest" in c and b"qemu-system" in c:
            return int(p.split("/")[-1])
    return None


def rd(path, default=""):
    try:
        with open(path) as f:
            return f.read().strip()
    except OSError:
        return default


def main(out, period):
    pid = None
    while pid is None:
        pid = find_pid()
        if pid is None:
            time.sleep(0.2)
    last = {}
    with open(out, "w") as o:
        o.write("# sampler pid=%d period=%s\n" % (pid, period))
        while os.path.exists("/proc/%d" % pid):
            now = time.time()
            for t in glob.glob("/proc/%d/task/*" % pid):
                tid = t.rsplit("/", 1)[-1]
                st = rd(t + "/stat")
                if not st:
                    continue
                r = st.rsplit(")", 1)
                comm = r[0].split("(", 1)[-1]
                f = r[1].split()
                state, cpu = f[0], int(f[11]) + int(f[12])
                d = cpu - last.get(tid, cpu)
                last[tid] = cpu
                wchan = rd(t + "/wchan", "0")
                sysc = rd(t + "/syscall", "").split(" ")[0]
                if d == 0 and state in "SI" and wchan in QUIET_WCHAN and sysc not in ("16",):
                    continue
                extra = ""
                if sysc == "16" or state == "D":
                    extra = ";".join(l.split("] ", 1)[-1].split("+")[0] for l in rd(t + "/stack").splitlines()[:6])
                o.write("%.3f\t%s\t%s\t%s\t%s\t%s\t%d\t%s\n" % (now, tid, comm, state, wchan, sysc, d, extra))
            o.flush()
            time.sleep(period)


if __name__ == "__main__":
    main(sys.argv[1], float(sys.argv[2]) if len(sys.argv) > 2 else 0.25)
