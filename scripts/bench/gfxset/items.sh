#!/usr/bin/env bash
# items.sh <side> <item>...|all|list — run headless-graphics test-set items ON THE MACHINE THAT
# HAS THE GPU: the kf3 fat guest (side=guest) or the bare-metal host inside the guest image's own
# userspace (side=host, via hostroot.sh). The commands are identical on both sides; only the
# kernel + GPU path differs, which is what the comparison is about.
#
# One process tree per item, bounded by its own timeout (setsid + timeout -k, so a spawned child
# cannot outlive it). Output protocol: lib.sh. Per-item logs: $GSET_OUT/<side>/<item>.log.
# ⊘ Grading is by CONTENT (GSET_DIG compared host vs guest by the suite) wherever the workload has
#   a deterministic output; an item's own verdict here is only its self-check (the floor that holds
#   without a host). A number printed by a benchmark is never a pass by itself (glxgears, gfx7).
set -uo pipefail
SIDE=${1:?side}; shift
HERE="$(cd "$(dirname "$0")" && pwd)"
. "$HERE/lib.sh"
export GSET_HOME=${GSET_HOME:-/opt/gfxset}          # provisioned binaries + data (guest image)
export GSET_OUT=${GSET_OUT:-/var/tmp/gfxset/out}
export GSET_BIN=$HERE                                  # this directory (shipped at run time)
O=$GSET_OUT/$SIDE; mkdir -p "$O"
. "$HERE/itemdefs.sh"

[ "${1:-}" = list ] && { gset_items; exit 0; }
[ "${1:-}" = all ] && set -- $(gset_items)
for ITEM in "$@"; do
    fn="item_$ITEM"; tmo=$(gset_timeout "$ITEM")
    if ! declare -F "$fn" >/dev/null; then
        echo "GSET_RES side=$SIDE item=$ITEM verdict=NOTRUN rc=- secs=0 note=unknown-item"; continue
    fi
    log=$O/$ITEM.log; W=$O/$ITEM.d; rm -rf "$W"; mkdir -p "$W"
    echo "=== $ITEM side=$SIDE start=$(date -Is) host=$(hostname) kernel=$(uname -r) drv=$(cat /sys/module/nvidia/version 2>/dev/null)" > "$log"
    # ★ opt-in ioctl differential (GSET_NVDIFF=1): the nvdiff recorder (nvidia-gpu-passthrough
    #   tests/mode2/nvdiff, 875c50c, copied verbatim to src/nvdiff/) LD_PRELOADed into the item's whole process
    #   tree; the shim is built from source into $GSET_OUT on first use (gcc is in the image, so the SAME .so
    #   runs on both sides). Records carry the thread id; single-process items align directly with nvdiff.py.
    unset NVD_ENV; if [ "${GSET_NVDIFF:-0}" = 1 ]; then
        SO=$GSET_OUT/nvdiff_shim.so
        [ -s "$SO" ] || gcc -shared -fPIC -O2 -I"$HERE/src/nvdiff" -o "$SO" "$HERE/src/nvdiff/nvdiff_shim.c" -ldl -lpthread \
            || echo "GSET_NVDIFF shim build FAILED" >> "$log"
        NVD_ENV="LD_PRELOAD=$SO NVDIFF_OUT=$W/nvdiff.jsonl"
    fi
    t0=$(date +%s)
    # the item runs in its own session; its stdout is the log (GSET_DIG/VAL lines included)
    env ${NVD_ENV:-} SIDE=$SIDE ITEM=$ITEM W=$W timeout -k 20 "$tmo" setsid bash -c ". '$HERE/lib.sh'; . '$HERE/itemdefs.sh'; cd '$W' && $fn" >> "$log" 2>&1 < /dev/null
    rc=$?; t1=$(date +%s)
    echo "=== end rc=$rc secs=$((t1-t0)) $(date -Is)" >> "$log"
    # the first GSET_FAIL wins; else the first error-looking line (the log's own header excluded)
    note=$(grep -a -m1 -E '^GSET_(FAIL|NEED_MISSING)' "$log" || grep -a -v -E '^=== |^GSET_(DIG|VAL)|errors: 0|Validation: Success' "$log" \
           | grep -a -m1 -iE 'error|fail|illegal|cannot|unable|Xid|abort|segmentation|core dumped|not found')
    note=$(echo "$note" | head -1 | tr -s ' \t' ' ' | cut -c1-160 | tr '|' '/')
    if [ $rc -eq 124 ] || [ $rc -eq 137 ]; then v=TIMEOUT
    elif grep -qa '^GSET_NEED_MISSING' "$log"; then v=NOTRUN
    elif [ $rc -eq 0 ] && grep -qa '^GSET_OK' "$log" && ! grep -qa '^GSET_FAIL' "$log"; then v=PASS; note=${note:+warn:$note}
    else v=FAIL; fi
    echo "GSET_RES side=$SIDE item=$ITEM verdict=$v rc=$rc secs=$((t1-t0)) note=${note:--}"
    grep -a -E '^GSET_(DIG|VAL) ' "$log"
done
exit 0
