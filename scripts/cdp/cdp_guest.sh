#!/usr/bin/env bash
# cdp_guest.sh BIN TAG "RUNS" — ONE FRESH kf3 fat-guest boot (the app-matrix image) running the
# CDP probe per RUNS entry (`cdp_hook.sh`). A poisoned CUDA context answers 999 to every later
# process in the same boot, so put the run that may hang LAST.
#   BIN : a kf3 QEMU (kf3-bins/<rev>/qemu-system-x86_64)
#   RUNS: space-separated `mode:sched[:t]` (t = nvdiff ioctl trace) or `qs` (cdpSimpleQuicksort)
# Results: /workspace/apps/results/cdpp/<TAG>/ — per run: <name>.log, .kf3.log, .guest_dmesg.log,
# .jsonl; per boot: boot.*.log, qemu.log.zst, host_kernel.log (journalctl since start: the host
# dmesg ring on a long-lived box is full, so boot_capture.sh's line-count watermark reads 0 lines).
# Writes CDPG_START … CDPG_EXIT rc= lines; a file without the EXIT line is a killed job.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
BIN=${1:?kf3 binary}; TAG=${2:?tag}; RUNS=${3:-4:auto}
OUT=/workspace/apps/results/cdpp/$TAG; mkdir -p "$OUT"
echo "CDPG_START $(date -Is) bin=$BIN tag=$TAG runs=[$RUNS] scripts=$(git -C "$REPO" rev-parse --short=8 HEAD)"
[ -x "$BIN" ] || { echo "⊘ no kf3 binary at $BIN"; echo "CDPG_EXIT rc=2 $(date -Is)"; exit 2; }
bash "$HERE/cdp_build.sh" "$OUT/bin" || { echo "CDPG_EXIT rc=3 $(date -Is)"; exit 3; }
busy(){ pgrep -x qemu-system-x86 >/dev/null || pgrep -x cargo >/dev/null || pgrep -x rustc >/dev/null; }
while busy; do sleep 10; done
T0=$(date +%s)
exec 9>"${KF_LOCK:-/tmp/kayfabe-fastguest.lock}"; flock 9
env KF_DEVICE=kf3 QEMU_BIN="$BIN" KF_GUEST_IMG=${KF_GUEST_IMG:-/workspace/bench/guest_apps.qcow2} \
    NVKVM_RAM_MB=${NVKVM_RAM_MB:-16384} KF_SMP=${KF_SMP:-6} GQ_TIMEOUT=${GQ_TIMEOUT:-600} \
    CDP_OUT="$OUT" CDP_RUNS="$RUNS" CDP_BINDIR="$OUT/bin" POST_CAPTURE_HOOK="$HERE/cdp_hook.sh" \
    bash "$REPO/scripts/bench/boot_capture.sh" "cdpp_$TAG" > "$OUT/boot.driver.log" 2>&1
rc=$?
flock -u 9; exec 9>&-
for x in probe dmesg dmesg_after serial; do cp -f "/workspace/bench/run_cdpp_${TAG}_$x.log" "$OUT/boot.$x.log" 2>/dev/null; done
zstd -q -f "/workspace/bench/run_cdpp_${TAG}_qemu.log" -o "$OUT/qemu.log.zst" 2>/dev/null
journalctl -k --since=@"$T0" --no-pager -o short-iso > "$OUT/host_kernel.log" 2>&1
echo "boot_capture rc=$rc; kf3 rev stamp: $(grep -ao 'kf3-bin-rev:[0-9a-f-]*' "/workspace/bench/run_cdpp_${TAG}_qemu.log" "$OUT/boot.driver.log" 2>/dev/null | head -1)"
echo "host kernel log since start: $(wc -l < "$OUT/host_kernel.log") lines, $(grep -c 'Xid' "$OUT/host_kernel.log") Xid"
grep -ah '^CDPP\|^rc=\|^=== CDPH\|^QS ' "$OUT"/*.log 2>/dev/null | head -80
echo "CDPG_EXIT rc=$rc $(date -Is)"
