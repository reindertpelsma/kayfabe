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
# ★ [2026-09-28] `HOST_DRIVER=<version>` names the version instead of a URL (the driver-matrix
# sweep, `scripts/drivermatrix/sweep.sh`); `RUN_URL` still wins when given. The default stays the
# bench's 580.159.04 on the XFree86/ path.
[ -n "${HOST_DRIVER:-}" ] && [ -z "${RUN_URL:-}" ] && \
  RUN_URL=https://us.download.nvidia.com/XFree86/Linux-x86_64/$HOST_DRIVER/NVIDIA-Linux-x86_64-$HOST_DRIVER.run
RUN_URL=${RUN_URL:-https://us.download.nvidia.com/XFree86/Linux-x86_64/580.159.04/NVIDIA-Linux-x86_64-580.159.04.run}
RUN=/root/$(basename "$RUN_URL")
echo "DRIVER_SWAP_START $(date -Is)"
echo "before: $(cat /proc/driver/nvidia/version 2>/dev/null | head -1)"

# ⊘ [2026-09-28] XFree86/ IS NOT EVERY VERSION'S PATH. `[measured 2026-09-26, box 52788835]`
# 570.148.08's .run is 404 under XFree86/Linux-x86_64/ and present only on the datacenter path
# `tesla/<v>/` (hostwalk2: `SWAP host=570.148.08 rc=3`); hostwalk3 swapped it from there. An
# XFree86/ URL is therefore tried as given, then as its tesla/ twin — the same two paths
# `stage_guest_driver.sh` tries for the guest's .run. Downloaded to `.part` and renamed, so a cut
# download is never mistaken for a .run on the next swap.
if [ ! -s "$RUN" ]; then
  URLS=("$RUN_URL")
  case "$RUN_URL" in
    */XFree86/Linux-x86_64/*)
      _v=$(basename "$RUN" .run | sed 's/^NVIDIA-Linux-x86_64-//')
      URLS+=("https://us.download.nvidia.com/tesla/$_v/$(basename "$RUN")") ;;
  esac
  for u in "${URLS[@]}"; do
    echo "downloading $u"
    curl -fsSL -o "$RUN.part" "$u" && { mv "$RUN.part" "$RUN"; break; }
  done
  rm -f "$RUN.part"
  [ -s "$RUN" ] || { echo "DOWNLOAD_FAILED (tried: ${URLS[*]})"; exit 3; }
fi
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
# ⊘ [2026-10-03, box 54032077] The "Ubuntu Desktop (VM)" template runs sddm → Xorg → KDE on the GPU,
# which holds nvidia_drm/nvidia_modeset, so the rmmods below cannot unload anything. Stop the host
# display manager for the swap and keep it off (the bench needs no host desktop). Recorded in
# /root/prov/host_dm_stopped so a later broker test knows to start it again on the new driver.
for dm in display-manager sddm gdm3 lightdm; do
  if systemctl is-active --quiet "$dm" 2>/dev/null; then
    systemctl stop "$dm"; systemctl disable "$dm" >/dev/null 2>&1
    echo "stopped host display manager: $dm"; mkdir -p /root/prov; echo "$dm" >> /root/prov/host_dm_stopped
  fi
done
for m in nvidia_uvm nvidia_drm nvidia_modeset nvidia; do rmmod $m 2>/dev/null; done
echo "modules still loaded: $(lsmod | grep -c '^nvidia')"

# ⊘ [2026-10-03, box 54032077] THE PURGE MUST NOT NAME A VERSION. It used to purge the 575 packages
# the vast CLI template ships. The "Ubuntu Desktop (VM)" template ships Ubuntu's packaged
# nvidia-driver-580-open 580.105.08 plus nvidia-driver-pinning-580, so 21 driver packages survived,
# purge_rc was still 0, and the .run refused with "alternate driver installation". Purge every
# installed or config-files NVIDIA DRIVER package, at whatever version, and keep the container
# toolkit (libnvidia-container*, nvidia-container-*), which is not a driver.
driver_pkgs() {
  dpkg-query -W -f='${Package}:${Architecture} ${db:Status-Abbrev}\n' 2>/dev/null \
    | awk '$2 ~ /^(ii|hi|rc|iF|iU|hF|hU)/ {print $1}' \
    | grep -E '^(nvidia-|libnvidia-|xserver-xorg-video-nvidia)' | grep -v -E 'container'
}
PKGS=$(driver_pkgs | tr '\n' ' ')
echo "driver packages to purge: ${PKGS:-<none>}"
PURGE_RC=0
if [ -n "$PKGS" ]; then
  DEBIAN_FRONTEND=noninteractive apt-get purge -y --allow-change-held-packages $PKGS 2>&1 | tail -5
  PURGE_RC=${PIPESTATUS[0]}
fi
NREM=$(driver_pkgs | wc -l)
echo "purge_rc=$PURGE_RC  nvidia driver packages remaining: $NREM  ⇒ MUST be 0"
[ "$NREM" -eq 0 ] || PURGE_RC=7
# ⊘ Do NOT run the installer over a failed purge: it produces the documented symptom from
#    the wrong cause, which is exactly how this cost an hour.
if [ "$PURGE_RC" -ne 0 ]; then
  echo "⊘ PURGE FAILED (rc=$PURGE_RC) -- refusing to run the installer over it."
  echo "DRIVER_SWAP_DONE rc=6"; exit 6
fi
rm -rf /var/lib/dkms/nvidia
# ⊘⊘ THE STALE MODULE SHADOWS THE NEW ONE. `[measured 2026-09-26, box 52788835]` a second swap
# (580.159.04 -> 580.95.05, then -> 580.65.06) reported the installer complete and
# `OPEN_MODULE=yes` while `modprobe` kept loading the FIRST install's `updates/dkms/nvidia.ko`
# (580.159.04): userspace at the new version, the kernel at the old one, NVML "Driver/library
# version mismatch". Clearing /var/lib/dkms does not remove the installed .ko files. Remove them,
# re-index, and check the loaded version against the .run below.
rm -f /lib/modules/"$(uname -r)"/updates/dkms/nvidia*.ko* /lib/modules/"$(uname -r)"/kernel/drivers/video/nvidia*.ko* 2>/dev/null
depmod -a

# ⊘⊘ [2026-09-28] THE RUNNING KERNEL'S OWN COMPILER, never the default `cc`. `[measured 2026-09-26,
# box 52788835, the kept installer logs]` 565.57.01 and 550.54.14 failed "Building kernel modules"
# on Ubuntu 22.04's HWE 6.8 kernel: the installer ran `CC="/usr/bin/cc"` (gcc-11) while the kernel
# was built by `x86_64-linux-gnu-gcc-12` — `Failed CC version check`, then
# `cc: error: unrecognized command-line option '-ftrivial-auto-var-init=zero'` on every object.
# 570+ pick the kernel's compiler themselves; older .run installers take `$CC` (the installer's
# own advice: "set the CC environment variable to the compiler that was used to compile the
# kernel"). ★ DERIVED from the kernel, not hard-coded: its build tree's `CONFIG_CC_VERSION_TEXT`
# (the first word is the compiler's name), else `/proc/version` (the same string, for the
# RUNNING kernel — which is the one the installer builds for). A missing compiler is installed
# from its package (`x86_64-linux-gnu-gcc-12` ships in `gcc-12`); if that fails the installer runs
# with its default and the log says so.
KREL=$(uname -r)
KCC=$(sed -n 's/^CONFIG_CC_VERSION_TEXT="\([^ ]*\) .*/\1/p' "/lib/modules/$KREL/build/.config" 2>/dev/null)
[ -n "$KCC" ] || KCC=$(sed -n 's/^Linux version [^ ]* ([^)]*) (\([^ ]*\) .*/\1/p' /proc/version 2>/dev/null)
if [ -n "$KCC" ] && ! command -v "$KCC" >/dev/null 2>&1; then
  echo "installing ${KCC##*-linux-gnu-} (the compiler kernel $KREL was built with: $KCC)"
  DEBIAN_FRONTEND=noninteractive apt-get install -y -qq "${KCC##*-linux-gnu-}" >/dev/null 2>&1 \
    || { apt-get update -qq >/dev/null 2>&1; DEBIAN_FRONTEND=noninteractive apt-get install -y -qq "${KCC##*-linux-gnu-}" >/dev/null 2>&1; }
  echo "kernel_cc_install_rc=$?"
fi
if [ -n "$KCC" ] && command -v "$KCC" >/dev/null 2>&1; then
  export CC="$KCC"
  echo "KERNEL_CC=$KCC ($(command -v "$KCC")) for kernel $KREL"
else
  echo "KERNEL_CC=${KCC:-unknown} NOT AVAILABLE for kernel $KREL ⚠ the installer uses its default cc (a <570 .run fails on a gcc-12 kernel)"
fi

sh "$RUN" --silent --no-x-check --no-nouveau-check --no-questions --dkms -m=kernel-open -j8 2>&1 | tail -15
echo "installer_rc_IGNORED_ON_PURPOSE=$?"
# ⊘ The installer overwrites /var/log/nvidia-installer.log on every run, so the NEXT swap destroys
# the only statement of why THIS one failed (measured 2026-09-26: 565.57.01 and 550.54.14 failed
# "Building kernel modules" and the log was gone one swap later). Keep a per-version copy.
cp /var/log/nvidia-installer.log "/root/nvidia-installer-$(basename "$RUN" .run).log" 2>/dev/null

# ---- verification: CONTENT, not exit codes ----
modprobe nvidia 2>/dev/null
VER=$(cat /proc/driver/nvidia/version 2>/dev/null | head -1)
echo "after: $VER"
echo "modinfo: $(modinfo nvidia 2>/dev/null | grep -E '^(version|license)' | tr '\n' ' ')"
if echo "$VER" | grep -q "Open Kernel Module"; then
  echo "OPEN_MODULE=yes"
else
  echo "OPEN_MODULE=no  ⊘ THE SWAP DID NOT TAKE -- do not run anything downstream"
  grep -a -n -E "error:|Error |\*\*\*|conftest|FATAL" /var/log/nvidia-installer.log 2>/dev/null | head -20 | sed 's/^/  installer.log: /'
  echo "DRIVER_SWAP_DONE rc=4"; exit 4
fi
WANT=$(basename "$RUN" .run | sed 's/^NVIDIA-Linux-x86_64-//')
if echo "$VER" | grep -q " $WANT "; then
  echo "VERSION_MATCH=yes ($WANT)"
else
  echo "VERSION_MATCH=no ⊘ the loaded module is not the $WANT this .run installed -- do not run anything downstream"
  echo "DRIVER_SWAP_DONE rc=7"; exit 7
fi

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
