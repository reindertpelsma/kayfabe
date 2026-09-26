#!/usr/bin/env bash
# ★ ONE CELL OF THE DRIVER MATRIX ON THE GUEST AXIS: the thin guest (30 arms) and, optionally,
# the fat-guest CUDA ladder, with the guest driver at <version> and the host driver as installed.
#
#   usage: guest_walk.sh <version> <tag> [budget=180] [ladder-reps=0]
#   needs: stage_guest_driver.sh <version> (and stage_fat_guest.sh <version> for a ladder)
#          a kf3 binary for this checkout's revision (scripts/bench/build_kf3.sh), or QEMU_BIN
#   prints, last: MATRIX_ROW host=<v> guest=<v> rev=<r> thin=<pass>/<arms> ladder=<pass>/<n>
#
# ⊘ The thin guest's initrd is built into a TEMPORARY directory and renamed into place only
# after it finished (BUILT marker). `[measured 2026-09-26]` a walk that reused a half-written
# initrd from a killed build booted `Failed to execute /init (error -2)` and was scored as a
# 182 s TIMEOUT of the `--timer` arm — a harness fault that reads exactly like a kayfabe hang.
set -uo pipefail
V=${1:?usage: guest_walk.sh <version> <tag> [budget] [ladder-reps]}
TAG=${2:?tag}
BUDGET=${3:-180}
REPS=${4:-0}
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BENCH=${BENCH_DIR:-/workspace/bench}
DRV=${KF_DRIVER_STAGE:-/workspace/drivers}/$V
FG=$BENCH/fastguest-$V
CLIENT=${CLIENT:-$REPO/target/x86_64-unknown-linux-musl/release/kayfabe-rm-ladder}
export PATH=$HOME/.cargo/bin:$PATH
HOSTV=$(nvidia-smi --query-gpu=driver_version --format=csv,noheader 2>/dev/null | head -1)
REV=$(git -C "$REPO" rev-parse --short=8 HEAD)
[ -z "$(git -C "$REPO" status --porcelain --untracked-files=no)" ] || REV="$REV-dirty"
echo "GUEST_WALK_START $(date -Is) guest=$V host=$HOSTV rev=$REV"
[ -f "$DRV/STAGED" ] || { echo "GUEST_WALK_REFUSED: $DRV not staged (stage_guest_driver.sh $V)"; exit 2; }
[ -x "$CLIENT" ] || { echo "GUEST_WALK_REFUSED: no raw client at $CLIENT"; exit 2; }

if [ ! -f "$FG/BUILT" ]; then
    tmp=$(mktemp -d "$BENCH/.fastguest-$V.XXXX")
    if CLIENT="$CLIENT" KF_FROM_HOST=1 KF_GUEST_DRIVER_DIR="$DRV" \
         bash "$REPO/scripts/fastguest/build_fast_guest.sh" "$BENCH/guest.qcow2" "$tmp" > "$tmp.log" 2>&1 \
       && [ -s "$tmp/initrd.cpio.gz" ] && [ -s "$tmp/vmlinuz" ]; then
        { echo "guest_driver=$V"; echo "built=$(date -Is)"; } > "$tmp/BUILT"
        rm -rf "$FG"; mv "$tmp" "$FG"; mv "$tmp.log" "$FG/build.log"
    else
        echo "GUEST_WALK_REFUSED: the thin guest for $V did not build ($tmp.log)"; exit 2
    fi
fi
echo "thin guest: $(grep -E 'GUEST DRIVER' "$FG/build.log" | head -1)"

KF_FASTGUEST_DIR=$FG KF3_DEV_EXTRA="guest-driver=$V" KF_DEVICE=kf3 \
    bash "$REPO/scripts/fastguest/fast_suite.sh" "$TAG" "$BUDGET" > "$BENCH/${TAG}_suite.log" 2>&1
line=$(grep -E "FAST_SUITE_PASS" "$BENCH/${TAG}_suite.log" | tail -1)
echo "$line"
grep -E "verdict=(FAIL|CRASH|TIMEOUT|NOTRUN)" "$BENCH/${TAG}_suite.out" | head -10
pass=$(sed -n 's/.*FAST_SUITE_PASS=\([0-9]*\).*/\1/p' <<<"$line"); arms=$(sed -n 's/.*ARMS=\([0-9]*\).*/\1/p' <<<"$line")

lad="-"
if [ "$REPS" -gt 0 ]; then
    IMG=$BENCH/guest-$V.qcow2
    if [ -f "$IMG.STAGED" ]; then
        KF_GUEST_IMG=$IMG KF3_DEV_EXTRA="guest-driver=$V" KF_DEVICE=kf3 \
            bash "$REPO/scripts/bench/cuda_ladder.sh" guest "$TAG" "$REPS" > "$BENCH/${TAG}_ladder.log" 2>&1
        grep -a '^CL_ROW' "$BENCH/cl_${TAG}_guest.out" 2>/dev/null | sed 's/ ledger=.*//'
        lp=$(grep -ac '^CL_ROW .*verdict=PASS' "$BENCH/cl_${TAG}_guest.out" 2>/dev/null)
        ln=$(grep -ac '^CL_ROW ' "$BENCH/cl_${TAG}_guest.out" 2>/dev/null)
        lad="${lp:-0}/${ln:-0}"
    else
        lad="unstaged"
    fi
fi
echo "MATRIX_ROW host=$HOSTV guest=$V rev=$REV thin=${pass:-?}/${arms:-?} ladder=$lad"
echo "GUEST_WALK_EXIT $(date -Is)"
