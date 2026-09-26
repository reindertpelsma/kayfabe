#!/usr/bin/env bash
# hook.sh <tag> — POST_CAPTURE_HOOK (boot_capture.sh, KF_DEVICE=kf3): run the headless-graphics
# test set INSIDE the fat guest, ONE ITEM PER ssh CALL, and for each item persist
#   $GSET_RES_DIR/<item>.guest.log        the item's own log (from inside the guest)
#   $GSET_RES_DIR/<item>.guest_dmesg.log  the guest dmesg lines this item added (NVRM / Xid)
#   $GSET_RES_DIR/<item>.kf3.log          the kf3 device's stderr lines this item added
#   $GSET_RES_DIR/<item>.host_dmesg.log   the HOST dmesg lines this item added (host Xid = the GPU
#                                         faulted on the guest's work, whatever the guest printed)
# and append its GSET_RES line (+ host_xid / guest_xid / kf3_rc / kf3_refusals) to guest.res and its
# digests to guest.dig.  Stops the boot at the first item after which the guest no longer answers
# (GUEST_DEAD) or no longer renders (WEDGED: the wedge probe = a Vulkan compute job, checked).
# env: GSET_ITEMS (space list), GSET_RES_DIR (host dir), GSET_ITEM_TMO (outer bound, default 1800)
set -uo pipefail
TAG=${1:?tag}
HERE="$(cd "$(dirname "$0")" && pwd)"; G="$HERE/../gssh_nv"
OUT=${GSET_RES_DIR:?GSET_RES_DIR}; mkdir -p "$OUT"
QLOG=${BENCH_DIR:-/workspace/bench}/run_${TAG}_qemu.log
$G true >/dev/null 2>&1 || { echo "GSET_HOOK guest unreachable at start"; exit 0; }
# the scripts ship at run time (a harness fix never needs a re-provisioned image)
tar -C "$HERE" -cf - . -C "$HERE/.." video_lane.sh | $G 'rm -rf /var/tmp/gfxset/bin && mkdir -p /var/tmp/gfxset/bin && tar -xf - -C /var/tmp/gfxset/bin && echo GSET_SHIPPED'
echo "GSET_HOOK tag=$TAG items=[$GSET_ITEMS] guest=$($G 'nvidia-smi --query-gpu=name,driver_version,persistence_mode --format=csv,noheader' 2>&1 | head -1)"
# ★ nvidia-drm modeset=1, displayless (V3_HEADLESS_GRAPHICS.md §3): boot_capture.sh loads only `nvidia`.
echo "GSET_GUEST_DRM $($G 'sudo modprobe nvidia-drm modeset=1; echo rc=$?; sudo cat /sys/module/nvidia_drm/parameters/modeset; ls /dev/dri | tr "\n" " "' 2>&1 | tr '\n' ' ')"
[ "${GSET_GUEST_PM:-0}" = 1 ] && echo "GSET_GUEST_PM=$($G 'sudo nvidia-smi -pm 1 2>&1 | tail -1')"
wedge_probe(){  # a checked Vulkan compute job: prints 1 when the boot still renders
    timeout 120 "$G" 'cd /tmp && timeout -k 5 90 /opt/gfxset/gfxbin/vk_gfx compute 2>&1 | grep -c "^VKC_BAD=0$"' 2>/dev/null | tr -d '\r'
}
for item in $GSET_ITEMS; do
    q0=$(wc -l < "$QLOG" 2>/dev/null || echo 0)
    h0=$(dmesg 2>/dev/null | wc -l)
    d0=$($G 'sudo dmesg | wc -l' 2>/dev/null | tr -d '\r'); d0=${d0:-0}
    res=$(timeout "${GSET_ITEM_TMO:-1800}" "$G" "bash /var/tmp/gfxset/bin/items.sh guest $item" 2>&1 | tr -d '\r')
    src=$?
    alive=1; $G true >/dev/null 2>&1 || { sleep 20; $G true >/dev/null 2>&1 || alive=0; }
    line=$(grep -a '^GSET_RES ' <<<"$res" | tail -1)
    if [ -z "$line" ]; then
        v=HANG; [ $alive = 0 ] && v=GUEST_DEAD
        line="GSET_RES side=guest item=$item verdict=$v rc=ssh$src secs=- note=no-result-line(guest_alive=$alive)"
    fi
    if [ $alive = 1 ]; then
        $G "cat /var/tmp/gfxset/out/guest/$item.log" > "$OUT/$item.guest.log" 2>&1
        $G "sudo dmesg | tail -n +$((d0+1))" > "$OUT/$item.guest_dmesg.log" 2>&1
        # the item's artefacts (images, streams) for eyes and for a byte compare
        $G "cd /var/tmp/gfxset/out/guest/$item.d 2>/dev/null && tar -cf - --exclude='*.yuv' --exclude='*.raw' . 2>/dev/null" > "$OUT/$item.guest_art.tar" 2>/dev/null
    fi
    dmesg 2>/dev/null | tail -n +"$((h0+1))" > "$OUT/$item.host_dmesg.log"
    tail -n +"$((q0+1))" "$QLOG" 2>/dev/null | tail -5000 > "$OUT/$item.kf3.log"
    hx=$(grep -c 'Xid' "$OUT/$item.host_dmesg.log"); gx=$(grep -c 'Xid' "$OUT/$item.guest_dmesg.log" 2>/dev/null); gx=${gx:-0}
    nrc=$(grep -cE 'RC host twin|RC_TRIGGERED' "$OUT/$item.kf3.log")
    nr=$(grep -v 'kf3: family=' "$OUT/$item.kf3.log" | grep -ciE 'refus')
    nu=$(grep -c 'GSP rpc UNSERVICED' "$OUT/$item.kf3.log")   # a guest RM call nobody answers (unserviced.rs)
    echo "$line boot=$TAG host_xid=$hx guest_xid=$gx kf3_rc=$nrc kf3_refusals=$nr kf3_unserviced=$nu kf3_lines=$(wc -l < "$OUT/$item.kf3.log")" | tee -a "$OUT/guest.res"
    grep -a -E '^GSET_(DIG|VAL) ' <<<"$res" | tee -a "$OUT/guest.dig" >/dev/null
    [ $alive = 0 ] && { echo "GSET_HOOK guest dead after $item — stopping this boot"; break; }
    case "$line" in *verdict=PASS*) ;; *)
        if [ "${GSET_WEDGE_PROBE:-1}" = 1 ] && [ "$(wedge_probe)" != 1 ]; then
            echo "GSET_WEDGE boot=$TAG after=$item" | tee -a "$OUT/guest.res"
            echo "GSET_HOOK boot wedged after $item — stopping this boot"; break
        fi ;;
    esac
done
echo "GSET_HOOK_DONE"
