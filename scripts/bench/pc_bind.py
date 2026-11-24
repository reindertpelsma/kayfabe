#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Safe VFIO bind and restore of the pc's GPU (STATUS: LIVE, 2026-11-24).

The bind and restore functions of the pc session driver `pcsession.py` (the safe-bind rewrite of 2026-11-18, used unchanged by
the VFIO sessions of 2026-11-18 and 2026-11-24 from their evidence directories), moved into the repository with two guards
added (pc_guard.py):

(a) For the bind's duration a modprobe block (`install nvidia /bin/false` for nvidia, nvidia_modeset, nvidia_uvm, nvidia_drm)
    makes every nvidia load fail at once. It is installed before the first unload and removed at the restore just before the
    driver is loaded again. After the hand-over to vfio-pci the bind refuses to go on when the nvidia module is loaded anyway.
(b) The restore loads the driver only after the udev queue is empty and no nvidia modprobe/rmmod runs (restore_gate). On a
    timeout the code runs the repair ladder: restart systemd-udevd, wait again, then (after the reload) re-trigger the device
    events and restart gdm. A queue that stays busy is an abort with the block left in place.

Safety rules kept from the rewrite: nothing writes a sysfs unbind file of the NVIDIA driver (the GPU's audio function is
released through snd_hda_intel's own unbind file, the one exception); a module in use is an ABORT, never worked around; nothing
is retried in a loop; vfio-pci is left by unloading the vfio_pci module.

Library only: the session drivers import it (`import pc_bind as PB; PB.bind_vfio(); ...; PB.restore_vfio()`).
GPU-free tests: scripts/ci/test_pc_bind_guards.py.
"""

from __future__ import annotations

import glob
import os
import re
import signal
import subprocess
import time
from pathlib import Path

import pc_guard as G

GPU_FN = ("0000:01:00.0", "0000:01:00.1")
AUDIO_FN = "0000:01:00.1"
NV_MODS = ("nvidia_uvm", "nvidia_drm", "nvidia_modeset", "nvidia")
LAST_RESTORE = Path("/workspace/bench/pc_vfio_20261116/last_restore_epoch.txt")
OWN_MARKERS = ("pc_vfio_20261116", "pcvfio_work")
SETTLE_S = 60
DRM_QUIET_S = 45


class Abort(Exception):
    pass


class WedgeLite(Abort):
    """A child of ours is stuck in D state (cannot be killed). Nothing further is touched; the caller only restarts gdm."""


class UvmWedge(Abort):
    """nvidia_uvm stuck in state 'going': the wedge class. No second unload attempt."""


def sh(cmd, timeout):
    """Run cmd with a hard wait limit. A D-state child cannot be killed: on timeout it is NOT waited for and the caller aborts."""
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
    nodes = glob.glob("/dev/nvidia*") + glob.glob("/dev/dri/*")
    if not nodes:
        return [], ""
    _, o = sh(["fuser", "-v"] + nodes, 20)
    _, o2 = sh(["fuser"] + nodes, 20)
    pids = sorted({int(x) for x in re.findall(r"\b\d+\b", re.sub(r"/dev/\S+", " ", str(o2))) if int(x) > 1})
    return pids, o


def is_own(pid):
    try:
        cmd = Path("/proc/%d/cmdline" % pid).read_bytes().replace(b"\0", b" ").decode(errors="replace")
        ppid = int(Path("/proc/%d/stat" % pid).read_text().rsplit(")", 1)[1].split()[1])
    except OSError:
        return False
    return any(m in cmd for m in OWN_MARKERS) or ppid == os.getpid()


def module_refcnt(name):
    """Refcount of a loaded module, None when it is not loaded."""
    try:
        return int(Path("/sys/module/%s/refcnt" % name).read_text().strip())
    except OSError:
        return None


def driver_name(dev):
    p = "/sys/bus/pci/devices/%s/driver" % dev
    try:
        return os.path.basename(os.path.realpath(p)) if os.path.exists(p) else None
    except OSError:
        return None


def release_audio(sh, driver_of, log):
    """First step after the holders check, before any nvidia unload: the audio function leaves snd_hda_intel (60 s limit)."""
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
    """After gdm and every DRM client are stopped nvidia_drm must reach refcount 0 (or not be loaded) before ANY unload."""
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


def bind_vfio(sh=None, holders=None, own=None, refcnt=None, sleep=None, driver_of=None, block_install=None, block_remove=None):
    """Unload the nvidia modules with rmmod only and hand the GPU to vfio-pci (driver_override + drivers_probe).

    Guard (a): the modprobe block is installed before the first unload. On an abort before any module was unloaded it is
    removed again (the host keeps its driver); once a module is unloaded it stays until restore_vfio.
    """
    sh = sh or globals()["sh"]
    holders = holders or holder_pids
    own = own or is_own
    refcnt = refcnt or module_refcnt
    sleep = sleep or time.sleep
    driver_of = driver_of or driver_name
    block_install = block_install or G.install_block
    block_remove = block_remove or G.remove_block
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
    log.append("modprobe block installed: %s" % (block_install(),))
    unloaded = False
    try:
        for m in NV_MODS:
            if refcnt(m) is None:
                log.append("%s not loaded" % m)
                continue
            rc, o = sh(["rmmod", m], 90)
            log.append("rmmod %s rc=%s %s" % (m, rc, o.strip()[:200]))
            if rc != 0:
                raise Abort("\n".join(log) + "\nABORT: rmmod %s did not succeed (%s): module in use, stopping (no unbind, no retry)" % (m, rc))
            unloaded = True
    except Abort:
        if not unloaded:
            block_remove()
        raise
    rc, o = sh(["modprobe", "vfio-pci"], 60)
    log.append("modprobe vfio-pci rc=%s %s" % (rc, o.strip()[:200]))
    for d in GPU_FN:
        cur = driver_of(d)
        if cur is not None:
            raise Abort("\n".join(log) + "\nABORT: %s still has driver %s after the module unload; not touching it" % (d, cur))
        rc, o = sh(["bash", "-c", "echo vfio-pci > /sys/bus/pci/devices/%s/driver_override; echo %s > /sys/bus/pci/drivers_probe" % (d, d)], 60)
        if rc != 0:
            raise Abort("\n".join(log) + "\nABORT: driver_override/drivers_probe for %s rc=%s %s" % (d, rc, o.strip()[:200]))
    loaded = [m for m in NV_MODS if refcnt(m) is not None]
    if loaded:
        raise Abort("\n".join(log) + "\nABORT: %s loaded after the hand-over to vfio-pci although the modprobe block is in place: something loads nvidia; not going on" % loaded)
    ok = True
    for d in GPU_FN:
        drv = driver_of(d)
        log.append("%s %s" % (d, drv))
        ok = ok and drv == "vfio-pci"
    return ok, "\n".join(log)


def restore_vfio(sh=None, sleep=None, driver_of=None, settle=True, gate=None, block_remove=None, refcnt=None):
    """Leave vfio-pci (unload vfio_pci), clear the overrides, pass guard (b), lift the block, load nvidia, start the services, verify."""
    sh = sh or globals()["sh"]
    sleep = sleep or time.sleep
    driver_of = driver_of or driver_name
    gate = gate or (lambda: G.restore_gate(sh))
    block_remove = block_remove or G.remove_block
    refcnt = refcnt or module_refcnt
    log = []
    early = [m for m in NV_MODS if refcnt(m) is not None]
    if early:
        log.append("UNEXPECTED: %s loaded while the GPU was on vfio-pci (something loaded nvidia during the bind)" % early)
    for m in ("vfio_pci", "vfio_pci_core"):
        rc, o = sh(["rmmod", m], 90)
        log.append("rmmod %s rc=%s %s" % (m, rc, o.strip()[:120]))
        if rc == "TIMEOUT":
            raise Abort("\n".join(log) + "\nABORT: rmmod %s stuck" % m)
    for d in GPU_FN:
        sh(["bash", "-c", "echo > /sys/bus/pci/devices/%s/driver_override" % d], 30)
    ok, ladder_used, glog = gate()
    log.extend(glog)
    if not ok:
        raise Abort("\n".join(log) + "\nABORT: udev queue not quiet before the driver reload (modprobe block left in place; manual check)")
    log.append("modprobe block removed: %s" % (block_remove(),))
    for m in ("nvidia", "nvidia_modeset", "nvidia_drm", "nvidia_uvm"):
        rc, o = sh(["modprobe", m], 90)
        log.append("modprobe %s rc=%s %s" % (m, rc, o.strip()[:200]))
        if rc == "TIMEOUT":
            raise Abort("\n".join(log) + "\nABORT: modprobe %s stuck" % m)
    for d in GPU_FN:
        sh(["bash", "-c", "echo %s > /sys/bus/pci/drivers_probe" % d], 60)
    if ladder_used:
        log.extend(G.retrigger_after_ladder(sh, GPU_FN))
    sleep(2)
    if driver_of(AUDIO_FN) is None:
        rc, o = sh(["bash", "-c", "echo %s > /sys/bus/pci/drivers/snd_hda_intel/bind" % AUDIO_FN], 30)
        log.append("audio function bind-back rc=%s %s" % (rc, o.strip()[:120]))
    log.append("audio function driver after restore: %s" % driver_of(AUDIO_FN))
    log.append("persistenced: %s" % (sh(["systemctl", "start", "nvidia-persistenced"], 30),))
    log.append("gdm %s: %s" % ("restart" if ladder_used else "start", sh(["systemctl", "restart" if ladder_used else "start", "gdm"], 60)))
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


def uvm_state():
    """(initstate, refcnt) of nvidia_uvm; (None, None) when not loaded."""
    try:
        st = Path("/sys/module/nvidia_uvm/initstate").read_text().strip()
    except OSError:
        return None, None
    return st, module_refcnt("nvidia_uvm")


def nvidia_module_procs():
    return [c for c in G.read_proc_cmdlines() if G.nvidia_load_pending([c])]


def wait_host_modules_settled(sh=None, sleep=None, now_fn=None, state_fn=None, procs_fn=None, stable_s=60, going_limit_s=120, timeout_s=900, log=None):
    """The settle after a restore and before a kf3 start: `udevadm settle` must succeed (a timed-out settle means a queue that
    is not empty, which is never 'settled'), no nvidia modprobe/rmmod may run, and nvidia_uvm is live and stable for stable_s
    (or absent and stable, then loaded once). 'going'/refcnt<0 longer than going_limit_s is the wedge (UvmWedge).
    Cause of the late unload: /usr/lib/udev/rules.d/71-nvidia.rules, `ACTION=="remove" ... RUN+="/sbin/modprobe -r nvidia-uvm"`."""
    sh = sh or globals()["sh"]
    sleep = sleep or time.sleep
    now_fn = now_fn or time.time
    state_fn = state_fn or uvm_state
    procs_fn = procs_fn or nvidia_module_procs
    log = log if log is not None else []
    t0 = now_fn()
    settled_queue = False
    stable = None
    going = None
    loaded_again = False
    while True:
        now = now_fn()
        if now - t0 > timeout_s:
            raise Abort("\n".join(log) + "\nABORT: host nvidia modules did not settle in %ds (udev queue settled=%s)" % (timeout_s, settled_queue))
        if not settled_queue:
            r = sh(["udevadm", "settle", "--timeout=60"], 90)
            log.append("udevadm settle: %s" % (r,))
            settled_queue = r[0] == 0
            if not settled_queue:
                stable = None
                sleep(3)
                continue
        st, ref = state_fn()
        procs = procs_fn()
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
                        loaded_again = True
                        stable = None
                    elif st == "live" and ref is not None and ref >= 0:
                        log.append("nvidia_uvm live refcnt %s stable %ds, no nvidia modprobe/rmmod running: settled" % (ref, stable_s))
                        return "\n".join(log)
        sleep(3)


def recover_after_bind_abort(e, sh=None, refcnt=None, driver_of=None, active_fn=None, block_remove=None):
    """After a bind ABORT. Returns True when the caller must run the full restore (nvidia modules already unloaded), else False."""
    sh = sh or globals()["sh"]
    refcnt = refcnt or module_refcnt
    driver_of = driver_of or driver_name
    active_fn = active_fn or (lambda m: None)
    block_remove = block_remove or G.remove_block
    if isinstance(e, WedgeLite):
        rc, _ = sh(["systemctl", "start", "gdm"], 60)
        active_fn("WEDGE-LITE: gdm start rc=%s; session stopped, coordinator to be told" % (rc,))
        return False
    if refcnt("nvidia") is None:
        return True
    block_remove()
    sh(["bash", "-c", "echo %s > /sys/bus/pci/drivers_probe" % AUDIO_FN], 30)
    sh(["systemctl", "start", "nvidia-persistenced"], 30)
    rc, _ = sh(["systemctl", "start", "gdm"], 60)
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
        log("waiting %ds after the last restore" % int(wait))
        sleep(wait)
        return wait
    return 0
