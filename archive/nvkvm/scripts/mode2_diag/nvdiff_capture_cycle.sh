#!/usr/bin/env bash
# nvdiff_capture_cycle.sh — ONE full guest-half capture cycle on the kayfabe bench.
#
# Boot -> wait -> stage -> capture both stages -> pull -> persist the evidence.
#
# ★ It exists because the first firing was driven by hand and the second must be
# re-derivable. Every trap the first run hit is encoded here rather than remembered:
#
#   ⊘ `pgrep -x qemu-system-x86_64` CAN NEVER MATCH -- /proc/PID/comm truncates to 15
#     chars. Any "nothing is running" check built on it passes VACUOUSLY. Use
#     `qemu-system-x86` and corroborate with the tap.
#   ⊘ The guest needs ~20-25 s to a login prompt and `-serial file:` output LAGS. A slow
#     boot is not a crash; this polls the guest over the tap instead of the serial log.
#   ⊘ `nvktap0` does not survive a host reboot and QEMU requires it to pre-exist.
#   ⊘ The SERIAL LOG IS NOT WHERE THE DRIVER'S OUTPUT IS: the driver is loaded over ssh
#     after boot, so its dmesg goes to whoever ran the command. The guest runner persists
#     `<stage>_dmesg.log` and this asserts it is non-empty AND contains NVRM -- a harness
#     that writes an empty file and exits 0 is worse than none.
#   ⊘ NVDIFF_MAXBUF=65536, always: the host reference was captured there and the default
#     8192 truncates 27 records, every one of which reads as a divergence.
#
#   usage: ssh vh2 'NVD_TAG=nvd2 bash /workspace/nvdiff_stage/nvdiff_capture_cycle.sh'
set -u
TAG=${NVD_TAG:?set NVD_TAG, e.g. nvd2}
S=${NVD_STAGEDIR:-/workspace/nvdiff_stage}
BENCH=${BENCH:-/workspace/bench}
G=${GSSH:-$BENCH/gssh_nv}
SRC=${KAYFABE:-/workspace/kayfabe}
RUNS=${NVD_RUNS:-2}
OUT=$BENCH/nvdiff_guest_$TAG

say() { echo "== $*"; }

say "provenance"
echo "  BUILD_REV = $(cat $BENCH/BUILD_REV.txt 2>/dev/null)"
echo "  qemu      = $(ls -l $BENCH/qemu-build/qemu-system-x86_64 | awk '{print $5, $6, $7, $8}')"
echo "  host gpu  = $(nvidia-smi --query-gpu=gpu_name,driver_version --format=csv,noheader 2>/dev/null)"

say "the box must be free (⊘ -x qemu-system-x86, NOT ...x86_64)"
pkill -9 -x qemu-system-x86 2>/dev/null && sleep 4
pgrep -x qemu-system-x86 && { echo "FATAL: a QEMU survived the kill"; exit 2; }
ip link show nvktap0 >/dev/null 2>&1 || { echo "FATAL: nvktap0 absent (host reboot?)"; exit 2; }

say "boot $TAG"
rm -rf "$OUT"; mkdir -p "$OUT"
cd "$SRC" || exit 2
nohup bash scripts/bench/boot_nvkvm.sh "$TAG" > "$BENCH/${TAG}_boot.log" 2>&1 &
for i in $(seq 1 40); do
    sleep 5
    $G 'echo GUEST_UP' 2>/dev/null | grep -q GUEST_UP && { say "guest answered after $((i*5))s"; break; }
done
$G 'echo GUEST_UP; uname -r' || { echo "FATAL: guest never answered over the tap"; exit 3; }

say "the device line the boot actually ran with"
grep -a "boot_nvkvm: -device" "$BENCH/${TAG}_boot.log" || echo "  (no device line logged)"
grep -a "presenting" "$BENCH/run_${TAG}_qemu.log" | head -1

say "push the instrument (a fresh overlay loses it every boot)"
for f in nvdiff_shim.c nvd_prog.c nvd_capture.sh uvm_sizes.h; do
    $G "cat > /tmp/$f" < "$S/$f" || { echo "FATAL: push $f"; exit 4; }
done
$G 'chmod +x /tmp/nvd_capture.sh; md5sum /tmp/nvdiff_shim.c /tmp/nvd_prog.c /tmp/uvm_sizes.h'

rc=0
for stage in dev ctx; do
    say "capture stage=$stage runs=$RUNS"
    $G "NVD_STAGE=$stage NVD_RUNS=$RUNS NVDIFF_MAXBUF=65536 bash -s" < "$S/nvdiff_run_guest.sh"
    [ $? -ne 0 ] && rc=1
    $G "cd /tmp/nvd_guest && tar cf - ${stage}_r*.jsonl ${stage}_r*.stdout env_${stage}.txt ${stage}_dmesg.log 2>/dev/null" \
        | tar xf - -C "$OUT"
done

say "ASSERT the evidence, rather than trusting the files exist"
for stage in dev ctx; do
    for f in "$OUT/${stage}"_r*.jsonl; do
        [ -s "$f" ] || { echo "  FATAL: $f EMPTY"; rc=1; continue; }
        echo "  $(wc -l < "$f") records  $(basename "$f")"
    done
    d="$OUT/${stage}_dmesg.log"
    if [ ! -s "$d" ]; then echo "  FATAL: $d EMPTY -- the driver's output was not captured"; rc=1
    elif ! grep -qi NVRM "$d"; then echo "  FATAL: $d has no NVRM lines"; rc=1
    else echo "  $(grep -ci NVRM "$d") NVRM lines  $(basename "$d")"; fi
done

say "the device's own report for this boot"
cp "$BENCH/run_${TAG}_qemu.log" "$OUT/run_${TAG}_qemu.log" 2>/dev/null
grep -a "unserviced fn 76 cmd" "$OUT/run_${TAG}_qemu.log" | wc -l | sed 's/^/  unserviced ids: /'

say "shut the guest down and free the box"
$G 'sudo poweroff' >/dev/null 2>&1
sleep 12
pkill -9 -x qemu-system-x86 2>/dev/null
say "cycle rc=$rc  ->  $OUT"
exit $rc
