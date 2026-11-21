#!/usr/bin/env python3
"""pc VFIO control session driver (runs ON the pc, detached). One session = one platform x one row set x N fresh-clone attempts.
Holds /root/prov/pc_gpu.lock for the whole session, writes START/END lines to /root/prov/PC_ACTIVE.log, binds the GPU to vfio-pci for
--platform vfio with the J4 reference script (pcnight_20261114/vfio_host.sh, stop gdm, bind, restore) and ALWAYS restores it.
Throwaway work dirs only (own dir, deleted after each attempt); results are appended to results_pc.tsv and every attempt dir is
complete on disk before the next one starts (the coordinator rsyncs them). A START marker without END = boot cut = NOT RUN.
usage: pcsession.py --tag T --platform kf3|vfio --image b|a --rows r1[,r2] --attempts N [--fb-mb N] [--rev 6bfa2dbc] [--code DIR] [--extra ARGS]"""
import argparse, fcntl, json, os, re, shutil, signal, subprocess, sys, time
from pathlib import Path

ROOT = Path("/workspace/bench/pc_vfio_20261116")
SEALED_B = Path("/workspace/pc1/winvm/propimage_b/sealed/20261115T223105Z")
DIR_A = Path("/workspace/bench/propimage_20261112/unp")
LOCK = "/root/prov/pc_gpu.lock"
ACTIVE = Path("/root/prov/PC_ACTIVE.log")
VFIO_HOST = "/workspace/bench/pcnight_20261114/vfio_host.sh"
BROKER = ROOT / "src_6bfa2dbc2/broker/nvkvm-display-broker"
GOP = "/workspace/pc1/winvm/propimage_b/firmware/kf-gop.signed.efi"
WORKROOT = Path("/workspace/bench/propimage_20261112/pcvfio_work")  # swtpm AppArmor allows only this tree and winvm/**
ISO = ROOT / "test-assets-20261116c.iso"


def now():
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def active(msg):
    with ACTIVE.open("a") as f:
        f.write("%s pcvfio %s\n" % (now(), msg))


def free_gb():
    s = os.statvfs("/")
    return s.f_bavail * s.f_frsize / 2**30


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def mask_argv(line):
    line = re.sub(r"(-uuid )\S+", r"\1<id>", line)
    line = re.sub(r"serial=[^,\s]+", "serial=<id>", line)
    line = re.sub(r"mac=[^,\s]+", "mac=<id>", line)
    line = re.sub(r"gpu-uuid=[^,\s]+", "gpu-uuid=<id>", line)
    line = re.sub(r"net=\d+\.\d+\.\d+\.\d+/\d+", "net=<subnet>", line)
    line = re.sub(r"hostfwd=tcp:127\.0\.0\.1:\d+", "hostfwd=tcp:127.0.0.1:<port>", line)
    line = re.sub(r"base=\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d", "base=<recorded-clock>", line)
    line = re.sub(r"(/workspace/\S*?)(work_[A-Za-z0-9_]+)", r"\1<work>", line)
    return line


def census_of(qlog):
    sys.path.insert(0, str(ROOT / "rows_b/scripts/bench/windows"))
    import kf3_census
    try:
        with open(qlog, "rb") as f:
            lines = [l.decode("utf-8", "replace") for l in f]
    except OSError:
        return None
    c = kf3_census.census(lines)
    # the refused RM controls (status 0x56) as a distinct list
    refused = sorted({m for l in lines for m in re.findall(r"refus\w*[^\n]*?(0x[0-9a-f]{6,8})", kf3_census.clean(l), re.I)})[:60]
    c["refused_rm_controls"] = refused
    return c


GPU_FN = ("0000:01:00.0", "0000:01:00.1")
NV_MODS = ("nvidia_uvm", "nvidia_drm", "nvidia_modeset", "nvidia")
LAST_RESTORE = ROOT / "last_restore_epoch.txt"
SETTLE_S = 60            # never start the next session sooner than this after a restore (the second nvidia wedge started 5 s after a restore)
DRM_QUIET_S = 45         # how long to wait for nvidia_drm to reach refcount 0 after stopping gdm and every DRM client
# SAFETY RULES (owner, 2026-11-17, after two wedges at the nvidia unbind): this file NEVER writes to a sysfs unbind file of the NVIDIA driver (2026-11-18 exception, coordinator-approved: the audio function through snd_hda_intel only, see release_audio). A module that is in
# use is an ABORT with a marker, not something to work around. The vfio-pci side is left by unloading the vfio_pci module (it releases its
# devices), the nvidia side is unloaded with rmmod only.


def sh(cmd, timeout):
    """Run cmd with a hard wait limit. A D-state child cannot be killed: on timeout the child is NOT waited for, the caller aborts (never loops)."""
    p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    try:
        out, _ = p.communicate(timeout=timeout)
        return p.returncode, out
    except subprocess.TimeoutExpired:
        try:
            st = Path("/proc/%d/stat" % p.pid).read_text().split()[2]
        except OSError:
            st = "?"
        try:
            p.kill()
        except OSError:
            pass
        return "TIMEOUT", "timeout after %ds: %s (pid %d state %s)" % (timeout, " ".join(cmd), p.pid, st)


def holder_pids():
    import glob
    nodes = glob.glob("/dev/nvidia*") + glob.glob("/dev/dri/*")
    if not nodes:
        return [], ""
    rc, o = sh(["fuser", "-v"] + nodes, 20)
    rc2, o2 = sh(["fuser"] + nodes, 20)
    pids = sorted({int(x) for x in re.findall(r"\b\d+\b", re.sub(r"/dev/\S+", " ", o2)) if int(x) > 1})
    return pids, o


def is_own(pid):
    try:
        cmd = Path("/proc/%d/cmdline" % pid).read_bytes().replace(b"\0", b" ").decode(errors="replace")
        ppid = int(Path("/proc/%d/stat" % pid).read_text().rsplit(")", 1)[1].split()[1])
    except OSError:
        return False
    return "pc_vfio_20261116" in cmd or "pcvfio_work" in cmd or ppid == os.getpid()


def module_refcnt(name):
    """refcount of a loaded module, None when it is not loaded, -1 style values returned as they are (a module mid-unload)."""
    try:
        return int(Path("/sys/module/%s/refcnt" % name).read_text().strip())
    except OSError:
        return None


class Abort(Exception):
    pass


class WedgeLite(Abort):
    """A child of ours is stuck in D state (cannot be killed). Nothing further is touched; the caller only restarts gdm and reports."""


def release_audio(sh, driver_of, log):
    """FIRST step after the holders check, before any nvidia unload (coordinator condition 1). Child process with a 60 s timeout; pid/state are logged
    (the sh() timeout text carries them); a child left in D state is wedge-lite (condition 2)."""
    if driver_of(AUDIO_FN) != "snd_hda_intel":
        log.append("audio function %s driver: %s (nothing to release)" % (AUDIO_FN, driver_of(AUDIO_FN)))
        return
    rc, o = sh(["bash", "-c", "echo %s > /sys/bus/pci/drivers/snd_hda_intel/unbind" % AUDIO_FN], 60)
    log.append("audio function %s released from snd_hda_intel rc=%s %s" % (AUDIO_FN, rc, o.strip()[:200]))
    if rc != 0:
        if re.search(r"state D", str(o)):
            raise WedgeLite("\n".join(log) + "\nWEDGE-LITE: snd_hda_intel unbind child stuck in D state; nothing else touched, nvidia side NOT started")
        raise Abort("\n".join(log) + "\nABORT: snd_hda_intel unbind of %s rc=%s: stopping before any nvidia unload (no retry)" % (AUDIO_FN, rc))
    if driver_of(AUDIO_FN) is not None:
        raise Abort("\n".join(log) + "\nABORT: %s still has driver %s after the audio unbind" % (AUDIO_FN, driver_of(AUDIO_FN)))


def wait_drm_quiet(log, sleep=time.sleep, refcnt=module_refcnt, timeout_s=None):
    """After gdm and every DRM client are stopped nvidia_drm must reach refcount 0 (or not be loaded) before ANY unload. Otherwise abort."""
    timeout_s = DRM_QUIET_S if timeout_s is None else timeout_s
    t0 = time.time()
    while True:
        r = refcnt("nvidia_drm")
        if r in (None, 0):
            log.append("nvidia_drm refcnt %s: quiet" % r)
            return
        if time.time() - t0 > timeout_s:
            raise Abort("\n".join(log) + "\nABORT: nvidia_drm refcnt %s after %ds with gdm and the DRM clients stopped: module in use, not unloading (no unbind, no retry)" % (r, timeout_s))
        sleep(3)


AUDIO_FN = "0000:01:00.1"
# OWNER-RULE EXCEPTION (2026-11-18, supervised session, announced to the coordinator): the GPU's HDMI audio function is bound to snd_hda_intel (not
# nvidia). vfio needs every device of the IOMMU group on vfio-pci or driverless, so that ONE function is released through snd_hda_intel's own sysfs
# unbind file (the pre-rewrite step of 8 sessions without a wedge; both wedges were the NVIDIA unbind). 60 s timeout, abort on failure, no retry.
# Nothing else in this file writes an unbind file, and never one of the nvidia driver.


def driver_name(dev):
    try:
        return os.path.basename(os.path.realpath("/sys/bus/pci/devices/%s/driver" % dev)) if os.path.exists("/sys/bus/pci/devices/%s/driver" % dev) else None
    except OSError:
        return None


def bind_vfio(sh=None, holders=None, own=None, refcnt=None, sleep=None, driver_of=None):
    """Unload the nvidia modules with rmmod only and hand the GPU to vfio-pci through driver_override + drivers_probe (the device has no driver by then).
    If anything is in use it is an ABORT: nothing writes to an unbind file, nothing is retried in a loop."""
    sh = sh or globals()["sh"]; holders = holders or holder_pids; own = own or is_own; refcnt = refcnt or module_refcnt; sleep = sleep or time.sleep
    driver_of = driver_of or driver_name
    log = []
    pids, txt = holders()
    log.append("holders before stopping gdm: %s\n%s" % (pids, txt))
    log.append("gdm stop: %s" % (sh(["systemctl", "stop", "gdm"], 60),))
    sleep(3)
    log.append("persistenced stop: %s" % (sh(["systemctl", "stop", "nvidia-persistenced"], 30),))
    pids, txt = holders()
    log.append("holders after: %s\n%s" % (pids, txt))
    for pid in pids:
        if own(pid):
            log.append("killing OWN holder pid %d" % pid)
            try:
                os.kill(pid, signal.SIGKILL)
            except OSError as e:
                log.append("kill failed: %s" % e)
    sleep(2)
    pids, txt = holders()
    if pids:
        log.append("ABORT: DRM/nvidia clients remain that are not ours: %s\n%s" % (pids, txt))
        raise Abort("\n".join(log))
    release_audio(sh, driver_of, log)
    wait_drm_quiet(log, sleep=sleep, refcnt=refcnt)
    for m in NV_MODS:
        if refcnt(m) is None:
            log.append("%s not loaded" % m)
            continue
        rc, o = sh(["rmmod", m], 90)
        log.append("rmmod %s rc=%s %s" % (m, rc, o.strip()[:200]))
        if rc != 0:      # TIMEOUT or 'in use': never work around it
            raise Abort("\n".join(log) + "\nABORT: rmmod %s did not succeed (%s): module in use, stopping (no unbind, no retry)" % (m, rc))
    rc, o = sh(["modprobe", "vfio-pci"], 60)
    log.append("modprobe vfio-pci rc=%s %s" % (rc, o.strip()[:200]))
    for d in GPU_FN:
        cur = driver_of(d)
        if cur is not None:
            raise Abort("\n".join(log) + "\nABORT: %s still has driver %s after the module unload; not touching it" % (d, cur))
        rc, o = sh(["bash", "-c", "echo vfio-pci > /sys/bus/pci/devices/%s/driver_override; echo %s > /sys/bus/pci/drivers_probe" % (d, d)], 60)
        if rc != 0:
            raise Abort("\n".join(log) + "\nABORT: driver_override/drivers_probe for %s rc=%s %s" % (d, rc, o.strip()[:200]))
    ok = True
    for d in GPU_FN:
        drv = os.path.basename(os.path.realpath("/sys/bus/pci/devices/%s/driver" % d))
        log.append("%s %s" % (d, drv))
        ok = ok and drv == "vfio-pci"
    return ok, "\n".join(log)


def restore_vfio(sh=None, sleep=None, driver_of=None, settle=True):
    """Leave vfio-pci by unloading the vfio_pci module (no unbind write), clear the overrides, load the nvidia modules, start the services, verify."""
    sh = sh or globals()["sh"]; sleep = sleep or time.sleep; driver_of = driver_of or driver_name
    log = []
    for m in ("vfio_pci", "vfio_pci_core"):
        rc, o = sh(["rmmod", m], 90)
        log.append("rmmod %s rc=%s %s" % (m, rc, o.strip()[:120]))
        if rc == "TIMEOUT":
            raise Abort("\n".join(log) + "\nABORT: rmmod %s stuck" % m)
    for d in GPU_FN:
        sh(["bash", "-c", "echo > /sys/bus/pci/devices/%s/driver_override" % d], 30)
    for m in ("nvidia", "nvidia_modeset", "nvidia_drm", "nvidia_uvm"):
        rc, o = sh(["modprobe", m], 90)
        log.append("modprobe %s rc=%s %s" % (m, rc, o.strip()[:200]))
        if rc == "TIMEOUT":
            raise Abort("\n".join(log) + "\nABORT: modprobe %s stuck" % m)
    for d in GPU_FN:
        sh(["bash", "-c", "echo %s > /sys/bus/pci/drivers_probe" % d], 60)
    sleep(2)
    if driver_of(AUDIO_FN) is None:       # bind (not unbind) the audio function back to its own driver
        rc, o = sh(["bash", "-c", "echo %s > /sys/bus/pci/drivers/snd_hda_intel/bind" % AUDIO_FN], 30)
        log.append("audio function bind-back rc=%s %s" % (rc, o.strip()[:120]))
    log.append("audio function driver after restore: %s" % driver_of(AUDIO_FN))
    log.append("persistenced: %s" % (sh(["systemctl", "start", "nvidia-persistenced"], 30),))
    log.append("gdm start: %s" % (sh(["systemctl", "start", "gdm"], 60),))
    sleep(6)
    rc, o = sh(["nvidia-smi", "--query-gpu=name,driver_version", "--format=csv,noheader"], 60)
    log.append("nvidia-smi rc=%s %s" % (rc, o.strip()))
    if rc != 0:
        raise Abort("\n".join(log) + "\nABORT: nvidia-smi does not answer after the restore")
    if settle:
        wait_host_modules_settled(sh=sh, sleep=sleep, log=log)
    try:
        LAST_RESTORE.write_text("%d\n" % int(time.time()))
    except OSError:
        pass
    return "\n".join(log)


class UvmWedge(Abort):
    """nvidia_uvm stuck in state 'going' (a late udev `modprobe -r nvidia-uvm`, rules.d/71-nvidia.rules line 19): the wedge class. No second unload attempt."""


def uvm_state():
    """(initstate, refcnt) of nvidia_uvm; (None, None) when not loaded."""
    try:
        st = Path("/sys/module/nvidia_uvm/initstate").read_text().strip()
    except OSError:
        return None, None
    return st, module_refcnt("nvidia_uvm")


def nvidia_module_procs():
    rc, o = sh(["bash", "-c", "pgrep -af '^(/sbin/|/usr/sbin/)?(modprobe|rmmod)' | grep -i nvidia | grep -v pgrep || true"], 20)
    return [l for l in str(o).splitlines() if l.strip()]


def wait_host_modules_settled(sh=None, sleep=None, now_fn=None, state_fn=None, procs_fn=None, stable_s=60, going_limit_s=120, timeout_s=900, log=None):
    """The REAL settle after any restore (and before a kf3 start): `udevadm settle`, then no modprobe/rmmod of nvidia* running AND nvidia_uvm either
    absent-and-stable for stable_s (then loaded once, load only) or 'live' with refcnt >= 0 stable for stable_s. 'going'/refcnt<0 longer than
    going_limit_s is the wedge: raise UvmWedge (the caller stops, no second unload attempt). Cause (MEASURED 2026-11-18): /usr/lib/udev/rules.d/71-nvidia.rules
    line 19 `ACTION=="remove", DEVPATH=="/bus/pci/drivers/nvidia", RUN+="/sbin/modprobe -r nvidia-uvm"` runs late and races the reload."""
    sh = sh or globals()["sh"]; sleep = sleep or time.sleep; now_fn = now_fn or time.time
    state_fn = state_fn or uvm_state; procs_fn = procs_fn or nvidia_module_procs; log = log if log is not None else []
    log.append("udevadm settle: %s" % (sh(["udevadm", "settle", "--timeout=60"], 90),))
    t0 = now_fn(); stable = None; going = None; loaded_again = False
    while True:
        now = now_fn()
        if now - t0 > timeout_s:
            raise Abort("\n".join(log) + "\nABORT: host nvidia modules did not settle in %ds" % timeout_s)
        st, ref = state_fn(); procs = procs_fn()
        if st == "going" or (ref is not None and ref < 0):
            going = going if going is not None else now
            stable = None
            if now - going > going_limit_s:
                raise UvmWedge("\n".join(log) + "\nWEDGE: nvidia_uvm initstate=%s refcnt=%s for %ds, modprobe/rmmod procs: %s: stop, SysRq ladder, no second unload" % (st, ref, now - going, procs))
        else:
            going = None
            if procs:
                stable = None
            else:
                stable = stable if stable is not None else now
                if now - stable >= stable_s:
                    if st is None and not loaded_again:
                        rc, o = sh(["modprobe", "nvidia_uvm"], 90)
                        log.append("nvidia_uvm absent and stable %ds: modprobe nvidia_uvm rc=%s %s" % (stable_s, rc, str(o).strip()[:120]))
                        if rc != 0:
                            raise Abort("\n".join(log) + "\nABORT: modprobe nvidia_uvm rc=%s" % (rc,))
                        loaded_again = True; stable = None
                    elif st == "live" and ref is not None and ref >= 0:
                        log.append("nvidia_uvm live refcnt %s stable %ds, no nvidia modprobe/rmmod running: settled" % (ref, stable_s))
                        return "\n".join(log)
        sleep(3)


def recover_after_bind_abort(e, sh=None, refcnt=None, driver_of=None, active_fn=None):
    """After a bind ABORT. Returns True when the caller must run the full restore (nvidia modules already unloaded), else False.
    WedgeLite: only gdm is started. Nothing unloaded: audio function re-probed, persistenced and gdm started."""
    sh = sh or globals()["sh"]; refcnt = refcnt or module_refcnt; driver_of = driver_of or driver_name; active_fn = active_fn or active
    if isinstance(e, WedgeLite):
        rc, o = sh(["systemctl", "start", "gdm"], 60)
        active_fn("WEDGE-LITE: gdm start rc=%s; session stopped, coordinator to be told" % (rc,))
        return False
    if refcnt("nvidia") is None:
        return True
    sh(["bash", "-c", "echo %s > /sys/bus/pci/drivers_probe" % AUDIO_FN], 30)
    sh(["systemctl", "start", "nvidia-persistenced"], 30)
    rc, o = sh(["systemctl", "start", "gdm"], 60)
    active_fn("aborted before any unload: persistenced/gdm restarted rc=%s; audio driver %s" % (rc, driver_of(AUDIO_FN)))
    return False


def settle_after_restore(log=print, sleep=time.sleep, now_fn=time.time, path=None):
    """No session starts within SETTLE_S of the last restore (any platform)."""
    path = path or LAST_RESTORE
    try:
        last = int(Path(path).read_text().strip())
    except (OSError, ValueError):
        return 0
    wait = SETTLE_S - (now_fn() - last)
    if wait > 0:
        log("%s waiting %ds after the last restore" % (now(), int(wait)))
        sleep(wait)
        return wait
    return 0


def smi_name():
    rc, o = sh(["nvidia-smi", "--query-gpu=name", "--format=csv,noheader"], 30)
    return (o.strip().splitlines() or ["?"])[0][:40] if rc == 0 else "nvidia-smi rc=%s" % rc


def abort_marker(tag, why):
    active("ABORT session %s: %s (no reboot, no retry loop; STOP_CHAIN written; coordinator to be told)" % (tag, why.replace("\n", " | ")[:400]))
    (ROOT / "STOP_CHAIN").write_text(now() + " " + tag + "\n")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--tag", required=True)
    ap.add_argument("--platform", choices=("kf3", "vfio"), required=True)
    ap.add_argument("--image", choices=("a", "b"), default="b")
    ap.add_argument("--rows", required=True)
    ap.add_argument("--attempts", type=int, default=3)
    ap.add_argument("--fb-mb", type=int)
    ap.add_argument("--rev", default="6bfa2dbc")
    ap.add_argument("--code", default=str(ROOT / "rows_b/scripts/bench/windows"))
    ap.add_argument("--extra", default="", help="extra propimage_check args (space separated)")
    ap.add_argument("--timeout", type=int, default=3600)
    a = ap.parse_args()
    qemu = "/workspace/bench/kf3-bins/%s/qemu-system-x86_64" % a.rev
    assert os.access(qemu, os.X_OK), qemu
    code = Path(a.code)
    (ROOT / "ev").mkdir(exist_ok=True); WORKROOT.mkdir(exist_ok=True); (ROOT / "private").mkdir(mode=0o700, exist_ok=True)
    state = {"vfio_bound": False}

    def cleanup(*_):
        if state["vfio_bound"]:
            state["vfio_bound"] = False
            try:
                out = restore_vfio()
            except Abort as e:
                out = str(e)
                abort_marker(a.tag, out)
            (ROOT / "ev" / ("%s_restore.txt" % a.tag)).write_text(out)
        active("END %s %s (session %s); GPU: %s" % (a.platform, a.rows, a.tag, smi_name()))

    def term(sig, frm):
        cleanup()
        os._exit(130)

    for s in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
        signal.signal(s, term)

    fd = os.open(LOCK, os.O_RDWR | os.O_CREAT)
    t0 = time.time()
    while True:
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB); break
        except OSError:
            if int(time.time() - t0) % 300 < 20:
                print("%s waiting for the GPU lock" % now(), flush=True)
            time.sleep(20)
    settle_after_restore(log=lambda m: print(m, flush=True))
    active("START %s rows=%s attempts=%d tag=%s rev=%s image=%s fb-mb=%s: throwaway clones restrict=on, flock" % (a.platform, a.rows, a.attempts, a.tag, a.rev, a.image, a.fb_mb))
    try:
        if a.platform == "kf3":
            rc, o = sh(["nvidia-smi", "--query-gpu=name", "--format=csv,noheader"], 30)
            if rc != 0:
                abort_marker(a.tag, "kf3 session: nvidia-smi does not answer before the start (rc=%s %s)" % (rc, o.strip()[:200]))
                return 9
            # MEASURED 2026-11-17: kf3 started seconds after a vfio restore failed "cuInit refused: 999 (CUDA_ERROR_UNKNOWN)" (3 sessions, 5 s each).
            # The host CUDA runtime must answer before kf3 starts: probe cuInit from a child process (30 s apart, 8 tries), then go on.
            try:
                (ROOT / "ev" / ("%s_host_settle.txt" % a.tag)).write_text(wait_host_modules_settled() + "\n")
            except Abort as e:
                (ROOT / "ev" / ("%s_host_settle.txt" % a.tag)).write_text(str(e) + "\n")
                abort_marker(a.tag, str(e)); return 9
            probe = "import ctypes,sys; l=ctypes.CDLL('libcuda.so.1'); r=l.cuInit(0); print('cuInit',r); sys.exit(0 if r==0 else 1)"
            ready = False
            for i in range(8):
                rc, o = sh(["python3", "-c", probe], 40)
                (ROOT / "ev" / ("%s_cuda_ready.txt" % a.tag)).open("a").write("%s try %d rc=%s %s\n" % (now(), i, rc, o.strip()[:100]))
                if rc == 0:
                    ready = True
                    break
                time.sleep(30)
            if not ready:
                active("kf3 session %s: host cuInit never ready, session not run (NOT RUN)" % a.tag)
                return 8
        if a.platform == "vfio":
            try:
                ok, out = bind_vfio()
            except Abort as e:
                (ROOT / "ev" / ("%s_bind.txt" % a.tag)).write_text(str(e))
                abort_marker(a.tag, str(e))
                print("ABORT bind", str(e)[-400:], flush=True)
                state["vfio_bound"] = recover_after_bind_abort(e)
                return 9
            (ROOT / "ev" / ("%s_bind.txt" % a.tag)).write_text(out)
            if not ok:
                print("bind failed", out[-300:], flush=True)
                return 4
            state["vfio_bound"] = True
        for n in range(1, a.attempts + 1):
            if free_gb() < 20:
                print("disk low: %.1f GB free, stop" % free_gb(), flush=True)
                break
            name = "%s_a%d" % (a.tag, n)
            work = WORKROOT / name
            out = ROOT / "ev" / name
            assert not work.exists() and not out.exists(), name
            out.mkdir(parents=True)
            (out / "START").write_text("%s\n" % now())
            dmesg0 = run(["dmesg"]).stdout.count("\n")
            cmd = ["python3", "-B", str(ROOT / "pcrun.py"), "--code", str(code),
                   "--dir", str(SEALED_B if a.image == "b" else DIR_A), "--work", str(work), "--out", str(out / "h"),
                   "--qemu", qemu, "--flags", str(code / "kf3_flags.sample.env"), "--apps", a.rows, "--platform", a.platform]
            if a.image == "b":
                cmd += ["--app-set", "b", "--data-iso", str(ISO)]
            if a.platform == "kf3":
                cmd += ["--display", "broker", "--broker", str(BROKER), "--gop-efi", GOP]
            else:
                cmd += ["--display", "off"]
                if a.image == "a":
                    cmd += ["--setres"]
            if a.fb_mb and a.platform == "kf3":
                cmd += ["--fb-mb", str(a.fb_mb)]
            cmd += a.extra.split()
            (out / "cmd.txt").write_text(" ".join(cmd) + "\n")
            t1 = time.time()
            with open(out / "stdout.txt", "w") as so:
                try:
                    rc = subprocess.run(cmd, stdout=so, stderr=subprocess.STDOUT, timeout=a.timeout).returncode
                except subprocess.TimeoutExpired:
                    rc = "TIMEOUT"
            dur = int(time.time() - t1)
            # harness argv: raw stays on the pc (private), masked copy goes to the evidence
            lc = work / "run/last_cmd.txt"
            if lc.exists():
                raw = lc.read_text()
                (ROOT / "private" / (name + "_argv.txt")).write_text(raw)
                os.chmod(ROOT / "private" / (name + "_argv.txt"), 0o600)
                (out / "argv_masked.txt").write_text(mask_argv(raw))
            ql = work / "run/qemu.log"
            if ql.exists():
                c = census_of(ql)
                (out / "census.json").write_text(json.dumps(c, indent=1))
                sys.path.insert(0, str(ROOT / "rows_b/scripts/bench/windows"))
                import kf3_census
                lines = [kf3_census.clean(l.rstrip("\n")) for l in open(ql, "rb").read().decode("utf-8", "replace").splitlines()]
                (out / "qemu_log_tail.txt").write_text("\n".join(lines[-150:]) + "\n")
                (out / "qemu_log_lines.txt").write_text("%d lines\n" % len(lines))
            d = run(["dmesg"]).stdout.splitlines()[dmesg0:]
            keep = [l for l in d if re.search(r"NVRM|Xid|vfio|kf3|nvidia|DMAR|AMD-Vi|IOMMU|AER|BUG:|Oops|oom|Out of memory|Killed process|kvm", l, re.I)]
            (out / "dmesg_new_filtered.txt").write_text("\n".join(re.sub(r"GPU-[0-9a-f-]{20,}", "GPU-<uuid>", l) for l in keep) + "\n")
            # the row verdicts from the harness lines
            so_txt = (out / "stdout.txt").read_text(errors="replace")
            rows = [l for l in so_txt.splitlines() if l.startswith("app:")]
            crash = 0
            for f in sorted(out.glob("h/app_log_events_*.txt")):
                t = f.read_text(errors="replace")
                crash += len(re.findall(r"id=1000 Application Error: .*(InDesign|Resolve|Photoshop|AfterFX|aerender)", t))
            cd = out / "h" / "crash_detect.txt"
            detect = None
            if cd.exists():
                t = cd.read_text(errors="replace")
                mm = re.search(r"KFEVENT .*?Faulting application name: (\S+), version: ([\d.]+).*?Faulting module name: ([^,]+), version: ([\d.]+).*?Exception code: (0x[0-9a-fA-F]+) \| Fault offset: (0x[0-9a-fA-F]+)", t)
                detect = "app-crash %s" % (" ".join(mm.groups()) if mm else "(event text not parsed)")
                rows = rows + ["CLASS " + detect]
            end = now()
            (out / "END").write_text("%s rc=%s dur=%d\n" % (end, rc, dur))
            summ = {"name": name, "tag": a.tag, "platform": a.platform, "image": a.image, "rows": a.rows, "attempt": n, "rev": a.rev, "rc": rc,
                    "duration_s": dur, "row_lines": rows, "app_crash_events": crash, "start": (out / "START").read_text().strip(), "end": end,
                    "crash_detect": detect, "census": (json.loads((out / "census.json").read_text())["counts"] if (out / "census.json").exists() else None)}
            (out / "summary.json").write_text(json.dumps(summ, indent=1))
            with open(ROOT / "results_pc.tsv", "a") as f:
                f.write("\t".join(str(x) for x in (a.tag, a.rows, a.image, a.platform, n, a.rev, ";".join(r[:140] for r in rows) or "NO_ROW_LINE", dur, "crash_events=%d" % crash, json.dumps(summ["census"]), summ["start"], end)) + "\n")
            print("ATTEMPT", name, "done rc=%s dur=%d rows=%s crash_events=%d" % (rc, dur, rows, crash), flush=True)
            shutil.rmtree(work, ignore_errors=False) if work.exists() else None
        return 0
    finally:
        cleanup()


if __name__ == "__main__":
    sys.exit(main())
