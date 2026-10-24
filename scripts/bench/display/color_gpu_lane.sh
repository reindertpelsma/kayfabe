#!/usr/bin/env bash
# color_gpu_lane.sh [--out DIR] [--bdf BDF] [--wait SECONDS] [--quick] [--no-build]
#
# PR #92's colour path on a REAL GPU, automated (the CL1 lane did this by hand on a Vast box):
# the GPU-free colour tests first (same revision), then the hardware examples on the host GPU,
# synthetic frames only (no guest, no VM, no host display setting is touched):
#   kf-disp  --test color_hue_vibrance --test color_output_ptx   (GPU-free, must pass first)
#   kf-cuda  --example color_mirror        (the four hand-computed cases)
#   kf-cuda  --example color_sdr           (8 SDR checks, unchanged by #92)
#   kf-cuda  --example color_tmo           (8 tone checks, unchanged by #92)
#   kf-cuda  --example color_hue_vibrance  (the hue/vibrance/boundary table on the GPU,
#                                           every byte vs the CPU oracle and the f64 reference)
#
# Runs from a checkout (or a build-farm worktree on the GPU host):
#   bf autochk92 <branch> bash scripts/bench/display/color_gpu_lane.sh --out /data/claude-autochk92/gpu-<stamp>
#
# GPU use is strictly serial: the lane takes $KF_LOCK (default /tmp/kayfabe-fastguest.lock, the
# fast-guest lock) with `flock -w $WAIT` (default 0: one non-blocking try). A held lock is not a
# failure: verdict BUSY, exit 75, nothing ran. It never stops a VM, a broker or another job.
#
# Result (machine-readable): $OUT/result.json {"lane":"color_gpu","verdict":"PASS|FAIL|BUSY|ERROR",
# "rev":..., "steps":{name: rc}}, plus RESULT lines in $OUT/lane.log. Start marker
# COLOR_GPU_LANE_START and exit line COLOR_GPU_LANE_EXIT rc=N (also as files START/EXIT).
# Exit: 0 PASS, 1 FAIL, 2 ERROR (setup), 75 BUSY.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../../.." && pwd)"
OUT="" BDF="" WAIT=0 QUICK="" BUILD=1
while [ $# -gt 0 ]; do
    case "$1" in
        --out) OUT=$2; shift 2 ;;
        --bdf) BDF=$2; shift 2 ;;
        --wait) WAIT=$2; shift 2 ;;
        --quick) QUICK=--quick; shift ;;
        --no-build) BUILD=0; shift ;;
        *) echo "usage: color_gpu_lane.sh [--out DIR] [--bdf BDF] [--wait S] [--quick] [--no-build]" >&2; exit 2 ;;
    esac
done
REV=$(git -C "$REPO" rev-parse HEAD 2>/dev/null || echo unknown)
DIRTY=$(git -C "$REPO" status --porcelain 2>/dev/null | grep -vc '^??')
OUT=${OUT:-/tmp/color_gpu_lane-$(date +%Y%m%d-%H%M%S)}
mkdir -p "$OUT" || exit 2
LOG="$OUT/lane.log"
declare -A RC
VERDICT=ERROR
say() { echo "$*" | tee -a "$LOG"; }
finish() {
    local rc=$?
    case "$VERDICT" in PASS) rc=0 ;; FAIL) rc=1 ;; BUSY) rc=75 ;; *) rc=2 ;; esac
    {
        printf '{"lane":"color_gpu","classification":"MEASURED","verdict":"%s","rev":"%s","dirty":%s,"bdf":"%s","quick":%s,"steps":{' \
            "$VERDICT" "$REV" "${DIRTY:-0}" "$BDF" "$([ -n "$QUICK" ] && echo true || echo false)"
        local first=1
        for k in "${!RC[@]}"; do
            [ $first = 1 ] || printf ','
            printf '"%s":%s' "$k" "${RC[$k]}"
            first=0
        done
        printf '},"out":"%s","at":"%s"}\n' "$OUT" "$(date -Is)"
    } > "$OUT/result.json"
    say "RESULT lane=color_gpu verdict=$VERDICT rev=$REV"
    say "COLOR_GPU_LANE_EXIT rc=$rc $(date -Is)"
    echo "COLOR_GPU_LANE_EXIT rc=$rc $(date -Is)" > "$OUT/EXIT"
    exit $rc
}
trap finish EXIT
say "COLOR_GPU_LANE_START rev=$REV dirty=$DIRTY out=$OUT $(date -Is)"
echo "COLOR_GPU_LANE_START rev=$REV $(date -Is)" > "$OUT/START"
command -v cargo >/dev/null || { say "COLOR_GPU_ERROR no cargo in PATH"; exit 2; }

# the GPU: the one named, or the first NVIDIA GPU nvidia-smi lists (normalised to 0000:BB:DD.F)
if [ -z "$BDF" ]; then
    raw=$(nvidia-smi --query-gpu=pci.bus_id --format=csv,noheader 2>/dev/null | head -1)
    [ -n "$raw" ] || { say "COLOR_GPU_ERROR no GPU (nvidia-smi lists none); pass --bdf"; exit 2; }
    BDF=$(echo "$raw" | sed -E 's/^0*([0-9a-fA-F]{4}):/\1:/' | tr 'A-F' 'a-f')
fi
say "COLOR_GPU_BDF $BDF driver=$(nvidia-smi --query-gpu=driver_version --format=csv,noheader 2>/dev/null | head -1)"

# build first (no lock needed: no GPU), then the GPU-free tests of the same revision
TGT=${CARGO_TARGET_DIR:-$REPO/target}
step() { # step <name> <cmd...>: run, log, record rc
    local name=$1; shift
    say "COLOR_GPU_STEP_START $name"
    ( cd "$REPO" && "$@" ) > "$OUT/$name.log" 2>&1
    RC[$name]=$?
    grep -aE '^(COLOR_GPU|TMO_GPU|COLOR_HV_GPU_SUMMARY|test result)' "$OUT/$name.log" | grep -v ' PASS ' | tail -20 | tee -a "$LOG" >/dev/null
    grep -acE ' PASS( |$)' "$OUT/$name.log" | sed "s/^/COLOR_GPU_STEP_PASSLINES $name /" | tee -a "$LOG" >/dev/null
    say "COLOR_GPU_STEP_EXIT $name rc=${RC[$name]}"
}
if [ "$BUILD" = 1 ]; then
    step build cargo build --locked --release -p kf-cuda --examples
    [ "${RC[build]}" = 0 ] || { VERDICT=ERROR; exit 2; }
fi
step gpu_free_tests cargo test --locked --release -p kf-disp --test color_hue_vibrance --test color_output_ptx
[ "${RC[gpu_free_tests]}" = 0 ] || { say "COLOR_GPU_FAIL GPU-free colour tests failed: the GPU run would test a broken reference"; VERDICT=FAIL; exit 1; }

# the GPU, serially
exec 9>"${KF_LOCK:-/tmp/kayfabe-fastguest.lock}"
if ! flock -w "$WAIT" 9; then
    say "COLOR_GPU_BUSY ${KF_LOCK:-/tmp/kayfabe-fastguest.lock} is held by another GPU job; nothing ran on the GPU"
    VERDICT=BUSY; exit 75
fi
say "COLOR_GPU_LOCKED ${KF_LOCK:-/tmp/kayfabe-fastguest.lock}"
EX="$TGT/release/examples"
for e in color_mirror color_sdr color_tmo; do
    step "$e" "$EX/$e" "$BDF"
done
step color_hue_vibrance "$EX/color_hue_vibrance" "$BDF" --out "$OUT/color_hue_vibrance.json" $QUICK
grep -a 'COLOR_HV_GPU_SUMMARY' "$OUT/color_hue_vibrance.log" | tee -a "$LOG" >/dev/null

VERDICT=PASS
for k in "${!RC[@]}"; do [ "${RC[$k]}" = 0 ] || VERDICT=FAIL; done
[ "$VERDICT" = PASS ] && exit 0 || exit 1
