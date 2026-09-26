#!/usr/bin/env bash
# ★ WHERE DOES A GUEST DRIVER VERSION STOP? One thin-guest boot (the `--timer` arm) with the guest
# driver at <version>, and the first refusal on each side, read out of the logs:
#   - the guest's own NVRM lines (RmInitAdapter / kgspInitRm failure codes),
#   - kayfabe's named refusals from the QEMU log (SET_GUEST_SYSTEM_INFO, GET_GSP_STATIC_INFO,
#     W349REFUSE with the guest version, unserviced functions/controls),
#   - the raw client's R2 verdict (the grader's own driver gate).
#
#   usage: failure_point.sh <version> [budget=120]
#   prints, last: FAILPOINT guest=<v> host=<v> rev=<r> boot=<ok|stopped> first_refusal=<...>
#
# ⊘ This is a MEASUREMENT of how far a version gets, not a verdict on kayfabe: a version whose
# layouts are not yet moved onto the matrix is expected to stop, and the point is to see WHERE.
set -uo pipefail
V=${1:?usage: failure_point.sh <version> [budget]}
BUDGET=${2:-120}
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BENCH=${BENCH_DIR:-/workspace/bench}
DRV=${KF_DRIVER_STAGE:-/workspace/drivers}/$V
FG=$BENCH/fastguest-$V
CLIENT=${CLIENT:-$REPO/target/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder}
HOSTV=$(nvidia-smi --query-gpu=driver_version --format=csv,noheader 2>/dev/null | head -1)
REV=$(git -C "$REPO" rev-parse --short=8 HEAD)
[ -f "$DRV/STAGED" ] || { echo "FAILPOINT guest=$V REFUSED: not staged"; exit 2; }
if [ ! -f "$FG/BUILT" ]; then
    tmp=$(mktemp -d "$BENCH/.fastguest-$V.XXXX")
    CLIENT="$CLIENT" KF_FROM_HOST=1 KF_GUEST_DRIVER_DIR="$DRV" \
        bash "$REPO/scripts/fastguest/build_fast_guest.sh" "$BENCH/guest.qcow2" "$tmp" > "$tmp.log" 2>&1 \
      && [ -s "$tmp/initrd.cpio.gz" ] || { echo "FAILPOINT guest=$V REFUSED: thin guest build failed ($tmp.log)"; exit 2; }
    { echo "guest_driver=$V"; echo "built=$(date -Is)"; } > "$tmp/BUILT"
    rm -rf "$FG"; mv "$tmp" "$FG"; mv "$tmp.log" "$FG/build.log"
fi
T=fp_${V//./}_$REV
KF_FASTGUEST_DIR=$FG KF3_DEV_EXTRA="guest-driver=$V" KF_DEVICE=kf3 KF_ARMS=--timer \
    bash "$REPO/scripts/fastguest/run_fast_guest.sh" "$T" "$BUDGET" > "$BENCH/${T}_run.log" 2>&1
SER=$BENCH/fast_${T}_serial.log; Q=$BENCH/fast_${T}_qemu.log
echo "== guest $V: NVRM"
grep -a -E "NVRM: (loading|.*RmInitAdapter|.*kgspInitRm|.*failed|.*Failed)" "$SER" 2>/dev/null | head -8 | cut -c1-220
echo "== kayfabe refusals"
grep -a -E "refused|W349REFUSE|UNSERVICED|unserviced|NoEncoding|never measured" "$Q" 2>/dev/null | sort | uniq -c | sort -rn | head -10 | cut -c1-240
echo "== raw client"
grep -a -E "R2 |FAIL .*R[0-9]+|client rc=" "$SER" 2>/dev/null | head -4 | cut -c1-200
first=$(grep -a -m1 -E "refused|W349REFUSE" "$Q" 2>/dev/null | cut -c1-160)
boot=stopped; grep -aq "FASTGUEST: nodes .*nvidia0" "$SER" 2>/dev/null && grep -aq -E "R2 +version" "$SER" && boot=ok
echo "FAILPOINT guest=$V host=$HOSTV rev=$REV verdict=$(grep -a FAST_VERDICT "$BENCH/${T}_run.log" | tail -1) first_refusal=[${first}]"
