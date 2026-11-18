#!/usr/bin/env python3
"""Shared helpers for the win11-base build (edited on the coordinator, copied to the pc; never edited on the pc).
STATUS: scratch tooling 2026-11-15. Every path is under PROP_DIR (default /workspace/pc1/winvm/win11_base).
Image A (/workspace/bench/propimage_20261112) is read-only for us: refuse any path under it.
"""
import importlib.util
import json
import os
import shlex
import socket
import subprocess
import sys
import time
from pathlib import Path

D = Path(os.environ.get("PROP_DIR", "/workspace/pc1/winvm/win11_base")).resolve()
A_DIR = Path("/workspace/bench/propimage_20261112")
B_DIR = Path("/workspace/pc1/winvm/propimage_b")
PROTECTED = [A_DIR, B_DIR, Path("/var/lib/kf-windows-20261005"), Path("/workspace/pc1/winvm/ownerwin"), Path("/data"),
             Path("/srv/kayfabe-coord-backup"), Path("/workspace/ansys_student")]
PACK_DIR = Path("/workspace/pc1/winvm/win11_base_pack")
EVID = Path("/workspace/pc1/winvm/win11_base/evidence")
SU = "ubuntu"
DISTRO_QEMU = "/usr/bin/qemu-system-x86_64"
KF_QEMU = "/workspace/bench/kf3-bins/6bfa2dbc/qemu-system-x86_64"   # origin/next 6bfa2dbc2: contains #438 (Windows profile default)
ISO = {
    "win": "/srv/win-storage/win11x64.iso",
    "win_sha": "768984706b909479417b2368438909440f2967ff05c6a9195ed2667254e465e3",
    "virtio_sha": "303f7ae40dad495d6ae474fdc571df58958a4dbc5c37a522d80f9a203867949d",
}


def guard(p):
    p = Path(p).resolve()
    for prot in PROTECTED:
        if p == prot or prot in p.parents:
            raise SystemExit("REFUSED: %s is under %s (existing image or data: read-only for this build)" % (p, prot))
    return p


guard(D)
if not str(D).startswith("/workspace/pc1/winvm/win11_base"):
    raise SystemExit("REFUSED: PROP_DIR must be under /workspace/pc1/winvm/win11_base (AppArmor swtpm profile)")


def load_pi():
    spec = importlib.util.spec_from_file_location("propimage", D / "bin/propimage.py")
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def load_check():
    spec = importlib.util.spec_from_file_location("propimage_check", D / "bin/propimage_check.py")
    m = importlib.util.module_from_spec(spec)
    sys.modules["propimage_check"] = m
    spec.loader.exec_module(m)
    return m


def log(msg, fname="progress.log"):
    line = "%s %s" % (time.strftime("%Y-%m-%dT%H:%M:%S%z"), msg)
    print(line, flush=True)
    (D / "run").mkdir(exist_ok=True)
    with open(D / "run" / fname, "a") as f:
        f.write(line + "\n")


def sh(cmd, check=True, **kw):
    kw.setdefault("capture_output", True)
    kw.setdefault("text", True)
    p = subprocess.run(cmd, check=False, **kw)
    if check and p.returncode:
        raise SystemExit("command failed rc=%d: %s\n%s" % (p.returncode, shlex.join(cmd), (p.stderr or p.stdout)[-800:]))
    return p


def free_gb(path="/"):
    s = os.statvfs(path)
    return s.f_bavail * s.f_frsize / 1e9


def need_free(gb, why):
    f = free_gb()
    if f < gb + 20:
        raise SystemExit("REFUSED: %s needs ~%.0f GB and the pc must keep >= 20 GB free; free now %.1f GB" % (why, gb, f))


class Qmp:
    def __init__(self, path, timeout=10):
        self.s = socket.socket(socket.AF_UNIX)
        self.s.settimeout(timeout)
        self.s.connect(str(path))
        self.f = self.s.makefile("rw")
        json.loads(self.f.readline())
        self.cmd("qmp_capabilities")

    def cmd(self, name, **args):
        self.f.write(json.dumps({"execute": name, "arguments": args}) + "\n")
        self.f.flush()
        while True:
            r = json.loads(self.f.readline())
            if "return" in r:
                return r["return"]
            if "error" in r:
                raise RuntimeError(json.dumps(r["error"]))

    def key(self, *qcodes):
        self.cmd("send-key", keys=[{"type": "qcode", "data": q} for q in qcodes])

    def shot(self, path):
        self.cmd("screendump", filename=str(path), format="png")

    def close(self):
        try:
            self.s.close()
        except OSError:
            pass


def ssh_base(state, key):
    return ["ssh", "-i", str(key), "-p", str(state["ids"]["net"]["ssh_port"]), "-o", "StrictHostKeyChecking=no",
            "-o", "UserKnownHostsFile=/dev/null", "-o", "LogLevel=ERROR", "-o", "ConnectTimeout=15", "-o", "BatchMode=yes",
            "-o", "IdentitiesOnly=yes", "%s@127.0.0.1" % state["guest"]["user"]]


def ssh_ps(state, key, script, timeout=300):
    """One PowerShell script on ssh stdin (what propimage.py does for its probe)."""
    p = subprocess.run(ssh_base(state, key) + ["powershell", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", "-"],
                       input=script, text=True, capture_output=True, timeout=timeout, errors="replace")
    return p


def qemu_pid():
    """pid of OUR qemu only: its command line starts with the qemu path and the unique -name propimage-w11-."""
    r = subprocess.run(["pgrep", "-f", "-n", r"^[^ ]*qemu-system-x86_64 -name propimage-w11-"], capture_output=True, text=True)
    return int(r.stdout.split()[0]) if r.returncode == 0 and r.stdout.split() else None


def start_swtpm(workdir):
    wd = str(workdir)
    for stale in ("swtpm.sock", "run/swtpm.pid"):
        (Path(wd) / stale).unlink(missing_ok=True)
    cmd = ["runuser", "-u", SU, "--", "swtpm", "socket", "--tpm2", "--tpmstate", "dir=%s/tpm,mode=0600" % wd,
           "--ctrl", "type=unixio,path=%s/swtpm.sock,mode=0600" % wd, "--log", "file=%s/run/swtpm.log,level=20" % wd,
           "--pid", "file=%s/run/swtpm.pid" % wd, "--terminate", "--daemon"]
    sh(cmd)
    for _ in range(50):
        if (Path(wd) / "swtpm.sock").exists():
            return
        time.sleep(0.2)
    raise SystemExit("swtpm made no socket (see %s/run/swtpm.log; AppArmor?)" % wd)


def start_qemu(argv, workdir):
    """QEMU as the desktop user, low priority (the pc is shared with the owner's live session). Detached."""
    wd = Path(workdir)
    out = open(wd / "run/qemu.log", "ab")
    full = ["runuser", "-u", SU, "--", "nice", "-n", "19", "ionice", "-c2", "-n7"] + argv
    p = subprocess.Popen(full, stdout=out, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL, start_new_session=True)
    time.sleep(1.5)
    if p.poll() is not None:
        raise SystemExit("qemu exited at once rc=%s: %s" % (p.returncode, (wd / "run/qemu.log").read_text()[-600:]))
    pid = qemu_pid()
    if pid:
        (wd / "run/qemu.pid").write_text(str(pid))
    return p
