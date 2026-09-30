#!/usr/bin/env bash
# ★ dbfast_exit_hook.sh — POST_CAPTURE_HOOK: the vCPU cost of ONE doorbell store, measured in the
# guest (docs/design/V3_DOORBELL_IOEVENTFD.md §7). ⚠ On a vast box every number is NESTED.
#
#   KF3_DBFAST_PROBE=0x007f07ff KF3_DEV_EXTRA=doorbell-ioeventfd=on[,dummy-bar=on] \
#     POST_CAPTURE_HOOK=scripts/bench/dbfast_exit_hook.sh scripts/bench/boot_capture.sh <tag>
#
# Stores, each timed in batches by dbfast_exitbench.c:
#   probe   PROBE (default 0x007f07ff) to the BAR0 doorbell (0xbb0090) — with the fast path ON and
#           KF3_DBFAST_PROBE set, a KVM ioeventfd matches it in the host kernel; OFF, it exits to
#           QEMU. Its slot holds no channel, so either way the device then does NOTHING: the
#           difference between an ON and an OFF boot is the transport alone.
#   unknown 0x007f07fe to the same register — never registered: the trapped reference in THIS boot.
#   dummy   (only with dummy-bar=on) the MSI-X BAR's w827 pages: +0x8000 lockless no-op MMIO,
#           +0x9000 BQL no-op MMIO, +0xa000 a plain (non-DATAMATCH) ioeventfd with no memslot.
# Output: `DBX_<name> …` lines (p50/p90/p99/mean/max ns per store).
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
G="$HERE/gssh_nv"
PROBE=${KF3_DBFAST_PROBE:-0x007f07ff}
N=${DBX_STORES:-200000}
echo "DBX_START $(date -Is) probe=$PROBE stores=$N extra=${KF3_DEV_EXTRA:-none}"
$G 'cat > /tmp/dbx.c' < "$HERE/dbfast_exitbench.c" || { echo "DBX_OUTCOME=push-failed"; exit 0; }
$G 'rm -f /tmp/dbx; gcc -O2 -o /tmp/dbx /tmp/dbx.c 2>&1; echo GCC_DBX_RC=$?'
BDF=$($G "lspci -Dnd 10de: | awk '{print \$1}' | head -1" 2>/dev/null | tr -d '\r')
[ -n "$BDF" ] || { echo "DBX_OUTCOME=no-nvidia-function"; exit 0; }
R0=/sys/bus/pci/devices/$BDF/resource0
R5=/sys/bus/pci/devices/$BDF/resource5
echo "DBX_BDF=$BDF"
run() {  # $1 name, $2 resource, $3 offset, $4 value
    local out
    out=$($G "sudo timeout 600 /tmp/dbx $2 $3 $4 $N 100" 2>&1 | tr -d '\r' | grep '^DBX ')
    echo "DBX_$1 ${out#DBX }"
}
for rep in 1 2 3; do
    run "probe_r$rep" "$R0" 0xbb0090 "$PROBE"
    run "unknown_r$rep" "$R0" 0xbb0090 0x007f07fe
done
case ",${KF3_DEV_EXTRA:-}," in
*,dummy-bar=on,*)
    run dummy_lockless "$R5" 0x8000 0x1
    run dummy_bql "$R5" 0x9000 0x1
    run dummy_ioeventfd "$R5" 0xa000 0x1
    ;;
esac
echo "DBX_DONE $(date -Is)"
