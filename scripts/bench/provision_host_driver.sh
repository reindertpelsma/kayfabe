#!/usr/bin/env bash
# ★★★ PHASE 2 of the bench rebuild — swap the rented box's host driver to the OPEN module.
# The recipe is `docs/reference/bench_rebuild_notes.md` §2, measured on four boxes. This is
# that recipe as a script, so the fifth rebuild does not retype it.
#
# WHY IT IS NEEDED: vast templates ship the CLOSED 575 kernel module. The campaign's whole
# oracle story (ogkm as ground truth) needs the OPEN module at 580.159.04.
#
# ⚠ THE TRAP, reproduced on three separate boxes and now a settled template property:
# the nvidia-*-575 packages are `apt-mark hold`-ed. A plain purge exits 100 having removed
# NOTHING, and the .run installer then fails with the DOCUMENTED symptom
#   "installation was canceled due to ... an alternate driver installation"
# which points at the wrong cause. Unhold FIRST.
#
# ⚠ VERIFY ON CONTENT, NEVER ON THE INSTALLER'S EXIT CODE. The property that matters is the
# string "Open Kernel Module" in /proc/driver/nvidia/version -- an installer that exits 0 and
# leaves the closed module loaded is the exact false green this tree keeps paying for.
set -uo pipefail
RUN_URL=${RUN_URL:-https://us.download.nvidia.com/XFree86/Linux-x86_64/580.159.04/NVIDIA-Linux-x86_64-580.159.04.run}
RUN=/root/$(basename "$RUN_URL")
echo "DRIVER_SWAP_START $(date -Is)"
echo "before: $(cat /proc/driver/nvidia/version 2>/dev/null | head -1)"

[ -s "$RUN" ] || { echo "downloading $(basename "$RUN")"; curl -fsSL -o "$RUN" "$RUN_URL" || { echo "DOWNLOAD_FAILED"; exit 3; }; }
ls -la "$RUN"

# ⚠⚠ SECOND TRAP, MEASURED 2026-09-06 ON THIS EXACT PATH — and it is worth more than the
# first one, because it produces the FIRST one's symptom from a DIFFERENT cause.
# A fresh cloud box runs `unattended-upgrade` at boot and holds the dpkg lock for 15+ min.
# With the lock held, BOTH `apt-mark unhold` AND `apt-get purge` fail (exit 100, nothing
# removed), and the .run installer then prints
#     ERROR: The installation was canceled due to the availability or presence of an
#            alternate driver installation
# ⇒ That is the SAME string §2 of bench_rebuild_notes.md documents for the apt-hold trap,
#   and the same string the C's notes document for the original cause. THREE causes, ONE
#   symptom. Diagnosing this from the installer's message alone is impossible; the purge's
#   own exit status is the only thing that separates them, so PRINT IT and act on it.
# ★ Also mask the apt timers: an unattended upgrade landing mid-bench can pull a new kernel
#   out from under a DKMS module and a pinned guest kernel.
systemctl mask --now apt-daily.timer apt-daily-upgrade.timer >/dev/null 2>&1
wait_for_dpkg() {
  local waited=0
  while fuser /var/lib/dpkg/lock-frontend /var/lib/apt/lists/lock >/dev/null 2>&1; do
    [ "$waited" -ge 1200 ] && { echo "DPKG_LOCK_TIMEOUT after ${waited}s"; return 1; }
    [ $((waited % 60)) -eq 0 ] && echo "waiting for dpkg lock (${waited}s): $(fuser -v /var/lib/dpkg/lock-frontend 2>&1 | tail -1)"
    sleep 10; waited=$((waited + 10))
  done
  echo "dpkg lock free after ${waited}s"
}
wait_for_dpkg || { echo "DRIVER_SWAP_DONE rc=5 (dpkg lock never cleared)"; exit 5; }

HELD=$(apt-mark showhold | tr '\n' ' ')
echo "held packages: ${HELD:-<none>}"
if [ -n "$HELD" ]; then apt-mark unhold $HELD; echo "unhold_rc=$?"; fi
remaining_holds=$(apt-mark showhold | wc -l)
echo "holds remaining after unhold: $remaining_holds  ⇒ MUST be 0"

# Stop anything holding the module open, then purge. ⚠ rmmod may return non-zero on the
# second attempt because the first already succeeded -- check lsmod, not $?.
systemctl stop nvidia-persistenced 2>/dev/null
for m in nvidia_uvm nvidia_drm nvidia_modeset nvidia; do rmmod $m 2>/dev/null; done
echo "modules still loaded: $(lsmod | grep -c '^nvidia')"

DEBIAN_FRONTEND=noninteractive apt-get purge -y --allow-change-held-packages \
  'nvidia-driver-575*' 'nvidia-dkms-575*' 'nvidia-kernel-source-575*' \
  'nvidia-kernel-common-575*' 'nvidia-compute-utils-575*' 'nvidia-utils-575*' \
  'libnvidia-*-575*' 'xserver-xorg-video-nvidia-575*' 'nvidia-firmware-575*' \
  nvidia-settings 2>&1 | tail -5
PURGE_RC=$?
NREM=$(dpkg -l | grep -c nvidia)
echo "purge_rc=$PURGE_RC  nvidia packages remaining: $NREM"
# ⊘ Do NOT run the installer over a failed purge: it produces the documented symptom from
#    the wrong cause, which is exactly how this cost an hour.
if [ "$PURGE_RC" -ne 0 ]; then
  echo "⊘ PURGE FAILED (rc=$PURGE_RC) -- refusing to run the installer over it."
  echo "DRIVER_SWAP_DONE rc=6"; exit 6
fi
rm -rf /var/lib/dkms/nvidia

sh "$RUN" --silent --no-x-check --no-nouveau-check --no-questions --dkms -m=kernel-open -j8 2>&1 | tail -15
echo "installer_rc_IGNORED_ON_PURPOSE=$?"

# ---- verification: CONTENT, not exit codes ----
modprobe nvidia 2>/dev/null
VER=$(cat /proc/driver/nvidia/version 2>/dev/null | head -1)
echo "after: $VER"
echo "modinfo: $(modinfo nvidia 2>/dev/null | grep -E '^(version|license)' | tr '\n' ' ')"
if echo "$VER" | grep -q "Open Kernel Module"; then
  echo "OPEN_MODULE=yes"
else
  echo "OPEN_MODULE=no  ⊘ THE SWAP DID NOT TAKE -- do not run anything downstream"
  echo "DRIVER_SWAP_DONE rc=4"; exit 4
fi
echo "$VER" | grep -q "580\." && echo "VERSION_580=yes" || echo "VERSION_580=no ⚠ unexpected version"

# ⚠ ORDERING, measured 2026-09-06: the device nodes are created LAZILY, by `nvidia-modprobe`
# on the first privileged open. Immediately after a fresh install they DO NOT EXIST, so an
# open() test here reports ENOENT on all three and reads as a failed install. That is a FALSE
# NEGATIVE -- the mirror of the false greens elsewhere in this file, and just as misleading.
# Poke the driver first so the nodes exist, THEN test them.
nvidia-modprobe -c 0 -u >/dev/null 2>&1 || nvidia-smi >/dev/null 2>&1
# ★ node existence is not the property that matters -- open() them. EIO here means the GPU
#   never completed GFW boot, which is a hardware state, not a build error.
python3 - <<'PY'
import os
for n in ("/dev/nvidiactl","/dev/nvidia0","/dev/nvidia-uvm"):
    try:
        fd=os.open(n,os.O_RDWR); os.close(fd); print(f"open {n}: OK")
    except OSError as e:
        print(f"open {n}: FAILED {e.errno} {e.strerror}")
PY
nvidia-smi --query-gpu=name,driver_version --format=csv,noheader 2>&1 | head -3
echo "DRIVER_SWAP_DONE rc=0 $(date -Is)"
