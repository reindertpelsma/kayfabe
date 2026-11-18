#!/usr/bin/env python3
"""Run ONE build stage as one job: take /root/prov/pc_gpu.lock (blocking, bounded wait), write START/END lines to /root/prov/PC_ACTIVE.log,
run the stage script as a child, release. Own PID only. usage: w_stage.py NAME [--wait-s N] -- CMD... (stages that take the lock themselves, w_gpu.py, are run directly)."""
import fcntl, os, subprocess, sys, time

LOCK, ACTIVE = "/root/prov/pc_gpu.lock", "/root/prov/PC_ACTIVE.log"


def now():
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def active(msg):
    with open(ACTIVE, "a") as f:
        f.write("%s win11base %s\n" % (now(), msg))


def main():
    a = sys.argv[1:]
    name = a[0]
    wait = int(a[a.index("--wait-s") + 1]) if "--wait-s" in a else 3600
    cmd = a[a.index("--") + 1:]
    fd = os.open(LOCK, os.O_RDWR | os.O_CREAT)
    t0 = time.time()
    while True:
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            break
        except OSError:
            if time.time() - t0 > wait:
                print("GPU lock busy for %d s: giving up (nothing started)" % wait, flush=True)
                return 7
            time.sleep(15)
    active("START %s (new Windows 11 base build; no GPU use unless stated): %s" % (name, " ".join(cmd)[:120]))
    rc = 1
    try:
        rc = subprocess.run(cmd).returncode
        return rc
    finally:
        active("END %s rc=%s" % (name, rc))
        fcntl.flock(fd, fcntl.LOCK_UN)


if __name__ == "__main__":
    sys.exit(main())
