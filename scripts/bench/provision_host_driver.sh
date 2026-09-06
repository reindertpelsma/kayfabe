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

HELD=$(apt-mark showhold | tr '\n' ' ')
echo "held packages: ${HELD:-<none>}"
[ -n "$HELD" ] && apt-mark unhold $HELD

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
echo "purge_rc=$?  nvidia packages remaining: $(dpkg -l | grep -c nvidia)"
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
