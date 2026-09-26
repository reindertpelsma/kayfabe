#!/usr/bin/env bash
# host.sh <resdir> [item...] — the BARE-METAL baseline of the headless-graphics test set, on this
# box's GPU, inside the guest image's userspace (hostroot.sh). Same items.sh, same binaries, same
# NVIDIA 580.159.04 userspace as the guest; per item the HOST dmesg slice is kept (an Xid here means
# the workload faults on bare metal — the item is then NOTRUN for kayfabe, not a kayfabe result).
# Writes <resdir>/host.res, host.dig, <item>.host.log, <item>.host_baremetal_dmesg.log, <item>.host_art.tar
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
R=${1:?resdir}; shift; mkdir -p "$R"
HR="$HERE/hostroot.sh"; ROOT=${GSET_ROOT:-/mnt/gset}/root
say(){ echo "[gset-host $(date +%T)] $*"; }
pgrep -x qemu-system-x86 >/dev/null && { say "⊘ a QEMU is running — the bench is serial"; exit 2; }
# ★ the host's nvidia-drm with modeset=1, like the guest (the Vulkan ICD / EGL device platform / GBM
#   all go through it). ⊘ A loaded module keeps its parameter: reload only when it differs.
if [ "$(cat /sys/module/nvidia_drm/parameters/modeset 2>/dev/null)" != Y ]; then
    rmmod nvidia_drm 2>/dev/null; modprobe nvidia-drm modeset=1 || say "⚠ modprobe nvidia-drm modeset=1 failed"
fi
say "host nvidia-drm modeset=$(cat /sys/module/nvidia_drm/parameters/modeset 2>/dev/null) dri=$(ls /dev/dri 2>/dev/null | tr '\n' ' ')"
bash "$HR" up || exit 2
trap 'bash "$HR" down' EXIT
mkdir -p "$ROOT/var/tmp/gfxset/bin" && cp -a "$HERE"/. "$HERE/../video_lane.sh" "$ROOT/var/tmp/gfxset/bin/" && chown -R 1000:1000 "$ROOT/var/tmp/gfxset"
ITEMS=${*:-$(bash "$HR" run bash /var/tmp/gfxset/bin/items.sh host list)}
say "items: $ITEMS"
for item in $ITEMS; do
    h0=$(dmesg | wc -l)
    res=$(timeout "${GSET_ITEM_TMO:-1800}" bash "$HR" run bash /var/tmp/gfxset/bin/items.sh host "$item" 2>&1)
    dmesg | tail -n +"$((h0+1))" > "$R/$item.host_baremetal_dmesg.log"
    cp -f "$ROOT/var/tmp/gfxset/out/host/$item.log" "$R/$item.host.log" 2>/dev/null
    ( cd "$ROOT/var/tmp/gfxset/out/host/$item.d" 2>/dev/null && tar -cf - --exclude='*.yuv' --exclude='*.raw' . ) > "$R/$item.host_art.tar" 2>/dev/null
    line=$(grep -a '^GSET_RES ' <<<"$res" | tail -1)
    [ -n "$line" ] || line="GSET_RES side=host item=$item verdict=HANG rc=- secs=- note=no-result-line"
    hx=$(grep -c 'Xid' "$R/$item.host_baremetal_dmesg.log")
    echo "$line host_xid=$hx" | tee -a "$R/host.res"
    grep -a -E '^GSET_(DIG|VAL) ' <<<"$res" >> "$R/host.dig"
done
say "HOST_DONE $(grep -c 'verdict=PASS' "$R/host.res") pass / $(grep -c '^GSET_RES' "$R/host.res") rows"
