#!/usr/bin/env bash
# ★ perf_compare.sh - the Linux performance baseline: ONE command, the SAME set of lanes, against any
# kf3 revision, on the GPU host. Written 2026-10-10 for the Windows-impact report (owner): run it for the
# revision BEFORE the Windows work, then for the integrated branch, and read the diff table.
#
#   perf_compare.sh <rev> [--baseline <rev|dir>] [--lanes a,b,..] [--reps N] [--summarize-only]
#
#   <rev>        the kf3 revision (8 hex). Its QEMU must already be built:
#                  $PERF_BINS/<rev>/qemu-system-x86_64   (scripts/bench/build_kf3.sh from that revision's
#                  checkout, then copy kf3-bins/<rev> WITH its pc-bios and qemu-bundle links - see
#                  docs/design/V3_LINUX_PERF_BASELINE_6692e621.md, "Reproduce")
#   --baseline   a stored result to diff against (a rev under $PERF_OUT_ROOT, or a directory). The diff
#                table is printed at the end and saved as compare.txt.
#   --lanes      default: bare,fast,ladder_host,ladder_guest,exit
#                  bare          the 30-arm raw client on the bare host, no VMM (the denominator; revision-free)
#                  fast          the 30-arm fast thin guest (scripts/fastguest/fast_suite.sh), kf3, `client_ms` per arm
#                  ladder_host   cup2/cup3/cup8/cup8bench on the bare host (cuda_ladder.sh host)
#                  ladder_guest  the same four rungs in the FAT guest on kf3 (cuda_ladder.sh guest): launch
#                                latency/rate, H2D/D2H, matmul, trapped-exit count
#                  exit          vCPU cost of one doorbell store (dbfast_exit_hook.sh), fast path off and on
#   --reps       repetitions (default 3). One rep = one full pass of the lane.
#
# ## Like for like (read this before comparing two revisions)
#   * The harness scripts are THIS checkout's, for both ends. Only the QEMU binary (the kf3 device) is the
#     revision's. Never run the BEFORE end with the BEFORE checkout's scripts and the AFTER end with another set.
#   * The fast thin guest's initrd (kernel + host-mode nvidia modules + the raw client) is built ONCE and
#     reused for every revision: PERF_FASTGUEST_DIR (default $PERF_OUT_ROOT/$PERF_REF_REV/fastguest). The
#     raw client in it is the REFERENCE revision's, so `client_ms` differs between ends only by the device.
#   * The fat guest image is PERF_FAT_IMG (default /workspace/bench/guest-595.84.qcow2, a qcow2 overlay staged
#     by scripts/drivermatrix/stage_fat_guest.sh: the base image carries 580.159.04, which this host's 595.91.07
#     cannot GSP-boot). The same image for both ends.
#   * Every GPU boot takes `flock -o $PERF_LOCK` (default /tmp/kayfabe-fastguest.lock) - the lock the owner's
#     Windows boots use. A held lock means WAIT, not skip. Nothing here kills a process that it did not start.
#   * Load, other QEMU processes and the GPU's used memory are logged before every rep (raw/noise.log): a rep
#     that ran beside another tenant is identifiable afterwards.
#
# Output: $PERF_OUT_ROOT/<rev>/{raw/,work/,metrics.tsv,summary.tsv,summary.md,meta.txt,run.log,compare.txt}.
# Every lane writes a START and an EXIT marker into run.log, so a killed run is not read as a running one.
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
REV=${1:-}
[ -n "$REV" ] && [ "${REV#-}" = "$REV" ] || { sed -n '2,36p' "$0"; exit 2; }
shift
PERF_ROOT=${PERF_ROOT:-/var/lib/kf-windows-20261005}
PERF_BINS=${PERF_BINS:-$PERF_ROOT/kf3-bins}
PERF_OUT_ROOT=${PERF_OUT_ROOT:-$PERF_ROOT/perf-baseline}
PERF_REF_REV=${PERF_REF_REV:-6692e621}
PERF_LOCK=${PERF_LOCK:-/tmp/kayfabe-fastguest.lock}
PERF_BENCH=${PERF_BENCH:-/workspace/bench}            # guest_key, the fat guest image, gssh_nv's fixed path
PERF_FAT_IMG=${PERF_FAT_IMG:-$PERF_BENCH/guest-595.84.qcow2}
PERF_FASTGUEST_DIR=${PERF_FASTGUEST_DIR:-$PERF_OUT_ROOT/$PERF_REF_REV/fastguest}
PERF_CLIENT=${PERF_CLIENT:-$PERF_OUT_ROOT/$PERF_REF_REV/bin/kayfabe-rm-ladder}
PERF_FAST_BUDGET=${PERF_FAST_BUDGET:-180}
# cup8bench: 20 timed iterations (the ladder's default) but 100 launches per timed batch, so the batched
# per-launch time (BATCH_TOTAL_MS / BATCH) resolves 0.01 us instead of 1 us. N=16 isolates launch overhead.
export KAYFABE_BENCH_ITERS=${PERF_BENCH_ITERS:-20} KAYFABE_BENCH_BATCH=${PERF_BENCH_BATCH:-100}
LANES=bare,fast,ladder_host,ladder_guest,exit
REPS=3
BASE=""
SUMONLY=0
while [ $# -gt 0 ]; do
    case "$1" in
        --lanes) LANES=$2; shift 2 ;;
        --reps) REPS=$2; shift 2 ;;
        --baseline) BASE=$2; shift 2 ;;
        --summarize-only) SUMONLY=1; shift ;;
        *) echo "perf_compare: unknown option $1" >&2; exit 2 ;;
    esac
done
OUT=$PERF_OUT_ROOT/$REV
RAW=$OUT/raw; WORK=$OUT/work
BIN=$PERF_BINS/$REV/qemu-system-x86_64
PY="python3 -I $HERE/perf_summarize.py"
has() { case ",$LANES," in *",$1,"*) return 0 ;; esac; return 1; }
mkdir -p "$RAW" "$WORK"
LOG=$OUT/run.log
say() { echo "[perf_compare $(date -Is)] $*" | tee -a "$LOG"; }

noise() {  # $1 label: who else is on the box right now
    { echo "NOISE $1 $(date -Is) load=$(cut -d' ' -f1-3 /proc/loadavg) qemu=$(pgrep -c qemu-system 2>/dev/null || true)" \
           "gpu_mem_used_mib=$(nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits 2>/dev/null | head -1 | tr -dc 0-9)" \
           "gpu_util=$(nvidia-smi --query-gpu=utilization.gpu --format=csv,noheader,nounits 2>/dev/null | head -1 | tr -dc 0-9)"; } >> "$RAW/noise.log"
}
gpu_idle() {  # the previous VM can hold its GPU reservation for a moment after QEMU exits
    local u
    for _ in $(seq 1 120); do
        u=$(nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits 2>/dev/null | head -1 | tr -dc 0-9)
        [ -n "$u" ] && [ "$u" -le 512 ] && return 0
        sleep 0.5
    done
    say "WARN gpu memory.used still ${u:-?} MiB after 60 s: this rep ran beside another tenant"
}

meta() {
    {
        echo "perf_compare rev=$REV started=$(date -Is)"
        echo "harness_checkout=$REPO harness_rev=$(git -C "$REPO" rev-parse --short=8 HEAD) dirty=$([ -z "$(git -C "$REPO" status --porcelain --untracked-files=no)" ] && echo no || echo yes)"
        echo "lanes=$LANES reps=$REPS"
        echo "host=$(hostname) kernel=$(uname -r) nproc=$(nproc) $(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2 | sed 's/^ *//')"
        echo "gpu=$(nvidia-smi --query-gpu=name,driver_version,pci.device_id,memory.total --format=csv,noheader 2>/dev/null | head -1)"
        echo "nvidia_module=$(grep -m1 -o 'Open Kernel Module\|NVIDIA UNIX x86_64 Kernel Module' /proc/driver/nvidia/version 2>/dev/null)"
        echo "qemu_bin=$BIN sha256=$(sha256sum "$BIN" 2>/dev/null | cut -c1-16) size=$(stat -c %s "$BIN" 2>/dev/null)"
        echo "fastguest_dir=$PERF_FASTGUEST_DIR initrd_sha256=$(sha256sum "$PERF_FASTGUEST_DIR/initrd.cpio.gz" 2>/dev/null | cut -c1-16)"
        echo "client=$PERF_CLIENT sha256=$(sha256sum "$PERF_CLIENT" 2>/dev/null | cut -c1-16)"
        echo "fat_img=$PERF_FAT_IMG (backing: $(qemu-img info "$PERF_FAT_IMG" 2>/dev/null | sed -n 's/^backing file: //p'))"
        echo "lock=$PERF_LOCK  qemu_default_device_args: see raw/*driver.log '== kf3 binary' lines"
    } > "$OUT/meta.txt"
}

lane_bare() {
    for i in $(seq 1 "$REPS"); do
        noise "bare r$i"
        # bare_metal_suite takes the lock itself, for the whole suite
        KF_LOCK=$PERF_LOCK BENCH_DIR=$WORK KF_LADDER=$PERF_CLIENT timeout 1500 \
            bash "$HERE/../fastguest/bare_metal_suite.sh" "perf_${REV}_bare_r$i" 180 > "$RAW/bare_r$i.out" 2>&1
        say "bare r$i rc=$? $(grep -a -m1 '^BARE_SUITE_PASS' "$RAW/bare_r$i.out")"
    done
}

lane_fast() {
    for i in $(seq 1 "$REPS"); do
        noise "fast r$i"
        # fast_suite -> run_fast_guest takes the lock itself, per arm
        KF_DEVICE=kf3 KF_LOCK=$PERF_LOCK BENCH_DIR=$WORK KF_FASTGUEST_DIR=$PERF_FASTGUEST_DIR QEMU_BIN=$BIN \
            timeout 3000 bash "$HERE/../fastguest/fast_suite.sh" "perf_${REV}_fast_r$i" "$PERF_FAST_BUDGET" > "$RAW/fast_r$i.out" 2>&1
        say "fast r$i rc=$? $(grep -a -m1 '^FAST_SUITE_PASS' "$RAW/fast_r$i.out")"
        # keep the per-arm serial logs (boot timestamps) but not the multi-MB qemu logs
        mkdir -p "$RAW/fast_r${i}_serial"; cp -f "$WORK"/fast_"perf_${REV}_fast_r$i"_*_serial.log "$RAW/fast_r${i}_serial/" 2>/dev/null
        rm -f "$WORK"/fast_"perf_${REV}_fast_r$i"_*_ttyS0.log
        for q in "$WORK"/fast_"perf_${REV}_fast_r$i"_*_qemu.log; do [ -e "$q" ] && zstd -q -f --rm "$q" 2>/dev/null; done
    done
}

lane_ladder_host() {
    noise "ladder_host"
    # cuda_ladder host locks per run; its reps are consecutive runs of each rung
    KF_LOCK=$PERF_LOCK BENCH_DIR=$WORK timeout 3000 bash "$HERE/cuda_ladder.sh" host "perf_${REV}_h" "$REPS" > "$RAW/cl_host_all.txt" 2>&1
    say "ladder_host rc=$?"
    cp -f "$WORK/cl_perf_${REV}_h_host.out" "$RAW/cl_host_r1.out"
    cp -f "$WORK"/cl_perf_"${REV}"_h_host_*.log "$RAW/" 2>/dev/null
}

fat_boot() {  # $@ = the command; one GPU boot under the lock. -o: the child (QEMU) does not inherit the lock fd.
    flock -o "$PERF_LOCK" "$@"
}

lane_ladder_guest() {
    [ -f "$PERF_FAT_IMG" ] || { say "SKIP ladder_guest: no fat guest image $PERF_FAT_IMG (scripts/drivermatrix/stage_fat_guest.sh)"; return; }
    for i in $(seq 1 "$REPS"); do
        for r in cup2 cup3 cup8 cup8bench; do
            noise "ladder_guest r$i $r"
            local t=perf_${REV}_g${i}_$r
            fat_boot env BENCH_DIR=$PERF_BENCH KF_GUEST_IMG=$PERF_FAT_IMG QEMU_BIN=$BIN \
                timeout 1500 bash "$HERE/cuda_ladder.sh" guest "$t" 1 "$r" > "$RAW/cl_guest_r${i}_$r.run" 2>&1
            say "ladder_guest r$i $r rc=$? $(grep -a '^CL_ROW' "$PERF_BENCH/cl_${t}_guest.out" | cut -c1-120)"
            cp -f "$PERF_BENCH/cl_${t}_guest.out" "$RAW/cl_guest_r${i}_$r.out"
            cp -f "$PERF_BENCH/run_cl_${t}_${r}_1_probe.log" "$RAW/cl_guest_r${i}_${r}_probe.log" 2>/dev/null
            zstd -q -f "$PERF_BENCH/run_cl_${t}_${r}_1_qemu.log" -o "$RAW/cl_guest_r${i}_${r}_qemu.log.zst" 2>/dev/null
            cp -f "$PERF_BENCH/cl_${t}_${r}_1_driver.log" "$RAW/cl_guest_r${i}_${r}_driver.log" 2>/dev/null
        done
    done
}

lane_exit() {
    [ -f "$PERF_FAT_IMG" ] || { say "SKIP exit: no fat guest image $PERF_FAT_IMG"; return; }
    for i in $(seq 1 "$REPS"); do
        for mode in off on; do
            noise "exit $mode r$i"
            local extra=dummy-bar=on t=perf_${REV}_exit_${mode}_r$i
            [ "$mode" = on ] && extra=doorbell-ioeventfd=on,dummy-bar=on
            gpu_idle
            fat_boot env BENCH_DIR=$PERF_BENCH KF_GUEST_IMG=$PERF_FAT_IMG QEMU_BIN=$BIN KF_DEVICE=kf3 \
                KF3_DBFAST_PROBE=0x007f07ff KF3_DEV_EXTRA=$extra POST_CAPTURE_HOOK="$HERE/dbfast_exit_hook.sh" GQ_TIMEOUT=900 \
                timeout 1500 bash "$HERE/boot_capture.sh" "$t" > "$RAW/exit_${mode}_r${i}_driver.log" 2>&1
            say "exit $mode r$i rc=$?"
            cp -f "$PERF_BENCH/run_${t}_probe.log" "$RAW/exit_${mode}_r${i}_probe.log" 2>/dev/null
            zstd -q -f "$PERF_BENCH/run_${t}_qemu.log" -o "$RAW/exit_${mode}_r${i}_qemu.log.zst" 2>/dev/null
        done
    done
}

if [ "$SUMONLY" = 0 ]; then
    [ -x "$BIN" ] || { echo "perf_compare: no kf3 binary at $BIN - build it first (see the header)" >&2; exit 2; }
    if has fast && [ ! -f "$PERF_FASTGUEST_DIR/initrd.cpio.gz" ]; then
        echo "perf_compare: no fast guest at $PERF_FASTGUEST_DIR (scripts/fastguest/build_fast_guest.sh)" >&2; exit 2
    fi
    if has bare && [ ! -x "$PERF_CLIENT" ]; then echo "perf_compare: no raw client at $PERF_CLIENT" >&2; exit 2; fi
    meta
    say "START rev=$REV lanes=$LANES reps=$REPS bin=$BIN"
    for l in bare fast ladder_host ladder_guest exit; do
        has "$l" || continue
        say "LANE_START $l"
        "lane_$l"
        say "LANE_EXIT $l"
    done
    # not run here (reported by name so an empty cell is never read as a pass): apps, llm_parity, gfx_suite, video_lane
    for l in apps llm gfx video; do
        say "NOT_IN_THIS_HARNESS $l (needs a provisioned guest and host; see the report's 'could not be run')"
    done
fi
$PY extract "$OUT" | tee -a "$LOG"
$PY summary "$OUT" | tee -a "$LOG"
if [ -n "$BASE" ]; then
    [ -d "$BASE" ] || BASE=$PERF_OUT_ROOT/$BASE
    $PY compare "$OUT" "$BASE" | tee "$OUT/compare.txt"
fi
say "EXIT"
