#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Session driver of the Premiere Pro sequence-crash control (runs ON the pc, detached; PREREGISTRATION.md beside this file).

One process = one boot of one arm:
  K/S: kf3 binary kf3-bins/<rev> (no bind), V: stock QEMU with the RTX 4070 on vfio-pci (scripts/bench/pc_bind.py, guards of #586).
Holds the pc GPU lock for its whole life, writes PC_ACTIVE START/END, records the host state before, builds a throwaway thin
overlay on the add-on overlay (never written), copies OVMF_VARS and the swtpm state, runs the guest OFFLINE (restrict=on),
waits until a STOP file appears or the wall cap passes, shuts the guest down, restores (V) and verifies the host.
The UI actions are driven from outside (drv.py over the guest's ssh); this process only owns the VM and the GPU.

usage: ppsession.py --arm K|S|V --tag T [--rev ed41beea] [--wall-min 75]
"""
import argparse
import fcntl
import json
import os
import re
import shlex
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

CODE = Path("/workspace/bench/pc_premiere_20261124/code")
sys.path.insert(0, str(CODE))
import pc_bind as PB  # noqa: E402
import pc_guard as G  # noqa: E402

HERE = Path("/workspace/bench/pc_premiere_20261124")
SRC = Path("/workspace/pc1/winvm/propimage_b/runs/b2_addon_cand")
TEMPLATE = SRC / "run/last_cmd.txt"
BASE_DISK = SRC / "run/clone.qcow2"
ISO = Path("/workspace/bench/pc_vfio_20261116/test-assets-20261116c.iso")
VMROOT = Path("/workspace/pc1/winvm/pc_premiere_20261124")
LOCK = "/root/prov/pc_gpu.lock"
ACTIVE = Path("/root/prov/PC_ACTIVE.log")
STOCK_QEMU = "/usr/bin/qemu-system-x86_64"
# the virtual sound card of the Vast run (scripts/bench/windows/propimage.py AUDIO_ARGS), in every arm
AUDIO_ARGS = ("-audiodev", "none,id=kfaudio", "-device", "ich9-intel-hda,id=kfhda", "-device", "hda-duplex,bus=kfhda.0,audiodev=kfaudio")


def now():
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def active(msg):
    with ACTIVE.open("a") as f:
        f.write("%s pc-premiere %s\n" % (now(), msg))


def say(out, msg):
    with (out / "session.log").open("a") as f:
        f.write("%s %s\n" % (now(), msg))


def scrub(t):
    t = re.sub(r"GPU-[0-9a-fA-F-]{20,}", "GPU-<uuid>", t)
    t = re.sub(r"gpu-uuid=[^,\s']+", "gpu-uuid=<id>", t)
    t = re.sub(r"(-uuid )\S+", r"\1<uuid>", t)
    t = re.sub(r"serial=[^,\s']+", "serial=<serial>", t)
    t = re.sub(r"mac=[^,\s']+", "mac=<mac>", t)
    t = re.sub(r"net=\d+\.\d+\.\d+\.\d+/\d+", "net=<subnet>", t)
    t = re.sub(r"hostfwd=tcp:127\.0\.0\.1:\d+", "hostfwd=tcp:127.0.0.1:<port>", t)
    t = re.sub(r"PARTUUID=\S+", "PARTUUID=<id>", t)
    return t


def host_state(tag):
    """Read-only host facts; never runs an nvidia tool (safe while bound)."""
    def rd(p):
        try:
            return Path(p).read_text().strip()
        except OSError:
            return None
    dp = {}
    for p in sorted(Path("/sys/class/drm").glob("card*-DP-*")):
        dp[p.name] = rd(p / "status")
    st = {
        "tag": tag, "utc": now(),
        "nvidia_version": rd("/sys/module/nvidia/version"),
        "driver_01_00_0": G.gpu_driver("0000:01:00.0"), "driver_01_00_1": G.gpu_driver("0000:01:00.1"),
        "modules": sorted(m for m in ("nvidia", "nvidia_drm", "nvidia_modeset", "nvidia_uvm", "vfio_pci", "vfio_pci_core", "vfio_iommu_type1", "vfio")
                          if Path("/sys/module/%s" % m).exists() and Path("/sys/module/%s/initstate" % m).exists()),
        "gdm": subprocess.run(["systemctl", "is-active", "gdm"], capture_output=True, text=True).stdout.strip(),
        "dp_status": dp,
        "udev_queue_empty": G.udev_queue_empty(PB.sh(["udevadm", "settle", "--timeout=0"], 30)[0], G.UDEV_QUEUE_FLAG.exists()),
        "modprobe_block_present": G.block_present(),
    }
    return json.dumps(st, indent=1)


def cuinit_probe():
    probe = "import ctypes,sys; l=ctypes.CDLL('libcuda.so.1'); r=l.cuInit(0); print('cuInit',r); sys.exit(0 if r==0 else 1)"
    return PB.sh(["python3", "-c", probe], 60)


def build_argv(arm, qemu, work, rtc_base=""):
    a = shlex.split(TEMPLATE.read_text())
    a[0] = qemu
    out = []
    i = 0
    while i < len(a):
        v = a[i]
        if v == "-name":
            out += ["-name", "pc-premiere-%s" % work.name]; i += 2; continue
        if v == "-drive" and "OVMF_VARS.fd" in a[i + 1]:
            out += ["-drive", re.sub(r"file=\S+", "file=%s" % (work / "OVMF_VARS.fd"), a[i + 1])]; i += 2; continue
        if v == "-drive" and "clone.qcow2" in a[i + 1]:
            out += ["-drive", re.sub(r"file=\S+", "file=%s" % (work / "disk.qcow2"), a[i + 1])]; i += 2; continue
        if v == "-chardev" and "swtpm.sock" in a[i + 1]:
            out += ["-chardev", re.sub(r"path=\S+", "path=%s" % (work / "swtpm.sock"), a[i + 1])]; i += 2; continue
        if v == "-rtc" and rtc_base:
            out += ["-rtc", a[i + 1].replace("base=utc", "base=%s" % rtc_base)]; i += 2; continue
        if v == "-netdev":
            out += ["-netdev", a[i + 1].replace("restrict=off", "restrict=on")]; i += 2; continue
        if v == "-qmp":
            out += ["-qmp", "unix:%s,server=on,wait=off" % (work / "qmp.sock")]; i += 2; continue
        if v == "-device" and a[i + 1].startswith("kf3-gpu"):
            if arm == "V":
                out += ["-device", "vfio-pci,host=0000:01:00.0,multifunction=on,addr=0x4.0", "-device", "vfio-pci,host=0000:01:00.1,addr=0x4.1"]
            else:
                props = [p for p in a[i + 1].split(",") if not p.startswith("display-broker")]
                out += ["-device", ",".join(props)]
            i += 2; continue
        out.append(v); i += 1
    out += list(AUDIO_ARGS)
    out += ["-drive", "if=none,id=cd0,media=cdrom,readonly=on,format=raw,file=%s" % ISO, "-device", "ide-cd,drive=cd0",
            "-pidfile", str(work / "qemu.pid")]
    if arm != "V":
        out += ["-vnc", "unix:%s" % (work / "vnc.sock")]
    j = " ".join(out)
    assert "restrict=on" in j and "restrict=off" not in j and "clone.qcow2" not in j and "display-broker" not in j, j
    assert ("vfio-pci,host=0000:01:00.0" in j) == (arm == "V") and ("kf3-gpu" in j) == (arm != "V")
    return out


def qmp(work, cmd):
    import socket
    s = socket.socket(socket.AF_UNIX)
    s.settimeout(20)
    s.connect(str(work / "qmp.sock"))
    f = s.makefile("rw")
    f.readline()
    for c in ({"execute": "qmp_capabilities"}, {"execute": cmd}):
        f.write(json.dumps(c) + "\n"); f.flush()
        r = f.readline()
    s.close()
    return r


def qemu_alive(work):
    try:
        pid = int((work / "qemu.pid").read_text())
        os.kill(pid, 0)
        return pid
    except (OSError, ValueError):
        return None


def stop_guest(work, out):
    pid = qemu_alive(work)
    if not pid:
        say(out, "qemu already gone")
        return
    rc = subprocess.run(["ssh", "-i", str(SRC / "id_kf"), "-p", "39934", "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null",
                         "-o", "BatchMode=yes", "-o", "ConnectTimeout=15", "kfimage@127.0.0.1", "shutdown /s /t 0"],
                        capture_output=True, text=True, timeout=60).returncode
    say(out, "guest shutdown via ssh rc=%s" % rc)
    for _ in range(90):
        if not qemu_alive(work):
            say(out, "qemu exited after the guest shutdown")
            return
        time.sleep(2)
    try:
        say(out, "system_powerdown: %s" % qmp(work, "system_powerdown").strip())
    except OSError as e:
        say(out, "qmp powerdown failed: %s" % e)
    for _ in range(60):
        if not qemu_alive(work):
            return
        time.sleep(2)
    try:
        say(out, "qmp quit: %s" % qmp(work, "quit").strip())
    except OSError as e:
        say(out, "qmp quit failed: %s" % e)
    time.sleep(10)
    if qemu_alive(work):
        os.kill(pid, signal.SIGKILL)
        say(out, "qemu SIGKILL")
        time.sleep(5)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--arm", choices=("K", "S", "V"), required=True)
    ap.add_argument("--tag", required=True)
    ap.add_argument("--rev", default="")
    ap.add_argument("--wall-min", type=int, default=75)
    ap.add_argument("--rtc-base", default="", help="guest RTC start (the Vast run's recorded clock is 2026-11-22T02:43:26); default the real clock")
    ap.add_argument("--print-argv", action="store_true", help="print the scrubbed argv and exit (no lock, no VM)")
    a = ap.parse_args()
    qemu = STOCK_QEMU if a.arm == "V" else "/workspace/bench/kf3-bins/%s/qemu-system-x86_64" % a.rev
    assert os.access(qemu, os.X_OK), qemu
    if a.print_argv:
        print(scrub(shlex.join(build_argv(a.arm, qemu, VMROOT / a.tag, a.rtc_base))))
        return 0
    out = HERE / "runs" / a.tag
    out.mkdir(parents=True, exist_ok=False)
    work = VMROOT / a.tag
    fd = os.open(LOCK, os.O_RDWR | os.O_CREAT)
    fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)        # a held lock is an error here: one pc GPU job at a time
    state = {"bound": False}

    def term(sig, frm):
        raise SystemExit("signal %d" % sig)
    for s in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
        signal.signal(s, term)
    what = {"K": "kf3 %s (origin/next)" % a.rev, "S": "kf3 %s (PR #565 head)" % a.rev, "V": "VFIO passthrough of the RTX 4070, stock QEMU: GPU 01:00.0/.1 to vfio-pci, desktop dark until the restore"}[a.arm]
    active("START Premiere sequence-crash control arm %s tag=%s: %s; throwaway overlay on b2_addon_cand (offline restrict=on, clock %s), GPU lock held" % (a.arm, a.tag, what, a.rtc_base or "real"))
    rc = 0
    try:
        (out / "host_before.json").write_text(host_state("before"))
        if a.arm != "V":
            r = cuinit_probe()
            say(out, "host cuInit before kf3: %s" % (r,))
            if r[0] != 0:
                active("arm %s: host cuInit not ready, NOT RUN" % a.arm)
                return 8
        else:
            try:
                ok, log = PB.bind_vfio()
            except PB.Abort as e:
                (out / "bind.txt").write_text(scrub(str(e)))
                active("ABORT bind: %s" % str(e).replace("\n", " | ")[-300:])
                state["bound"] = PB.recover_after_bind_abort(e, active_fn=active)
                return 9
            (out / "bind.txt").write_text(scrub(log))
            state["bound"] = True
            if not ok:
                active("bind incomplete, restoring")
                return 4
            active("BOUND: both functions on vfio-pci (modprobe block in place)")
        work.mkdir(parents=True)
        shutil.chown(VMROOT, "ubuntu", "ubuntu")
        PB.sh(["qemu-img", "create", "-q", "-f", "qcow2", "-b", str(BASE_DISK), "-F", "qcow2", str(work / "disk.qcow2")], 60)
        shutil.copy2(SRC / "firmware/OVMF_VARS.fd", work / "OVMF_VARS.fd")
        shutil.copytree(SRC / "tpm", work / "tpm")
        (work / "run").mkdir()
        for p in [work] + list(work.rglob("*")):
            shutil.chown(p, "ubuntu", "ubuntu")
        r = PB.sh(["runuser", "-u", "ubuntu", "--", "swtpm", "socket", "--tpm2", "--tpmstate", "dir=%s/tpm,mode=0600" % work,
                   "--ctrl", "type=unixio,path=%s/swtpm.sock,mode=0600" % work, "--log", "file=%s/run/swtpm.log,level=20" % work,
                   "--pid", "file=%s/run/swtpm.pid" % work, "--terminate", "--daemon"], 30)
        say(out, "swtpm: %s" % (r,))
        for _ in range(50):
            if (work / "swtpm.sock").exists():
                break
            time.sleep(0.2)
        else:
            raise RuntimeError("swtpm made no socket")
        argv = build_argv(a.arm, qemu, work, a.rtc_base)
        (out / "argv_scrubbed.txt").write_text(scrub(shlex.join(argv)) + "\n")
        env = dict(os.environ)
        if a.arm != "V":
            env["KF3_RPC_TRACE"] = "0"
        qlog = out / "qemu.log"
        p = subprocess.Popen(["nice", "-n", "10"] + argv, env=env, stdout=open(qlog, "wb"), stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL)
        say(out, "qemu started pid %d (%s)" % (p.pid, qemu))
        active("GUEST-UP arm %s tag=%s qemu=%s (wall cap %d min, STOP file %s)" % (a.arm, a.tag, qemu, a.wall_min, out / "STOP"))
        t0 = time.time()
        while time.time() - t0 < a.wall_min * 60:
            if (out / "STOP").exists():
                say(out, "STOP file")
                break
            if p.poll() is not None:
                say(out, "qemu exited rc=%s" % p.returncode)
                break
            time.sleep(5)
        else:
            say(out, "wall cap reached")
        stop_guest(work, out)
        if p.poll() is None:
            p.kill()
        p.wait(timeout=60)
        return 0
    except SystemExit as e:
        say(out, "exit: %s" % e)
        if qemu_alive(work):
            stop_guest(work, out)
        rc = 2
        return rc
    finally:
        if state["bound"]:
            try:
                rlog = PB.restore_vfio()
            except PB.Abort as e:
                rlog = "RESTORE ABORT\n" + str(e)
                active("RESTORE ABORT: %s" % str(e).replace("\n", " | ")[-300:])
            (out / "restore.txt").write_text(scrub(rlog))
        hs = host_state("after")
        (out / "host_after.json").write_text(hs)
        cu = cuinit_probe() if not G.block_present() and G.gpu_driver() == "nvidia" else ("skipped", "")
        say(out, "host cuInit after: %s" % (cu,))
        h = json.loads(hs)
        active("END Premiere sequence-crash control arm %s tag=%s: nvidia %s on 01:00.0=%s, 01:00.1=%s, gdm %s, DP-3 %s, udev queue empty=%s, vfio modules %s, cuInit %s; lock released" % (
            a.arm, a.tag, h["nvidia_version"], h["driver_01_00_0"], h["driver_01_00_1"], h["gdm"], h["dp_status"].get("card1-DP-3"),
            h["udev_queue_empty"], [m for m in h["modules"] if m.startswith("vfio")], cu[0]))
        os.close(fd)


if __name__ == "__main__":
    sys.exit(main())
