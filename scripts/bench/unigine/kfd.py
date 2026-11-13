#!/usr/bin/env python3
"""kfd: interactive driver daemon for the PC (root). Run under flock; polls <dir>/cmd.py, exec()s it with
helpers in scope, writes <dir>/out.txt, then removes cmd.py. Exits after IDLE seconds without a command.
Helpers: start(cmd_args)->proc (offline netns, as ubuntu), wins(), shot(winid,name), mouse(x,y), click(x,y), key(name), ptr()"""
import sys, os, time, subprocess, signal, io, contextlib, traceback, ctypes
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import kfin
D = os.path.dirname(os.path.abspath(__file__)); RUN = D + "/run"; HOME = D + "/home"
IDLE = int(sys.argv[2]) if len(sys.argv) > 2 else 600
SHOTS = RUN + "/shots"; os.makedirs(SHOTS, exist_ok=True)
sw, sh = kfin.screen_size(); inp = kfin.Input(sw, sh)
procs = []

def start(args, cwd=None, offline=True, extra_env=None):
    pw, e = kfin.env(); e["HOME"] = HOME; e["QT_QPA_PLATFORM"] = "xcb"; e.update(extra_env or {})
    pre = ["unshare", "-n"] if offline else []
    cmd = pre + ["setpriv", "--reuid=1000", "--regid=1000", "--clear-groups", "env", "-i"] + [f"{k}={v}" for k, v in e.items()] + args
    log = open(f"{RUN}/proc{len(procs)}.log", "wb")
    p = subprocess.Popen(cmd, stdout=log, stderr=subprocess.STDOUT, cwd=cwd or D, start_new_session=True)
    procs.append(p); return p
def stop(p, sig=signal.SIGTERM):
    if p.poll() is None:
        try: os.killpg(p.pid, sig)
        except ProcessLookupError: pass
wins = lambda: [w for w in kfin.windows() if w[1] > 50 and w[2] > 50 and "mutter" not in w[5]]
def shot(wid, name):
    return kfin.xwd_png(wid, f"{SHOTS}/{name}.png")
def mouse(x, y): inp.move(x, y)
def click(x, y): inp.click(x, y)
def key(k): inp.key(k)
def ptr():
    X = ctypes.CDLL("libX11.so.6"); os.environ.setdefault("DISPLAY", ":0")
    pw, e = kfin.env(); os.environ.update({"XAUTHORITY": e["XAUTHORITY"], "DISPLAY": ":0"})
    X.XOpenDisplay.restype = ctypes.c_void_p; d = X.XOpenDisplay(None)
    X.XDefaultRootWindow.restype = ctypes.c_ulong; root = X.XDefaultRootWindow(ctypes.c_void_p(d))
    r, c = ctypes.c_ulong(), ctypes.c_ulong(); rx, ry, wx, wy = (ctypes.c_int() for _ in range(4)); m = ctypes.c_uint()
    X.XQueryPointer(ctypes.c_void_p(d), ctypes.c_ulong(root), ctypes.byref(r), ctypes.byref(c), ctypes.byref(rx), ctypes.byref(ry), ctypes.byref(wx), ctypes.byref(wy), ctypes.byref(m))
    X.XCloseDisplay(ctypes.c_void_p(d)); return (rx.value, ry.value, wx.value, wy.value, hex(c.value))

if __name__ == "__main__":
    last = time.time(); cmdf = RUN + "/cmd.py"
    open(RUN + "/daemon.pid", "w").write(str(os.getpid()))
    try:
        while time.time() - last < IDLE:
            if os.path.exists(cmdf):
                src = open(cmdf).read(); os.remove(cmdf); last = time.time(); buf = io.StringIO()
                with contextlib.redirect_stdout(buf):
                    try: exec(compile(src, "cmd", "exec"), globals())
                    except SystemExit: break
                    except Exception: traceback.print_exc(file=buf)
                open(RUN + "/out.txt.tmp", "w").write(buf.getvalue()); os.rename(RUN + "/out.txt.tmp", RUN + "/out.txt")
            time.sleep(0.3)
    finally:
        for p in procs: stop(p)
        time.sleep(2)
        for p in procs: stop(p, signal.SIGKILL)
        inp.close()
