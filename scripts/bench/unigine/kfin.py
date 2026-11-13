#!/usr/bin/env python3
"""kfin: stdlib+PIL helpers for GUI driving on the PC (GNOME Wayland + Xwayland :0).
Input via /dev/uinput (root): one virtual keyboard + one absolute pointer.
Screenshots via `xwd -id <win>` run as the desktop user, converted by PIL.
CLI: kfin.py shot <winid> out.png | click X Y | key NAME   (X Y in X11 root px)"""
import os, sys, struct, fcntl, time, subprocess, re, pwd
from PIL import Image

USER = "ubuntu"
UI_SET_EVBIT, UI_SET_KEYBIT, UI_SET_ABSBIT = 0x40045564, 0x40045565, 0x40045567
UI_DEV_CREATE, UI_DEV_DESTROY = 0x5501, 0x5502
EV_SYN, EV_KEY, EV_ABS = 0, 1, 3
BTN_LEFT = 0x110
KEYS = {"enter": 28, "esc": 1, "tab": 15, "space": 57, "up": 103, "down": 108, "left": 105, "right": 106,
        "f12": 88, "backspace": 14, "alt": 56, "pgdn": 109}
for c, k in zip("qwertyuiop", range(16, 26)): KEYS[c] = k
for c, k in zip("asdfghjkl", range(30, 39)): KEYS[c] = k
for c, k in zip("zxcvbnm", range(44, 51)): KEYS[c] = k

def env():
    pw = pwd.getpwnam(USER); rt = f"/run/user/{pw.pw_uid}"
    xa = sorted(f for f in os.listdir(rt) if f.startswith(".mutter-Xwaylandauth"))
    e = {"HOME": pw.pw_dir, "USER": USER, "PATH": "/usr/bin:/bin", "XDG_RUNTIME_DIR": rt, "DISPLAY": ":0",
         "XAUTHORITY": os.path.join(rt, xa[0]) if xa else ""}
    return pw, e

def as_user(cmd, **kw):
    pw, e = env()
    def pre():
        os.setgid(pw.pw_gid); os.setgroups([pw.pw_gid]); os.setuid(pw.pw_uid)
    return subprocess.run(cmd, env=e, preexec_fn=pre, capture_output=True, **kw)

def screen_size():
    out = as_user(["xdpyinfo"]).stdout.decode()
    m = re.search(r"dimensions:\s+(\d+)x(\d+)", out); return int(m[1]), int(m[2])

class Input:
    def __init__(self, sw, sh):
        self.sw, self.sh = sw, sh
        self.fd = os.open("/dev/uinput", os.O_WRONLY | os.O_NONBLOCK)
        f = self.fd
        for e in (EV_KEY, EV_ABS, EV_SYN): fcntl.ioctl(f, UI_SET_EVBIT, e)
        for k in set(KEYS.values()) | {BTN_LEFT}: fcntl.ioctl(f, UI_SET_KEYBIT, k)
        fcntl.ioctl(f, UI_SET_ABSBIT, 0); fcntl.ioctl(f, UI_SET_ABSBIT, 1)
        absmax = [0] * 64; absmin = [0] * 64; absmax[0] = 65535; absmax[1] = 65535
        dev = struct.pack("80sHHHHi", b"kfin-input", 3, 0x1234, 0x5679, 1, 0)
        dev += struct.pack("64i", *absmax) + struct.pack("64i", *absmin) + b"\0" * 512
        os.write(f, dev); fcntl.ioctl(f, UI_DEV_CREATE); time.sleep(1.5)
    def ev(self, t, c, v): os.write(self.fd, struct.pack("llHHi", 0, 0, t, c, v))
    def syn(self): self.ev(EV_SYN, 0, 0)
    def move(self, x, y):
        self.ev(EV_ABS, 0, int(x * 65535 / (self.sw - 1))); self.ev(EV_ABS, 1, int(y * 65535 / (self.sh - 1))); self.syn(); time.sleep(0.3)
    def click(self, x, y):
        self.move(x, y); self.move(x + 1, y + 1); self.move(x, y); time.sleep(0.2)
        self.ev(EV_KEY, BTN_LEFT, 1); self.syn(); time.sleep(0.1); self.ev(EV_KEY, BTN_LEFT, 0); self.syn(); time.sleep(0.3)
    def key(self, name):
        k = KEYS[name]; self.ev(EV_KEY, k, 1); self.syn(); time.sleep(0.08); self.ev(EV_KEY, k, 0); self.syn(); time.sleep(0.15)
    def close(self):
        try: fcntl.ioctl(self.fd, UI_DEV_DESTROY)
        finally: os.close(self.fd)

def xwd_png(winid, path):
    r = as_user(["xwd", "-silent", "-id", str(winid)])
    if r.returncode or not r.stdout: return None
    b = r.stdout; hs = struct.unpack(">25I", b[:100]); hsize = hs[0]
    w, h, bpl, ncol = hs[4], hs[5], hs[12], hs[19]
    data = b[hsize + ncol * 12: hsize + ncol * 12 + bpl * h]
    img = Image.frombuffer("RGBX", (bpl // 4, h), data, "raw", "BGRX", 0, 1).crop((0, 0, w, h)).convert("RGB")
    img.save(path); return img

def windows():
    """[(id, w, h, x, y, name)] of X windows with a name (any map state; filter by size)."""
    out = as_user(["xwininfo", "-root", "-tree"]).stdout.decode(errors="replace")
    res = []
    for l in out.splitlines():
        m = re.match(r'\s+(0x[0-9a-f]+) "(.+?)":.*?\s(\d+)x(\d+)\+(-?\d+)\+(-?\d+)\s+\+(-?\d+)\+(-?\d+)', l)
        if m: res.append((int(m[1], 16), int(m[3]), int(m[4]), int(m[7]), int(m[8]), m[2]))
    return res

if __name__ == "__main__":
    a = sys.argv[1:]
    if a[0] == "shot": xwd_png(a[1], a[2])
    elif a[0] in ("click", "key"):
        sw, sh = screen_size(); i = Input(sw, sh)
        try: i.click(int(a[1]), int(a[2])) if a[0] == "click" else i.key(a[1])
        finally: i.close()
