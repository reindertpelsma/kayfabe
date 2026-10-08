#!/usr/bin/env bash
# ★ USERD IOVA spike runner — one `kf-userd-iova` process per case, host dmesg diffed per case.
#
# Usage (on the trusted host, in a checkout):  bash scripts/bench/userd_iova_spike.sh <out_dir> [bin]
#
# Rules it encodes (CLAUDE.md "Bench traps"): a start marker and an exit line per case; the
# revision is the first line; the host's own state (other qemu processes, GPU, free memory) is
# recorded, never touched; the harness pins up to 8 GiB of host RAM per big case, so a case waits
# until MemAvailable covers it; every case has a hard timeout and the GPU is checked alive after it.
# The Windows experiment VM and any raw-client job share this GPU: this script never kills anything.
set -uo pipefail
cd "$(dirname "$0")/../.."
OUT=${1:?out dir}
BIN=${2:-${CARGO_TARGET_DIR:-target}/release/kf-userd-iova}
mkdir -p "$OUT"
LOG=$OUT/matrix.log
NEED_KB=$((12 * 1024 * 1024))

# case offset  (offsets: 1 GiB, 4 GiB+1 MiB, ~7.9 GiB, 7.75 GiB+20 KiB for the (C) page)
CASES=(
  "ctl_vram 0"
  "big 0"
  "big 40000000"
  "big 100100000"
  "big 1e6666000"
  "small_alone 40000000"
  "small_before 40000000"
  "small_after 40000000"
  "small_alone 1f0005000"
  "small_before 1f0005000"
  "small_after 1f0005000"
  "small_before 1e6666000"
  "small_after 1e6666000"
  "big 40000000"
  "small_before 40000000"
  "small_after 40000000"
)

{
  echo "USERD_IOVA_MATRIX_START $(date -Is)"
  echo "HEAD=$(git rev-parse --short=8 HEAD) dirty=$(git status --porcelain --untracked-files=no | wc -l)"
  echo "BIN=$BIN"
  nvidia-smi --query-gpu=name,driver_version,memory.used,memory.total --format=csv,noheader | sed 's/^/GPU=/'
  echo "uname=$(uname -r)"
  echo "iommu_groups_by_type: $(cat /sys/kernel/iommu_groups/*/type 2>/dev/null | sort | uniq -c | tr '\n' ' ')"
  echo "qemu_running: $(pgrep -c qemu-system || true)"
  n=0
  for c in "${CASES[@]}"; do
    n=$((n + 1))
    set -- $c
    case=$1 off=$2
    while [ "$(awk '/MemAvailable/{print $2}' /proc/meminfo)" -lt "$NEED_KB" ]; do
      echo "WAIT memory (MemAvailable < 12 GiB)"; sleep 10
    done
    before=$(dmesg | wc -l)
    echo "=== CASE $n $case $off $(date -Is) MemAvailable_kB=$(awk '/MemAvailable/{print $2}' /proc/meminfo)"
    timeout -k 5 160 "$BIN" "$case" "$off" 2>&1 | grep -v '^kf-host: map\|^$'
    echo "CASE_EXIT $n ${PIPESTATUS[0]} $(date -Is)"
    dmesg | tail -n +$((before + 1)) | grep -iE 'NVRM|Xid|userd|physical addr|invalid' | sed 's/^/DMESG /' | head -20
    nvidia-smi --query-gpu=memory.used --format=csv,noheader | sed 's/^/GPU_ALIVE memory.used=/'
    sleep 2
  done
  echo "USERD_IOVA_MATRIX_END $(date -Is)"
} 2>&1 | tee "$LOG"
