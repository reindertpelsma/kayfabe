#!/usr/bin/env bash
# ★★★ THE HEADLESS-GRAPHICS ARMS — `V3_HEADLESS_GRAPHICS.md` §6. One command, one verdict.
#
#   usage (on the box, in the kayfabe checkout):  bash scripts/bench/gfx_suite.sh [tag]
#
# 1. BARE METAL FIRST: `gfx/gfx_steps.sh host` on this box's GPU → $BENCH/gfx_host.out (the hashes
#    every guest result is graded against — same GPU, same 580.159.04 userspace, same sources).
# 2. THE FAT GUEST on the kf3 device built from THIS revision (`boot_capture.sh`, KF_DEVICE=kf3),
#    `gfx_hook.sh` running the same steps inside it.
# 3. Verdict: GFX_SUITE_VERDICT=PASS iff the guest's GFX_S1..S5 are all PASS **and** every render
#    hash (Vulkan compute; Vulkan / EGL / GLX colour, depth and sampled pass) MATCHES bare metal.
#    ⊘ The host's own S1 is not graded (its nvidia-drm is the box's, not ours).
# Preconditions: provision_guest_gfx.sh ran (guest) and `provision_guest_gfx.sh host` (host).
# A start marker and an exit line, so a killed run is not read as a running one.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; REPO="$(cd "$HERE/../.." && pwd)"
BENCH=${BENCH_DIR:-/workspace/bench}; TAG=${1:-gfxsuite}
OUT=$BENCH/${TAG}_gfxsuite.out
REV=$(git -C "$REPO" rev-parse --short=8 HEAD)
echo "GFX_SUITE_STARTED=$(date -Is) rev=$REV" > "$OUT"
pgrep -x qemu-system-x86 >/dev/null && { echo "GFX_SUITE_VERDICT=NOTRUN (a QEMU is running — the bench is serial)" | tee -a "$OUT"; exit 2; }
[ -f "$BENCH/gfx_lane.receipt" ] && grep -q '^GFX_LANE_PROVISIONED=yes' "$BENCH/gfx_lane.receipt" \
  || { echo "GFX_SUITE_VERDICT=NOTRUN (guest not provisioned: run provision_guest_gfx.sh)" | tee -a "$OUT"; exit 2; }
bash "$HERE/gfx/gfx_steps.sh" host /root/gfxbin > "$BENCH/gfx_host.out" 2>&1
grep -E '^GFX_S[2-5]=' "$BENCH/gfx_host.out" | sed 's/^/host /' >> "$OUT"
if grep -qE '^GFX_S[2-5]=(FAIL|NOTRUN)' "$BENCH/gfx_host.out"; then
    echo "GFX_SUITE_VERDICT=NOTRUN (bare metal itself fails a step: the workload, not kayfabe)" | tee -a "$OUT"; exit 3
fi
KF_DEVICE=kf3 POST_CAPTURE_HOOK="$HERE/gfx_hook.sh" bash "$HERE/boot_capture.sh" "$TAG" > "$BENCH/${TAG}.console" 2>&1
P=$BENCH/run_${TAG}_probe.log
grep -E '^GFX_S[1-5]=|^GFX_CMP_' "$P" >> "$OUT"
steps=$(grep -cE '^GFX_S[1-5]=PASS$' "$P"); cmps=$(grep -c '^GFX_CMP_.*=MATCH' "$P"); ncmp=$(grep -c '^GFX_CMP_' "$P")
if [ "$steps" -eq 5 ] && [ "$ncmp" -ge 10 ] && [ "$cmps" -eq "$ncmp" ]; then v=PASS; else v=FAIL; fi
echo "GFX_SUITE_STEPS=$steps/5 GFX_SUITE_HASHES=$cmps/$ncmp" >> "$OUT"
echo "GFX_SUITE_VERDICT=$v" >> "$OUT"
echo "GFX_SUITE_EXIT=$(date -Is)" >> "$OUT"
cat "$OUT"
[ "$v" = PASS ]
