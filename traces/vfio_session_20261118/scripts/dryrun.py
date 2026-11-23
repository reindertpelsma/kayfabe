#!/usr/bin/env python3
"""pc dry runs of the safe bind (no nvidia unload, no VFIO). usage: dryrun.py services|audio
services: gdm off, persistenced off, holders, wait for nvidia_drm refcnt 0 (observe only), persistenced+gdm on, nvidia-smi.
audio: gdm off, holders, release_audio (snd_hda_intel unbind, 60 s child), re-probe the audio function, aplay -l, gdm on, nvidia-smi."""
import fcntl, os, sys, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import pcsession as P

mode = sys.argv[1]
fd = os.open(P.LOCK, os.O_RDWR | os.O_CREAT)
fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)      # busy -> raises, never waits
P.settle_after_restore(log=print)
P.active("START dry run %s (no module unload, no VFIO; gdm off for about a minute), flock" % mode)
log = []
rc_out = 0
try:
    pids, txt = P.holder_pids()
    log.append("holders with gdm up: %s\n%s" % (pids, txt))
    log.append("gdm stop: %s" % (P.sh(["systemctl", "stop", "gdm"], 60),))
    time.sleep(3)
    if mode == "services":
        log.append("persistenced stop: %s" % (P.sh(["systemctl", "stop", "nvidia-persistenced"], 30),))
        pids, txt = P.holder_pids()
        log.append("holders after: %s\n%s" % (pids, txt))
        t0 = time.time()
        while time.time() - t0 < 60:
            r = P.module_refcnt("nvidia_drm")
            log.append("t+%ds nvidia_drm refcnt=%s nvidia=%s modeset=%s" % (time.time() - t0, r, P.module_refcnt("nvidia"), P.module_refcnt("nvidia_modeset")))
            if r in (None, 0):
                break
            time.sleep(5)
    else:
        pids, txt = P.holder_pids()
        log.append("holders after gdm stop: %s\n%s" % (pids, txt))
        log.append("fuser snd: %s" % (P.sh(["bash", "-c", "fuser -v /dev/snd/* 2>&1 | head -8"], 20),))
        try:
            P.release_audio(P.sh, P.driver_name, log)
        except P.Abort as e:
            log.append("ABORT/WEDGE: %s" % e)
            rc_out = 3
            if isinstance(e, P.WedgeLite):
                P.recover_after_bind_abort(e)
                raise SystemExit(3)
        log.append("audio driver now: %s" % P.driver_name(P.AUDIO_FN))
        time.sleep(2)
        log.append("probe back: %s" % (P.sh(["bash", "-c", "echo %s > /sys/bus/pci/drivers_probe" % P.AUDIO_FN], 30),))
        time.sleep(3)
        log.append("audio driver after re-probe: %s" % P.driver_name(P.AUDIO_FN))
        log.append("aplay -l: %s" % (P.sh(["bash", "-c", "aplay -l 2>&1 | grep -c '^card'; cat /proc/asound/cards | grep -i nvidia"], 20),))
    log.append("persistenced start: %s" % (P.sh(["systemctl", "start", "nvidia-persistenced"], 30),))
    log.append("gdm start: %s" % (P.sh(["systemctl", "start", "gdm"], 60),))
    time.sleep(6)
    log.append("nvidia-smi: %s" % (P.sh(["nvidia-smi", "--query-gpu=name,driver_version,memory.used", "--format=csv,noheader"], 60),))
    log.append("gdm active: %s" % (P.sh(["systemctl", "is-active", "gdm"], 20),))
    open(P.LAST_RESTORE, "w").write("%d\n" % int(time.time()))
finally:
    P.ROOT.joinpath("ev").mkdir(exist_ok=True)
    P.ROOT.joinpath("ev", "dryrun_%s.txt" % mode).write_text("\n".join(log) + "\n")
    print("\n".join(log))
    P.active("END dry run %s; GPU: %s" % (mode, P.smi_name()))
