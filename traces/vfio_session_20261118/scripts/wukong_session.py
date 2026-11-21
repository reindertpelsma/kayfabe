#!/usr/bin/env python3
"""pc Wukong session (image A, one boot, N attempts inside it, driver = go11.sh = go10.sh without lock/bind). Reuses the SAFE bind/restore of pcsession.py.
usage: wukong_session.py --tag T --platform kf3|vfio --attempts N [--rev 6bfa2dbc] [--fb-mb N]"""
import argparse, fcntl, os, signal, subprocess, sys, time
from pathlib import Path
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import pcsession as P

ap = argparse.ArgumentParser()
ap.add_argument("--tag", required=True); ap.add_argument("--platform", choices=("kf3", "vfio"), required=True)
ap.add_argument("--attempts", type=int, default=3); ap.add_argument("--rev", default="6bfa2dbc"); ap.add_argument("--fb-mb", type=int)
ap.add_argument("--timeout", type=int, default=5400)
a = ap.parse_args()
state = {"vfio_bound": False}
EV = P.ROOT / "ev"; EV.mkdir(exist_ok=True)


def cleanup(*_):
    if state["vfio_bound"]:
        state["vfio_bound"] = False
        try:
            out = P.restore_vfio()
        except P.Abort as e:
            out = str(e); P.abort_marker(a.tag, out)
        (EV / ("%s_restore.txt" % a.tag)).write_text(out)
    P.active("END %s wukong (session %s); GPU: %s" % (a.platform, a.tag, P.smi_name()))


def term(sig, frm):
    cleanup(); os._exit(130)


for s in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
    signal.signal(s, term)
fd = os.open(P.LOCK, os.O_RDWR | os.O_CREAT)
while True:
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB); break
    except OSError:
        time.sleep(20)
P.settle_after_restore(log=lambda m: print(m, flush=True))
P.active("START %s wukong attempts=%d tag=%s rev=%s fb-mb=%s image=a FG-default(on): throwaway clone restrict=on, flock" % (a.platform, a.attempts, a.tag, a.rev, a.fb_mb))
try:
    if a.platform == "kf3":
        rc, o = P.sh(["nvidia-smi", "--query-gpu=name", "--format=csv,noheader"], 30)
        if rc != 0:
            P.abort_marker(a.tag, "kf3: nvidia-smi does not answer (rc=%s)" % rc); sys.exit(9)
        try:
            (EV / ("%s_host_settle.txt" % a.tag)).write_text(P.wait_host_modules_settled() + "\n")
        except P.Abort as e:
            (EV / ("%s_host_settle.txt" % a.tag)).write_text(str(e) + "\n"); P.abort_marker(a.tag, str(e)); sys.exit(9)
        probe = "import ctypes,sys; l=ctypes.CDLL('libcuda.so.1'); r=l.cuInit(0); print('cuInit',r); sys.exit(0 if r==0 else 1)"
        ok = False
        for i in range(8):
            rc, o = P.sh(["python3", "-c", probe], 40)
            (EV / ("%s_cuda_ready.txt" % a.tag)).open("a").write("%s try %d rc=%s %s\n" % (P.now(), i, rc, o.strip()[:100]))
            if rc == 0:
                ok = True; break
            time.sleep(30)
        if not ok:
            P.active("kf3 session %s: cuInit never ready, NOT RUN" % a.tag); sys.exit(8)
    else:
        try:
            ok, out = P.bind_vfio()
        except P.Abort as e:
            (EV / ("%s_bind.txt" % a.tag)).write_text(str(e)); P.abort_marker(a.tag, str(e)); print("ABORT bind", str(e)[-400:], flush=True)
            state["vfio_bound"] = P.recover_after_bind_abort(e)
            sys.exit(9)
        (EV / ("%s_bind.txt" % a.tag)).write_text(out)
        if not ok:
            print("bind failed", out[-300:], flush=True); sys.exit(4)
        state["vfio_bound"] = True
    env = dict(os.environ, PLATFORM=a.platform, REV=a.rev, FLAGS="flags_off.env", NOLOCK="1")
    if a.fb_mb and a.platform == "kf3":
        env["EXTRA"] = "--fb-mb %d" % a.fb_mb
    t1 = time.time()
    with open(P.ROOT / ("session_%s_go11.out" % a.tag), "w") as so:
        try:
            rc = subprocess.run(["bash", "go11.sh", a.tag, str(a.attempts)], cwd="/workspace/bench/pcwukong_20261115", env=env, stdout=so, stderr=subprocess.STDOUT, timeout=a.timeout).returncode
        except subprocess.TimeoutExpired:
            rc = "TIMEOUT"
    print("WUKONG-DONE %s rc=%s dur=%d" % (a.tag, rc, time.time() - t1), flush=True)
finally:
    cleanup()
