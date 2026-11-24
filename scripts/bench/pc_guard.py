#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
"""Guards for a VFIO bind of the pc's GPU (STATUS: LIVE, 2026-11-24; used by pc_bind.py and pc_safe_probe.py).

While the GPU is on vfio-pci nothing on the host may load the nvidia module. A load (an `nvidia-smi`, which loads it through
`nvidia-modprobe`, or any `modprobe nvidia*`) probes no device and unloads again, and Ubuntu's stock
`/usr/lib/udev/rules.d/71-nvidia.rules` turns that into a loop: every `add` of the nvidia driver runs `modprobe nvidia-drm` and
friends, every `remove` runs `modprobe -r`. On the pc this queued about 320 module loads a minute during a bind, and at the
restore the queued events kept unloading the reloaded driver, so gdm could not take the card
(traces/vfio_s3_control_20261124/README.md).

Two guards:

(a) `block_conf_text()`: a modprobe.d file `install <module> /bin/false` for the four nvidia modules, written for the bind's
    duration under /run/modprobe.d (tmpfs: a reboot cannot leave the host without its driver) and removed at the restore.
    Any load attempt then fails at once instead of loading the module.
(b) `restore_gate()`: before the driver is loaded again, wait until the udev queue is empty and no nvidia modprobe/rmmod
    runs. On a timeout the ladder that repaired the loop by hand is run by the code: restart systemd-udevd (drops the stale
    queue), wait again; still not quiet is an abort (the driver is never loaded under a looping queue). After a ladder
    restart the caller re-triggers the device events and restarts gdm (`ladder_used`).

The decisions are pure functions over facts (`bind_active`, `udev_queue_empty`, `nvidia_load_pending`), so the tests need no
hardware. The functions that read the host are small and are injected into the decisions.
"""

from __future__ import annotations

import os
import re
import subprocess
import time
from pathlib import Path

NV_BLOCK_MODULES = ("nvidia", "nvidia_modeset", "nvidia_uvm", "nvidia_drm")
BLOCK_CONF = Path("/run/modprobe.d/kayfabe-vfio-bind-block.conf")
UDEV_QUEUE_FLAG = Path("/run/udev/queue")
ACTIVE_LOG = Path("/root/prov/PC_ACTIVE.log")
GPU_LOCK = Path("/root/prov/pc_gpu.lock")
GPU_FN = "0000:01:00.0"

BLOCK_HEADER = "# kayfabe VFIO bind: no nvidia module may load while the GPU is on vfio-pci; removed at the restore\n"


class GuardError(Exception):
    pass


def block_conf_text(modules=NV_BLOCK_MODULES):
    """The modprobe.d text that makes every load of the nvidia modules fail at once."""
    return BLOCK_HEADER + "".join("install %s /bin/false\n" % m for m in modules)


def block_is_complete(text, modules=NV_BLOCK_MODULES):
    """True when the text blocks every module (dash and underscore spellings are the same module to modprobe)."""
    blocked = {m.replace("-", "_") for m in re.findall(r"^\s*install\s+(\S+)\s+/bin/false\s*$", text or "", re.M)}
    return all(m.replace("-", "_") in blocked for m in modules)


def install_block(path=BLOCK_CONF):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + ".tmp")
    tmp.write_text(block_conf_text())
    os.replace(tmp, path)
    if not block_is_complete(path.read_text()):
        raise GuardError("modprobe block %s did not read back complete" % path)
    return path


def remove_block(path=BLOCK_CONF):
    """Returns True when a block file was removed, False when there was none."""
    try:
        Path(path).unlink()
        return True
    except FileNotFoundError:
        return False


def block_present(path=BLOCK_CONF):
    return Path(path).exists()


_LINE = re.compile(r"^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ)\s+(\S+)\s+(\S+)(.*)$")
_BIND_WORDS = re.compile(r"vfio|VFIO|bind", re.I)


def open_binds(active_lines):
    """Tags whose last START/BOUND/END line in PC_ACTIVE.log leaves a VFIO bind open.

    A tag opens a bind with `BOUND` (any session) or with a `START` line that names vfio/bind. Its next `END` closes it. A
    `START` that does not name a bind (a kf3 session, a build) changes nothing. `NOGPU-INFO` lines never open a bind.
    """
    state = {}
    for line in active_lines:
        m = _LINE.match(line.strip())
        if not m:
            continue
        _, tag, verb, rest = m.groups()
        verb = verb.rstrip(":")
        if verb == "BOUND":
            state[tag] = line.strip()
        elif verb == "START" and _BIND_WORDS.search(rest) and not re.search(r"\bno (VFIO|bind)\b|no GPU", rest, re.I):
            state[tag] = line.strip()
        elif verb == "END":
            state.pop(tag, None)
    return state


def bind_active(active_lines, lock_held, block_file_present, gpu_driver):
    """Is a VFIO bind (possibly) active? Returns (bool, reasons). Any one fact is enough; the answer errs towards 'active'.

    active_lines: the lines of PC_ACTIVE.log; lock_held: the GPU lock is held by someone; block_file_present: the modprobe block
    of a bind exists; gpu_driver: the driver name of the GPU's function 0 (None when it has none).
    """
    reasons = []
    if gpu_driver == "vfio-pci":
        reasons.append("GPU %s is on vfio-pci" % GPU_FN)
    elif gpu_driver is None:
        reasons.append("GPU %s has no driver (mid bind or restore)" % GPU_FN)
    if block_file_present:
        reasons.append("modprobe block %s present" % BLOCK_CONF)
    for tag, line in open_binds(active_lines).items():
        reasons.append("PC_ACTIVE: bind of %s open: %s" % (tag, line[:160]))
    if lock_held:
        reasons.append("GPU lock held")
    return bool(reasons), reasons


def udev_queue_empty(settle_rc, queue_flag_exists):
    """The udev queue is empty when `udevadm settle --timeout=0` returned 0 AND udevd's queue flag file does not exist."""
    return settle_rc == 0 and not queue_flag_exists


_MODPROC = re.compile(r"(^|/)(modprobe|rmmod|insmod|nvidia-modprobe)(\s|$)")


def nvidia_load_pending(proc_cmdlines):
    """True when a process is loading or unloading an nvidia module (modprobe/rmmod/insmod/nvidia-modprobe naming nvidia)."""
    for c in proc_cmdlines:
        c = c.strip()
        if not c:
            continue
        argv0 = c.split()[0]
        if _MODPROC.search(argv0 + " ") and ("nvidia" in c.lower()):
            return True
    return False


def read_proc_cmdlines(proc=Path("/proc")):
    out = []
    for d in Path(proc).iterdir():
        if not d.name.isdigit():
            continue
        try:
            out.append((d / "cmdline").read_bytes().replace(b"\0", b" ").decode(errors="replace"))
        except OSError:
            continue
    return out


def lock_held(path=GPU_LOCK):
    """True when another process holds the GPU lock (a non-blocking shared try fails)."""
    import fcntl
    try:
        fd = os.open(str(path), os.O_RDONLY)
    except OSError:
        return False
    try:
        fcntl.flock(fd, fcntl.LOCK_SH | fcntl.LOCK_NB)
        fcntl.flock(fd, fcntl.LOCK_UN)
        return False
    except OSError:
        return True
    finally:
        os.close(fd)


def gpu_driver(fn=GPU_FN):
    p = "/sys/bus/pci/devices/%s/driver" % fn
    return os.path.basename(os.path.realpath(p)) if os.path.exists(p) else None


def read_active_tail(path=ACTIVE_LOG, n=400):
    try:
        with open(path, "rb") as f:
            f.seek(0, 2)
            f.seek(max(0, f.tell() - 200_000))
            return f.read().decode("utf-8", "replace").splitlines()[-n:]
    except OSError:
        return []


def udev_settle_rc(sh):
    rc, _ = sh(["udevadm", "settle", "--timeout=0"], 30)
    return rc


def wait_quiet(settle_rc_fn, flag_fn, procs_fn, sleep, now_fn, timeout_s, stable_s, log):
    """Wait until the udev queue is empty and no nvidia load is pending, continuously for stable_s. True on success."""
    t0 = now_fn()
    quiet_since = None
    while True:
        now = now_fn()
        empty = udev_queue_empty(settle_rc_fn(), flag_fn())
        pending = nvidia_load_pending(procs_fn())
        if empty and not pending:
            quiet_since = quiet_since if quiet_since is not None else now
            if now - quiet_since >= stable_s:
                log.append("udev queue empty and no nvidia load pending for %ds" % stable_s)
                return True
        else:
            quiet_since = None
        if now - t0 > timeout_s:
            log.append("not quiet after %ds: udev queue empty=%s, nvidia load pending=%s" % (timeout_s, empty, pending))
            return False
        sleep(2)


def restore_gate(sh, settle_rc_fn=None, flag_fn=None, procs_fn=None, sleep=time.sleep, now_fn=time.monotonic,
                 timeout_s=180, ladder_timeout_s=120, stable_s=10, log=None):
    """Guard (b): call BEFORE the nvidia modules are loaded again at a restore. Returns (ok, ladder_used, log lines).

    Step 1: wait for a quiet udev queue (timeout_s). Step 2 (ladder): restart systemd-udevd, which drops the stale queue, and
    wait again (ladder_timeout_s). Not quiet after that: ok=False, the caller must not load the driver.
    """
    log = log if log is not None else []
    settle_rc_fn = settle_rc_fn or (lambda: udev_settle_rc(sh))
    flag_fn = flag_fn or UDEV_QUEUE_FLAG.exists
    procs_fn = procs_fn or read_proc_cmdlines
    if wait_quiet(settle_rc_fn, flag_fn, procs_fn, sleep, now_fn, timeout_s, stable_s, log):
        return True, False, log
    log.append("ladder 1: systemctl restart systemd-udevd: %s" % (sh(["systemctl", "restart", "systemd-udevd"], 60),))
    sleep(3)
    if wait_quiet(settle_rc_fn, flag_fn, procs_fn, sleep, now_fn, ladder_timeout_s, stable_s, log):
        return True, True, log
    log.append("udev queue still not quiet after the udevd restart: the driver is NOT loaded")
    return False, True, log


def retrigger_after_ladder(sh, functions):
    """Ladder 2, after the driver is loaded again: replay the add events that the udevd restart dropped, then settle."""
    log = []
    for fn in functions:
        log.append("udevadm trigger %s: %s" % (fn, sh(["udevadm", "trigger", "--action=add", "/sys/bus/pci/devices/%s" % fn], 60)))
    log.append("udevadm trigger drm: %s" % (sh(["udevadm", "trigger", "--action=add", "--subsystem-match=drm"], 60),))
    log.append("udevadm settle: %s" % (sh(["udevadm", "settle", "--timeout=60"], 90),))
    return log


def run_cmd(cmd, timeout):
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        return p.returncode, p.stdout + p.stderr
    except subprocess.TimeoutExpired:
        return "TIMEOUT", "timeout after %ds" % timeout
