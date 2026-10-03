#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# ★★★ libcuda_channel_census.sh — which privilege host RM stamped on EVERY channel a kf3 QEMU
# allocates, libcuda's included (docs/design/THE_CONSTRAINTS.md §30).
#
# kf-host checks the reply of each channel it births and logs one `kf-host: channel birth` line.
# libcuda's channels (the walker's context, the display's) are allocated inside libcuda, where no
# kf3 code sees the reply. This runs one guest arm per configuration with QEMU under the
# LD_PRELOAD observer `chan_alloc_observer.c`, which logs RM's reply for every GPFIFO channel
# alloc in the process, and summarises the result.
#
# usage (on a provisioned box, from a checkout): libcuda_channel_census.sh <tag> <qemu-binary> [config ...]
#   configs: walker (default arm, the walker's context), display (display=on adds the display
#   context), negctl (KF3_NEGCTL_SKIP_CAP_BRACKET=1: kf-host's first birth is ADMIN and refused,
#   the observer's known-positive). Default: walker display negctl.
# Output: /root/prov/<tag>_chanobs.log — a START line, per-config CHANOBS_SUMMARY lines, the raw
# observer lines, and an EXIT line written by this script.
set -uo pipefail
TAG=${1:?tag}; QREAL=${2:?qemu binary}; shift 2
[[ "$TAG" =~ ^[a-zA-Z0-9][a-zA-Z0-9._-]*$ ]] || { echo "invalid tag" >&2; exit 2; }
CONFIGS=("$@"); [ "${#CONFIGS[@]}" -gt 0 ] || CONFIGS=(walker display negctl)
SRC=$(cd "$(dirname "$0")/../../.." && pwd)
BENCH=${BENCH_DIR:-/workspace/bench}
PROV=${KF_PROV_DIR:-/root/prov}
OUT="$PROV/${TAG}_chanobs.log"
mkdir -p "$PROV" "$BENCH/chanobs"
[ ! -e "$OUT" ] || { echo "refusing to overwrite $OUT" >&2; exit 2; }
exec >"$OUT" 2>&1
trap 'rc=$?; echo "EXIT rc=$rc $(date -Is)"' EXIT
echo "START $(date -Is) checkout=$(git -C "$SRC" rev-parse --short=8 HEAD) qemu=$QREAL"
echo "qemu_built=$(date -r "$QREAL" -Is 2>/dev/null || echo missing)"
[ -x "$QREAL" ] || { echo "no qemu binary at $QREAL"; exit 2; }
SO="$BENCH/chanobs/${TAG}.so"
cc -shared -fPIC -O2 -Wall -Wextra -o "$SO" "$SRC/scripts/bench/sec/chan_alloc_observer.c" -ldl || { echo "observer build failed"; exit 2; }
fails=0
for cfg in "${CONFIGS[@]}"; do
    log="$BENCH/chanobs/${TAG}_${cfg}.obs"; rm -f "$log"
    wrap="$BENCH/chanobs/${TAG}_${cfg}_qemu.sh"
    printf '#!/bin/sh\nexec env LD_PRELOAD=%s KF_CHANOBS_LOG=%s %s "$@"\n' "$SO" "$log" "$QREAL" > "$wrap"
    chmod 755 "$wrap"
    extra=(); case "$cfg" in
        walker)  ;;
        display) extra=(KF3_DEV_EXTRA=display=on) ;;
        negctl)  extra=(KF3_NEGCTL_SKIP_CAP_BRACKET=1) ;;
        *) echo "unknown config $cfg"; exit 2 ;;
    esac
    echo "=== config=$cfg $(date -Is)"
    env "${extra[@]}" QEMU_BIN="$wrap" KF_ARMS=--timer \
        bash "$SRC/scripts/fastguest/run_fast_guest.sh" "${TAG}_${cfg}" 180 2>&1 | grep -aE 'FAST_VERDICT|qemu rc=' | head -3
    qlog="$BENCH/fast_${TAG}_${cfg}_qemu.log"
    echo "--- qemu process: $(grep -ac . "$qlog" 2>/dev/null || echo 0) log lines in $qlog"
    grep -a 'kf-cuda: cuda thread posture\|CUDA THREAD REFUSED' "$qlog" | sed 's/^/    /'
    grep -a 'kf-host: channel birth\|PRIVILEGED CHANNEL REFUSED\|CHANNEL BIRTH REFUSED\|CHANNEL CLASS REFUSED' "$qlog" | sed 's/^/    /'
    if [ ! -s "$log" ]; then
        echo "CHANOBS_SUMMARY config=$cfg observed=0 — NOTHING OBSERVED (the observer did not load, or no channel was allocated): not a measurement"
        fails=$((fails+1)); continue
    fi
    sed 's/^/    /' "$log"
    # kf-host mints its handles from 0xcafe0001; every other channel in the process is libcuda's.
    awk -v cfg="$cfg" '
        { h=""; v=""; for (i=1;i<=NF;i++) { if ($i ~ /^h=/) h=substr($i,3); if ($i ~ /^PRIVILEGED_CHANNEL=/) v=substr($i,20) } }
        { who = (h ~ /^0xcafe/) ? "kfhost" : "libcuda"; n[who]++; c[who "_" v]++ }
        END { printf "CHANOBS_SUMMARY config=%s libcuda_channels=%d libcuda_user=%d libcuda_admin=%d libcuda_unmeasured=%d kfhost_channels=%d kfhost_user=%d kfhost_admin=%d kfhost_unmeasured=%d\n",
                cfg, n["libcuda"], c["libcuda_0"], c["libcuda_1"], c["libcuda_?"], n["kfhost"], c["kfhost_0"], c["kfhost_1"], c["kfhost_?"] }' "$log"
done
echo "CHANOBS_CONFIGS_WITHOUT_OBSERVATIONS=$fails"
[ "$fails" -eq 0 ]
