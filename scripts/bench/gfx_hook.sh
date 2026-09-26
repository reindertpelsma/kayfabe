#!/usr/bin/env bash
# ★ POST_CAPTURE_HOOK for the headless-graphics lane (fat guest, KF_DEVICE=kf3).
#   KF_DEVICE=kf3 POST_CAPTURE_HOOK=scripts/bench/gfx_hook.sh scripts/bench/boot_capture.sh <tag>
# Ships scripts/bench/gfx/ into the guest, builds it THERE (same sources as the host baseline),
# runs gfx_steps.sh, and grades the render hashes against the bare-metal baseline written by
#   scripts/bench/gfx/gfx_steps.sh host <dir> > $BENCH/gfx_host.out
# Graded lines (column 0): GFX_S1..S5 from the guest, then GFX_CMP_<key>=MATCH|DIFF|NOBASE.
set -uo pipefail
TAG=${1:?tag}
HERE="$(cd "$(dirname "$0")" && pwd)"
BENCH=${BENCH_DIR:-/workspace/bench}
GSSH="$HERE/gssh_nv"
OUT=$BENCH/run_${TAG}_gfx.out
BASE=${GFX_BASELINE:-$BENCH/gfx_host.out}
echo "GFX_HOOK_START $(date -Is)"
tar -C "$HERE" -cf - gfx | timeout 60 "$GSSH" 'rm -rf ~/gfx && tar -xf - -C ~ && echo SHIPPED' || echo "GFX_HOOK_SHIP_FAILED"
# The guest step runner has its own per-step timeouts; this outer one only bounds a wedge.
timeout "${GFX_HOOK_TIMEOUT:-900}" "$GSSH" 'bash ~/gfx/gfx_steps.sh guest ~/gfxbin' > "$OUT" 2>&1
echo "GFX_GUEST_RC=$?"
grep -q '^GFX_STEPS_END' "$OUT" || echo "⊘ GFX_STEPS_END missing — the guest run did not finish; every NOTRUN/absent step is UNMEASURED"
sed 's/^/  guest| /' "$OUT"
grep -E '^GFX_S[1-5]=' "$OUT"
for k in VKC_HASH VKR_HASH_A VKR_HASH_Z VKR_HASH_B EGLR_HASH_A EGLR_HASH_Z EGLR_HASH_B GLXR_HASH_A GLXR_HASH_Z GLXR_HASH_B; do
    g=$(grep -a "^$k=" "$OUT" | tail -1 | cut -d= -f2)
    h=$(grep -a "^$k=" "$BASE" 2>/dev/null | tail -1 | cut -d= -f2)
    if [ -z "$h" ]; then echo "GFX_CMP_$k=NOBASE guest=${g:-absent}"
    elif [ -z "$g" ]; then echo "GFX_CMP_$k=ABSENT host=$h"
    elif [ "$g" = "$h" ]; then echo "GFX_CMP_$k=MATCH $g"
    else echo "GFX_CMP_$k=DIFF guest=$g host=$h"; fi
done
# the images, for eyes
timeout 30 "$GSSH" 'cat /tmp/vkr_guest_A.ppm' > "$BENCH/run_${TAG}_vkr_A.ppm" 2>/dev/null
timeout 30 "$GSSH" 'cat /tmp/vkr_guest_B.ppm' > "$BENCH/run_${TAG}_vkr_B.ppm" 2>/dev/null
echo "GFX_HOOK_END $(date -Is)"
